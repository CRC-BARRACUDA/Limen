//! Discover modules published in a GitHub organization.
//!
//! Lists the org's repos via the GitHub REST API and keeps the ones that are
//! modules — a candidate must be tagged with the `limen-module` topic (or, as a
//! fallback, named `limen-<name>` and not an SDK/contract library) **and**
//! actually contain a `limen.toml` manifest at its root. Repos without the
//! manifest (e.g. empty or non-module repos) are never shown. The module name is
//! the repo name with the `limen-` prefix stripped.
//!
//! Uses `curl` so we don't pull in an HTTP+TLS stack. Unauthenticated (public
//! repos only; subject to GitHub's anonymous rate limit — note that the manifest
//! check costs one extra request per candidate repo).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::RwLock;

use anyhow::{bail, Context, Result};
use limen_proto::NoConsole;
use serde::{Deserialize, Serialize};

/// An optional GitHub token applied to every registry request when set — raising
/// the rate limit from 60/hour (unauthenticated, per IP) to 5,000/hour and making
/// conditional (304) requests free. Set from settings at startup and whenever an
/// administrator changes it in Developer mode; `None` = unauthenticated (default).
static TOKEN: RwLock<Option<String>> = RwLock::new(None);

/// Set (or clear) the GitHub token used for registry requests. A blank token
/// clears it — back to the unauthenticated default.
pub fn set_token(token: Option<String>) {
    let cleaned = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    *TOKEN.write().unwrap() = cleaned;
}

/// Verify a token by making one authenticated request (`GET /rate_limit`, which
/// needs no scope). `Ok(())` if GitHub accepts it (HTTP 200); `Err(reason)` if it
/// doesn't (401/403/…) or the request couldn't be made — so a bad token is never
/// saved.
pub fn test_token(token: &str) -> Result<(), String> {
    let t = token.trim();
    if t.is_empty() {
        return Err("empty token".into());
    }
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "15",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            &format!("Authorization: Bearer {t}"),
            "-w",
            "\n@@LIMEN_META@@ %{http_code}",
            "https://api.github.com/rate_limit",
        ])
        .no_console()
        .output()
        .map_err(|e| format!("running curl: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, meta) = text.rsplit_once("\n@@LIMEN_META@@ ").ok_or("no response from GitHub")?;
    match meta.trim() {
        "200" => Ok(()),
        "401" => Err("invalid token — GitHub returned 401 (bad credentials)".into()),
        code => {
            // Surface GitHub's own message when present (e.g. 403 blocked/SSO).
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
                && let Some(m) = v.get("message").and_then(|m| m.as_str())
            {
                return Err(format!("GitHub: {m} ({code})"));
            }
            Err(format!("token rejected (HTTP {code})"))
        }
    }
}

/// A registry `curl` GET: the given flags, the standard `User-Agent`, an optional
/// `Accept`, an `Authorization: Bearer` header when a token is set, then the URL.
fn curl_get(flags: &[&str], accept: Option<&str>, url: &str) -> std::io::Result<Output> {
    let mut cmd = Command::new("curl");
    cmd.args(flags).args(["-H", "User-Agent: limen"]);
    if let Some(a) = accept {
        cmd.arg("-H").arg(format!("Accept: {a}"));
    }
    if let Some(t) = TOKEN.read().unwrap().as_deref() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {t}"));
    }
    cmd.arg(url).no_console().output()
}

// --------------------------------------------------------------------------- //
// Conditional requests (ETag / If-None-Match)
// --------------------------------------------------------------------------- //
/// Where the ETag+body cache lives (set once from settings). Without it, requests
/// still work — just not conditionally.
static CACHE_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Set the directory for the registry's conditional-request cache. Enables ETag /
/// `If-None-Match`: unchanged data returns `304` (free against the rate limit when
/// authenticated) and the cached body is reused.
pub fn set_cache_dir(dir: PathBuf) {
    *CACHE_DIR.write().unwrap() = Some(dir);
}

#[derive(Clone, Serialize, Deserialize)]
struct CacheEntry {
    etag: String,
    body: String,
}

fn cache_path() -> Option<PathBuf> {
    CACHE_DIR.read().unwrap().clone().map(|d| d.join("registry-cache.json"))
}

