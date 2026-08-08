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
        let launch = build_launch(dir, &manifest)?;
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

pub struct Host {
    broker: Arc<Broker>,
    order: Vec<ModuleSpec>,
    connections: Vec<Arc<dyn Module>>,
    logger: Logger,
    /// Runtimes that were needed but unavailable, so a module was skipped.
    missing_runtimes: Vec<crate::runtimes::Runtime>,
    /// Modules that failed to start (spawn / load / initialize): name -> error.
    /// A failure here is isolated — other modules still start.
    failed: HashMap<String, String>,
}

impl Host {
    /// Load module manifests from the given directories and resolve their
    /// startup order. Does not spawn anything yet — call [`Host::start`].
    pub fn load(dirs: &[PathBuf]) -> Result<Self> {
        let mut specs = Vec::with_capacity(dirs.len());
        let mut seen_names = std::collections::HashSet::new();
        for dir in dirs {
            let spec = ModuleSpec::from_manifest_dir(dir)
                .with_context(|| format!("loading module at {}", dir.display()))?;
            // The same module can appear in several search dirs (e.g. the portable
            // base and a local ./modules). Keep the first; skip re-discoveries so
            // it isn't mistaken for a duplicate-capability conflict.
            if seen_names.insert(spec.name.clone()) {
                specs.push(spec);
            }
        }
        let (order, failed) = resolve_order(&specs);
        Ok(Self {
            broker: Broker::new(),
            order,
            connections: Vec::new(),
            logger: stderr_logger(),
            missing_runtimes: Vec::new(),
            failed,
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

    /// Modules that failed to start (name -> error). Their failure is isolated;
    /// the rest of the engine runs. The GUI shows the error in the module's tab.
    pub fn failed_modules(&self) -> &HashMap<String, String> {
        &self.failed
    }

    /// Runtimes used by loaded scripted modules that are **not yet bundled** under
    /// `<base>/runtimes/`. Installing these makes the app self-contained (portable)
    /// so it no longer depends on a system interpreter — even when one is present.
    pub fn unbundled_runtimes(&self) -> Vec<crate::runtimes::Runtime> {
        let base = limen_home();
        let mut out: Vec<crate::runtimes::Runtime> = Vec::new();
        for spec in &self.order {
            if let Launch::Script { runtime, .. } = &spec.launch
                && crate::runtimes::bundled(&base, *runtime).is_none()
                && !out.contains(runtime)
            {
                out.push(*runtime);
            }
        }
        out
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
        // Note: `self.failed` is NOT cleared — `load` already recorded modules
        // whose dependencies can't be satisfied; we keep those and add any that
        // fail to spawn/init below.
        // Clone the order so we can mutate self (broker, connections, …) per module.
        let order = self.order.clone();
        for spec in &order {
            // Skip modules load already flagged (unsatisfiable deps): they aren't
            // started, but stay listed so the GUI shows the reason in their tab.
            if self.failed.contains_key(&spec.name) {
                continue;
            }
            if let Err(e) = self.start_one(spec, &sdk, &handler, &logger) {
                let msg = format!("{e:#}");
                logger(&format!("[host] {} failed to start: {msg}", spec.name));
                self.failed.insert(spec.name.clone(), msg);
            }
        }
        Ok(())
    }

    /// Start a single module: spawn/load, `initialize`, then register it. A
    /// missing interpreter is a non-fatal skip (recorded in `missing_runtimes`);
    /// any other failure returns an error the caller records in `failed`, so one
    /// broken module never stops the rest of the engine.
    fn start_one(
        &mut self,
        spec: &ModuleSpec,
        sdk: &SdkPaths,
        handler: &Arc<IncomingHandler>,
        logger: &Logger,
    ) -> Result<()> {
        // Give this module a handler that knows where the module lives. The
        // shared dispatcher is one closure for every module and so has no idea
        // who is calling; a module that manages content of its own — a fetched
        // tool under `tools/` — has to be able to find its own directory.
        let handler: Arc<IncomingHandler> = {
            let shared = handler.clone();
            let dir = spec.cwd.clone();
            // Whether this module may ask for elevation is a property of *this*
            // module, so the check lives here rather than in the shared
            // dispatcher, which cannot tell who is calling.
            let may_elevate = spec.permissions.elevate;
            // Elevation is the one thing here the user cannot watch happen: a
            // prompt they answered, a command they never saw, run as root. It
            // goes to the console so there is a record of what was asked for.
            let log = logger.clone();
            let who = spec.name.clone();
            Arc::new(move |method: &str, params: Value| match method {
                "host.module_dir" => Ok(json!(dir.to_string_lossy())),
                "host.elevate" => host_elevate(params, may_elevate, &log, &who),
                "host.can_elevate" => Ok(host_can_elevate()),
                "host.elevate_status" => host_elevate_status(params),
                "host.elevate_stop" => host_elevate_stop(params, &log, &who),
                _ => shared(method, params),
            })
        };
        let handler = &handler;

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
                        return Ok(());
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

        // Initialize BEFORE registering, so a module that fails to init is never
        // left in the broker as a dead provider.
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

        for capability in &spec.capabilities {
            self.broker.register(capability, conn.clone());
        }
        self.broker.register_name(&spec.name, conn.clone());

        logger(&format!(
            "[host] started {} v{} caps={:?} -> {info}",
            spec.name, spec.version, spec.capabilities
        ));
        self.connections.push(conn);
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
        // Anything elevated on a module's behalf is still running, and closing
        // the window is not a reason to leave a root process chewing through a
        // disk with nothing left to report to.
        stop_all_elevations(&self.logger);
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
        "host.about" => Ok(host_about()),
        "host.capabilities" => Ok(json!(broker.capabilities())),
        "host.locale" => Ok(json!(limen_proto::locale::current())),
        "host.open" => host_open(params),
        "host.notify" => host_notify(params),
        "host.pick_file" => Ok(host_pick_file()),
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

/// Raise a desktop notification on the machine running Limen, for work the user
/// is not sitting and watching — a scan that has finished, an install that has
/// landed. `params`:
/// `{ "title": "...", "body": "...", "urgency": "low"|"normal"|"critical" }`.
///
/// Best-effort and fire-and-forget, like [`host_open`]: a desktop with no
/// notification daemon, or a locked-down session, is not a module error. Going
/// through the host rather than each module shelling out for itself means one
/// implementation to keep working per platform, and one place to put policy.
/// Run a command with administrator / root privileges, letting the operating
/// system ask the user for them.
///
/// Limen itself stays unprivileged. What is elevated is the child — the same
/// shape as launching `regedit` through `ShellExecuteW`, and the reason this
/// belongs to the host rather than to each module: elevation is gated by a
/// declared permission, the argument vector never goes near a shell, and there
/// is one implementation to get right per platform instead of one per module.
///
/// Blocks until the command finishes and returns its exit status, so a module
/// can tell "the user said no" from "it ran and failed". Callers must therefore
/// invoke it off any thread that draws.
/// Elevations still running, so a module can start one and keep drawing.
///
/// A module's `Host` handle holds a raw pointer and cannot cross threads, so the
/// module cannot wait on this itself without blocking every other call it makes
/// — including the one that draws its progress. The host does the waiting and
/// the module asks how it went.
type Elevations = std::sync::Mutex<std::collections::HashMap<u64, Arc<std::sync::Mutex<Value>>>>;

fn elevations() -> &'static Elevations {
    static E: std::sync::OnceLock<Elevations> = std::sync::OnceLock::new();
    E.get_or_init(Default::default)
}

/// End every elevation still running, on the way out.
///
/// They are our own children, but root ones — the kernel refuses an
/// unprivileged signal — so this asks for the privileges to end them, which
/// polkit usually grants without a second prompt so soon after the first.
fn stop_all_elevations(log: &Logger) {
    let live: Vec<(u64, u32)> = elevations()
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(id, st)| {
            let v = st.lock().unwrap();
            (v.get("running").and_then(Value::as_bool) == Some(true))
                .then(|| v.get("pid").and_then(Value::as_u64).map(|p| (*id, p as u32)))
                .flatten()
        })
        .collect();
    for (_, pid) in &live {
        log(&format!("[elevate] closing: stopping {pid}"));
        kill_pid(*pid);
    }
    if live.is_empty() {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(150));

    // Whatever is left is running as root, and an unprivileged parent cannot
    // signal it — so ask for the privileges to end it. Closing the window has to
    // mean the scan is over: leaving one grinding through a disk with nothing
    // left to report to is worse than a prompt on the way out.
    //
    // In practice there is usually no prompt: polkit keeps the authorization
    // from the one that started the scan for a few minutes. If there is, and it
    // goes unanswered, the wait below gives up and the app closes anyway.
    #[cfg(unix)]
    {
        let survivors: Vec<u32> = live
            .iter()
            .map(|(_, pid)| *pid)
            .filter(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
            .collect();
        if survivors.is_empty() {
            return;
        }
        let kill = program_on_path("kill").unwrap_or_else(|| std::path::PathBuf::from("/bin/kill"));
        for pid in &survivors {
            log(&format!("[elevate] closing: asking to stop {pid} as root"));
            let argv = vec![
                kill.to_string_lossy().into_owned(),
                "-TERM".to_string(),
                pid.to_string(),
            ];
            start_elevation(argv, None, log, "closing");
        }
        // Bounded: a prompt nobody answers must not hold the window open.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while std::time::Instant::now() < deadline {
            if survivors
                .iter()
                .all(|pid| !std::path::Path::new(&format!("/proc/{pid}")).exists())
            {
                log("[elevate] closing: stopped");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        for pid in survivors {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                log(&format!(
                    "[elevate] closing: {pid} would not stop — end it with: sudo kill {pid}"
                ));
            }
        }
    }
}

/// Stop an elevation that is still running.
///
/// Best effort, and honest about it: once authorized the program runs as root,
/// and an unprivileged process cannot signal one — the kernel refuses. So this
/// reports whether it actually stopped, and a caller that gets `false` has to
/// say so rather than pretend.
fn host_elevate_stop(params: Value, log: &Logger, who: &str) -> std::result::Result<Value, RpcError> {
    let id = params.get("id").and_then(Value::as_u64).unwrap_or(0);
    let slot = elevations().lock().unwrap().get(&id).cloned();
    let Some(state) = slot else {
        return Ok(json!({ "stopped": false }));
    };
    let pid = state.lock().unwrap().get("pid").and_then(Value::as_u64);
    let Some(pid) = pid.filter(|p| *p > 0) else {
        // Nothing to signal — macOS runs it inside osascript, which gives us no
        // handle at all.
        return Ok(json!({ "stopped": false }));
    };
    let still_running = |state: &Arc<std::sync::Mutex<Value>>| {
        state.lock().unwrap().get("running").and_then(Value::as_bool) == Some(true)
    };

    // The polite attempt first: it works when the command was never elevated,
    // or when we are root already.
    kill_pid(pid as u32);
    std::thread::sleep(std::time::Duration::from_millis(250));
    if !still_running(&state) {
        log(&format!("[elevate] {who}: stopped {pid}"));
        return Ok(json!({ "stopped": true }));
    }

    // It is running as root, so the kernel refused us. Ask for the privileges to
    // end it — in the background, with an id to poll, because the operating
    // system may put a prompt on screen and the caller has to be able to say so
    // rather than freeze with nothing showing.
    log(&format!(
        "[elevate] {who}: {pid} would not stop unprivileged, asking to stop it as root"
    ));
    let kill = program_on_path("kill").unwrap_or_else(|| std::path::PathBuf::from("/bin/kill"));
    let argv = vec![
        kill.to_string_lossy().into_owned(),
        "-TERM".to_string(),
        pid.to_string(),
    ];
    let pending = start_elevation(argv, None, log, who);
    Ok(json!({ "stopped": false, "pending": pending }))
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    use limen_proto::NoConsole;
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .no_console()
        .status();
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    use limen_proto::NoConsole;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .no_console()
        .status();
}

/// How an elevation started with `wait: false` is going.
fn host_elevate_status(params: Value) -> std::result::Result<Value, RpcError> {
    let id = params.get("id").and_then(Value::as_u64).unwrap_or(0);
    let slot = elevations().lock().unwrap().get(&id).cloned();
    match slot {
        Some(state) => {
            let v = state.lock().unwrap().clone();
            // Finished results are dropped once collected — a module that polls
            // forever should not pin them, and there is nothing more to say.
            if v.get("running").and_then(Value::as_bool) == Some(false) {
                elevations().lock().unwrap().remove(&id);
            }
            Ok(v)
        }
        None => Ok(json!({ "running": false, "ran": false, "reason": "error",
                           "message": "no such elevation" })),
    }
}

fn host_elevate(
    params: Value,
    may_elevate: bool,
    log: &Logger,
    who: &str,
) -> std::result::Result<Value, RpcError> {
    if !may_elevate {
        log(&format!("[elevate] {who}: refused, no `elevate` permission"));
        return Err(RpcError::new(
            limen_proto::rpc::INVALID_REQUEST,
            "this module does not declare the `elevate` permission",
        ));
    }
    let argv: Vec<String> = params
        .get("argv")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if argv.is_empty() {
        return Err(RpcError::new(limen_proto::rpc::INVALID_PARAMS, "argv must be non-empty"));
    }
    let cwd = params.get("cwd").and_then(Value::as_str).map(str::to_string);
    // The whole command, so a scan that fails inside an elevated child can be
    // reproduced by hand from the console.
    log(&format!(
        "[elevate] {who}: {}{}",
        argv.join(" "),
        cwd.as_deref()
            .map(|d| format!("   (in {d})"))
            .unwrap_or_default()
    ));

    // The default waits and returns the outcome, which is what a short command
    // wants. `wait: false` returns an id instead, for something long enough that
    // the caller has to stay responsive while it runs — the authorization prompt
    // and then, often, minutes of work.
    if params.get("wait").and_then(Value::as_bool).unwrap_or(true) {
        return elevate_native(&argv, cwd.as_deref(), &|| {}, &|_| {});
    }

    Ok(json!({ "id": start_elevation(argv, cwd, log, who), "running": true }))
}

/// Start an elevated command in the background and return the id to poll it by.
fn start_elevation(argv: Vec<String>, cwd: Option<String>, log: &Logger, who: &str) -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // `phase` is what a caller shows: while the prompt is up there is nothing to
    // report but the asking, and once the program is running its own progress
    // takes over. Guessing at that from the program's output is what a caller
    // has to do otherwise, and it guesses wrong — output can lag the prompt by
    // seconds, leaving "waiting for authorization" on screen long after it was
    // given.
    let state = Arc::new(std::sync::Mutex::new(json!({
        "running": true, "phase": "authorizing",
        "ran": false, "reason": "", "message": ""
    })));
    elevations().lock().unwrap().insert(id, state.clone());

    let log = log.clone();
    let who = who.to_string();
    std::thread::spawn(move || {
        let running = state.clone();
        let announce = log.clone();
        let name = who.clone();
        let started = move || {
            let mut slot = running.lock().unwrap();
            slot["phase"] = json!("running");
            announce(&format!("[elevate] {name}: authorized, running"));
        };
        let noted = state.clone();
        let note_pid = move |p: u32| {
            noted.lock().unwrap()["pid"] = json!(p);
        };
        let done = elevate_native(&argv, cwd.as_deref(), &started, &note_pid)
            .unwrap_or_else(|e| elevate_result(false, None, "error", &e.to_string()));
        log(&format!(
            "[elevate] {who}: {} (code {:?}){}",
            done.get("reason")
                .and_then(Value::as_str)
                .filter(|r| !r.is_empty())
                .unwrap_or("ok"),
            done.get("code").and_then(Value::as_i64),
            done.get("message")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map(|m| format!(" — {m}"))
                .unwrap_or_default()
        ));
        let mut slot = state.lock().unwrap();
        *slot = done;
        slot["running"] = json!(false);
        slot["phase"] = json!("done");
    });
    id
}

/// Result shape shared by every platform.
///
/// `reason` is a fixed word rather than prose because the caller has to be able
/// to act on it and to say it in the user's language: "" (ran cleanly),
/// `unavailable` (nothing on this machine can ask), `refused` (the user said
/// no), `failed` (it ran and exited non-zero), `error` (it could not start).
/// `message` is the English detail, for logs and as a fallback.
fn elevate_result(ran: bool, code: Option<i32>, reason: &str, message: &str) -> Value {
    json!({ "ran": ran, "code": code, "reason": reason, "message": message })
}

/// Find an executable on `PATH`.
///
/// Nothing is assumed to be installed: a minimal or hardened Linux may have
/// neither polkit nor sudo, and the honest answer there is to say so rather than
/// to run the command unprivileged and let the caller believe otherwise.
#[cfg(unix)]
fn program_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

/// Whether this machine has any way to ask the user for privileges.
///
/// Offered separately so a module can tell the user *before* they choose an
/// action that needs it, rather than after the attempt fails.
fn host_can_elevate() -> Value {
    #[cfg(target_os = "linux")]
    {
        // Already root: nothing to ask for.
        if std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(2).map(|u| u == "0"))
            })
            .unwrap_or(false)
        {
            return json!({ "available": true, "how": "already-root" });
        }
        if program_on_path("pkexec").is_some() {
            return json!({ "available": true, "how": "pkexec" });
        }
        // sudo is only usable without a terminal if a graphical askpass is
        // configured; otherwise it would sit waiting for input nobody can give.
        if program_on_path("sudo").is_some() && std::env::var_os("SUDO_ASKPASS").is_some() {
            return json!({ "available": true, "how": "sudo-askpass" });
        }
        json!({ "available": false, "how": "" })
    }

