//! The broker that outlives nothing: an elevated child dies with the app
//! that started it.

use std::path::{Path};


/// One end of the link to a supervisor.
///
/// A Unix socket and a Windows named pipe are set up differently and are alike
/// in everything after that: both carry a line, and both let each side see the
/// other close. Naming them once keeps the logic that uses them single-sourced
/// rather than written twice and drifting.
#[cfg(unix)]
pub type SupLink = std::os::unix::net::UnixStream;
#[cfg(windows)]
pub type SupLink = std::fs::File;

/// What waits for the supervisor to arrive.
#[cfg(unix)]
pub type SupServer = std::os::unix::net::UnixListener;
#[cfg(windows)]
pub type SupServer = PipeServer;

/// Live supervisors, by elevation id.
///
/// Holding the link *is* the mechanism: the supervisor ends its child when this
/// closes, and the operating system closes it when Limen exits — however Limen
/// exits. Nothing has to run on the way out, which is the point: shutdown code
/// cannot be relied on, and a crash leaves no chance to run any.
#[cfg(any(unix, windows))]
pub(crate) type Supervisors = std::sync::Mutex<std::collections::HashMap<u64, SupLink>>;

#[cfg(any(unix, windows))]
pub(crate) fn supervisors() -> &'static Supervisors {
    static S: std::sync::OnceLock<Supervisors> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// Wait for the supervisor to connect, and take the link.
#[cfg(unix)]
#[doc(hidden)]
pub fn sup_accept(server: SupServer) -> Option<SupLink> {
    server.accept().ok().map(|(stream, _)| stream)
}

#[cfg(windows)]
#[doc(hidden)]
pub fn sup_accept(server: SupServer) -> Option<SupLink> {
    server.accept()
}

/// Clear away whatever the server left on disk. A Unix socket is a file and
/// stays until removed; a named pipe is not a file and goes with its handle.
#[cfg(unix)]
#[doc(hidden)]
pub fn sup_cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(windows)]
#[doc(hidden)]
pub fn sup_cleanup(_path: &Path) {}

/// The server end of a supervisor pipe.
///
/// Windows has no separate listener to accept from: a named pipe *is* both, and
/// `ConnectNamedPipe` waits for the client on the very handle that then carries
/// the traffic. Held as an `isize` rather than a `HANDLE` so the value can move
/// to the thread that waits — a raw pointer is not `Send`, and this one is only
/// ever used by the thread it is handed to.
#[cfg(windows)]
pub struct PipeServer(isize);

#[cfg(windows)]
impl PipeServer {
    /// Create the pipe before anything is elevated, so the supervisor has
    /// something to connect back to the moment it starts.
    ///
    /// The default security descriptor is what restricts it: it grants the
    /// creating user and the local administrators, and nobody else — so the
    /// elevated supervisor (the same user, higher integrity) can open it while
    /// another user on the machine cannot. One instance, because exactly one
    /// supervisor is expected and a second connection would mean something is
    /// wrong rather than something to serve.
    fn create(name: &str) -> Option<Self> {
        use std::ffi::{c_void, OsStr};
        use std::os::windows::ffi::OsStrExt;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateNamedPipeW(
                name: *const u16,
                open_mode: u32,
                pipe_mode: u32,
                max_instances: u32,
                out_buf: u32,
                in_buf: u32,
                default_timeout: u32,
                security: *mut c_void,
            ) -> *mut c_void;
        }

        const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
        // Byte stream, blocking. The link carries one short line each way, so
        // there is nothing message framing would buy.
        const PIPE_TYPE_BYTE: u32 = 0x0000_0000;
        const PIPE_WAIT: u32 = 0x0000_0000;
        const INVALID_HANDLE_VALUE: isize = -1;

