//! The elevated supervisor.
//!
//! An elevated scan is a root process, and an unprivileged parent cannot signal
//! one — so Limen could neither stop it on request nor end it when the window
//! closed. It ran on, writing to a report nobody was left to read.
//!
//! So the privileges are given to something that *stays*: a supervisor which is
//! elevated once, spawns the real command as its own child, and holds a
//! connection back to Limen. It ends the child when Limen asks, and when Limen
//! goes away — the socket closing is the signal, so a crash counts as much as a
//! clean exit.
//!
//! ```text
//!   Limen (you)  ──socket──  supervisor (root)  ──child──  the command (root)
//! ```
//!
//! **The socket may never carry a command to run.** The argument vector is fixed
//! on the command line, which is exactly what the authorization covered; a
//! supervisor that accepted "now run this" would turn one prompt into a
//! root-execution service for anything that could reach the socket. It accepts
//! `stop` and nothing else.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};

/// Run `argv` and keep it only as long as Limen is there to want it.
///
/// Returns the child's exit code, or 143 (terminated) if it was stopped.
pub fn run(socket: &str, cwd: Option<&str>, argv: &[String]) -> Result<i32> {
    if argv.is_empty() {
        bail!("nothing to supervise");
    }

    // Connect *before* spawning anything. If Limen is not there, there is
    // nobody to supervise for, and a root process must not be started on the
    // strength of an authorization whose asker has already gone.
    let mut link = connect(socket).with_context(|| format!("connecting to {socket}"))?;

    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("starting {}", argv[0]))?;
    let child = Arc::new(std::sync::Mutex::new(child));

    // Tell Limen what it got, so it can report progress against a real pid.
    let _ = writeln!(link, "started {}", child.lock().unwrap().id());
    let _ = link.flush();

    // One thread reads the link. Anything that ends it — `stop`, Limen exiting,
    // Limen being killed — means the same thing: end the child.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let child = child.clone();
        let reader = link.try_clone().context("cloning the link")?;
        std::thread::spawn(move || {
            let mut lines = BufReader::new(reader).lines();
            // `stop` from Limen, or EOF because Limen is gone.
            let asked = matches!(lines.next(), Some(Ok(l)) if l.trim() == "stop");
            let _ = asked;
            stop.store(true, Ordering::Relaxed);
            if let Ok(mut c) = child.lock() {
                let _ = c.kill();
            }
        });
    }

    let code = wait_for(&child)?;
    if stop.load(Ordering::Relaxed) {
        // Killed on request (or because Limen went): report it as terminated
        // rather than as whatever exit the signal produced.
        return Ok(143);
    }
    Ok(code)
}

fn wait_for(child: &Arc<std::sync::Mutex<Child>>) -> Result<i32> {
    loop {
        // Held only long enough to poll, so the reader thread can take the lock
        // to kill it.
        let status = { child.lock().unwrap().try_wait()? };
        match status {
            Some(st) => return Ok(st.code().unwrap_or(143)),
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

#[cfg(unix)]
fn connect(socket: &str) -> Result<std::os::unix::net::UnixStream> {
    Ok(std::os::unix::net::UnixStream::connect(socket)?)
}

#[cfg(windows)]
fn connect(socket: &str) -> Result<std::fs::File> {
    // A named pipe opens as a file; the same read/EOF semantics apply, so the
    // rest of this module does not care which it is.
    use std::fs::OpenOptions;
    Ok(OpenOptions::new().read(true).write(true).open(socket)?)
}
