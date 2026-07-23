//! The engine worker thread.
//!
//! `limen-core`'s `Engine` runs modules and makes *blocking* calls (a module can
//! take a while to answer), and registry operations hit the disk/network. To
//! keep the UI responsive, all of it lives on a dedicated thread: the UI sends
//! [`Command`]s and drains [`Event`]s each frame.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use limen_core::{paths, Engine, ModuleSpec};
use limen_registry::Registry;
use serde_json::Value;

/// A request from the UI to the engine.
pub enum Command {
    /// Re-list installed modules.
    Refresh,
    /// Invoke `capability.method(params)`. `tag` is echoed back so the UI knows
    /// which panel asked.
    Run {
        tag: RunTag,
        capability: String,
        method: String,
        params: Value,
    },
    /// Install a module (and deps) from `owner/repo[@ver]` or a local path,
    /// then reload the engine.
    AddModule(String),
    /// Uninstall a registry-installed module, then reload the engine.
    RemoveModule(String),
    /// Shut modules down and exit the worker.
    Quit,
}

/// What a run was for, so its result routes back correctly.
#[derive(Clone, PartialEq, Eq)]
pub enum RunTag {
    /// Fetching a module's self-described UI (result is a view spec).
    Ui { module: String },
    /// A button in a module's UI fired (result is output to display).
    Action,
}

/// A message from the engine back to the UI.
pub enum Event {
    /// Engine (re)started; here are the modules.
    Ready(Vec<ModuleSpec>),
    /// Updated module list (after `Refresh`).
    Modules(Vec<ModuleSpec>),
    /// A run finished.
    RunDone { tag: RunTag, result: Result<Value, String> },
    /// A registry operation finished (human-readable outcome).
    Status(String),
    /// The engine could not start.
    Fatal(String),
}

/// Handle the UI keeps to talk to the worker.
pub struct Worker {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
}

impl Worker {
    /// Spawn the worker, loading modules from `dirs`.
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

/// Restart the engine after the installed set changed.
fn reload(engine: &mut Engine, dirs: &[PathBuf], evt_tx: &Sender<Event>) -> bool {
    engine.shutdown();
    match start_engine(dirs) {
        Ok(e) => {
            *engine = e;
            let _ = evt_tx.send(Event::Ready(engine.modules().to_vec()));
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
    let _ = evt_tx.send(Event::Ready(engine.modules().to_vec()));

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            Command::Refresh => {
                let _ = evt_tx.send(Event::Modules(engine.modules().to_vec()));
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
                        format!("{name} is a dev module (not installed via the registry)")
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