fn load_cache() -> HashMap<String, CacheEntry> {
    cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &HashMap<String, CacheEntry>) {
    if let Some(p) = cache_path() {
        if let Ok(s) = serde_json::to_string(cache) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// A conditional GET: sends `If-None-Match` from the cache; on `304` returns the
/// cached body (which doesn't count against the rate limit when authenticated),
/// on `2xx` caches the new ETag + body. Returns the effective body — or, on an
/// error status, the error body for the caller to inspect. `None` only if curl
/// couldn't run at all.
fn conditional_get(flags: &[&str], accept: Option<&str>, url: &str) -> Option<String> {
    let mut cache = load_cache();
    let prev = cache.get(url).cloned();

    let mut cmd = Command::new("curl");
    cmd.args(flags).args(["-H", "User-Agent: limen"]);
    if let Some(a) = accept {
        cmd.arg("-H").arg(format!("Accept: {a}"));
    }
    if let Some(t) = TOKEN.read().unwrap().as_deref() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {t}"));
    }
    if let Some(p) = &prev {
        if !p.etag.is_empty() {
            cmd.arg("-H").arg(format!("If-None-Match: {}", p.etag));
        }
    }
    // Append `\n@@LIMEN_META@@ <status> <etag>` after the body (curl 7.84+ for
    // %header{}; older curl yields no etag → we simply don't cache).
    cmd.arg("-w").arg("\n@@LIMEN_META@@ %{http_code} %header{etag}");
    let out = cmd.arg(url).no_console().output().ok()?;

    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, meta) = text.rsplit_once("\n@@LIMEN_META@@ ")?;
    let mut it = meta.trim().splitn(2, ' ');
    let code = it.next().unwrap_or("");
    let raw_etag = it.next().unwrap_or("").trim();
    // Only accept a real ETag (`"…"` or weak `W/"…"`), never the literal `-w`
    // template a pre-7.84 curl would echo back.
    let etag = if raw_etag.starts_with('"') || raw_etag.starts_with("W/") {
        raw_etag.to_string()
    } else {
        String::new()
    };

    if code == "304" {
        return prev.map(|p| p.body);
    }
    let body = body.to_string();
    if code.starts_with('2') && !etag.is_empty() {
        cache.insert(url.to_string(), CacheEntry { etag, body: body.clone() });
        save_cache(&cache);
    }
    Some(body)
}

/// A module available to install from the org. Its metadata is read from the
/// repo's `limen.toml` on GitHub, so the manager can show it **before** install.
#[derive(Debug, Clone)]
pub struct RemoteModule {
    /// Module name (repo name without the `limen-` prefix).
    pub name: String,
    /// `owner/repo`, for `limen add`.
    pub repo: String,
    pub description: Option<String>,
    /// Browsable repo URL.
    pub url: String,
    /// Version declared in the manifest.
    pub version: Option<String>,
    /// Capabilities the module provides (from the manifest).
    pub capabilities: Vec<String>,
    /// The repo's default branch on GitHub.
    pub branch: Option<String>,
    /// Short commit of that branch's tip (what a fresh install would fetch).
    pub commit: Option<String>,
}

#[derive(Deserialize)]
struct GhRepo {
    name: String,
    description: Option<String>,
    html_url: String,
    #[serde(default = "default_branch")]
    default_branch: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    archived: bool,
}

fn default_branch() -> String {
    "main".to_string()
}

/// Repos in the org that share the `limen-` prefix but are libraries, not modules.
const LIBRARY_REPOS: &[&str] = &["limen-proto", "limen-sdk-rust"];

/// The `module` topic that authoritatively marks a repo as a Limen module.
const MODULE_TOPIC: &str = "limen-module";

/// A repo that passed the module filter, before its manifest is fetched. The
/// GUI lists these first, then fetches each one's metadata in parallel (see
/// [`fetch_remote_module`]) so cards can appear as they arrive.
#[derive(Debug, Clone)]
pub struct RepoCandidate {
    pub org: String,
    /// Repo name (e.g. `limen-devices`).
    pub name: String,
    pub default_branch: String,
    pub description: Option<String>,
    pub html_url: String,
}

