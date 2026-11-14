//! Signing in to the Barracuda platform.
//!
//! OAuth 2.0 authorization code with PKCE, as a native app does it (RFC 8252):
//! Limen is a *public* client — it is installed on the user's machine and can
//! keep no secret — so the platform requires PKCE with S256 of it, and the code
//! comes back to a loopback address, `http://127.0.0.1:<port>/callback`, on a
//! port that is whatever is free at that moment.
//!
//! 1. [`SignIn::start`] binds the port and builds the sign-in address, with a
//!    fresh PKCE verifier and `state`.
//! 2. The caller opens that address in the user's browser.
//! 3. [`SignIn::finish`] waits for the one callback, checks `state`, exchanges
//!    the code for tokens and reads who signed in from `/me`.
//!
//! The refresh token is kept in the OS keychain (Secret Service, Windows
//! Credential Manager, macOS Keychain) and nowhere else; the access token only
//! in memory. The platform rotates a public client's refresh token on every use,
//! so each refresh stores the new one, and refreshes never run two at a time.
//!
//! HTTP goes through `curl`, as everywhere in Limen — with the token handed to
//! it on stdin, so it never appears in a process list.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Config;


/// What Limen asks for: who the user is.
pub const SCOPES: &str = "openid profile email";

/// The path the platform redirects back to, on the loopback port.
pub const CALLBACK_PATH: &str = "/callback";

/// The keychain's name for Limen's entries; the user part is the platform URL,
/// so two platforms (local and production) keep separate sessions.
const KEYRING_SERVICE: &str = "limen";

/// Which platform to sign in to, and as which client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    /// The API root: `https://auth…/api`, under which `/oauth/…` and `/me` live.
    pub api_url: String,
    /// Limen's public client id, as registered on the platform.
    pub client_id: String,
}

impl Platform {
    /// From the settings, with `LIMEN_PLATFORM_URL` and
    /// `LIMEN_PLATFORM_CLIENT_ID` in the environment taking precedence. `None`
    /// when no client id is set anywhere: there is nothing to sign in as.
    pub fn from_config(cfg: &Config) -> Option<Self> {
        let env = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let api_url = env("LIMEN_PLATFORM_URL")
            .or_else(|| cfg.platform_url.clone().filter(|v| !v.trim().is_empty()))
            .unwrap_or_else(|| crate::environment::current().auth_api.to_string());
        let client_id = env("LIMEN_PLATFORM_CLIENT_ID")
            .or_else(|| cfg.platform_client_id.clone().filter(|v| !v.trim().is_empty()))?;
        Some(Self { api_url: api_url.trim().trim_end_matches('/').to_string(), client_id: client_id.trim().to_string() })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.api_url)
    }

    /// The user's own settings page in auth-ui: the API's origin (the API is
    /// served under `/api` beside the UI) and `/settings`.
    pub fn account_settings_url(&self) -> String {
        format!("{}/settings", self.api_url.strip_suffix("/api").unwrap_or(&self.api_url))
    }
}

/// Who signed in, as `/me` says. Not a secret: kept in the settings so the
/// account shows at once on the next start, before the session is checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
}

impl Account {
    /// "First Last", or the username when there is no name.
    pub fn display_name(&self) -> String {
        let full = format!("{} {}", self.first_name.trim(), self.last_name.trim());
        let full = full.trim();
        if full.is_empty() {
            self.username.clone()
        } else {
            full.to_string()
        }
    }
}

/// The tokens of a session.
#[derive(Debug, Clone)]
pub struct Tokens {
    pub access_token: String,
    /// When the access token stops working.
    pub expires_at: SystemTime,
    /// Rotated on every refresh; `None` if the platform gave none.
    pub refresh_token: Option<String>,
}

impl Tokens {
    /// Whether the access token has less than a minute left.
    pub fn expiring(&self) -> bool {
        self.expires_at
            .duration_since(SystemTime::now())
            .map(|left| left < Duration::from_secs(60))
            .unwrap_or(true)
    }
}

// --------------------------------------------------------------------------- //
// PKCE and the sign-in address.
// --------------------------------------------------------------------------- //

/// Unpadded base64url (RFC 4648 §5), as PKCE and `state` use.
pub fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// `n` bytes from the operating system's random source, base64url-encoded.
fn random(n: usize) -> Result<String> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).map_err(|e| anyhow!("no random source: {e}"))?;
    Ok(base64url(&buf))
}

