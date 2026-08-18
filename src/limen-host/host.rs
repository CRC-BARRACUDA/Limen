//! The [`Host`]: load modules, resolve their dependency graph, launch them, and
//! expose a simple `invoke(capability, method, params)` entry point.

use std::collections::{HashMap};
use std::path::{PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use limen_proto::rpc::METHOD_NOT_FOUND;
use limen_proto::{RpcError};
use serde_json::{json, Value};

use crate::broker::Broker;
use crate::connection::ModuleConnection;
use crate::module::{stderr_logger, IncomingHandler, Logger, Module};
use crate::native::NativeModule;

// The runtime is one module but not one file. Each of these is a piece of it;
// the re-exports keep every name reachable from where it was.
pub(crate) use crate::elevate::*;
pub(crate) use crate::files::*;
pub(crate) use crate::notify::*;
pub(crate) use crate::sdk::*;
pub(crate) use crate::spec::*;
pub(crate) use crate::supervisor::*;

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
pub(crate) fn host_handler(
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
        "host.save_file" => Ok(host_save_file(params)),
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

/// Host environment info returned by `host.about`: the OS/arch Limen is running
/// on, its version, the hostname, and the portable base directory it runs from.
pub(crate) fn host_about() -> Value {
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

pub(crate) const PY_SDK: &str = include_str!("../../sdk/python/limen_sdk.py");
pub(crate) const JS_SDK: &str = include_str!("../../sdk/js/limen.js");
