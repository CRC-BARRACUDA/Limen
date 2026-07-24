//! The [`Registry`]: resolve a dependency graph, install it, and maintain the
//! lockfile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use limen_proto::{Abi, Language, Manifest};

use crate::lockfile::{LockEntry, Lockfile};
use crate::source::SourceSpec;
use crate::util::{copy_tree, digest_dir};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// What an `add`/`update` installed.
#[derive(Debug, Default)]
pub struct InstallReport {
    pub installed: Vec<LockEntry>,
}

/// The result of verifying one installed module against the lockfile.
#[derive(Debug)]
pub struct VerifyItem {
    pub name: String,
    pub status: VerifyStatus,
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyStatus {
    /// On-disk digest matches the lockfile.
    Ok,
    /// The files changed since install (tamper / local edit).
    Modified { expected: String, actual: String },
    /// Locked but the files are gone.
    Missing,
}

/// One fetched-but-not-yet-installed module, held in a temp dir during resolve.
struct Resolved {
    version: String,
    source: SourceSpec,
    tempdir: PathBuf,
}

pub struct Registry {
    home: PathBuf,
}

impl Registry {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn modules_dir(&self) -> PathBuf {
        self.home.join("modules")
    }

    pub fn lock_path(&self) -> PathBuf {
        self.home.join("limen.lock")
    }

    fn temp_root(&self) -> PathBuf {
        self.home.join(".tmp")
    }

    /// Install a module (and its dependency graph) from a ref like
    /// `owner/repo@1.2.0`, a `file:`/local path, updating the lockfile.
    pub fn add(&self, reference: &str) -> Result<InstallReport> {
        let root = SourceSpec::parse_ref(reference);
        self.install_from(root)
    }

    /// List the currently locked (installed) modules.
    pub fn list(&self) -> Result<Vec<LockEntry>> {
        Ok(Lockfile::load(&self.lock_path())?.modules)
    }

    /// Recompute each installed module's digest and compare to the lockfile,
    /// detecting tampering or drift.
    pub fn verify(&self) -> Result<Vec<VerifyItem>> {
        let lock = Lockfile::load(&self.lock_path())?;
        let mut items = Vec::with_capacity(lock.modules.len());
        for entry in &lock.modules {
            let dir = self.modules_dir().join(&entry.name);
            let status = if !dir.exists() {
                VerifyStatus::Missing
            } else {
                let actual = digest_dir(&dir)?;
                if actual == entry.digest {
                    VerifyStatus::Ok
                } else {
                    VerifyStatus::Modified {
                        expected: entry.digest.clone(),
                        actual,
                    }
                }
            };
            items.push(VerifyItem {
                name: entry.name.clone(),
                status,
            });
        }
        Ok(items)
    }

    /// Remove an installed module: delete its files and drop it from the lock.
    pub fn remove(&self, name: &str) -> Result<bool> {
        let mut lock = Lockfile::load(&self.lock_path())?;
        let existed = lock.remove(name);
        let dir = self.modules_dir().join(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("removing {}", dir.display()))?;
        }
        if existed {
            lock.save(&self.lock_path())?;
        }
        Ok(existed)
    }

    /// Re-fetch and reinstall locked modules (all, or just `name`), refreshing
    /// their digests — e.g. to pick up a moved git tag or an edited local path.
    pub fn update(&self, name: Option<&str>) -> Result<InstallReport> {
        let lock = Lockfile::load(&self.lock_path())?;
        let targets: Vec<&LockEntry> = lock
            .modules
            .iter()
            .filter(|m| name.is_none_or(|n| n == m.name))
            .collect();
        if targets.is_empty() {
            bail!("nothing to update{}", name.map(|n| format!(" (no module {n})")).unwrap_or_default());
        }

        let mut report = InstallReport::default();
        for entry in targets {
            let spec = SourceSpec::from_lock(&entry.source, &entry.reference, &entry.version);
            let sub = self.install_from(spec)?;
            report.installed.extend(sub.installed);
        }
        Ok(report)
    }