    // Both have it built in — osascript and the UAC prompt.
    #[cfg(target_os = "macos")]
    {
        json!({ "available": true, "how": "osascript" })
    }
    #[cfg(target_os = "windows")]
    {
        json!({ "available": true, "how": "runas" })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        json!({ "available": false, "how": "" })
    }
}

/// Linux: polkit. `pkexec` shows the desktop's own authentication dialog and
/// runs the command as root; the arguments go to execve, never to a shell.
///
/// Note for callers: pkexec deliberately replaces the environment with a minimal
/// one, so a command that resolves anything relative to `PATH` or to its own
/// environment must be given absolute paths. The working directory is set
/// explicitly here for the same reason.
#[cfg(target_os = "linux")]
fn elevate_native(
    argv: &[String],
    cwd: Option<&str>,
    started: &dyn Fn(),
    pid: &dyn Fn(u32),
) -> std::result::Result<Value, RpcError> {
    // Already root — run it directly rather than asking for what we have.
    let how = host_can_elevate();
    let how = how.get("how").and_then(Value::as_str).unwrap_or("");
    // The working directory has to survive the helper.
    //
    // `pkexec` runs the program from root's home unless told otherwise — its
    // `--keep-cwd` exists but not in every polkit — so a program that resolves
    // anything relative to `.` finds nothing and fails in a way that looks like
    // a clean result. `env -C` sets it in the child itself, which works whatever
    // the helper does, and passes the arguments as argv rather than through a
    // shell that would have to quote them.
    let env_bin = program_on_path("env").unwrap_or_else(|| std::path::PathBuf::from("/usr/bin/env"));
    let with_cwd = |c: &mut std::process::Command| {
        if let Some(d) = cwd {
            c.arg(&env_bin).arg("-C").arg(d);
        }
    };

    let mut cmd = match how {
        "already-root" => std::process::Command::new(&argv[0]),
        "pkexec" => {
            let mut c = std::process::Command::new("pkexec");
            // The desktop's own polkit agent shows the dialog; the internal one
            // is a text prompt on a terminal that may not exist.
            c.arg("--disable-internal-agent");
            with_cwd(&mut c);
            c.args(argv);
            c
        }
        "sudo-askpass" => {
            let mut c = std::process::Command::new("sudo");
            // -A uses $SUDO_ASKPASS, a graphical helper; -n would rather fail
            // than block, but with an askpass there is something to answer with.
            c.arg("-A");
            with_cwd(&mut c);
            c.args(argv);
            c
        }
        _ => {
            return Ok(elevate_result(
                false,
                None,
                "unavailable",
                "neither pkexec nor a graphical sudo askpass is available on this machine",
            ))
        }
    };
    if how == "already-root" {
        cmd.args(&argv[1..]);
    }
    // Keep the caller's working directory: elevation helpers do not guarantee
    // one, and a tool that resolves its data relative to `.` would otherwise
    // find nothing and report an empty result rather than an error.
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    // Spawned rather than waited on, so the moment authentication finishes can
    // be noticed: pkexec and sudo both `exec` the target program in their own
    // process, so `/proc/<pid>/comm` changing away from the helper's name is
    // exactly that moment. Nothing else tells us — the prompt belongs to the
    // desktop, not to us.
    // What the process is called until authentication finishes. With a cwd it
    // is `env` that pkexec execs first, and `env` execs the target in turn — so
    // either name means "not started yet".
    let helpers: &[&str] = match (how, cwd.is_some()) {
        ("pkexec", true) => &["pkexec", "env"],
        ("pkexec", false) => &["pkexec"],
        ("sudo-askpass", true) => &["sudo", "env"],
        ("sudo-askpass", false) => &["sudo"],
        _ => &[],
    };
    let helper = helpers.first().copied().unwrap_or("");
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ok(elevate_result(
                false,
                None,
                "error",
                &format!("the elevation helper could not be started: {e}"),
            ))
        }
    };
    if helper.is_empty() {
        // Already root: there was never anything to authorize.
        started();
    }
    pid(child.id());
    let comm = std::path::PathBuf::from(format!("/proc/{}/comm", child.id()));
    let mut announced = helper.is_empty();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Err(e) => break Err(e),
            Ok(None) => {}
        }
        if !announced {
            let now = std::fs::read_to_string(&comm).unwrap_or_default();
            let now = now.trim();
            if !now.is_empty() && !helpers.contains(&now) {
                announced = true;
                started();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    };
    match status {
        Ok(st) => {
            let code = st.code();
            // 126 is polkit's "not authorised" — the prompt was dismissed or the
            // password was wrong. 127 is "could not run it at all".
            let (reason, message) = match code {
                Some(0) => ("", ""),
                Some(126) => ("refused", "authentication was dismissed or refused"),
                Some(127) => ("error", "the command could not be started"),
                _ => ("failed", "the command exited with an error"),
            };
            Ok(elevate_result(
                !matches!(code, Some(126) | Some(127)),
                code,
                reason,
                message,
            ))
        }
        Err(e) => Ok(elevate_result(
            false,
            None,
            "error",
            &format!("the elevation helper could not be started: {e}"),
        )),
    }
}

