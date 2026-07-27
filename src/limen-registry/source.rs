//! Where a module comes from, and how to fetch it.
//!
//! A [`SourceSpec`] is either a GitHub repo or a local path (copied). Local
//! paths make the resolver fully testable offline.
//!
//! GitHub repos are fetched as a **source tarball** over `curl` + `tar` — the
//! same tools the app self-update already uses — so installing and updating
//! modules needs **no `git` on the machine**. A non-GitHub git URL (e.g. a
//! private SSH remote) falls back to `git clone` when git is available.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use limen_proto::{DepSpec, NoConsole};

use crate::util::copy_tree;

/// A resolved location for one module.
#[derive(Debug, Clone)]
pub enum SourceSpec {
    /// A GitHub repository (`owner/repo`, or a full URL) at an optional
    /// version (git tag/branch).
    Git {
        repo: String,
        version: Option<String>,
    },
    /// A local directory (dev installs and offline tests).
    Path { path: PathBuf },
}

impl SourceSpec {
    /// Parse a user-supplied ref: an existing/`./`/`/`/`file:` path becomes
    /// [`SourceSpec::Path`], otherwise it's a git `owner/repo[@version]`.
    pub fn parse_ref(input: &str) -> Self {
        if let Some(rest) = input.strip_prefix("file:") {
            return SourceSpec::Path { path: PathBuf::from(rest) };
        }
        let looks_like_path = input.starts_with("./")
            || input.starts_with("../")
            || input.starts_with('/')
            || Path::new(input).exists();
        if looks_like_path {
            return SourceSpec::Path { path: PathBuf::from(input) };
        }
        match input.split_once('@') {
            Some((repo, version)) => SourceSpec::Git {
                repo: repo.to_string(),
                version: Some(version.to_string()),
            },
            None => SourceSpec::Git {
                repo: input.to_string(),
                version: None,
            },
        }
    }

    /// Interpret a manifest `[dependencies]` entry, resolving any relative
    /// `path` against the depending module's directory (`base_dir`).
    pub fn from_dep(dep: &DepSpec, base_dir: &Path) -> Result<Self> {
        match dep {
            DepSpec::Short(s) => Ok(Self::parse_ref(s)),
            DepSpec::Detailed(d) => {
                if let Some(path) = &d.path {
                    let p = PathBuf::from(path);
                    let resolved = if p.is_absolute() { p } else { base_dir.join(p) };
                    Ok(SourceSpec::Path { path: resolved })
                } else if let Some(repo) = &d.repo {
                    Ok(SourceSpec::Git {
                        repo: repo.clone(),
                        version: d.version.clone(),
                    })
                } else {
                    bail!("dependency must set either `repo` or `path`")
                }
            }
        }
    }

    /// Short source-kind tag stored in the lockfile.
    pub fn kind(&self) -> &'static str {
        match self {
            SourceSpec::Git { .. } => "git",
            SourceSpec::Path { .. } => "path",
        }
    }

    /// The reference stored in the lockfile (repo for git, path for local).
    pub fn reference(&self) -> String {
        match self {
            SourceSpec::Git { repo, .. } => repo.clone(),
            SourceSpec::Path { path } => path.to_string_lossy().into_owned(),
        }
    }

    /// A human-readable description for logs.
    pub fn describe(&self) -> String {
        match self {
            SourceSpec::Git { repo, version: Some(v) } => format!("git {repo}@{v}"),
            SourceSpec::Git { repo, version: None } => format!("git {repo}"),
            SourceSpec::Path { path } => format!("path {}", path.display()),
        }
    }

    /// Rebuild a spec from a lockfile record, pinned to the recorded version.
    pub fn from_lock(source: &str, reference: &str, version: &str) -> Self {
        match source {
            "path" => SourceSpec::Path {
                path: PathBuf::from(reference),
            },
            _ => SourceSpec::Git {
                repo: reference.to_string(),
                version: Some(version.to_string()),
            },
        }
    }

    /// Rebuild a spec from a lockfile record for **update**: git sources are left
    /// unpinned so they re-resolve to the repo's latest (default branch), rather
    /// than re-fetching the currently-installed version forever.
    pub fn from_lock_latest(source: &str, reference: &str) -> Self {
        match source {
            "path" => SourceSpec::Path {
                path: PathBuf::from(reference),
            },
            _ => SourceSpec::Git {
                repo: reference.to_string(),
                version: None,
            },
        }
    }

    /// Fetch this source into `dest` (which must not already exist).
    pub fn fetch(&self, dest: &Path) -> Result<GitMeta> {
        match self {
            SourceSpec::Path { path } => {
                if !path.is_dir() {
                    bail!("local module path {} is not a directory", path.display());
                }
                copy_tree(path, dest)
                    .with_context(|| format!("copying module from {}", path.display()))?;
                Ok(GitMeta::default())
            }
            SourceSpec::Git { repo, version } => match github_slug(repo) {
                // A GitHub repo: fetch its source tarball (no git needed).
                Some(slug) => fetch_tarball(&slug, version.as_deref(), dest),
                // Some other git remote (private SSH, self-hosted): fall back to git.
                None => clone_git(repo, version.as_deref(), dest),
            },
        }
    }
}

