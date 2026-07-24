//! The engine worker thread.
//!
//! `limen-core`'s `Engine` runs modules and makes *blocking* calls, and registry
//! operations hit the disk/network. To keep the UI responsive, all of it lives
//! on a dedicated thread: the UI sends [`Command`]s and drains [`Event`]s each
//! frame.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use std::collections::HashMap;

use limen_core::{
    apply_update, check_update, install_runtime, is_newer, paths, restart_app, Config, Engine,
    ModuleSpec, UpdateInfo,
};
use limen_registry::{latest_release_version, list_org_modules, Lockfile, Registry, RemoteModule};
use serde_json::Value;

/// A snapshot of installed modules plus which ones came from a git install
/// (recorded in the lockfile) — the rest were placed manually.
pub struct ModuleSnapshot {
    pub specs: Vec<ModuleSpec>,
    pub git_installed: Vec<String>,
    /// name → (branch, short commit) for git-installed modules.
    pub git_meta: std::collections::HashMap<String, (String, String)>,
}

/// A request from the UI to the worker.
pub enum Command {
    /// Re-list installed modules.
    Refresh,
    /// List the modules published in the configured GitHub org.
    ListRemote,
    /// Invoke `capability.method(params)`. `tag` routes the result back.
    Run {
        tag: RunTag,
        capability: String,
        method: String,
        params: Value,
    },
    /// Install a module (and deps) from `owner/repo[@ver]` or a local path.
    AddModule(String),
    /// Re-install an installed module from its locked source (update in place).
    UpdateModule(String),
    /// Internal: a threaded module download finished — report it, then reload
    /// (engine work must happen back on the worker thread).
    FinishAdd { status: String, ok: bool },
    /// Internal: threaded interpreter downloads finished — report, then reload.
    FinishRuntimes { status: String },
    /// Uninstall a registry-installed module.
    RemoveModule(String),
    /// Download and apply an available app update, then restart.
    ApplyUpdate(UpdateInfo),
    /// Cleanly shut modules down and relaunch the app (e.g. to load an updated
    /// native library whose code can't hot-swap).
    Restart,
    /// Shut modules down and exit the worker.
    Quit,
}

/// What a run was for, so its result routes back correctly.
#[derive(Clone, PartialEq, Eq)]
pub enum RunTag {
    Ui { module: String },
    Action,
}

/// A message from the worker back to the UI.
pub enum Event {
    /// Engine (re)started; here's the installed set.
    Ready(ModuleSnapshot),
    /// Updated installed set (after `Refresh` / install / remove).
    Modules(ModuleSnapshot),
    /// Modules available in the org (or an error string).
    RemoteModules(Result<Vec<RemoteModule>, String>),
    /// A run finished.
    RunDone { tag: RunTag, result: Result<Value, String> },
    /// A registry operation finished (human-readable outcome).
    Status(String),
    /// A host/module log line (for the debug console).
    Log(String),
    /// A newer app version is available.
    UpdateAvailable(UpdateInfo),
    /// Installed git modules that have a newer release: name → latest version.
    ModuleUpdates(HashMap<String, String>),
    /// The engine could not start.
    Fatal(String),
}

pub struct Worker {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
}

impl Worker {
    pub fn spawn(dirs: Vec<PathBuf>) -> Self {
        let (cmd_tx, cmd_rx) = channel::<Command>();
        let (evt_tx, evt_rx) = channel::<Event>();
        // Background update check (own thread so it never blocks module loading).
        {
            let evt_tx = evt_tx.clone();
            thread::spawn(move || {
                if let Some(info) = check_update(env!("CARGO_PKG_VERSION")) {
                    let _ = evt_tx.send(Event::UpdateAvailable(info));
                }
            });
        }
        // The worker keeps a sender so background install threads can hand engine
        // work (reloads) back to it.
        let self_tx = cmd_tx.clone();
        thread::spawn(move || run(dirs, cmd_rx, evt_tx, self_tx));
        Worker { tx: cmd_tx, rx: evt_rx }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
    }
}