/// macOS: `osascript` asks for administrator privileges with the system's own
/// dialog. The command is embedded in AppleScript, so each argument is quoted
/// for the shell it ends up in.
#[cfg(target_os = "macos")]
fn elevate_native(
    argv: &[String],
    cwd: Option<&str>,
    _started: &dyn Fn(),
    _pid: &dyn Fn(u32),
) -> std::result::Result<Value, RpcError> {
    // `do shell script` runs its argument through sh, so anything a caller
    // passes has to be quoted rather than trusted — a path with a space in it is
    // the ordinary case, not the attack.
    fn shell_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
    let mut line = String::new();
    if let Some(d) = cwd {
        line.push_str(&format!("cd {} && ", shell_quote(d)));
    }
    line.push_str(
        &argv
            .iter()
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let script = format!(
        "do shell script {} with administrator privileges",
        shell_quote(&line)
    );
    match std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
    {
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            // AppleScript reports a dismissed prompt as error -128.
            let refused = err.contains("-128");
            let (reason, message) = if refused {
                ("refused", "authentication was dismissed")
            } else if out.status.success() {
                ("", "")
            } else {
                ("failed", "the command exited with an error")
            };
            Ok(elevate_result(
                out.status.success(),
                out.status.code(),
                reason,
                message,
            ))
        }
        Err(e) => Ok(elevate_result(
            false,
            None,
            &format!("osascript could not be started: {e}"),
        )),
    }
}

