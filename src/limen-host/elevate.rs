//! Running something as administrator, and the platform's way of asking.

use std::sync::Arc;
use limen_proto::{RpcError};
use serde_json::{json, Value};
use crate::module::{Logger};

use crate::host::*;

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
pub(crate) type Elevations = std::sync::Mutex<std::collections::HashMap<u64, Arc<std::sync::Mutex<Value>>>>;

pub(crate) fn elevations() -> &'static Elevations {
    static E: std::sync::OnceLock<Elevations> = std::sync::OnceLock::new();
    E.get_or_init(Default::default)
}

/// End every elevation still running, on the way out.
///
/// Almost nothing to do, deliberately. Each elevated command is held by a
/// supervisor that ends it when its socket closes, and the operating system
/// closes those when this process exits — however it exits. Asking for
/// privileges here, as this once did, meant a password prompt and a stall while
/// the window was already closing, to do what closing does by itself.
///
/// What remains is for anything not supervised: our own children, which we can
/// signal because they are ours.
pub(crate) fn stop_all_elevations(log: &Logger) {
    let live: Vec<u32> = elevations()
        .lock()
        .unwrap()
        .values()
        .filter_map(|st| {
            let v = st.lock().unwrap();
            (v.get("running").and_then(Value::as_bool) == Some(true))
                .then(|| v.get("pid").and_then(Value::as_u64).map(|p| p as u32))
                .flatten()
        })
        .collect();
    if live.is_empty() {
        return;
    }
    #[cfg(any(unix, windows))]
    {
        // Dropping the links is what ends the supervised ones; it would happen
        // at exit anyway, but doing it here ends them a moment sooner.
        let held = supervisors().lock().unwrap().drain().count();
        if held > 0 {
            log(&format!("[elevate] closing: {held} supervised, ending with us"));
        }
    }
    for pid in live {
        kill_pid(pid);
    }
}

/// Stop an elevation that is still running.
///
/// Asking the supervisor is the whole of it. It is elevated and it is the
/// command's parent, so it can end a root process where we cannot — and it was
/// authorized once, when the scan started. Stopping therefore never asks the
/// user for anything: a second password prompt to end something they have just
/// pressed Stop on is a prompt to undo, which is not a thing anyone agreed to.
///
/// The command is not dead by the time this returns — the supervisor has to
/// notice, kill it, and reap it. That is what the caller's next poll is for.
/// What is reported here is that the stop was *delivered*, and the only way it
/// is not is that there was nothing to deliver it to.
pub(crate) fn host_elevate_stop(params: Value, log: &Logger, who: &str) -> std::result::Result<Value, RpcError> {
    let id = params.get("id").and_then(Value::as_u64).unwrap_or(0);
    let slot = elevations().lock().unwrap().get(&id).cloned();
    let Some(state) = slot else {
        return Ok(json!({ "stopped": false }));
    };

    // Taken, not borrowed: once it has been told to stop there is nothing more
    // to say to it, and dropping the link says the same thing a second time —
    // the supervisor treats the socket closing exactly as it treats `stop`, so
    // a write that never lands still ends the command.
    #[cfg(any(unix, windows))]
    {
        use std::io::Write;
        let sup = supervisors().lock().unwrap().remove(&id);
        if let Some(mut s) = sup {
            let _ = writeln!(s, "stop");
            let _ = s.flush();
            drop(s);
            log(&format!("[elevate] {who}: asked the supervisor to stop {id}"));
            return Ok(json!({ "stopped": true }));
        }
    }

    // No supervisor — the command was elevated directly, which happens only when
    // there was no supervisor binary to use. Signal it ourselves: that works if
    // it was never really elevated, or if we are root already, and otherwise the
    // kernel refuses and there is nothing further to try that does not involve
    // asking the user to authorize a kill.
    let pid = state.lock().unwrap().get("pid").and_then(Value::as_u64);
    let Some(pid) = pid.filter(|p| *p > 0) else {
        // Nothing to signal — macOS runs it inside osascript, which gives us no
        // handle at all.
        return Ok(json!({ "stopped": false }));
    };
    kill_pid(pid as u32);
    std::thread::sleep(std::time::Duration::from_millis(250));
    let stopped = state.lock().unwrap().get("running").and_then(Value::as_bool) != Some(true);
    if stopped {
        log(&format!("[elevate] {who}: stopped {pid}"));
    } else {
        log(&format!(
            "[elevate] {who}: {pid} is elevated and unsupervised, so it cannot be stopped"
        ));
    }
    Ok(json!({ "stopped": stopped }))
}