/// The S256 code challenge for a verifier (RFC 7636 §4.2).
pub fn code_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// The address the browser is sent to.
pub fn authorize_url(p: &Platform, redirect_uri: &str, state: &str, challenge: &str) -> String {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", &p.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPES)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .finish();
    format!("{}?{query}", p.endpoint("/oauth/authorize"))
}

// --------------------------------------------------------------------------- //
// The callback.
// --------------------------------------------------------------------------- //

/// What a request to the loopback port turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    /// The platform's answer: the code.
    Code(String),
    /// The platform's answer: no — with why, in its words.
    Refused(String),
    /// Something else asking (a favicon, a stray request): not the answer.
    NotOurs,
}

/// Read a request's first line — `GET /callback?code=…&state=… HTTP/1.1` — as
/// the answer to the sign-in that sent `state`.
///
/// A callback whose `state` is not this sign-in's is refused, not ignored: it
/// is either a stale tab or someone trying to sign the user in as themselves.
pub fn parse_callback(request_line: &str, state: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::NotOurs;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Callback::NotOurs;
    }
    let mut code = None;
    let mut got_state = None;
    let mut error = None;
    let mut error_description = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => got_state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => error_description = Some(v.into_owned()),
            _ => {}
        }
    }
    if got_state.as_deref() != Some(state) {
        return Callback::Refused("the answer was not for this sign-in (state did not match)".into());
    }
    if let Some(e) = error {
        return Callback::Refused(error_description.unwrap_or(e));
    }
    match code {
        Some(c) if !c.is_empty() => Callback::Code(c),
        _ => Callback::Refused("the platform sent no code".into()),
    }
}

/// The page the browser shows once the answer has arrived.
fn page(ok: bool, message: &str) -> String {
    let (title, colour) = if ok { ("Signed in", "#3fb950") } else { ("Not signed in", "#f85149") };
    let escaped = message.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"background:#0d0d0d;color:#ddd;font-family:monospace;display:flex;\
         align-items:center;justify-content:center;height:100vh;margin:0\"><div>\
         <h2 style=\"color:{colour}\">{title}</h2><p>{escaped}</p></div></body></html>"
    )
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes());
    let _ = stream.flush();
    // Close our side first and let the browser finish: closing a socket that
    // still holds unread bytes sends a reset, which can reach the browser
    // before the page does — "connection reset" instead of "Signed in".
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut sink = [0u8; 1024];
    while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
}

/// The first line of a request. The whole head is read — up to the blank line
/// — so that nothing is left unread when the answer goes back; and with a
/// limit, as nothing from the network is read without one.
fn request_line(stream: &mut TcpStream) -> Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    Ok(text.lines().next().unwrap_or_default().to_string())
}

// --------------------------------------------------------------------------- //
// Talking to the platform.
// --------------------------------------------------------------------------- //

/// The platform said no: its HTTP status and its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSaid {
    pub status: u16,
    pub said: String,
}

impl std::fmt::Display for PlatformSaid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the platform answered {}: {}", self.status, self.said)
    }
}

impl std::error::Error for PlatformSaid {}