/// Windows: the `runas` verb, which is what raises the UAC prompt — the same
/// route the host already uses to open `regedit`.
///
/// `ShellExecuteExW` rather than `ShellExecuteW` so the process handle comes
/// back and the call can wait for it; fire-and-forget would leave the caller
/// unable to tell a finished scan from a refused prompt.
#[cfg(target_os = "windows")]
fn elevate_native(
    argv: &[String],
    cwd: Option<&str>,
    started: &dyn Fn(),
    pid: &dyn Fn(u32),
) -> std::result::Result<Value, RpcError> {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        lp_verb: *const u16,
        lp_file: *const u16,
        lp_parameters: *const u16,
        lp_directory: *const u16,
        n_show: i32,
        h_inst_app: *mut c_void,
        lp_id_list: *mut c_void,
        lp_class: *const u16,
        hkey_class: *mut c_void,
        dw_hot_key: u32,
        h_icon_or_monitor: *mut c_void,
        h_process: *mut c_void,
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    /// Arguments become one command line, so each is quoted — a path with a
    /// space would otherwise arrive as two arguments.
    fn quote(s: &str) -> String {
        if s.contains(['"', ' ', '\t']) {
            format!("\"{}\"", s.replace('"', "\\\""))
        } else {
            s.to_string()
        }
    }

    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NO_UI: u32 = 0x0000_0400;
    const SW_HIDE: i32 = 0;
    const INFINITE: u32 = 0xFFFF_FFFF;
    const ERROR_CANCELLED: u32 = 1223;

    let verb = wide("runas");
    let file = wide(&argv[0]);
    let params = wide(
        &argv[1..]
            .iter()
            .map(|a| quote(a))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let dir = cwd.map(wide);

    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_UI,
        hwnd: std::ptr::null_mut(),
        lp_verb: verb.as_ptr(),
        lp_file: file.as_ptr(),
        lp_parameters: params.as_ptr(),
        lp_directory: dir.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
        n_show: SW_HIDE,
        h_inst_app: std::ptr::null_mut(),
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: std::ptr::null_mut(),
        dw_hot_key: 0,
        h_icon_or_monitor: std::ptr::null_mut(),
        h_process: std::ptr::null_mut(),
    };

    // SAFETY: every pointer is null or a NUL-terminated UTF-16 buffer that
    // outlives the call, and `cb_size` matches the struct actually passed.
    let ok = unsafe { ShellExecuteExW(&mut info) } != 0;
    if !ok {
        let err = unsafe { GetLastError() };
        let (reason, message) = if err == ERROR_CANCELLED {
            ("refused", "the administrator prompt was dismissed")
        } else {
            ("error", "the command could not be started")
        };
        return Ok(elevate_result(false, None, reason, message));
    }
    // The call returns only once the elevation prompt has been answered, so
    // this is the exact moment authorization finished — the caller has been
    // showing "waiting for authorization" until now.
    started();
    if info.h_process.is_null() {
        // It launched but gave us nothing to wait on; report that honestly
        // rather than claim an exit status we do not have.
        return Ok(elevate_result(true, None, "", ""));
    }
    // SAFETY: `h_process` is a live handle returned by the call above, closed
    // exactly once below.
    let code = unsafe {
        WaitForSingleObject(info.h_process, INFINITE);
        let mut c: u32 = 0;
        let got = GetExitCodeProcess(info.h_process, &mut c) != 0;
        CloseHandle(info.h_process);
        got.then_some(c as i32)
    };
    let (reason, message) = if code == Some(0) {
        ("", "")
    } else {
        ("failed", "the command exited with an error")
    };
    Ok(elevate_result(true, code, reason, message))
}

