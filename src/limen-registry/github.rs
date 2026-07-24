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

/// A module available to install from the org.
#[derive(Debug, Clone)]
pub struct RemoteModule {
    /// Module name (repo name without the `limen-` prefix).
    pub name: String,
    /// `owner/repo`, for `limen add`.
    pub repo: String,
    pub description: Option<String>,
    /// Browsable repo URL.
    pub url: String,
}

#[derive(Deserialize)]
struct GhRepo {
    name: String,
    description: Option<String>,
    html_url: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    archived: bool,
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
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                    bail!("GitHub: {msg}");
                }
            }
            bail!("unexpected response from GitHub for org {org}");
        }
    };

    // Keep only repos that look like modules AND actually carry a limen.toml.
    let mut modules: Vec<RemoteModule> = repos
        .into_iter()
        .filter(|r| !r.archived && is_module(r))
        .filter(|r| repo_has_manifest(org, &r.name))
        .map(|r| RemoteModule {
            name: r.name.strip_prefix("limen-").unwrap_or(&r.name).to_string(),
            repo: format!("{org}/{}", r.name),
            description: r.description,
            url: r.html_url,
        })
        .collect();

    modules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(modules)
}

/// Does `org/repo` contain a `limen.toml` at its root? Queries the GitHub
/// contents API (uses the default branch). Any error / 404 counts as "no".
fn repo_has_manifest(org: &str, repo: &str) -> bool {
    let url = format!("https://api.github.com/repos/{org}/{repo}/contents/limen.toml");
    let output = Command::new("curl")
        .args([
            "-sSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output();
    let bytes = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return false,
    };
    // On success the API returns a file object ("type":"file"); on 404 it
    // returns {"message":"Not Found"} (no "type").
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
        .as_deref()
        == Some("file")
}

fn is_module(repo: &GhRepo) -> bool {
    if repo.topics.iter().any(|t| t == MODULE_TOPIC) {
        return true;
    }
    repo.name.starts_with("limen-") && !LIBRARY_REPOS.contains(&repo.name.as_str())
}
