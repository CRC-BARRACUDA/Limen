//! Discover modules published in a GitHub organization.
//!
//! Lists the org's repos via the GitHub REST API and keeps the ones that are
//! modules — tagged with the `limen-module` topic, or (fallback) named
//! `limen-<name>` and not one of the SDK/contract libraries. The module name is
//! the repo name with the `limen-` prefix stripped.
//!
//! Uses `curl` so we don't pull in an HTTP+TLS stack. Unauthenticated (public
//! repos only; subject to GitHub's anonymous rate limit).

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

    let mut modules: Vec<RemoteModule> = repos
        .into_iter()
        .filter(|r| !r.archived && is_module(r))
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

fn is_module(repo: &GhRepo) -> bool {
    if repo.topics.iter().any(|t| t == MODULE_TOPIC) {
        return true;
    }
    repo.name.starts_with("limen-") && !LIBRARY_REPOS.contains(&repo.name.as_str())
}