/// Everywhere else there is no agreed way to ask, so say so rather than run the
/// command unprivileged and let the caller believe it was elevated.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn elevate_native(
    _argv: &[String],
    _cwd: Option<&str>,
    _started: &dyn Fn(),
    _pid: &dyn Fn(u32),
) -> std::result::Result<Value, RpcError> {
    Ok(elevate_result(
        false,
        None,
        "unavailable",
        "this platform has no supported way to ask for administrator privileges",
    ))
}

fn host_notify(params: Value) -> std::result::Result<Value, RpcError> {
    let title = params.get("title").and_then(Value::as_str).unwrap_or("Limen");
    let body = params.get("body").and_then(Value::as_str).unwrap_or("");
    let urgency = params
        .get("urgency")
        .and_then(Value::as_str)
        .filter(|u| matches!(*u, "low" | "normal" | "critical"))
        .unwrap_or("normal");
    notify_native(title, body, urgency);
    Ok(Value::Null)
}

#[cfg(target_os = "linux")]
fn notify_native(title: &str, body: &str, urgency: &str) {
    // Arguments go straight to execve — no shell, so nothing in `title`/`body`
    // can be read as syntax however it is spelled.
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Limen", "-u", urgency, title, body])
        .spawn();
}

#[cfg(target_os = "macos")]
fn notify_native(title: &str, body: &str, _urgency: &str) {
    // `osascript -e` takes AppleScript *source*, so the text is escaped rather
    // than merely quoted — a stray `"` would otherwise end the literal and the
    // rest would be executed as script.
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .spawn();
}

