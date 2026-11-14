//! Signing in to the platform: PKCE, the callback, and a whole sign-in against
//! a fake platform on the loopback. The keychain is not touched here — it would
//! be the real one of whoever runs the tests.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use limen_core::account::*;

#[test]
fn base64url_is_rfc4648_without_padding() {
    for (raw, want) in [
        (&b""[..], ""),
        (b"f", "Zg"),
        (b"fo", "Zm8"),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg"),
        (b"fooba", "Zm9vYmE"),
        (b"foobar", "Zm9vYmFy"),
        (&[0xfb, 0xff], "-_8"),
    ] {
        assert_eq!(base64url(raw), want, "{raw:?}");
    }
}

/// RFC 7636, appendix B.
#[test]
fn the_code_challenge_is_s256_of_the_verifier() {
    assert_eq!(
        code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

fn platform(api_url: &str) -> Platform {
    Platform { api_url: api_url.into(), client_id: "limen-test".into() }
}

fn query(url: &str) -> HashMap<String, String> {
    let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

#[test]
fn the_sign_in_address_asks_for_a_code_with_pkce() {
    let s = SignIn::start(&platform("https://auth.example/api")).unwrap();
    let url = s.url().to_string();
    assert!(url.starts_with("https://auth.example/api/oauth/authorize?"), "{url}");
    let q = query(&url);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "limen-test");
    assert_eq!(q["scope"], SCOPES);
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["code_challenge"].len(), 43, "S256 of 32 bytes, base64url");
    assert!(q["state"].len() >= 22, "a state with 128 bits in it");
    let redirect = &q["redirect_uri"];
    assert!(redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/callback"), "{redirect}");
    let port: u16 = redirect["http://127.0.0.1:".len()..redirect.len() - "/callback".len()].parse().unwrap();
    assert_ne!(port, 0, "a real port, bound before the browser is sent");

    let other = SignIn::start(&platform("https://auth.example/api")).unwrap();
    let o = query(other.url());
    assert_ne!(o["state"], q["state"], "every sign-in has its own state");
    assert_ne!(o["code_challenge"], q["code_challenge"], "and its own verifier");
}

#[test]
fn the_callback_is_read_as_the_platform_meant_it() {
    let s = "st4te";
    assert_eq!(parse_callback("GET /callback?code=abc&state=st4te HTTP/1.1", s), Callback::Code("abc".into()));
    assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1", s), Callback::NotOurs);
    assert_eq!(parse_callback("POST /callback?code=abc&state=st4te HTTP/1.1", s), Callback::NotOurs);
    assert_eq!(parse_callback("", s), Callback::NotOurs);
    assert!(matches!(
        parse_callback("GET /callback?code=abc&state=other HTTP/1.1", s),
        Callback::Refused(w) if w.contains("state")
    ));
    assert!(matches!(
        parse_callback("GET /callback?code=abc HTTP/1.1", s),
        Callback::Refused(_)
    ), "no state at all is refused too");
    assert_eq!(
        parse_callback("GET /callback?error=access_denied&error_description=User+said+no&state=st4te HTTP/1.1", s),
        Callback::Refused("User said no".into())
    );
    assert!(matches!(parse_callback("GET /callback?state=st4te HTTP/1.1", s), Callback::Refused(_)));
}

// ---- a fake platform ------------------------------------------------------- //

/// What the fake platform saw, and decides with.
#[derive(Default)]
struct Seen {
    token_forms: Vec<HashMap<String, String>>,
    bearers: Vec<String>,
    /// Refresh tokens already used: rotated away, as the real platform does.
    spent: Vec<String>,
}

/// A platform on the loopback: `/api/oauth/token` and `/api/me`. It checks the
/// PKCE verifier against `challenge` as the real one does.
fn fake_platform(challenge: Arc<Mutex<String>>, seen: Arc<Mutex<Seen>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/api", listener.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &challenge, &seen);
        }
    });
    base
}