        let wide: Vec<u16> = OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that outlives the
        // call, and a null security pointer asks for the default descriptor.
        let h = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                512,
                512,
                0,
                std::ptr::null_mut(),
            )
        } as isize;
        (h != INVALID_HANDLE_VALUE && h != 0).then_some(Self(h))
    }

    /// Block until the supervisor connects, then hand back the pipe as a file.
    ///
    /// `File` because it owns the handle and closes it on drop — and closing is
    /// precisely the signal the supervisor waits for, so ownership and meaning
    /// line up rather than needing to be remembered.
    fn accept(self) -> Option<SupLink> {
        use std::ffi::c_void;
        use std::os::windows::io::FromRawHandle;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ConnectNamedPipe(handle: *mut c_void, overlapped: *mut c_void) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
            fn GetLastError() -> u32;
        }
        // The client can win the race and connect before we ask; that is a
        // success reported as an error, and the only one worth accepting.
        const ERROR_PIPE_CONNECTED: u32 = 535;

        // SAFETY: `self.0` came from `CreateNamedPipeW` above and is still open;
        // a null overlapped pointer asks for the blocking form.
        let ok = unsafe { ConnectNamedPipe(self.0 as *mut c_void, std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !ok {
            // SAFETY: still our handle, and nothing took ownership of it.
            unsafe { CloseHandle(self.0 as *mut c_void) };
            return None;
        }
        // SAFETY: ownership of the handle moves into the `File`, which closes it
        // exactly once; `self` is consumed so it cannot be closed twice.
        Some(unsafe { std::fs::File::from_raw_handle(self.0 as *mut c_void) })
    }
}