/// Give Windows an app identity of our own, so notifications are attributed to
/// **Limen** rather than to whatever process happened to raise them.
///
/// A toast from an *unregistered* AUMID is accepted and then silently dropped —
/// which is why borrowing PowerShell's registered id works at all, at the cost
/// of every module notification reading "Windows PowerShell". Registering ours
/// costs one HKCU key and makes the attribution honest. No elevation: this is
/// the user's own hive.
///
/// Done once per process, and idempotent besides. `IconUri` is only set when the
/// icon has actually been extracted (`limen-core` writes it on the app's first
/// notification); a missing file would just render no icon. Note the icon —
/// unlike the name — appears from the next sign-in, because `WpnUserService`
/// caches an app's display data when it starts.
#[cfg(target_os = "windows")]
fn ensure_app_id() {
    use limen_proto::proc::NoConsole;
    use std::sync::OnceLock;
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        const KEY: &str = r"HKCU\Software\Classes\AppUserModelId\Limen";
        let mut values = vec![("DisplayName", APP_ID.to_string())];
        if let Some(icon) = icon_file() {
            values.push(("IconUri", icon.to_string_lossy().into_owned()));
            values.push(("IconBackgroundColor", "00000000".to_string()));
        }
        for (name, value) in values {
            // `.output()` rather than `.status()`: reg's "The operation completed
            // successfully." would otherwise land in the debug build's console.
            let _ = std::process::Command::new("reg")
                .args(["add", KEY, "/v", name, "/t", "REG_SZ", "/d", &value, "/f"])
                .no_console()
                .output();
        }
    });
}

/// The app id Windows attributes Limen's toasts to. Matches the one
/// `limen-core` registers for update notifications, so both speak as one app.
#[cfg(target_os = "windows")]
const APP_ID: &str = "Limen";

/// The extracted app icon, if it is there. `limen-core` unpacks it to
/// `<base>/state/icon.png` at startup; the host only ever reads it, so a missing
/// file is not an error — it just means a toast without a picture.
#[cfg(target_os = "windows")]
fn icon_file() -> Option<std::path::PathBuf> {
    let path = limen_home().join("state").join("icon.png");
    path.is_file().then_some(path)
}

#[cfg(target_os = "windows")]
fn notify_native(title: &str, body: &str, _urgency: &str) {
    use limen_proto::proc::NoConsole;
    // A WinRT ToastGeneric shown through our own registered AUMID — registered
    // first, since an unregistered id is accepted and then silently dropped.
    ensure_app_id();
    // The text lands inside an XML document inside a single-quoted PowerShell
    // string, so it has to survive both: XML entities first, then PowerShell's
    // doubled-quote escape.
    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
    fn ps(s: &str) -> String {
        s.replace('\'', "''")
    }
    // `appLogoOverride` is the thumbnail *beside the text*. It is separate from
    // the small icon in the header, which comes from the AUMID's `IconUri` and
    // only refreshes when `WpnUserService` restarts — this one is per-toast, so
    // it shows immediately. A `file:///` src needs forward slashes.
    let logo = icon_file()
        .map(|p| {
            format!(
                "<image placement=\"appLogoOverride\" src=\"file:///{}\"/>",
                xml(&p.to_string_lossy().replace('\\', "/"))
            )
        })
        .unwrap_or_default();
    let doc = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text>{}</binding></visual></toast>",
        xml(title),
        xml(body),
        logo
    );
    let script = format!(
        "[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
         [void][Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom,ContentType=WindowsRuntime];\
         $x=New-Object Windows.Data.Xml.Dom.XmlDocument;$x.LoadXml('{}');\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{}')\
         .Show((New-Object Windows.UI.Notifications.ToastNotification $x))",
        ps(&doc),
        APP_ID
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .no_console()
        .spawn();
}

