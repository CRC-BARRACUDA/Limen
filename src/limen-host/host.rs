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
use crate::module::{IncomingHandler, Module};
use crate::native::NativeModule;

/// How a module is launched, chosen from its manifest.
#[derive(Debug, Clone)]
pub enum Launch {
    /// A subprocess speaking JSON-RPC over stdio (the argv to spawn).
    Process(Vec<String>),
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
        _ => Ok(Launch::Process(build_argv(dir, manifest)?)),
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

/// Map a module's language to the argv that launches it. (`native` here means a
/// compiled binary that speaks RPC over stdio; the in-process path is
/// [`resolve_native_lib`].)
fn build_argv(dir: &Path, manifest: &Manifest) -> Result<Vec<String>> {
    let interpreter = match manifest.module.language {
        Language::Python => Some("python3"),
        Language::Lua => Some("lua"),
        Language::Js => Some("node"),
        Language::Native => None,
    };
    match interpreter {
        Some(bin) => {
            // Absolute so it stays correct once the child's cwd is the module dir.
            let script = abspath(dir.join(&manifest.module.entry));
            Ok(vec![bin.to_string(), script])
        }
        None => Ok(vec![resolve_native(dir, &manifest.module.entry)?]),
    }
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
        })
    }

    /// Spawn every module in dependency order, register its capabilities, and
    /// `initialize` it.
    pub fn start(&mut self) -> Result<()> {
        let handler: Arc<IncomingHandler> = {
            let broker = self.broker.clone();
            Arc::new(move |method: &str, params: Value| host_handler(&broker, method, params))
        };

        for spec in &self.order {
            let conn: Arc<dyn Module> = match &spec.launch {
                Launch::Process(argv) => {
                    ModuleConnection::spawn(spec.name.clone(), argv, Some(&spec.cwd), handler.clone())
                        .with_context(|| format!("spawning module {}", spec.name))?
                }
                Launch::Native(path) => NativeModule::load(spec.name.clone(), path, handler.clone())
                    .with_context(|| format!("loading native module {}", spec.name))?,
            };

            for capability in &spec.capabilities {
                self.broker.register(capability, conn.clone());
            }

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

            eprintln!(
                "[host] started {} v{} caps={:?} -> {info}",
                spec.name, spec.version, spec.capabilities
            );
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
fn host_handler(broker: &Broker, method: &str, params: Value) -> std::result::Result<Value, RpcError> {
    match method {
        "host.call" => broker.route(params),
        "host.log" => {
            eprintln!("[module] {params}");
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
