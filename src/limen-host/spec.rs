//! What a module is: its manifest read into a spec, and the order they load in.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use anyhow::{anyhow, bail, Result};
use limen_proto::{Abi, Language, Manifest, Permissions};


/// How a module is launched, chosen from its manifest.
#[derive(Debug, Clone)]
pub enum Launch {
    /// A scripted module: run `<interpreter> <script>`. The interpreter is
    /// resolved at start time (bundled or system), so a missing interpreter is
    /// non-fatal — the module is skipped and Quick Setup is offered.
    Script {
        runtime: crate::runtimes::Runtime,
        script: String,
    },
    /// A compiled binary that speaks JSON-RPC over stdio (path to run).
    Binary(String),
    /// A dynamic library loaded in-process (path to the `.so`/`.dll`/`.dylib`).
    Native(String),
    /// The manifest read, but there is nothing to launch — the library was not
    /// built, the entry names a file that is not there. The module is still
    /// listed, because a module the user installed and cannot see is worse than
    /// one that says why it will not run.
    Unavailable(String),
}

/// Everything the host needs to launch and wire one module — derived from its
/// `limen.toml`.
#[derive(Debug, Clone)]
pub struct ModuleSpec {
    pub name: String,
    /// Pretty display name for the module list (falls back to `name`).
    pub display_name: Option<String>,
    pub version: String,
    /// One-line human description.
    pub description: Option<String>,
    /// Module authors.
    pub authors: Vec<String>,
    /// Free-form tags for grouping/filtering in the module manager.
    pub tags: Vec<String>,
    /// GitHub repo (`owner/repo` or full URL), if the module has one.
    pub repo: Option<String>,
    pub capabilities: Vec<String>,
    /// capability -> semver requirement (hard dependencies).
    pub requires: BTreeMap<String, String>,
    /// Optional capabilities: used if a provider is loaded, never required.
    pub optional: BTreeMap<String, String>,
    /// What the module declares it needs to do.
    pub permissions: Permissions,
    /// Implementation language (selects the SDK search-path env on spawn).
    pub language: Language,
    /// How to launch this module.
    pub launch: Launch,
    /// Working directory (the module's own folder).
    pub cwd: PathBuf,
}

impl ModuleSpec {
    /// Build a spec from the `limen.toml` in `dir`.
    pub fn from_manifest_dir(dir: &Path) -> Result<Self> {
        let manifest = Manifest::from_dir(dir)?;
        // A module that cannot be launched is still a module. Failing here
        // would abort the whole load — one un-built folder among ten and the
        // app has no modules at all — so the reason is carried on the spec and
        // surfaced when the user opens it.
        let launch =
            build_launch(dir, &manifest).unwrap_or_else(|e| Launch::Unavailable(format!("{e:#}")));
        Ok(Self {
            name: manifest.module.name,
            display_name: manifest.module.display_name,
            version: manifest.module.version,
            description: manifest.module.description,
            authors: manifest.module.authors,
            tags: manifest.module.tags,
            repo: manifest.module.repo,
            capabilities: manifest.provides.capabilities,
            requires: manifest.requires.capabilities,
            optional: manifest.optional.capabilities,
            permissions: manifest.permissions,
            language: manifest.module.language,
            launch,
            cwd: dir.to_path_buf(),
        })
    }

    /// Whether this module is loaded in-process as a dynamic library. Such
    /// modules can't hot-swap their code, so updating one needs an app restart.
    pub fn is_native_lib(&self) -> bool {
        matches!(self.launch, Launch::Native(_))
    }
}

