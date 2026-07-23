//! The engine worker thread.
//!
//! `limen-core`'s `Engine` runs modules and makes *blocking* calls, and registry
//! operations hit the disk/network. To keep the UI responsive, all of it lives
//! on a dedicated thread: the UI sends [`Command`]s and drains [`Event`]s each
//! frame.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use limen_core::{paths, Config, Engine, ModuleSpec};
use limen_registry::{list_org_modules, Lockfile, Registry, RemoteModule};
use serde_json::Value;

/// A snapshot of installed modules plus which ones came from a git install
/// (recorded in the lockfile) — the rest were placed manually.
pub struct ModuleSnapshot {
    pub specs: Vec<ModuleSpec>,
    pub git_installed: Vec<String>,
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
    /// Uninstall a registry-installed module.
    RemoveModule(String),
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
        thread::spawn(move || run(dirs, cmd_rx, evt_tx));
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

fn start_engine(dirs: &[PathBuf]) -> Result<Engine, String> {
    Engine::load(dirs)
        .and_then(|mut e| {
            e.start()?;
            Ok(e)
        })
        .map_err(|e| format!("{e:#}"))
}

/// Snapshot the installed modules and read the lockfile to mark which came from
/// a git install (vs. manually placed).
fn snapshot(engine: &Engine) -> ModuleSnapshot {
    let git_installed = Lockfile::load(&paths::home().join("limen.lock"))
        .map(|lock| {
            lock.modules
                .into_iter()
                .filter(|e| e.source == "git")
                .map(|e| e.name)
                .collect()
        })
        .unwrap_or_default();
    ModuleSnapshot {
        specs: engine.modules().to_vec(),
        git_installed,
    }
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
    match start_engine(dirs) {
        Ok(e) => {
            *engine = e;
            let _ = evt_tx.send(Event::Modules(snapshot(engine)));
            true
        }
        Err(err) => {
            let _ = evt_tx.send(Event::Fatal(err));
            false
        }
    }
}

fn run(dirs: Vec<PathBuf>, cmd_rx: Receiver<Command>, evt_tx: Sender<Event>) {
    let mut engine = match start_engine(&dirs) {
        Ok(e) => e,
        Err(err) => {
            let _ = evt_tx.send(Event::Fatal(err));
            return;
        }
    };
    let _ = evt_tx.send(Event::Ready(snapshot(&engine)));

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Refresh => {
                let _ = evt_tx.send(Event::Modules(snapshot(&engine)));
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
                let msg = match Registry::new(paths::home()).add(&reference) {
                    Ok(report) => format!("installed {} module(s)", report.installed.len()),
                    Err(e) => format!("install failed: {e:#}"),
                };
                let _ = evt_tx.send(Event::Status(msg));
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
            Command::Quit => {
                engine.shutdown();
                break;
            }
        }
    }
}
