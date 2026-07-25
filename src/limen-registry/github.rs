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

use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

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

/// List the modules published under `org` on GitHub.
pub fn list_org_modules(org: &str) -> Result<Vec<RemoteModule>> {
    let url = format!("https://api.github.com/orgs/{org}/repos?per_page=100&type=public");
    let output = Command::new("curl")
        .args([
            "-sSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .context("running curl (is it installed and on PATH?)")?;

    if !output.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let repos: Vec<GhRepo> = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(_) => {
            // GitHub returns `{ "message": "..." }` on errors (rate limit, 404…).
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
                && let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                    bail!("GitHub: {msg}");
                }
            bail!("unexpected response from GitHub for org {org}");
        }
    };

    // Keep only repos that look like modules AND carry a parseable limen.toml —
    // reading the manifest gives us the metadata to show before install.
    let mut modules: Vec<RemoteModule> = repos
        .into_iter()
        .filter(|r| !r.archived && is_module(r))
        .filter_map(|r| {
            let m = fetch_manifest(org, &r.name, &r.default_branch)?;
            Some(RemoteModule {
                name: r.name.strip_prefix("limen-").unwrap_or(&r.name).to_string(),
                repo: format!("{org}/{}", r.name),
                description: m.module.description.clone().or(r.description),
                url: r.html_url,
                version: Some(m.module.version.clone()),
                capabilities: m.provides.capabilities.clone(),
                commit: fetch_latest_commit(org, &r.name, &r.default_branch),
                branch: Some(r.default_branch),
            })
        })
        .collect();

    modules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(modules)
}

/// The short commit of `org/repo`'s `branch` tip on GitHub — the revision a
/// fresh install would fetch. `None` on any error.
fn fetch_latest_commit(org: &str, repo: &str, branch: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{org}/{repo}/commits/{branch}");
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let sha = json.get("sha")?.as_str()?;
    Some(sha.chars().take(7).collect())
}

/// Fetch and parse `org/repo`'s root `limen.toml` from its default branch.
/// `None` if the repo has no manifest, it doesn't parse, or the fetch fails —
/// which also serves to exclude non-module repos.
fn fetch_manifest(org: &str, repo: &str, branch: &str) -> Option<limen_proto::Manifest> {
    let url = format!("https://raw.githubusercontent.com/{org}/{repo}/{branch}/limen.toml");
    let out = Command::new("curl")
        .args(["-fsSL", "-H", "User-Agent: limen", &url])
        .output()
        .ok()?;
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