/// Git revision a module was installed from.
#[derive(Debug, Clone, Default)]
pub struct GitMeta {
    pub branch: String,
    pub commit: String,
}

/// The `owner/repo` slug if `repo` points at GitHub (shorthand or a github.com
/// URL), else `None` — meaning "not a GitHub repo, use plain git".
fn github_slug(repo: &str) -> Option<String> {
    let slug = if let Some(rest) = repo
        .strip_prefix("https://github.com/")
        .or_else(|| repo.strip_prefix("http://github.com/"))
        .or_else(|| repo.strip_prefix("git@github.com:"))
    {
        rest
    } else if repo.contains("://") || repo.ends_with(".git") {
        // Some other remote (GitLab, self-hosted, SSH) — not a GitHub tarball.
        return None;
    } else {
        repo
    };
    let slug = slug.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = slug.split_once('/')?;
    // Reject nested paths (`owner/repo/tree/...`) and empty parts.
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// Fetch a GitHub repo's **source tarball** via `curl` + `tar` (no `git`).
///
/// `version` is an optional git ref (tag or branch); with none, GitHub's API
/// serves the default branch. The archive holds a single top-level directory
/// (`owner-repo-<sha>`), from which we recover the short commit; the branch is
/// the requested ref, or the repo's default branch when unpinned.
fn fetch_tarball(slug: &str, version: Option<&str>, dest: &Path) -> Result<GitMeta> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating {}", parent.display()))?;
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("module");
    let tarball = parent.join(format!(".{name}.download.tar.gz"));
    let staging = parent.join(format!(".{name}.staging"));
    let _ = std::fs::remove_file(&tarball);
    let _ = std::fs::remove_dir_all(&staging);

    // The API `tarball` endpoint 302-redirects to codeload; `-L` follows it.
    let url = match version {
        Some(v) => format!("https://api.github.com/repos/{slug}/tarball/{v}"),
        None => format!("https://api.github.com/repos/{slug}/tarball"),
    };
    let status = Command::new("curl")
        .args(["-fsSL", "-H", "User-Agent: limen", "-o"])
        .arg(&tarball)
        .arg(&url)
        .no_console()
        .status()
        .context("running curl (is it installed and on PATH?)")?;
    if !status.success() {
        let _ = std::fs::remove_file(&tarball);
        bail!("downloading {url} failed (curl exit {status})");
    }

    std::fs::create_dir_all(&staging)
        .with_context(|| format!("creating {}", staging.display()))?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&staging)
        .no_console()
        .status()
        .context("running tar (is it installed and on PATH?)");
    let _ = std::fs::remove_file(&tarball);
    let status = status?;
    if !status.success() {
        let _ = std::fs::remove_dir_all(&staging);
        bail!("extracting the module tarball failed (tar exit {status})");
    }

    let top = single_child_dir(&staging).with_context(|| {
        let _ = std::fs::remove_dir_all(&staging);
        "module tarball had no top-level directory".to_string()
    })?;
    // `owner-repo-<sha>` → the trailing segment is the abbreviated commit.
    let commit = top
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit('-').next())
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();

    // Move the extracted tree to `dest` (rename within the same dir is cheap;
    // fall back to a copy if that ever crosses a filesystem boundary).
    if std::fs::rename(&top, dest).is_err() {
        copy_tree(&top, dest)
            .with_context(|| format!("installing module into {}", dest.display()))?;
    }
    let _ = std::fs::remove_dir_all(&staging);

    let branch = match version {
        Some(v) => v.to_string(),
        None => default_branch(slug).unwrap_or_default(),
    };
    Ok(GitMeta { branch, commit })
}

