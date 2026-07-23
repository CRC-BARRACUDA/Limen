//! Where a module comes from, and how to fetch it.
//!
//! A [`SourceSpec`] is either a GitHub repo (cloned with `git`) or a local path
//! (copied). Local paths make the resolver fully testable offline.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use limen_proto::DepSpec;

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

    /// Rebuild a spec from a lockfile record.
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

    /// Fetch this source into `dest` (which must not already exist).
    pub fn fetch(&self, dest: &Path) -> Result<()> {
        match self {
            SourceSpec::Path { path } => {
                if !path.is_dir() {
                    bail!("local module path {} is not a directory", path.display());
                }
                copy_tree(path, dest)
                    .with_context(|| format!("copying module from {}", path.display()))
            }
            SourceSpec::Git { repo, version } => clone_git(repo, version.as_deref(), dest),
        }
    }
}

/// `git clone --depth 1` the repo (optionally at a tag/branch), then strip the
/// `.git` directory so only the module tree remains.
fn clone_git(repo: &str, version: Option<&str>, dest: &Path) -> Result<()> {
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
        .status()
        .context("running `git clone` (is git installed and on PATH?)")?;
    if !status.success() {
        bail!("git clone {url} failed (exit {status})");
    }
    let _ = std::fs::remove_dir_all(dest.join(".git"));
    Ok(())
}