fn start_engine(dirs: &[PathBuf], evt_tx: &Sender<Event>) -> Result<Engine, String> {
    let mut engine = Engine::load(dirs).map_err(|e| format!("{e:#}"))?;
    // Route all host + module logs into the debug console.
    let log_tx = evt_tx.clone();
    engine.set_logger(Arc::new(move |line: &str| {
        let _ = log_tx.send(Event::Log(line.to_string()));
    }));
    engine.start().map_err(|e| format!("{e:#}"))?;
    Ok(engine)
}

/// Snapshot the installed modules and read the lockfile to mark which came from
/// a git install (vs. manually placed).
fn snapshot(engine: &Engine) -> ModuleSnapshot {
    let mut git_installed = Vec::new();
    let mut git_meta = std::collections::HashMap::new();
    if let Ok(lock) = Lockfile::load(&paths::home().join("limen.lock")) {
        for e in lock.modules.into_iter().filter(|e| e.source == "git") {
            if !e.branch.is_empty() || !e.commit.is_empty() {
                git_meta.insert(e.name.clone(), (e.branch, e.commit));
            }
            git_installed.push(e.name);
        }
    }
    ModuleSnapshot {
        specs: engine.modules().to_vec(),
        git_installed,
        git_meta,
    }
}

/// `(name, repo, installed_version)` for each git-installed module with a repo —
/// the set to check for available updates.
fn update_check_refs(snap: &ModuleSnapshot) -> Vec<(String, String, String)> {
    let git: std::collections::HashSet<&String> = snap.git_installed.iter().collect();
    snap.specs
        .iter()
        .filter(|m| git.contains(&m.name))
        .filter_map(|m| m.repo.clone().map(|r| (m.name.clone(), r, m.version.clone())))
        .collect()
}

/// In the background, check each git module's latest release vs its installed
/// version and report which have an update available.
fn spawn_update_check(refs: Vec<(String, String, String)>, evt_tx: Sender<Event>) {
    if refs.is_empty() {
        return;
    }
    thread::spawn(move || {
        let mut updates = HashMap::new();
        for (name, repo, version) in refs {
            if let Some(latest) = latest_release_version(&repo)
                && is_newer(&latest, &version)
            {
                updates.insert(name, latest);
            }
        }
        let _ = evt_tx.send(Event::ModuleUpdates(updates));
    });
}

fn org() -> String {
    Config::load()
        .ok()
        .and_then(|c| c.default_org)
        .unwrap_or_else(|| limen_core::config::DEFAULT_ORG.to_string())
}

/// Restart the engine after the installed set changed.
fn reload(engine: &mut Engine, dirs: &[PathBuf], evt_tx: &Sender<Event>) -> bool {
    engine.shutdown();
    match start_engine(dirs, evt_tx) {
        Ok(e) => {
            *engine = e;
            let snap = snapshot(engine);
            spawn_update_check(update_check_refs(&snap), evt_tx.clone());
            let _ = evt_tx.send(Event::Modules(snap));
            true
        }
        Err(err) => {
            let _ = evt_tx.send(Event::Fatal(err));
            false
        }
    }
}

