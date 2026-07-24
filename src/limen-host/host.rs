//! The [`Host`]: load modules, resolve their dependency graph, launch them, and
//! expose a simple `invoke(capability, method, params)` entry point.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use limen_proto::rpc::METHOD_NOT_FOUND;
use limen_proto::{Abi, Language, Manifest, Permissions, RpcError};
use serde_json::{json, Value};

use crate::broker::Broker;
use crate::connection::ModuleConnection;
use crate::module::{stderr_logger, IncomingHandler, Logger, Module};
use crate::native::NativeModule;

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
}

/// Everything the host needs to launch and wire one module — derived from its
/// `limen.toml`.
#[derive(Debug, Clone)]
pub struct ModuleSpec {
    pub name: String,
    pub version: String,
    /// One-line human description.
    pub description: Option<String>,
    /// Module authors.
    pub authors: Vec<String>,
    /// GitHub repo (`owner/repo` or full URL), if the module has one.
    pub repo: Option<String>,
    pub capabilities: Vec<String>,
    /// capability -> semver requirement.
    pub requires: BTreeMap<String, String>,
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
        let launch = build_launch(dir, &manifest)?;
        Ok(Self {
            name: manifest.module.name,
            version: manifest.module.version,
            description: manifest.module.description,
            authors: manifest.module.authors,
            repo: manifest.module.repo,
            capabilities: manifest.provides.capabilities,
            requires: manifest.requires.capabilities,
            permissions: manifest.permissions,
            language: manifest.module.language,
            launch,
            cwd: dir.to_path_buf(),
        })
    }
}

/// Decide how to launch a module. `native` + `abi = "native"` loads in-process;
/// everything else (scripted languages, and compiled binaries with `abi = rpc`)
/// runs as a subprocess.
fn build_launch(dir: &Path, manifest: &Manifest) -> Result<Launch> {
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
fn resolve_native_lib(dir: &Path, entry: &str) -> Result<String> {
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            bases.push(exe_dir.to_path_buf());
        }
    }

    for base in &bases {
        for name in &candidates {
            let p = base.join(name);
            if p.exists() {
                return Ok(abspath(p));
            }
        }
    }
    bail!("could not find native library for {entry:?} — did you `cargo build` the module? (looked in {bases:?})")
}

/// Make a path absolute (lexically, without touching the filesystem), falling
/// back to the original string if that fails.
fn abspath(p: PathBuf) -> String {
    std::path::absolute(&p)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Locate a native (compiled) module's executable: next to its manifest, else
/// next to the host binary (where Cargo puts sibling binaries), else trust PATH.
fn resolve_native(dir: &Path, entry: &str) -> Result<String> {
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
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for name in &candidates {
                let p = exe_dir.join(name);
                if p.exists() {
                    return Ok(abspath(p));
                }
            }
        }
    }
    Ok(entry.to_string())
}

pub struct Host {
    broker: Arc<Broker>,
    order: Vec<ModuleSpec>,
    connections: Vec<Arc<dyn Module>>,
    logger: Logger,
    /// Runtimes that were needed but unavailable, so a module was skipped.
    missing_runtimes: Vec<crate::runtimes::Runtime>,
}

impl Host {
    /// Load module manifests from the given directories and resolve their
    /// startup order. Does not spawn anything yet — call [`Host::start`].
    pub fn load(dirs: &[PathBuf]) -> Result<Self> {
        let mut specs = Vec::with_capacity(dirs.len());
        for dir in dirs {
            specs.push(
                ModuleSpec::from_manifest_dir(dir)
                    .with_context(|| format!("loading module at {}", dir.display()))?,
            );
        }
        let order = resolve_order(&specs)?;
        Ok(Self {
            broker: Broker::new(),
            order,
            connections: Vec::new(),
            logger: stderr_logger(),
            missing_runtimes: Vec::new(),
        })
    }

    /// Install a log sink for host + module log lines (defaults to stderr).
    /// Call before [`Host::start`] to capture startup logs.
    pub fn set_logger(&mut self, logger: Logger) {
        self.logger = logger;
    }

    /// Runtimes that were needed by a module but unavailable at start (so the
    /// module was skipped). Drives the GUI's Quick Setup prompt.
    pub fn missing_runtimes(&self) -> &[crate::runtimes::Runtime] {
        &self.missing_runtimes
    }