/// Decide how to launch a module. `native` + `abi = "native"` loads in-process;
/// everything else (scripted languages, and compiled binaries with `abi = rpc`)
/// runs as a subprocess.
pub(crate) fn build_launch(dir: &Path, manifest: &Manifest) -> Result<Launch> {
    match (manifest.module.language, manifest.module.abi) {
        (Language::Native, Abi::Native) => {
            Ok(Launch::Native(resolve_native_lib(dir, &manifest.module.entry)?))
        }
        (Language::Native, _) => {
            // A compiled binary that speaks RPC over stdio.
            Ok(Launch::Binary(resolve_native(dir, &manifest.module.entry)?))
        }
        (lang, _) => {
            // A scripted module: remember its runtime + script; resolve the
            // interpreter at start time.
            let runtime = crate::runtimes::Runtime::for_language(lang)
                .ok_or_else(|| anyhow!("unsupported scripted language for {}", manifest.module.name))?;
            let script = abspath(dir.join(&manifest.module.entry));
            Ok(Launch::Script { runtime, script })
        }
    }
}

/// Locate a native module's **dynamic library**, trying the platform's
/// `lib<name>.so` / `<name>.dll` conventions next to the manifest, then next to
/// the host binary (where Cargo drops build artifacts).
pub(crate) fn resolve_native_lib(dir: &Path, entry: &str) -> Result<String> {
    let candidates = [
        entry.to_string(),
        format!(
            "{}{entry}{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ),
        format!("{entry}{}", std::env::consts::DLL_SUFFIX),
    ];

    // A module is an independent crate: its `cargo build` puts the library in
    // its own target/. Search (in order) the module dir, its build output, then
    // next to the host binary (a prebuilt/release-asset install lands here).
    let mut bases: Vec<PathBuf> = vec![
        dir.to_path_buf(),
        dir.join("target").join("debug"),
        dir.join("target").join("release"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent() {
            bases.push(exe_dir.to_path_buf());
        }

    for base in &bases {
        for name in &candidates {
            let p = base.join(name);
            if p.exists() {
                return Ok(abspath(p));
            }
        }
    }

    // Fallback: scan the dirs for any platform library whose name carries the
    // module's name — so a release-style asset dropped in as-is (e.g.
    // `limen-devices-0.3.0-linux-x86_64.so`) still resolves without renaming.
    // Prefer one that also names this arch (in case several are present).
    let suffix = std::env::consts::DLL_SUFFIX;
    let arch = std::env::consts::ARCH;
    for require_arch in [true, false] {
        for base in &bases {
            let Ok(entries) = std::fs::read_dir(base) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(suffix)
                    && name.contains(entry)
                    && (!require_arch || name.contains(arch))
                {
                    return Ok(abspath(e.path()));
                }
            }
        }
    }

    bail!("could not find native library for {entry:?} — did you `cargo build` the module? (looked in {bases:?})")
}

/// Make a path absolute (lexically, without touching the filesystem), falling
/// back to the original string if that fails.
pub(crate) fn abspath(p: PathBuf) -> String {
    std::path::absolute(&p)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Locate a native (compiled) module's executable: next to its manifest, else
/// next to the host binary (where Cargo puts sibling binaries), else trust PATH.
pub(crate) fn resolve_native(dir: &Path, entry: &str) -> Result<String> {
    let candidates = [
        entry.to_string(),
        format!("{entry}{}", std::env::consts::EXE_SUFFIX),
    ];
    for name in &candidates {
        let p = dir.join(name);
        if p.exists() {
            return Ok(abspath(p));
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent() {
            for name in &candidates {
                let p = exe_dir.join(name);
                if p.exists() {
                    return Ok(abspath(p));
                }
            }
        }
    Ok(entry.to_string())
}

/// Topologically sort modules so every provider starts before its dependents,
/// validating that each required capability exists and satisfies its semver
/// requirement. Rejects duplicate providers and dependency cycles.
/// Resolve module startup order. A module that can't be satisfied — a required
/// capability with no (working) provider, a semver mismatch, a duplicate
/// capability, or a dependency cycle — is **isolated**: recorded in the returned
/// `failed` map and excluded from the order, rather than failing the whole engine.
/// The returned specs include the failed modules (appended, unordered) so the GUI
/// still lists them and shows the reason in their tab.
pub(crate) fn resolve_order(specs: &[ModuleSpec]) -> (Vec<ModuleSpec>, HashMap<String, String>) {
    let mut failed: HashMap<String, String> = HashMap::new();

    // capability -> index of the module providing it. A duplicate provider is a
    // conflict: keep the first, fail the later one.
    let mut provider: HashMap<&str, usize> = HashMap::new();
    for (i, spec) in specs.iter().enumerate() {
        for cap in &spec.capabilities {
            match provider.get(cap.as_str()) {
                Some(prev) => {
                    failed.insert(
                        spec.name.clone(),
                        format!("capability {cap} is already provided by **{}**", specs[*prev].name),
                    );
                }
                None => {
                    provider.insert(cap, i);
                }
            }
        }
    }

    // Fail any module whose requirements can't be met — repeat until stable, so a
    // failed provider cascades to everything that depends on it.
    loop {
        let mut changed = false;
        for spec in specs.iter() {
            if failed.contains_key(&spec.name) {
                continue;
            }
            let mut reason = None;
            for (cap, req) in &spec.requires {
                reason = match provider.get(cap.as_str()) {
                    None => Some(format!("requires capability **{cap}**, but no module provides it")),
                    Some(&j) if failed.contains_key(&specs[j].name) => {
                        Some(format!("requires {cap} from **{}**, which failed to load", specs[j].name))
                    }
                    Some(&j) => match (
                        semver::VersionReq::parse(req),
                        semver::Version::parse(&specs[j].version),
                    ) {
                        (Ok(rq), Ok(hv)) if !rq.matches(&hv) => Some(format!(
                            "requires {cap} {req}, but **{}** is v{}",
                            specs[j].name, specs[j].version
                        )),
                        (Err(_), _) => Some(format!("bad version requirement {req:?} for {cap}")),
                        (_, Err(_)) => Some(format!("**{}** has invalid version", specs[j].name)),
                        _ => None,
                    },
                };
                if reason.is_some() {
                    break;
                }
            }
            if let Some(r) = reason {
                failed.insert(spec.name.clone(), r);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Dependencies (among still-good modules), for the topological sort.
    let is_good = |i: usize, failed: &HashMap<String, String>| !failed.contains_key(&specs[i].name);
    let deps: Vec<Vec<usize>> = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            if !is_good(i, &failed) {
                return Vec::new();
            }
            spec.requires
                .keys()
                .filter_map(|cap| provider.get(cap.as_str()).copied())
                .filter(|&j| is_good(j, &failed))
                .collect()
        })
        .collect();

    // Kahn-style topo sort: emit a good module once all its deps are emitted.
    // Anything left over is in a cycle — fail it.
    let mut order_idx: Vec<usize> = Vec::new();
    let mut emitted: std::collections::HashSet<usize> = std::collections::HashSet::new();
    loop {
        let mut added = false;
        #[allow(clippy::needless_range_loop)]
        for i in 0..specs.len() {
            if !is_good(i, &failed) || emitted.contains(&i) {
                continue;
            }
            if deps[i].iter().all(|j| emitted.contains(j)) {
                order_idx.push(i);
                emitted.insert(i);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    for (i, spec) in specs.iter().enumerate() {
        if is_good(i, &failed) && !emitted.contains(&i) {
            failed.insert(spec.name.clone(), "part of a dependency cycle".to_string());
        }
    }

    // Good modules in dependency order, then the failed ones (still listed).
    let mut result: Vec<ModuleSpec> = order_idx.into_iter().map(|i| specs[i].clone()).collect();
    for spec in specs {
        if failed.contains_key(&spec.name) {
            result.push(spec.clone());
        }
    }
    (result, failed)
}

// --------------------------------------------------------------------------- //
// Language SDK injection.
//
// The scripted-language SDKs are embedded in the host binary and extracted to
// ~/.limen/sdk/<lang>/ at startup, so a module can `import limen_sdk` (etc.)
// with no vendoring. When spawning a module we set the interpreter's search-path
// env var to that directory.
// --------------------------------------------------------------------------- //