    /// Core flow: resolve the graph rooted at `root`, install every module,
    /// digest it, and upsert the lockfile.
    fn install_from(&self, root: SourceSpec) -> Result<InstallReport> {
        let mut resolved: BTreeMap<String, Resolved> = BTreeMap::new();
        let result = self.resolve(root, &mut resolved);

        // Whatever happens, don't leave temp dirs behind.
        let cleanup = |resolved: &BTreeMap<String, Resolved>| {
            for r in resolved.values() {
                let _ = std::fs::remove_dir_all(&r.tempdir);
            }
            let _ = std::fs::remove_dir_all(self.temp_root());
        };
        if let Err(e) = result {
            cleanup(&resolved);
            return Err(e);
        }

        let mut lock = Lockfile::load(&self.lock_path())?;
        let mut report = InstallReport::default();
        std::fs::create_dir_all(self.modules_dir())
            .with_context(|| format!("creating {}", self.modules_dir().display()))?;

        for (name, r) in &resolved {
            let dest = self.modules_dir().join(name);
            let _ = std::fs::remove_dir_all(&dest);
            copy_tree(&r.tempdir, &dest)
                .with_context(|| format!("installing {name} into {}", dest.display()))?;
            let digest = digest_dir(&dest)?;
            let entry = LockEntry {
                name: name.clone(),
                version: r.version.clone(),
                source: r.source.kind().to_string(),
                reference: r.source.reference(),
                digest,
            };
            lock.upsert(entry.clone());
            report.installed.push(entry);
        }

        lock.save(&self.lock_path())?;
        cleanup(&resolved);
        Ok(report)
    }

    /// Recursively fetch `spec` and its dependencies into temp dirs.
    fn resolve(&self, spec: SourceSpec, acc: &mut BTreeMap<String, Resolved>) -> Result<()> {
        let tempdir = self.new_temp()?;
        spec.fetch(&tempdir)
            .with_context(|| format!("fetching {}", spec.describe()))?;

        let manifest = Manifest::from_dir(&tempdir)
            .with_context(|| format!("reading manifest of {}", spec.describe()))?;
        let name = manifest.module.name.clone();
        let version = manifest.module.version.clone();

        // Relative `path` dependencies resolve against the module's *original*
        // location (a local dir), not its temp copy. A git module's path deps —
        // unusual — fall back to the fetched tree.
        let origin_dir = match &spec {
            SourceSpec::Path { path } => path.clone(),
            SourceSpec::Git { .. } => tempdir.clone(),
        };

        if let Some(existing) = acc.get(&name) {
            if existing.version != version {
                eprintln!(
                    "[registry] warning: {name} resolved to v{} and v{version}; keeping v{}",
                    existing.version, existing.version
                );
            }
            let _ = std::fs::remove_dir_all(&tempdir);
            return Ok(());
        }

        // For a native module installed from GitHub, fetch the prebuilt library
        // for this platform so the operator needs no toolchain. If there's no
        // matching release asset, the module still installs as source (and must
        // be built).
        if manifest.module.language == Language::Native && manifest.module.abi == Abi::Native {
            if let SourceSpec::Git { repo, version } = &spec {
                match fetch_native_asset(repo, &manifest.module.entry, version.as_deref(), &tempdir)
                {
                    Ok(lib) => eprintln!("[registry] {name}: downloaded prebuilt {lib}"),
                    Err(e) => eprintln!(
                        "[registry] {name}: no prebuilt library for {}/{} ({e}); \
                         installed as source — build it with `cargo build`",
                        std::env::consts::OS,
                        std::env::consts::ARCH
                    ),
                }
            }
        }

        // Record before recursing so a dependency cycle terminates.
        acc.insert(
            name.clone(),
            Resolved {
                version,
                source: spec,
                tempdir: tempdir.clone(),
            },
        );

        for (dep_name, dep_spec) in &manifest.dependencies {
            let sub = SourceSpec::from_dep(dep_spec, &origin_dir)
                .with_context(|| format!("dependency {dep_name} of {name}"))?;
            self.resolve(sub, acc)?;
        }
        Ok(())
    }