    /// Spawn every module in dependency order, register its capabilities, and
    /// `initialize` it.
    pub fn start(&mut self) -> Result<()> {
        // Make the embedded language SDKs available on disk so scripted modules
        // can `import limen_sdk` (etc.) without vendoring anything.
        let sdk = install_sdks()?;

        let logger = self.logger.clone();
        let handler: Arc<IncomingHandler> = {
            let broker = self.broker.clone();
            let logger = logger.clone();
            Arc::new(move |method: &str, params: Value| {
                host_handler(&broker, &logger, method, params)
            })
        };

        self.missing_runtimes.clear();
        for spec in &self.order {
            let conn: Arc<dyn Module> = match &spec.launch {
                Launch::Script { runtime, script } => {
                    // Resolve the interpreter now; if missing, skip the module
                    // (non-fatal) and record the runtime for Quick Setup.
                    match crate::runtimes::resolve(&limen_home(), *runtime) {
                        Some(interp) => {
                            let env = sdk.env_for(spec.language);
                            let argv = [interp, script.clone()];
                            ModuleConnection::spawn(
                                spec.name.clone(),
                                &argv,
                                Some(&spec.cwd),
                                &env,
                                handler.clone(),
                                logger.clone(),
                            )
                            .with_context(|| format!("spawning module {}", spec.name))?
                        }
                        None => {
                            logger(&format!(
                                "[host] skipping {}: no {} interpreter — run Quick Setup",
                                spec.name,
                                runtime.display()
                            ));
                            if !self.missing_runtimes.contains(runtime) {
                                self.missing_runtimes.push(*runtime);
                            }
                            continue;
                        }
                    }
                }
                Launch::Binary(path) => {
                    let argv = [path.clone()];
                    ModuleConnection::spawn(
                        spec.name.clone(),
                        &argv,
                        Some(&spec.cwd),
                        &[],
                        handler.clone(),
                        logger.clone(),
                    )
                    .with_context(|| format!("spawning module {}", spec.name))?
                }
                Launch::Native(path) => NativeModule::load(spec.name.clone(), path, handler.clone())
                    .with_context(|| format!("loading native module {}", spec.name))?,
            };

            for capability in &spec.capabilities {
                self.broker.register(capability, conn.clone());
            }
            self.broker.register_name(&spec.name, conn.clone());

            let info = conn
                .call(
                    "initialize",
                    json!({
                        "host": { "name": "limen", "version": env!("CARGO_PKG_VERSION") },
                        "module": { "name": spec.name, "version": spec.version },
                        "capabilities": spec.capabilities,
                    }),
                )
                .map_err(|e| anyhow!("initialize {}: {e}", spec.name))?;

            logger(&format!(
                "[host] started {} v{} caps={:?} -> {info}",
                spec.name, spec.version, spec.capabilities
            ));
            self.connections.push(conn);
        }
        Ok(())
    }

    /// Invoke `method` on whichever module provides `capability`.
    pub fn invoke(&self, capability: &str, method: &str, params: Value) -> Result<Value> {
        let conn = self
            .broker
            .get(capability)
            .ok_or_else(|| anyhow!("no provider for capability {capability}"))?;
        conn.call(
            "invoke",
            json!({ "capability": capability, "method": method, "params": params }),
        )
        .map_err(|e| anyhow!("invoke {capability}.{method}: {e}"))
    }

    /// The resolved module specs, in startup order (for listing / inspection).
    pub fn module_specs(&self) -> &[ModuleSpec] {
        &self.order
    }

    /// Ask the module providing `capability` to describe itself.
    pub fn describe(&self, capability: &str) -> Result<Value> {
        let conn = self
            .broker
            .get(capability)
            .ok_or_else(|| anyhow!("no provider for capability {capability}"))?;
        conn.call("describe", Value::Null)
            .map_err(|e| anyhow!("describe {capability}: {e}"))
    }

    /// Shut modules down in reverse dependency order.
    pub fn shutdown(&mut self) {
        for conn in self.connections.iter().rev() {
            conn.shutdown();
        }
        self.connections.clear();
    }
}

/// Dispatch a module→host request. Phase 1 supports `host.call` (broker routing)
/// and `host.log`.
fn host_handler(
    broker: &Broker,
    logger: &Logger,
    method: &str,
    params: Value,
) -> std::result::Result<Value, RpcError> {
    match method {
        "host.call" => broker.route(params),
        "host.subscribe" => broker.subscribe(params),
        "host.emit" => broker.emit(params),
        "host.log" => {
            let msg = params.as_str().map(str::to_string).unwrap_or_else(|| params.to_string());
            logger(&format!("[module] {msg}"));
            Ok(Value::Null)
        }
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("unknown host method {other}"),
        )),
    }
}