/// The single directory directly inside `dir` (a GitHub tarball's one root).
fn single_child_dir(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
}

/// The repo's default branch, via the GitHub API. `None` on any error.
fn default_branch(slug: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{slug}");
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .no_console()
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(json.get("default_branch")?.as_str()?.to_string())
}

/// `git clone --depth 1` the repo (optionally at a tag/branch), record the
/// checked-out branch + commit, then strip the `.git` directory so only the
/// module tree remains.
fn clone_git(repo: &str, version: Option<&str>, dest: &Path) -> Result<GitMeta> {
    let url = if repo.contains("://") || repo.ends_with(".git") {
        repo.to_string()
    } else {
        format!("https://github.com/{repo}.git")
    };

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1"]);
    if let Some(v) = version {
        cmd.args(["--branch", v]);
    }
    cmd.arg(&url).arg(dest);

    let status = cmd
        .no_console()
        .status()
        .context("running `git clone` (is git installed and on PATH?)")?;
    if !status.success() {
        bail!("git clone {url} failed (exit {status})");
    }

    // Capture the revision before the `.git` dir is removed.
    let commit = git_out(dest, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let branch = match git_out(dest, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        // A tag checkout is detached (`HEAD`); fall back to the requested ref.
        Some(b) if b != "HEAD" => b,
        _ => version.unwrap_or_default().to_string(),
    };

    let _ = std::fs::remove_dir_all(dest.join(".git"));
    Ok(GitMeta { branch, commit })
}

/// Run `git -C dir <args>` and return its trimmed stdout, if it succeeds.
fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).no_console().output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    #[test]
    fn from_lock_pins_version_but_update_does_not() {
        // Normal reconstruction pins to the recorded version (reproducible).
        match SourceSpec::from_lock("git", "owner/repo", "0.1.0") {
            SourceSpec::Git { repo, version } => {
                assert_eq!(repo, "owner/repo");
                assert_eq!(version.as_deref(), Some("0.1.0"));
            }
            _ => panic!("expected a git spec"),
        }
        // Update must be UNPINNED so a module can move past its installed version
        // (the bug fixed in 0.5.2 — pinned update could never reach a newer tag).
        match SourceSpec::from_lock_latest("git", "owner/repo") {
            SourceSpec::Git { repo, version } => {
                assert_eq!(repo, "owner/repo");
                assert_eq!(version, None, "update must not pin to the installed version");
            }
            _ => panic!("expected a git spec"),
        }
        // Path sources are identical either way.
        assert!(matches!(
            SourceSpec::from_lock_latest("path", "/some/dir"),
            SourceSpec::Path { .. }
        ));
    }

    #[test]
    fn github_slug_recognizes_github_and_rejects_others() {
        // Shorthand and github.com URLs → a clean owner/repo slug (tarball path).
        assert_eq!(github_slug("owner/repo").as_deref(), Some("owner/repo"));
        assert_eq!(
            github_slug("https://github.com/owner/repo").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            github_slug("https://github.com/owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        assert_eq!(
            github_slug("git@github.com:owner/repo.git").as_deref(),
            Some("owner/repo")
        );
        // Non-GitHub remotes → None (fall back to plain git).
        assert_eq!(github_slug("https://gitlab.com/owner/repo"), None);
        assert_eq!(github_slug("git@example.com:owner/repo.git"), None);
        // Malformed shorthands → None.
        assert_eq!(github_slug("justaword"), None);
        assert_eq!(github_slug("owner/repo/extra"), None);
    }
}