/// The platform's own words for a failure: `error_description`, else `error`.
fn platform_error(status: u16, body: &str) -> anyhow::Error {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let said = v["error_description"]
        .as_str()
        .or_else(|| v["error"].as_str())
        .or_else(|| v["message"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(200).collect());
    anyhow::Error::new(PlatformSaid { status, said })
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn tokens(p: &Platform, form: &[(&str, &str)]) -> Result<Tokens> {
    let (status, body) = crate::net::request(&p.endpoint("/oauth/token"), Some(form), None)?;
    if status != 200 {
        return Err(platform_error(status, &body));
    }
    let t: TokenAnswer = serde_json::from_str(&body).context("reading the platform's tokens")?;
    Ok(Tokens {
        access_token: t.access_token,
        expires_at: SystemTime::now() + Duration::from_secs(t.expires_in.max(1)),
        refresh_token: t.refresh_token.filter(|r| !r.is_empty()),
    })
}

/// Who the access token belongs to.
pub fn me(p: &Platform, access_token: &str) -> Result<Account> {
    let (status, body) = crate::net::request(&p.endpoint("/me"), None, Some(access_token))?;
    if status != 200 {
        return Err(platform_error(status, &body));
    }
    serde_json::from_str(&body).context("reading who signed in")
}

/// New tokens for a refresh token. The platform rotates it: the one passed in
/// stops working, and the answer carries its successor.
pub fn refresh(p: &Platform, refresh_token: &str) -> Result<Tokens> {
    tokens(
        p,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", &p.client_id)],
    )
}

// --------------------------------------------------------------------------- //
// Signing in.
// --------------------------------------------------------------------------- //

/// A sign-in under way: the loopback port is bound, the address is built.
pub struct SignIn {
    platform: Platform,
    listener: TcpListener,
    redirect_uri: String,
    state: String,
    verifier: String,
    url: String,
}

impl SignIn {
    /// Bind a free loopback port and build the address to send the browser to.
    pub fn start(p: &Platform) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).context("opening a local port for the sign-in")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let verifier = random(32)?;
        let state = random(16)?;
        let url = authorize_url(p, &redirect_uri, &state, &code_challenge(&verifier));
        Ok(Self { platform: p.clone(), listener, redirect_uri, state, verifier, url })
    }

    /// Where to send the browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Wait for the browser to come back, then trade the code for a session.
    /// Gives up when `cancel` is set or `timeout` passes.
    pub fn finish(self, cancel: &AtomicBool, timeout: Duration) -> Result<(Tokens, Account)> {
        let code = self.wait(cancel, timeout)?;
        let t = tokens(
            &self.platform,
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &self.platform.client_id),
                ("code_verifier", &self.verifier),
            ],
        )?;
        let who = me(&self.platform, &t.access_token)?;
        Ok((t, who))
    }

    fn wait(&self, cancel: &AtomicBool, timeout: Duration) -> Result<String> {
        self.listener.set_nonblocking(true)?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("the sign-in was cancelled");
            }
            if Instant::now() > deadline {
                bail!("the browser did not come back within {} minutes", timeout.as_secs() / 60);
            }
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false)?;
                    let line = request_line(&mut stream).unwrap_or_default();
                    match parse_callback(&line, &self.state) {
                        Callback::NotOurs => respond(&mut stream, "404 Not Found", ""),
                        Callback::Code(code) => {
                            respond(&mut stream, "200 OK", &page(true, "You can close this tab and go back to Limen."));
                            return Ok(code);
                        }
                        Callback::Refused(why) => {
                            respond(&mut stream, "200 OK", &page(false, &why));
                            bail!("the platform refused the sign-in: {why}");
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e).context("waiting for the browser"),
            }
        }
    }
}

// --------------------------------------------------------------------------- //
// Keeping the session.
// --------------------------------------------------------------------------- //

/// Refreshes never overlap: the platform rotates the refresh token, so two at
/// once would spend the same one twice and lose the session.
static REFRESHING: Mutex<()> = Mutex::new(());

fn entry(p: &Platform) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, &p.api_url).map_err(|e| anyhow!("the keychain is not available: {e}"))
}

/// Keep the refresh token in the keychain. `false` when there is no keychain to
/// keep it in — the session then lasts until Limen closes.
pub fn remember(p: &Platform, refresh_token: &str) -> bool {
    entry(p).and_then(|e| e.set_password(refresh_token).map_err(|e| anyhow!("{e}"))).is_ok()
}

/// The refresh token the keychain holds for this platform, if any.
pub fn remembered(p: &Platform) -> Option<String> {
    entry(p).ok()?.get_password().ok().filter(|t| !t.is_empty())
}

/// Forget the session: the keychain entry goes. The platform has no revoke
/// endpoint, so the refresh token simply stops being anywhere.
pub fn forget(p: &Platform) {
    if let Ok(e) = entry(p) {
        let _ = e.delete_credential();
    }
}

/// Swap a refresh token for a new session, and keep its successor. Whether it
/// could be kept is the `bool`.
pub fn renew(p: &Platform, refresh_token: &str) -> Result<(Tokens, bool)> {
    let _turn = REFRESHING.lock().unwrap_or_else(|e| e.into_inner());
    let t = refresh(p, refresh_token)?;
    let kept = match &t.refresh_token {
        Some(next) => remember(p, next),
        None => false,
    };
    Ok((t, kept))
}

/// The session the keychain holds, brought back to life: refreshed (and the
/// rotated token kept), and who it is for. `None` when nothing is remembered.
/// A refresh token the platform no longer accepts is forgotten.
pub fn restore(p: &Platform) -> Result<Option<(Tokens, Account)>> {
    let Some(refresh_token) = remembered(p) else {
        return Ok(None);
    };
    let (t, _) = match renew(p, &refresh_token) {
        Ok(v) => v,
        Err(e) => {
            // Refused outright (expired, revoked, already used): it will never
            // work again. A network failure is not that, and keeps it.
            if e.downcast_ref::<PlatformSaid>().is_some_and(|s| (400..500).contains(&s.status)) {
                forget(p);
            }
            return Err(e);
        }
    };
    let who = me(p, &t.access_token)?;
    Ok(Some((t, who)))
}