/// Topologically sort modules so every provider starts before its dependents,
/// validating that each required capability exists and satisfies its semver
/// requirement. Rejects duplicate providers and dependency cycles.
fn resolve_order(specs: &[ModuleSpec]) -> Result<Vec<ModuleSpec>> {
    // capability -> index of the module providing it.
    let mut provider: HashMap<&str, usize> = HashMap::new();
    for (i, spec) in specs.iter().enumerate() {
        for cap in &spec.capabilities {
            if let Some(prev) = provider.insert(cap, i) {
                bail!(
                    "capability {cap} is provided by both {} and {}",
                    specs[prev].name,
                    spec.name
                );
            }
        }
    }

    // Build dependency edges (i depends on j) with semver validation.
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); specs.len()];
    for (i, spec) in specs.iter().enumerate() {
        for (cap, req) in &spec.requires {
            let j = *provider.get(cap.as_str()).ok_or_else(|| {
                anyhow!("module {} requires capability {cap}, but no module provides it", spec.name)
            })?;
            let req = semver::VersionReq::parse(req)
                .with_context(|| format!("bad version requirement {req:?} in module {}", spec.name))?;
            let have = semver::Version::parse(&specs[j].version).with_context(|| {
                format!("module {} has invalid version {:?}", specs[j].name, specs[j].version)
            })?;
            if !req.matches(&have) {
                bail!(
                    "module {} requires {cap} {req}, but provider {} is v{}",
                    spec.name,
                    specs[j].name,
                    specs[j].version
                );
            }
            deps[i].push(j);
        }
    }

    // DFS topological sort with cycle detection.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Unvisited,
        InProgress,
        Done,
    }
    let mut marks = vec![Mark::Unvisited; specs.len()];
    let mut order = Vec::with_capacity(specs.len());

    fn visit(
        i: usize,
        deps: &[Vec<usize>],
        marks: &mut [Mark],
        order: &mut Vec<usize>,
        specs: &[ModuleSpec],
    ) -> Result<()> {
        match marks[i] {
            Mark::Done => return Ok(()),
            Mark::InProgress => bail!("dependency cycle involving module {}", specs[i].name),
            Mark::Unvisited => {}
        }
        marks[i] = Mark::InProgress;
        for &j in &deps[i] {
            visit(j, deps, marks, order, specs)?;
        }
        marks[i] = Mark::Done;
        order.push(i);
        Ok(())
    }

    for i in 0..specs.len() {
        visit(i, &deps, &mut marks, &mut order, specs)?;
    }

    Ok(order.into_iter().map(|i| specs[i].clone()).collect())
}

// --------------------------------------------------------------------------- //
// Language SDK injection.
//
// The scripted-language SDKs are embedded in the host binary and extracted to
// ~/.limen/sdk/<lang>/ at startup, so a module can `import limen_sdk` (etc.)
// with no vendoring. When spawning a module we set the interpreter's search-path
// env var to that directory.
// --------------------------------------------------------------------------- //

const PY_SDK: &str = include_str!("../../sdk/python/limen_sdk.py");

/// Paths to the extracted SDKs, and the env each language needs to find them.
struct SdkPaths {
    python: PathBuf,
}

impl SdkPaths {
    /// The env vars to set when spawning a module of `language`.
    fn env_for(&self, language: Language) -> Vec<(String, String)> {
        match language {
            Language::Python => vec![(
                "PYTHONPATH".to_string(),
                self.python.to_string_lossy().into_owned(),
            )],
            // Lua/JS SDKs land here once written.
            _ => Vec::new(),
        }
    }
}

/// Extract the embedded SDKs under `<limen home>/sdk/` and return their paths.
fn install_sdks() -> Result<SdkPaths> {
    let base = limen_home().join("sdk");
    let python = base.join("python");
    std::fs::create_dir_all(&python)
        .with_context(|| format!("creating {}", python.display()))?;
    std::fs::write(python.join("limen_sdk.py"), PY_SDK)
        .context("writing embedded python SDK")?;
    Ok(SdkPaths { python })
}

/// The Limen base dir: `$LIMEN_HOME`, else the executable's directory (portable).
/// Kept local so limen-host needn't depend on limen-core (must match
/// `limen_core::paths::home`).
fn limen_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("LIMEN_HOME") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}
