//! HTTP for the platform's services (sign-in, avatars), through `curl` as
//! everywhere in Limen.
//!
//! The request — URL, form, bearer token — goes to curl as a config on stdin,
//! never as arguments, so a token never shows in a process list.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use limen_proto::NoConsole;

/// A curl config value: in double quotes, with `\` and `"` escaped.
fn quoted(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

fn run(config: &str, extra: &[&str]) -> Result<std::process::Output> {
    let mut child = Command::new("curl")
        .args(["-sS", "--max-time", "30"])
        .args(extra)
        .args(["--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console()
        .spawn()
        .context("running curl (is it installed and on PATH?)")?;
    child
        .stdin
        .take()
        .context("curl's stdin")?
        .write_all(config.as_bytes())
        .context("handing curl its request")?;
    child.wait_with_output().context("waiting for curl")
}

/// One JSON call: its status and body. A form makes it a POST.
pub fn request(url: &str, form: Option<&[(&str, &str)]>, bearer: Option<&str>) -> Result<(u16, String)> {
    let mut config = format!(
        "url = {}\nheader = \"Accept: application/json\"\nheader = \"User-Agent: limen\"\n",
        quoted(url)
    );
    if let Some(fields) = form {
        let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish();
        config.push_str(&format!("data = {}\n", quoted(&body)));
    }
    if let Some(t) = bearer {
        config.push_str(&format!("header = {}\n", quoted(&format!("Authorization: Bearer {t}"))));
    }
    let out = run(&config, &["-w", "\n%{http_code}"])?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
    let status: u16 = status.trim().parse().unwrap_or(0);
    if status == 0 {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("the platform could not be reached: {}", if why.is_empty() { url.to_string() } else { why });
    }
    Ok((status, body.to_string()))
}

/// A file's bytes, refused past `max` bytes — nothing from the network is read
/// without a limit.
pub fn bytes(url: &str, max: u64) -> Result<Vec<u8>> {
    let config = format!("url = {}\nheader = \"User-Agent: limen\"\nfail\n", quoted(url));
    let out = run(&config, &["--max-filesize", &max.to_string()])?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("could not fetch {url}: {}", if why.is_empty() { "curl failed".into() } else { why });
    }
    if out.stdout.len() as u64 > max {
        bail!("{url} is larger than {max} bytes");
    }
    Ok(out.stdout)
}
