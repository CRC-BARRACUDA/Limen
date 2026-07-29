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
            version: manifest.module.version,
            description: manifest.module.description,
            authors: manifest.module.authors,
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
        "host.open" => host_open(params),
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
            version: "0".into(),
            description: None,
            authors: vec![],
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