fn run(
    dirs: Vec<PathBuf>,
    cmd_rx: Receiver<Command>,
    evt_tx: Sender<Event>,
    cmd_tx: Sender<Command>,
) {
    let mut engine = match start_engine(&dirs, &evt_tx) {
        Ok(e) => e,
        Err(err) => {
            let _ = evt_tx.send(Event::Fatal(err));
            return;
        }
    };
    let snap = snapshot(&engine);
    spawn_update_check(update_check_refs(&snap), evt_tx.clone());
    let _ = evt_tx.send(Event::Ready(snap));

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Refresh => {
                let snap = snapshot(&engine);
                spawn_update_check(update_check_refs(&snap), evt_tx.clone());
                let _ = evt_tx.send(Event::Modules(snap));
            }
            Command::ListRemote => {
                let result = list_org_modules(&org()).map_err(|e| format!("{e:#}"));
                let _ = evt_tx.send(Event::RemoteModules(result));
            }
            Command::Run { tag, capability, method, params } => {
                let result = engine
                    .run(&capability, &method, params)
                    .map_err(|e| format!("{e:#}"));
                let _ = evt_tx.send(Event::RunDone { tag, result });
            }
            Command::AddModule(reference) => {
                // Download on its own thread so the worker keeps serving other
                // commands (e.g. running another module's UI). The reload — which
                // needs the live engine — is marshaled back via `FinishAdd`.
                let cmd_tx = cmd_tx.clone();
                thread::spawn(move || {
                    let (status, ok) = match Registry::new(paths::home()).add(&reference) {
                        Ok(report) => {
                            (format!("installed {} module(s)", report.installed.len()), true)
                        }
                        Err(e) => (format!("install failed: {e:#}"), false),
                    };
                    let _ = cmd_tx.send(Command::FinishAdd { status, ok });
                });
            }
            Command::UpdateModule(name) => {
                // Same off-thread pattern as install: re-fetch from the locked
                // source, then reload on the worker via FinishAdd.
                let cmd_tx = cmd_tx.clone();
                thread::spawn(move || {
                    let (status, ok) = match Registry::new(paths::home()).update(Some(&name)) {
                        Ok(report) => {
                            (format!("updated {} module(s)", report.installed.len()), true)
                        }
                        Err(e) => (format!("module update failed: {e:#}"), false),
                    };
                    let _ = cmd_tx.send(Command::FinishAdd { status, ok });
                });
            }
            Command::FinishAdd { status, ok } => {
                let _ = evt_tx.send(Event::Status(status));
                if !ok {
                    // No reload on failure — re-send the snapshot so the UI clears
                    // its "installing" spinner.
                    let _ = evt_tx.send(Event::Modules(snapshot(&engine)));
                    continue;
                }
                // Start the newly-installed module(s); any whose interpreter is
                // missing get recorded in missing_runtimes.
                if !reload(&mut engine, &dirs, &evt_tx) {
                    return;
                }
                // Auto-install any interpreter a new module needs but that isn't
                // available yet — off-thread too; reload again via FinishRuntimes.
                let missing = engine.missing_runtimes().to_vec();
                if !missing.is_empty() {
                    let evt_tx = evt_tx.clone();
                    let cmd_tx = cmd_tx.clone();
                    thread::spawn(move || {
                        let mut done: Vec<String> = Vec::new();
                        for rt in missing {
                            let _ = evt_tx.send(Event::Status(format!(
                                "downloading {} interpreter…",
                                rt.display()
                            )));
                            match install_runtime(rt) {
                                Ok(()) => done.push(format!("installed {} runtime", rt.display())),
                                Err(e) => {
                                    let _ = evt_tx.send(Event::Status(format!(
                                        "{} setup failed: {e}",
                                        rt.display()
                                    )));
                                }
                            }
                        }
                        let _ = cmd_tx.send(Command::FinishRuntimes { status: done.join("; ") });
                    });
                }
            }
            Command::FinishRuntimes { status } => {
                if !status.is_empty() {
                    let _ = evt_tx.send(Event::Status(status));
                }
                if !reload(&mut engine, &dirs, &evt_tx) {
                    return;
                }
            }
            Command::RemoveModule(name) => {
                let msg = match Registry::new(paths::home()).remove(&name) {
                    Ok(true) => format!("removed {name}"),
                    Ok(false) => {
                        format!("{name} is a manual module (not in the registry) — deleted files")
                    }
                    Err(e) => format!("remove failed: {e:#}"),
                };
                let _ = evt_tx.send(Event::Status(msg));
                if !reload(&mut engine, &dirs, &evt_tx) {
                    return;
                }
            }
            Command::ApplyUpdate(info) => {
                let _ = evt_tx.send(Event::Status("downloading update…".into()));
                // On success this restarts the process and never returns.
                if let Err(e) = apply_update(&info) {
                    let _ = evt_tx.send(Event::Status(format!("update failed: {e}")));
                }
            }
            Command::Restart => {
                // Shut modules down cleanly, then relaunch (never returns).
                engine.shutdown();
                restart_app();
            }
            Command::Quit => {
                engine.shutdown();
                break;
            }
        }
    }
}