/// Which binary is elevated to be the supervisor: **the running executable**.
///
/// This is a security boundary, not a lookup convenience. The supervisor is
/// handed to `pkexec`/`sudo`/UAC *by path*, so whatever path is chosen here is
/// the thing that ends up running as root. Choosing a **sibling file** — as this
/// used to, resolving `limen-cli` next to us — meant root was granted to a file
/// that nothing in Limen ever checks: it is not in a module directory, so it is
/// covered by no content digest, no lockfile entry and no trust approval. On a
/// portable install that file sits on the USB stick, writable by anything that
/// stick is ever plugged into. Replacing it was enough to get root on the *next*
/// machine, at the moment the user approved an ordinary, expected scan prompt.
///
/// The running executable is the one binary we cannot be tricked about: it is
/// already executing, so an attacker who can change it has by definition already
/// won. Elevating it narrows the target from "either of two files, one of which
/// is never looked at" to "the binary the user actually launched" — which is the
/// irreducible baseline. Every Limen binary therefore understands `supervise`
/// (see [`crate::supervise`]).
///
/// The debug fallback stays for tests only: Cargo puts test binaries in
/// `target/debug/deps/`, and a test binary cannot supervise, so a test elevates
/// `limen-cli` from `target/debug/` instead. It is compiled out of release
/// builds, where it would widen exactly what this function exists to narrow.
pub(crate) fn supervisor_bin() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if can_supervise(&exe) {
        return Some(exe);
    }
    #[cfg(debug_assertions)]
    {
        // A test binary: fall back to the real CLI beside or above us.
        let name = if cfg!(windows) { "limen-cli.exe" } else { "limen-cli" };
        let dir = exe.parent()?;
        for base in [dir, dir.parent()?] {
            let candidate = base.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Whether `exe` is a Limen binary — one that answers `supervise`.
///
/// Matched on the name rather than by asking it, because asking means running
/// it. The set is every binary this workspace produces; a test harness
/// (`supervisor-122a739f`, under `deps/`) does not match, which is what sends
/// the debug fallback down its other path.
///
/// The extension is checked as well as the stem, and not out of tidiness:
/// `file_stem` alone reads `limen-cli.old` as `limen-cli`, so a leftover from an
/// update — or a file an attacker simply named that — would have passed as one
/// of ours. Case-insensitively, because Windows filesystems are.
fn can_supervise(exe: &std::path::Path) -> bool {
    let extension_ok = match exe.extension().and_then(|e| e.to_str()) {
        None => true,
        Some(ext) => ext.eq_ignore_ascii_case("exe"),
    };
    extension_ok
        && exe.file_stem().and_then(|s| s.to_str()).is_some_and(|stem| {
            ["limen", "limen-cli", "limen-gui"]
                .iter()
                .any(|ours| stem.eq_ignore_ascii_case(ours))
        })
}

/// Wrap `argv` so it runs under a supervisor that outlives the authorization but
/// not Limen.
///
/// A root child cannot be signalled by an unprivileged parent, so the privileges
/// go to something that stays: the supervisor is elevated, spawns the command
/// itself, and ends it when this socket closes or when told to. One
/// authorization, and control of it afterwards.
///
/// `None` when there is no supervisor to use — the caller then elevates the
/// command directly, as before.
#[cfg(unix)]
#[doc(hidden)]
pub fn supervised(
    id: u64,
    argv: &[String],
    cwd: Option<&str>,
) -> Option<(Vec<String>, SupServer, std::path::PathBuf)> {
    let bin = supervisor_bin()?;
    // Short path: AF_UNIX truncates around 108 bytes, and the module directory
    // is nowhere near short enough.
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let sock = dir.join(format!("limen-sup-{}-{id}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let listener = std::os::unix::net::UnixListener::bind(&sock).ok()?;
    // Only this user may connect: the supervisor runs as root on our say-so.
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));
    }

    let mut out = vec![
        bin.to_string_lossy().into_owned(),
        "supervise".into(),
        "--connect".into(),
        sock.to_string_lossy().into_owned(),
    ];
    if let Some(d) = cwd {
        out.push("--cwd".into());
        out.push(d.to_string());
    }
    out.push("--".into());
    out.extend(argv.iter().cloned());
    Some((out, listener, sock))
}

/// The same, over a named pipe.
///
/// The reason is the same as on Unix and so is the shape: an elevated process
/// runs at an integrity level this one cannot touch, so `OpenProcess` for
/// termination is refused exactly as `kill` is. The privileges therefore go to
/// the supervisor, which is the command's parent and can end it.
///
/// The pipe name carries this process's id as well as the elevation id, so two
/// copies of Limen running at once cannot collide on it.
#[cfg(windows)]
#[doc(hidden)]
pub fn supervised(
    id: u64,
    argv: &[String],
    cwd: Option<&str>,
) -> Option<(Vec<String>, SupServer, std::path::PathBuf)> {
    let bin = supervisor_bin()?;
    let name = format!(r"\\.\pipe\limen-sup-{}-{id}", std::process::id());
    let server = PipeServer::create(&name)?;

    let mut out = vec![
        bin.to_string_lossy().into_owned(),
        "supervise".into(),
        "--connect".into(),
        name.clone(),
    ];
    if let Some(d) = cwd {
        out.push("--cwd".into());
        out.push(d.to_string());
    }
    out.push("--".into());
    out.extend(argv.iter().cloned());
    // Nothing to clean up afterwards — the path is returned for symmetry with
    // the Unix socket, and `sup_cleanup` does nothing with it here.
    Some((out, server, std::path::PathBuf::from(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The set that may be elevated is the workspace's own binaries — nothing
    /// else, because whatever this admits is what `pkexec` runs as root.
    #[test]
    fn only_a_limen_binary_may_be_elevated() {
        // Including the capitalised `Limen` the GUI ships as, and the `.exe`
        // the same binaries carry on Windows.
        for ours in ["limen", "limen-cli", "limen-gui", "Limen"] {
            assert!(can_supervise(Path::new(&format!("/usb/{ours}"))), "{ours}");
            assert!(can_supervise(Path::new(&format!("/usb/{ours}.exe"))), "{ours}.exe");
        }
        // A neighbour on the stick with a plausible name is still not ours. This
        // is the whole point: the old lookup took any file called `limen-cli`
        // sitting next to us, and that file is writable by every machine the
        // stick has ever been plugged into.
        // `limen-cli.old` is the one to watch: `file_stem` reads it as
        // `limen-cli`, so on the stem alone an update leftover — or a file an
        // attacker simply named that — would have been accepted as ours.
        for theirs in [
            "limen-helper", "limenx", "curl", "sh", "Limen-setup",
            "limen-cli.old", "limen.exe.bak", "limen-cli.sh",
        ] {
            assert!(!can_supervise(Path::new(&format!("/usb/{theirs}"))), "{theirs}");
        }
    }

    /// A test binary is not a supervisor: it does not answer `supervise`, so
    /// elevating it would run the test harness as root and supervise nothing.
    #[test]
    fn a_test_binary_is_never_offered_as_the_supervisor() {
        let exe = std::env::current_exe().expect("our own path");
        assert!(
            !can_supervise(&exe),
            "the test harness {} passed as a Limen binary",
            exe.display()
        );
        // Which is exactly why the debug fallback exists — and it must point at
        // something else, never back at us.
        if let Some(bin) = supervisor_bin() {
            assert_ne!(bin, exe, "supervisor_bin handed back the test binary");
        }
    }
}