#[cfg(unix)]
pub(crate) fn kill_pid(pid: u32) {
    use limen_proto::NoConsole;
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .no_console()
        .status();
}

#[cfg(windows)]
pub(crate) fn kill_pid(pid: u32) {
    use limen_proto::NoConsole;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .no_console()
        .status();
}

/// How an elevation started with `wait: false` is going.
pub(crate) fn host_elevate_status(params: Value) -> std::result::Result<Value, RpcError> {
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

pub(crate) fn host_elevate(
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
pub(crate) fn start_elevation(argv: Vec<String>, cwd: Option<String>, log: &Logger, who: &str) -> u64 {
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

    // Elevate a supervisor rather than the command itself, where we can. It
    // holds the command and ends it when this side closes — which the operating
    // system does for us, however Limen exits.
    #[cfg(any(unix, windows))]
    let (argv, cwd) = match supervised(id, &argv, cwd.as_deref()) {
        Some((wrapped, listener, sock)) => {
            let state = state.clone();
            let log = log.clone();
            let who = who.to_string();
            std::thread::spawn(move || {
                // The supervisor connects once it is running elevated; that is
                // also the moment authorization finished.
                if let Some(stream) = sup_accept(listener) {
                    {
                        let mut slot = state.lock().unwrap();
                        slot["phase"] = json!("running");
                    }
                    log(&format!("[elevate] {who}: supervised, running"));
                    // Read the pid it reports, then hold the socket: closing it
                    // is what ends the command.
                    let mut line = String::new();
                    let mut r = std::io::BufReader::new(
                        stream.try_clone().expect("clone the supervisor link"),
                    );
                    use std::io::BufRead;
                    if r.read_line(&mut line).is_ok()
                        && let Some(p) =
                            line.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok())
                    {
                        state.lock().unwrap()["pid"] = json!(p);
                    }
                    supervisors().lock().unwrap().insert(id, stream);
                }
                sup_cleanup(&sock);
            });
            // The supervisor sets the directory itself, so the helper need not.
            (wrapped, None)
        }
        None => (argv, cwd),
    };


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
pub(crate) fn elevate_result(ran: bool, code: Option<i32>, reason: &str, message: &str) -> Value {
    json!({ "ran": ran, "code": code, "reason": reason, "message": message })
}

/// Find an executable on `PATH`.
///
/// Nothing is assumed to be installed: a minimal or hardened Linux may have
/// neither polkit nor sudo, and the honest answer there is to say so rather than
/// to run the command unprivileged and let the caller believe otherwise.
#[cfg(unix)]
pub(crate) fn program_on_path(name: &str) -> Option<std::path::PathBuf> {
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
pub(crate) fn host_can_elevate() -> Value {
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
pub(crate) fn elevate_native(
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
pub(crate) fn elevate_native(
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
pub(crate) fn elevate_native(
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
        fn GetProcessId(handle: *mut c_void) -> u32;
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
    // What was started, so it can be stopped later. Supervised, this is the
    // supervisor and the real command's id arrives over the link in a moment
    // and replaces it; unsupervised, this is all there will ever be.
    // SAFETY: `h_process` is the live handle returned above, not yet closed.
    let started_pid = unsafe { GetProcessId(info.h_process) };
    if started_pid != 0 {
        pid(started_pid);
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
pub(crate) fn elevate_native(
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