    fn new_temp(&self) -> Result<PathBuf> {
        let root = self.temp_root();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating {}", root.display()))?;
        let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = root.join(format!("fetch-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(dir)
    }
}

// --------------------------------------------------------------------------- //
// Prebuilt native-library download.
// --------------------------------------------------------------------------- //

/// The GitHub `owner/repo` slug from a repo ref (short form or full URL).
fn github_slug(repo: &str) -> &str {
    repo.strip_prefix("https://github.com/")
        .or_else(|| repo.strip_prefix("http://github.com/"))
        .unwrap_or(repo)
        .trim_end_matches(".git")
}

/// The filename the host expects for the loadable library, e.g.
/// `liblocal_devices.so` (unix) / `local_devices.dll` (windows).
fn native_lib_filename(entry: &str) -> String {
    format!(
        "{}{entry}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Fetch the list of assets `(name, download_url)` from a GitHub release. Uses
/// the release for `version` (a git tag) if given, else the latest release.
fn release_assets(slug: &str, version: Option<&str>) -> Result<Vec<(String, String)>> {
    let url = match version {
        Some(tag) => format!("https://api.github.com/repos/{slug}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{slug}/releases/latest"),
    };
    let output = std::process::Command::new("curl")
        .args([
            "-sSL",
            "-H",
            "User-Agent: limen",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .context("running curl")?;
    if !output.status.success() {
        bail!("curl failed fetching release");
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parsing release JSON")?;
    if let Some(msg) = json.get("message").and_then(|m| m.as_str()) {
        bail!("GitHub: {msg}"); // e.g. "Not Found" when there's no release
    }
    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some((
                        a.get("name")?.as_str()?.to_string(),
                        a.get("browser_download_url")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(assets)
}

/// Pick the release asset matching this platform's library: it must end with the
/// platform's extension (`.so`/`.dll`/`.dylib`); an arch-tagged name is preferred.
fn match_platform_asset(assets: &[(String, String)]) -> Option<&(String, String)> {
    let suffix = std::env::consts::DLL_SUFFIX;
    let arch = std::env::consts::ARCH;
    assets
        .iter()
        .find(|(n, _)| n.ends_with(suffix) && n.contains(arch))
        .or_else(|| assets.iter().find(|(n, _)| n.ends_with(suffix)))
}

/// List `repo`'s release files, pick the one for this platform, and download it
/// into `dest_dir` named as the host expects. Returns the filename on success.
fn fetch_native_asset(
    repo: &str,
    entry: &str,
    version: Option<&str>,
    dest_dir: &Path,
) -> Result<String> {
    let slug = github_slug(repo);
    let assets = release_assets(slug, version)?;
    if assets.is_empty() {
        bail!("release has no attached files");
    }
    let (name, url) = match_platform_asset(&assets).ok_or_else(|| {
        anyhow::anyhow!(
            "no {} asset among: {}",
            std::env::consts::DLL_SUFFIX,
            assets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
        )
    })?;

    let filename = native_lib_filename(entry);
    let dest = dest_dir.join(&filename);
    let status = std::process::Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&dest)
        .arg(url)
        .status()
        .context("running curl")?;
    if !status.success() {
        let _ = std::fs::remove_file(&dest);
        bail!("downloading {name} failed");
    }
    Ok(format!("{name} → {filename}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tmp(tag: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("limen-reg-{}-{tag}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn slug_and_asset_matching() {
        assert_eq!(github_slug("owner/repo"), "owner/repo");
        assert_eq!(github_slug("https://github.com/owner/repo.git"), "owner/repo");

        let lib = native_lib_filename("local_devices");
        assert!(lib.contains("local_devices"));
        assert!(lib.ends_with(std::env::consts::DLL_SUFFIX));

        // Among a realistic asset set, pick the one for this platform's extension.
        let assets = vec![
            ("local_devices-linux-x86_64.so".to_string(), "u1".to_string()),
            ("local_devices-windows-x86_64.dll".to_string(), "u2".to_string()),
            ("local_devices-macos-aarch64.dylib".to_string(), "u3".to_string()),
            ("README.txt".to_string(), "u4".to_string()),
        ];
        let picked = match_platform_asset(&assets).unwrap();
        assert!(picked.0.ends_with(std::env::consts::DLL_SUFFIX));
    }

    fn write_module(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("limen.toml"), body).unwrap();
        std::fs::write(dir.join("main.py"), "# stub\n").unwrap();
    }

    #[test]
    fn installs_a_module_and_its_path_dependency() {
        let fixtures = tmp("fix");
        // provider (no deps)
        write_module(
            &fixtures.join("provider"),
            "[module]\nname=\"provider\"\nversion=\"1.0.0\"\nlanguage=\"python\"\nentry=\"main.py\"\n\
             [provides]\ncapabilities=[\"p.cap\"]\n",
        );
        // consumer depends on provider by relative path
        write_module(
            &fixtures.join("consumer"),
            "[module]\nname=\"consumer\"\nversion=\"0.1.0\"\nlanguage=\"python\"\nentry=\"main.py\"\n\
             [requires.capabilities]\n\"p.cap\"=\">=1.0\"\n\
             [dependencies]\nprovider={ path=\"../provider\" }\n",
        );

        let home = tmp("home");
        let reg = Registry::new(&home);
        let report = reg
            .add(fixtures.join("consumer").to_str().unwrap())
            .unwrap();

        // Both modules installed.
        assert_eq!(report.installed.len(), 2);
        assert!(reg.modules_dir().join("provider/limen.toml").is_file());
        assert!(reg.modules_dir().join("consumer/limen.toml").is_file());

        // Lockfile has both, with digests.
        let locked = reg.list().unwrap();
        assert_eq!(locked.len(), 2);
        assert!(locked.iter().all(|e| e.digest.starts_with("sha256:")));

        // No temp dirs left behind.
        assert!(!home.join(".tmp").exists());

        // Remove one and confirm it's gone from disk and lock.
        assert!(reg.remove("provider").unwrap());
        assert!(!reg.modules_dir().join("provider").exists());
        assert_eq!(reg.list().unwrap().len(), 1);

        std::fs::remove_dir_all(&fixtures).ok();
        std::fs::remove_dir_all(&home).ok();
    }
}
