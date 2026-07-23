//! The stdio JSON-RPC transport to a single module process.
//!
//! A [`ModuleConnection`] owns the child process and a background reader thread.
//! Because the channel is bidirectional, the reader has to handle two kinds of
//! incoming frame:
//!
//! * a **response** to a request the host made — delivered to whoever is blocked
//!   in [`ModuleConnection::call`] via a per-call channel keyed by request id;
//! * a **request** the module made (e.g. `host.call`) — passed to the
//!   [`IncomingHandler`]. Each such request is handled on its own thread so a
//!   re-entrant broker call (module A → host → module B) can never block the
//!   reader that must keep draining A's output.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use limen_proto::rpc::MODULE_ERROR;
use limen_proto::{Message, Request, Response, RpcError};
use serde_json::Value;

use crate::module::{IncomingHandler, Module};

type PendingMap = Mutex<HashMap<u64, Sender<std::result::Result<Value, RpcError>>>>;

/// A live connection to one module running as a child process.
pub struct ModuleConnection {
    name: String,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: PendingMap,
}

impl ModuleConnection {
    /// Spawn `argv` (optionally in `cwd`) and start pumping its stdio. Incoming
    /// module→host requests are dispatched to `handler`.
    pub fn spawn(
        name: String,
        argv: &[String],
        cwd: Option<&Path>,
        handler: Arc<IncomingHandler>,
    ) -> Result<Arc<Self>> {
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // stderr is inherited so a module's logs surface on the host console.
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning module {name}: {argv:?}"))?;
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");

        let conn = Arc::new(Self {
            name,
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        });

        let reader_conn = conn.clone();
        thread::spawn(move || reader_loop(reader_conn, stdout, handler));
        Ok(conn)
    }

    fn send(&self, msg: &Message) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }
}

impl Module for ModuleConnection {
    fn name(&self) -> &str {
        &self.name
    }

    /// Issue a request and block until the matching response arrives. Safe to
    /// call concurrently from multiple threads (ids and pending slots are
    /// per-call).
    fn call(&self, method: &str, params: Value) -> std::result::Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = channel();
        self.pending.lock().unwrap().insert(id, tx);

        let req = Request::new(id, method.to_string(), params);
        if let Err(e) = self.send(&Message::Request(req)) {
            self.pending.lock().unwrap().remove(&id);
            return Err(RpcError::new(
                MODULE_ERROR,
                format!("send to {} failed: {e}", self.name),
            ));
        }

        match rx.recv() {
            Ok(result) => result,
            Err(_) => Err(RpcError::new(
                MODULE_ERROR,
                format!("module {} disconnected", self.name),
            )),
        }
    }

    /// Ask the module to shut down, then make sure the process is gone.
    fn shutdown(&self) {
        let _ = self.send(&Message::Request(Request::notification(
            "shutdown".into(),
            Value::Null,
        )));
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn reader_loop(conn: Arc<ModuleConnection>, stdout: ChildStdout, handler: Arc<IncomingHandler>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Message = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[host] {}: unparseable frame: {e}: {line}", conn.name);
                continue;
            }
        };
        match msg {
            Message::Response(resp) => {
                if let Some(tx) = conn.pending.lock().unwrap().remove(&resp.id) {
                    let result = match resp.error {
                        Some(err) => Err(err),
                        None => Ok(resp.result.unwrap_or(Value::Null)),
                    };
                    let _ = tx.send(result);
                }
            }
            Message::Request(req) => {
                // A module-initiated request (e.g. host.call). Handle it off the
                // reader thread so a re-entrant broker hop never deadlocks us.
                let conn2 = conn.clone();
                let handler2 = handler.clone();
                thread::spawn(move || {
                    let result = handler2(&req.method, req.params);
                    if let Some(id) = req.id {
                        let resp = match result {
                            Ok(v) => Response::ok(id, v),
                            Err(e) => Response::error(id, e),
                        };
                        let _ = conn2.send(&Message::Response(resp));
                    }
                });
            }
        }
    }

    // The module closed its stdout (exited). Fail every in-flight call so no
    // caller blocks forever.
    let mut pending = conn.pending.lock().unwrap();
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(RpcError::new(
            MODULE_ERROR,
            format!("module {} closed the connection", conn.name),
        )));
    }
}