fn serve(mut s: TcpStream, challenge: &Mutex<String>, seen: &Mutex<Seen>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head, mut body) = loop {
        let n = s.read(&mut chunk).unwrap();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break (String::from_utf8_lossy(&buf[..i]).to_string(), buf[i + 4..].to_vec());
        }
        if n == 0 {
            return;
        }
    };
    let len: usize = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
        .unwrap_or(0);
    while body.len() < len {
        let n = s.read(&mut chunk).unwrap();
        body.extend_from_slice(&chunk[..n]);
    }
    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
    let bearer = head
        .lines()
        .find_map(|l| l.strip_prefix("Authorization: Bearer ").map(str::to_string))
        .unwrap_or_default();
    let (status, json) = if path == "/api/oauth/token" {
        let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
        seen.lock().unwrap().token_forms.push(form.clone());
        match form.get("grant_type").map(String::as_str) {
            Some("authorization_code")
                if form.get("code").map(String::as_str) == Some("the-code")
                    && code_challenge(form.get("code_verifier").map(String::as_str).unwrap_or(""))
                        == *challenge.lock().unwrap() =>
            {
                ("200 OK", r#"{"access_token":"at-1","token_type":"Bearer","expires_in":900,"refresh_token":"rt-1"}"#)
            }
            Some("refresh_token")
                if form.get("refresh_token").map(String::as_str) == Some("rt-1")
                    && !seen.lock().unwrap().spent.contains(&"rt-1".to_string()) =>
            {
                seen.lock().unwrap().spent.push("rt-1".into());
                ("200 OK", r#"{"access_token":"at-2","token_type":"Bearer","expires_in":900,"refresh_token":"rt-2"}"#)
            }
            Some("refresh_token") => (
                "400 Bad Request",
                r#"{"error":"invalid_grant","error_description":"refresh token was already used"}"#,
            ),
            _ => ("400 Bad Request", r#"{"error":"invalid_grant","error_description":"PKCE verification failed"}"#),
        }
    } else if path == "/api/me" {
        seen.lock().unwrap().bearers.push(bearer.clone());
        if bearer.starts_with("at-") {
            ("200 OK", r#"{"id":"u-1","username":"jdoe","email":"j@x","first_name":"Jane","last_name":"Doe","roles":["user"]}"#)
        } else {
            ("401 Unauthorized", r#"{"error":"unauthorized","error_description":"No access token was provided"}"#)
        }
    } else {
        ("404 Not Found", "{}")
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    let _ = s.write_all(reply.as_bytes());
}

/// The browser coming back to the loopback port: what the page said.
fn browser_visits(url: &str) -> String {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut s = TcpStream::connect(host).unwrap();
    write!(s, "GET /{path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
    let mut page = String::new();
    s.read_to_string(&mut page).unwrap();
    page
}

#[test]
fn a_whole_sign_in_against_a_fake_platform() {
    let challenge = Arc::new(Mutex::new(String::new()));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let p = platform(&fake_platform(challenge.clone(), seen.clone()));

    let s = SignIn::start(&p).unwrap();
    let q = query(s.url());
    *challenge.lock().unwrap() = q["code_challenge"].clone();
    let redirect = q["redirect_uri"].clone();
    let state = q["state"].clone();

    let cancel = AtomicBool::new(false);
    let waiting = std::thread::spawn(move || s.finish(&cancel, Duration::from_secs(20)));
    // A favicon first, as a browser asks for: not the answer, and not fatal.
    let favicon = browser_visits(&redirect.replace("/callback", "/favicon.ico"));
    assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
    let page = browser_visits(&format!("{redirect}?code=the-code&state={state}"));
    assert!(page.starts_with("HTTP/1.1 200") && page.contains("Signed in"), "{page}");

    let (tokens, who) = waiting.join().unwrap().expect("signed in");
    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
    assert!(!tokens.expiring(), "15 minutes left");
    assert_eq!(who.display_name(), "Jane Doe");
    assert_eq!(who.username, "jdoe");

    let seen = seen.lock().unwrap();
    let form = &seen.token_forms[0];
    assert_eq!(form["redirect_uri"], redirect, "the same redirect, port included, both times");
    assert_eq!(form["client_id"], "limen-test");
    assert!(!form.contains_key("client_secret"), "a public client sends no secret");
    assert_eq!(seen.bearers, ["at-1"]);
}

#[test]
fn a_refresh_rotates_and_a_spent_token_is_the_platforms_no() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let p = platform(&fake_platform(Arc::new(Mutex::new(String::new())), seen));
    let t = refresh(&p, "rt-1").unwrap();
    assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref()), ("at-2", Some("rt-2")));
    let err = refresh(&p, "rt-1").unwrap_err();
    let said = err.downcast_ref::<PlatformSaid>().expect("the platform's own answer");
    assert_eq!(said.status, 400);
    assert_eq!(said.said, "refresh token was already used");
    assert_eq!(me(&p, "at-2").unwrap().id, "u-1");
    assert_eq!(me(&p, "nope").unwrap_err().downcast_ref::<PlatformSaid>().unwrap().status, 401);
}

#[test]
fn a_refused_sign_in_says_why_on_the_page_and_to_limen() {
    let p = platform("http://127.0.0.1:9/api");
    let s = SignIn::start(&p).unwrap();
    let q = query(s.url());
    let redirect = q["redirect_uri"].clone();
    let state = q["state"].clone();
    let cancel = AtomicBool::new(false);
    let waiting = std::thread::spawn(move || s.finish(&cancel, Duration::from_secs(20)));
    let page = browser_visits(&format!("{redirect}?error=access_denied&error_description=Denied&state={state}"));
    assert!(page.contains("Not signed in") && page.contains("Denied"), "{page}");
    let err = waiting.join().unwrap().unwrap_err().to_string();
    assert!(err.contains("Denied"), "{err}");
}

/// The port it held closes with it — `finish` takes the sign-in by value — but
/// that is not checked here: other sign-ins in this binary bind loopback ports
/// meanwhile, and the OS may hand one of them the port just freed.
#[test]
fn a_sign_in_can_be_cancelled() {
    let p = platform("http://127.0.0.1:9/api");
    let s = SignIn::start(&p).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    let waiting = std::thread::spawn(move || s.finish(&c, Duration::from_secs(20)));
    std::thread::sleep(Duration::from_millis(150));
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let err = waiting.join().unwrap().unwrap_err().to_string();
    assert!(err.contains("cancelled"), "{err}");
}

#[test]
fn the_platform_comes_from_the_settings_and_needs_a_client() {
    let mut cfg = limen_core::Config::default();
    // The environment wins over the settings; this test owns neither variable.
    if std::env::var("LIMEN_PLATFORM_CLIENT_ID").is_ok() || std::env::var("LIMEN_PLATFORM_URL").is_ok() {
        return;
    }
    // No client id set: this build's platform's own, if it has one built in.
    let built_in = limen_core::environment::current().client_id;
    assert_eq!(Platform::from_config(&cfg).map(|p| p.client_id), built_in.map(str::to_string));
    cfg.platform_client_id = Some(" abc ".into());
    let p = Platform::from_config(&cfg).unwrap();
    assert_eq!(
        (p.api_url.as_str(), p.client_id.as_str()),
        (limen_core::environment::current().auth_api, "abc"),
        "this build's platform"
    );
    cfg.platform_url = Some("https://auth.example/api/".into());
    assert_eq!(Platform::from_config(&cfg).unwrap().api_url, "https://auth.example/api");

    // Production's id is built in, and only for production.
    cfg.platform_client_id = None;
    cfg.platform_url = Some("https://auth.barracuda-team.tech/api/".into());
    assert_eq!(Platform::from_config(&cfg).unwrap().client_id, "szkyo06i69uinrmr");
    cfg.platform_url = Some("https://auth.example/api".into());
    assert_eq!(Platform::from_config(&cfg), None, "another platform needs its own id");
    cfg.platform_url = Some(limen_core::environment::LOCAL.auth_api.into());
    assert_eq!(Platform::from_config(&cfg), None, "a local platform's id is the developer's");
}

#[test]
fn a_debug_build_talks_to_local_and_a_release_build_to_production() {
    use limen_core::environment::{self, for_build, LOCAL, PRODUCTION};
    assert_eq!(for_build(true), &LOCAL);
    assert_eq!(for_build(false), &PRODUCTION);
    assert_eq!(environment::current(), for_build(cfg!(debug_assertions)));
    assert_eq!(PRODUCTION.auth_api, "https://auth.barracuda-team.tech/api");
    // Each environment's services are its own: no mixing hosts.
    for e in [environment::LOCAL, environment::DEV, environment::PRODUCTION] {
        let host = |u: &str| url::Url::parse(u).unwrap().host_str().unwrap().to_string();
        let suffix = host(e.auth_api).trim_start_matches("auth").to_string();
        assert!(host(e.avatar_api).ends_with(&suffix) && host(e.asset_base).ends_with(&suffix), "{}", e.name);
        assert!(e.asset_base.ends_with('/'), "a files base is a directory: {}", e.name);
    }
}