/// Show a native "open file" dialog on the host and return the chosen path as
/// `{ "path": "..." }`, or `Null` if the user cancelled. Shells out to the
/// platform's dialog (no extra dependency), matching how `host.open` works.
fn host_pick_file() -> Value {
    match pick_file_native() {
        Some(path) if !path.is_empty() => json!({ "path": path }),
        _ => Value::Null,
    }
}

#[cfg(target_os = "linux")]
fn pick_file_native() -> Option<String> {
    let out = std::process::Command::new("zenity")
        .args(["--file-selection", "--title=Choose a file"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // non-zero on cancel
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(target_os = "macos")]
fn pick_file_native() -> Option<String> {
    let out = std::process::Command::new("osascript")
        .args(["-e", "POSIX path of (choose file with prompt \"Choose a file\")"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // user cancelled
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(target_os = "windows")]
fn pick_file_native() -> Option<String> {
    use limen_proto::NoConsole;
    // STA is required for the WinForms dialog; write only the path to stdout.
    let ps = "Add-Type -AssemblyName System.Windows.Forms; \
              $d = New-Object System.Windows.Forms.OpenFileDialog; \
              $d.Filter = 'Images (*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.webp)|*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.webp|All files (*.*)|*.*'; \
              if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Out.Write($d.FileName) }";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", ps])
        .no_console()
        .output()
        .ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn pick_file_native() -> Option<String> {
    None
}

/// Open something in the OS on a module's behalf (e.g. the devices module's
/// "Open path" / "Registry" / "Device Manager"). `params`:
/// `{ "target": "path"|"url"|"registry"|"device_manager", "value": "..." }`.
/// Best-effort and fire-and-forget — a launch failure is not a module error.
fn host_open(params: Value) -> std::result::Result<Value, RpcError> {
    let target = params.get("target").and_then(Value::as_str).unwrap_or("path");
    let value = params.get("value").and_then(Value::as_str).unwrap_or("");
    open_target(target, value);
    Ok(Value::Null)
}

#[cfg(target_os = "linux")]
fn open_target(target: &str, value: &str) {
    use std::process::Command;
    // Registry / Device Manager are Windows-only; a path or URL opens in the
    // desktop's default handler (file manager for a directory).
    if matches!(target, "registry" | "device_manager") || value.is_empty() {
        return;
    }
    match target {
        // No portable "select this file" across file managers, so settle for
        // opening the containing directory.
        "reveal" => {
            let dir = std::path::Path::new(value).parent().unwrap_or(std::path::Path::new(value));
            let _ = Command::new("xdg-open").arg(dir).spawn();
        }
        // xdg-open already routes text files to the configured editor.
        _ => {
            let _ = Command::new("xdg-open").arg(value).spawn();
        }
    }
}

#[cfg(target_os = "macos")]
fn open_target(target: &str, value: &str) {
    use std::process::Command;
    if matches!(target, "registry" | "device_manager") || value.is_empty() {
        return;
    }
    match target {
        // -R reveals the file in Finder rather than opening it.
        "reveal" => {
            let _ = Command::new("open").args(["-R", value]).spawn();
        }
        // -t forces the default *text editor* instead of the file's handler.
        "edit" => {
            let _ = Command::new("open").args(["-t", value]).spawn();
        }
        _ => {
            let _ = Command::new("open").arg(value).spawn();
        }
    }
}

/// Launch something through the shell, optionally asking for elevation.
///
/// `CreateProcess` (what `std::process::Command` uses) cannot elevate: launching
/// a program whose manifest demands admin — `regedit` — fails outright with
/// `ERROR_ELEVATION_REQUIRED` (740) and, because these launches are
/// fire-and-forget, the user sees nothing happen at all. `ShellExecuteW` is the
/// API that can raise the UAC prompt, so elevation is *requested* up front via
/// the `runas` verb rather than the launch silently failing.
#[cfg(target_os = "windows")]
fn shell_exec(verb: Option<&str>, file: &str, params: Option<&str>) {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    const SW_SHOWNORMAL: i32 = 1;
    let verb_w = verb.map(wide);
    let file_w = wide(file);
    let params_w = params.map(wide);
    let ptr = |o: &Option<Vec<u16>>| o.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    // SAFETY: every pointer is either null or a NUL-terminated UTF-16 buffer
    // that outlives the call; a null hwnd/directory means "no owner window" and
    // "inherit the working directory", both valid here.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            ptr(&verb_w),
            file_w.as_ptr(),
            ptr(&params_w),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(target_os = "windows")]
fn open_target(target: &str, value: &str) {
    use limen_proto::NoConsole;
    use std::process::Command;
    match target {
        // A device instance id opens that device's own properties dialog; with
        // no id, fall back to the Device Manager console.
        "device_manager" => {
            if value.is_empty() {
                shell_exec(None, "devmgmt.msc", None);
            } else {
                // Empty /MachineName means the local machine.
                let args = format!(
                    "devmgr.dll,DeviceProperties_RunDLL /MachineName \"\" /DeviceID \"{value}\""
                );
                shell_exec(None, "rundll32.exe", Some(&args));
            }
        }
        // regedit reopens at its stored LastKey — set it, then launch regedit.
        // Writing LastKey is HKCU, so it needs no elevation; regedit itself does.
        "registry" => {
            if !value.is_empty() {
                let _ = Command::new("reg")
                    .args([
                        "add",
                        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Applets\Regedit",
                        "/v", "LastKey", "/t", "REG_SZ", "/d", value, "/f",
                    ])
                    .no_console()
                    .spawn()
                    .and_then(|mut c| c.wait());
            }
            shell_exec(Some("runas"), "regedit.exe", None);
        }
        _ if value.is_empty() => {}
        "url" => {
            let _ = Command::new("cmd").args(["/C", "start", "", value]).no_console().spawn();
        }
        // Open Explorer with the item itself selected, rather than just opening
        // its folder. `/select,<path>` must arrive as ONE argument with the path
        // quoted inside it — Rust's own argument quoting produces a form
        // explorer rejects, so the switch is passed raw.
        "reveal" => {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("explorer")
                .raw_arg(format!("/select,\"{value}\""))
                .no_console()
                .spawn();
        }
        // Show a file's contents as text, whatever its extension says. Notepad
        // is guaranteed present; the default handler would run a .bat, not open it.
        "edit" => {
            let _ = Command::new("notepad.exe").arg(value).no_console().spawn();
        }
        // A filesystem path → open in Explorer.
        _ => {
            let _ = Command::new("explorer").arg(value).spawn();
        }
    }
}

/// Host environment info returned by `host.about`: the OS/arch Limen is running
/// on, its version, the hostname, and the portable base directory it runs from.
fn host_about() -> Value {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default();
    json!({
        "os": std::env::consts::OS,          // "linux" | "windows" | "macos" | …
        "arch": std::env::consts::ARCH,      // "x86_64" | "aarch64" | …
        "family": std::env::consts::FAMILY,  // "unix" | "windows"
        "hostname": hostname,
        "limen_version": env!("CARGO_PKG_VERSION"),
        "base_dir": limen_home().to_string_lossy(),
    })
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
fn resolve_order(specs: &[ModuleSpec]) -> (Vec<ModuleSpec>, HashMap<String, String>) {
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

const PY_SDK: &str = include_str!("../../sdk/python/limen_sdk.py");
const JS_SDK: &str = include_str!("../../sdk/js/limen.js");
const LUA_SDK: &str = include_str!("../../sdk/lua/limen.lua");

/// Paths to the extracted SDKs, and the env each language needs to find them.
struct SdkPaths {
    python: PathBuf,
    js: PathBuf,
    lua: PathBuf,
}

impl SdkPaths {
    /// The env vars to set when spawning a module of `language` so its runtime
    /// can find the injected SDK.
    fn env_for(&self, language: Language) -> Vec<(String, String)> {
        match language {
            Language::Python => vec![(
                "PYTHONPATH".to_string(),
                self.python.to_string_lossy().into_owned(),
            )],
            // Node searches each NODE_PATH entry like a node_modules dir, so
            // `require("limen")` resolves to <js>/limen.js.
            Language::Js => vec![(
                "NODE_PATH".to_string(),
                self.js.to_string_lossy().into_owned(),
            )],
            // Lua's require uses package.path patterns; `require("limen")` maps
            // to <lua>/limen.lua.
            Language::Lua => vec![(
                "LUA_PATH".to_string(),
                format!("{}/?.lua;;", self.lua.to_string_lossy()),
            )],
            Language::Native => Vec::new(),
        }
    }
}

/// Extract the embedded SDKs under `<limen home>/sdk/` and return their paths.
fn install_sdks() -> Result<SdkPaths> {
    let base = limen_home().join("sdk");
    let python = base.join("python");
    let js = base.join("js");
    let lua = base.join("lua");
    for dir in [&python, &js, &lua] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(python.join("limen_sdk.py"), PY_SDK).context("writing python SDK")?;
    std::fs::write(js.join("limen.js"), JS_SDK).context("writing js SDK")?;
    std::fs::write(lua.join("limen.lua"), LUA_SDK).context("writing lua SDK")?;
    Ok(SdkPaths { python, js, lua })
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

#[cfg(test)]
mod spec_tests {
    use super::*;
    use crate::runtimes::Runtime;

    fn spec(launch: Launch) -> ModuleSpec {
        ModuleSpec {
            name: "m".into(),
            display_name: None,
            version: "0".into(),
            description: None,
            authors: vec![],
            tags: vec![],
            repo: None,
            capabilities: vec![],
            requires: BTreeMap::new(),
            optional: BTreeMap::new(),
            permissions: Permissions::default(),
            language: Language::Native,
            launch,
            cwd: PathBuf::from("."),
        }
    }

    #[test]
    fn is_native_lib_only_for_in_process_libraries() {
        // Only a dynamic library loaded in-process needs a restart to update.
        assert!(spec(Launch::Native("lib.so".into())).is_native_lib());
        // A compiled RPC binary runs as a subprocess — no restart needed.
        assert!(!spec(Launch::Binary("bin".into())).is_native_lib());
        // A scripted module re-runs its source — no restart needed.
        assert!(!spec(Launch::Script {
            runtime: Runtime::Python,
            script: "m.py".into(),
        })
        .is_native_lib());
    }
}
