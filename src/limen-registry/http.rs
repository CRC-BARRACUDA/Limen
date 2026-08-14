//! Every HTTP request this crate makes, in one place.
//!
//! Through `curl` rather than an HTTP+TLS stack of our own, and through here
//! rather than by each caller building its own command. That is not tidiness:
//! the token that raises GitHub's rate limit from 60 requests an hour to 5,000
//! — and that is the difference between reaching a private repository and not —
//! was attached by one of the four call sites and forgotten by the other three.
//! A request built anywhere else is a request that quietly goes out
//! unauthenticated.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::RwLock;

use anyhow::{bail, Context, Result};
use limen_proto::NoConsole;

/// An optional GitHub token applied to every request when set — raising the rate
/// limit from 60/hour (unauthenticated, per IP) to 5,000/hour, and making
/// private repositories reachable at all. Set from Developer mode; `None` is the
/// unauthenticated default.
static TOKEN: RwLock<Option<String>> = RwLock::new(None);

/// Set (or clear) the token. A blank token clears it.
pub fn set_token(token: Option<String>) {
    let cleaned = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    *TOKEN.write().unwrap() = cleaned;
}

/// The token in force, if any.
pub(crate) fn token() -> Option<String> {
    TOKEN.read().ok().and_then(|t| t.clone())
}

/// The headers every request carries: who we are, what we accept, and who we
/// are authenticated as.
pub(crate) fn with_headers(cmd: &mut Command, accept: Option<&str>, token: Option<&str>) {
    cmd.args(["-H", "User-Agent: limen"]);
    if let Some(a) = accept {
        cmd.arg("-H").arg(format!("Accept: {a}"));
    }
    if let Some(t) = token {
        cmd.arg("-H").arg(format!("Authorization: Bearer {t}"));
    }
}

/// A GET, returning curl's raw output.
pub(crate) fn get(flags: &[&str], accept: Option<&str>, url: &str) -> std::io::Result<Output> {
    let mut cmd = Command::new("curl");
    cmd.args(flags);
    with_headers(&mut cmd, accept, token().as_deref());
    cmd.arg(url).no_console().output()
}

/// Download a URL to a file, removing a partial file if it fails.
///
/// A half-written file left behind is worse than none: the next run finds it and
/// takes it for a complete download.
pub(crate) fn download(url: &str, dest: &Path) -> Result<()> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL"]);
    with_headers(&mut cmd, None, token().as_deref());
    let status = cmd
        .arg("-o")
        .arg(dest)
        .arg(url)
        .no_console()
        .status()
        .context("running curl (is it installed and on PATH?)")?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        bail!("download failed: {url}");
    }
    Ok(())
}