/// List the org's repos that *look* like modules (module topic or `limen-`
/// prefix, not archived) — **without** fetching their manifests. Cheap: one
/// request. The caller resolves each candidate via [`fetch_remote_module`].
pub fn list_org_module_repos(org: &str) -> Result<Vec<RepoCandidate>> {
    let url = format!("https://api.github.com/orgs/{org}/repos?per_page=100&type=public");
    // Conditional: a 304 (unchanged) reuses the cached body and, when
    // authenticated, doesn't count against the rate limit.
    let body = conditional_get(&["-sSL"], Some("application/vnd.github+json"), &url)
        .context("running curl (is it installed and on PATH?)")?;

    let repos: Vec<GhRepo> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            // GitHub returns `{ "message": "..." }` on errors (rate limit, 404…).
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body)
                && let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                    bail!("GitHub: {msg}");
                }
            bail!("unexpected response from GitHub for org {org}");
        }
    };

    Ok(repos
        .into_iter()
        .filter(|r| !r.archived && is_module(r))
        .map(|r| RepoCandidate {
            org: org.to_string(),
            name: r.name,
            default_branch: r.default_branch,
            description: r.description,
            html_url: r.html_url,
        })
        .collect())
}

/// Resolve one candidate into a [`RemoteModule`] by reading its `limen.toml`
/// (plus its tip commit). `None` if it has no parseable manifest, or it's a
/// native module with no prebuilt binary for this platform — those are hidden.
/// Two-to-three HTTP requests; the GUI runs many of these concurrently.
pub fn fetch_remote_module(c: &RepoCandidate) -> Option<RemoteModule> {
    let m = fetch_manifest(&c.org, &c.name, &c.default_branch)?;
    let repo = format!("{}/{}", c.org, c.name);
    // A native (compiled) module can only run where a prebuilt library for this
    // exact OS/arch/bitness exists in its release — plus a checksum. If not,
    // hide it from this platform's manager rather than offering a broken install.
    let native = m.module.language == limen_proto::Language::Native
        && m.module.abi == limen_proto::Abi::Native;
    if native && !crate::registry::native_release_ready(&repo) {
        return None;
    }
    Some(RemoteModule {
        name: c.name.strip_prefix("limen-").unwrap_or(&c.name).to_string(),
        repo,
        description: m.module.description.clone().or_else(|| c.description.clone()),
        url: c.html_url.clone(),
        version: Some(m.module.version.clone()),
        capabilities: m.provides.capabilities.clone(),
        commit: fetch_latest_commit(&c.org, &c.name, &c.default_branch),
        branch: Some(c.default_branch.clone()),
    })
}

/// List the modules published under `org`, metadata included — sequential. The
/// GUI streams these in parallel instead (via the two functions above); this
/// stays for the CLI and tests.
pub fn list_org_modules(org: &str) -> Result<Vec<RemoteModule>> {
    let mut modules: Vec<RemoteModule> = list_org_module_repos(org)?
        .iter()
        .filter_map(fetch_remote_module)
        .collect();
    modules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(modules)
}

/// The short commit of `org/repo`'s `branch` tip on GitHub — the revision a
/// fresh install would fetch. `None` on any error.
fn fetch_latest_commit(org: &str, repo: &str, branch: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{org}/{repo}/commits/{branch}");
    let body = conditional_get(&["-sSL"], Some("application/vnd.github+json"), &url)?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let sha = json.get("sha")?.as_str()?;
    Some(sha.chars().take(7).collect())
}

/// Fetch and parse `org/repo`'s root `limen.toml` from its default branch.
/// `None` if the repo has no manifest, it doesn't parse, or the fetch fails —
/// which also serves to exclude non-module repos.
fn fetch_manifest(org: &str, repo: &str, branch: &str) -> Option<limen_proto::Manifest> {
    let url = format!("https://raw.githubusercontent.com/{org}/{repo}/{branch}/limen.toml");
    // raw.githubusercontent is a CDN (not the rate-limited API); auth is harmless
    // and lets private-repo manifests resolve too.
    let out = curl_get(&["-fsSL"], None, &url).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    toml::from_str::<limen_proto::Manifest>(&text).ok()
}

fn is_module(repo: &GhRepo) -> bool {
    if repo.topics.iter().any(|t| t == MODULE_TOPIC) {
        return true;
    }
    repo.name.starts_with("limen-") && !LIBRARY_REPOS.contains(&repo.name.as_str())
}
