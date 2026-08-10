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
pub(crate) type SupLink = std::os::unix::net::UnixStream;
#[cfg(windows)]
pub(crate) type SupLink = std::fs::File;

/// What waits for the supervisor to arrive.
#[cfg(unix)]
pub(crate) type SupServer = std::os::unix::net::UnixListener;
#[cfg(windows)]
pub(crate) type SupServer = PipeServer;

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
pub(crate) fn sup_accept(server: SupServer) -> Option<SupLink> {
    server.accept().ok().map(|(stream, _)| stream)
}

#[cfg(windows)]
pub(crate) fn sup_accept(server: SupServer) -> Option<SupLink> {
    server.accept()
}

/// Clear away whatever the server left on disk. A Unix socket is a file and
/// stays until removed; a named pipe is not a file and goes with its handle.
#[cfg(unix)]
pub(crate) fn sup_cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(windows)]
pub(crate) fn sup_cleanup(_path: &Path) {}

/// The server end of a supervisor pipe.
///
/// Windows has no separate listener to accept from: a named pipe *is* both, and
/// `ConnectNamedPipe` waits for the client on the very handle that then carries
/// the traffic. Held as an `isize` rather than a `HANDLE` so the value can move
/// to the thread that waits — a raw pointer is not `Send`, and this one is only
/// ever used by the thread it is handed to.
#[cfg(windows)]
pub(crate) struct PipeServer(isize);

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

/// Where the supervisor binary lives: beside the running executable.
///
/// In a debug build the directory above is tried too, and only then: Cargo puts
/// test binaries in `target/debug/deps/` while `limen-cli` sits in
/// `target/debug/`, so without this the supervisor is unreachable from a test
/// and the whole path would go unexercised. A release install has both in one
/// directory, so the fallback never applies where it would widen what can be
/// elevated.
pub(crate) fn supervisor_bin() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "limen-cli.exe" } else { "limen-cli" };
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let beside = dir.join(name);
    if beside.exists() {
        return Some(beside);
    }
    if cfg!(debug_assertions) {
        let above = dir.parent()?.join(name);
        if above.exists() {
            return Some(above);
        }
    }
    None
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
pub(crate) fn supervised(
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
pub(crate) fn supervised(
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

/// The supervisor, without the elevation.
///
/// Supervising and elevating are separable, and only the first half is ours:
/// `supervised` builds the command, the link carries the pid, and dropping the
/// link ends the child. Running that command unelevated exercises every one of
/// those and leaves out only the platform's authorization prompt — which cannot
/// be driven from a test anyway, and which is the same call that shipped before
/// this.
#[cfg(all(test, any(unix, windows)))]
mod supervisor_tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    /// Whether a pid is still alive. Asked about the *command*, which is not our
    /// child and so cannot be waited on — only observed.
    fn still_running(pid: u32) -> bool {
        #[cfg(windows)]
        {
            use limen_proto::NoConsole;
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
            let out = std::process::Command::new(format!(r"{root}\System32\tasklist.exe"))
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .no_console()
                .output()
                .expect("ask tasklist");
            // With no match it says so in prose rather than listing nothing.
            String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
        }
        #[cfg(unix)]
        {
            std::path::Path::new(&format!("/proc/{pid}")).exists()
        }
    }

    /// Something that runs long enough to still be there when we look.
    fn slow_command() -> Vec<String> {
        if cfg!(windows) {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
            vec![
                format!(r"{root}\System32\ping.exe"),
                "-n".into(),
                "30".into(),
                "127.0.0.1".into(),
            ]
        } else {
            vec!["/bin/sleep".to_string(), "30".to_string()]
        }
    }

    #[test]
    fn the_supervisor_reports_its_child_and_ends_it_when_the_link_closes() {
        use limen_proto::NoConsole;

        let Some((argv, server, sock)) = supervised(9001, &slow_command(), None) else {
            // `cargo build` has not produced limen-cli yet; nothing to talk to.
            eprintln!("skipped: no limen-cli beside the test binary — run `cargo build` first");
            return;
        };

        // Run the supervisor as an ordinary child. Elevated it would be beyond a
        // test's reach; everything the test is about is the same either way.
        let mut sup = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .no_console()
            .spawn()
            .expect("start the supervisor");

        let link = sup_accept(server).expect("the supervisor connects back");

        // It reports the pid of the command it started, which is what a caller
        // later stops by.
        let mut line = String::new();
        BufReader::new(link.try_clone().expect("clone the link"))
            .read_line(&mut line)
            .expect("read the pid it reports");
        let mut words = line.split_whitespace();
        assert_eq!(words.next(), Some("started"), "unexpected greeting: {line:?}");
        let pid: u32 = words
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no pid in {line:?}"));
        assert!(pid > 0);

        // Still holding the link, so the command is still wanted — and running.
        assert!(
            sup.try_wait().expect("poll the supervisor").is_none(),
            "the supervisor left while the link was open"
        );
        assert!(still_running(pid), "the command was not started");

        // Closing the link is the whole mechanism — no message, no signal, and
        // nothing that has to run on the way out.
        drop(link);

        let left = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            matches!(sup.try_wait(), Ok(Some(_)))
        });
        assert!(left, "the supervisor outlived the link");

        // The point of all of it: the command goes too, rather than being
        // orphaned to run on with nobody left to read its report.
        let gone = (0..40).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            !still_running(pid)
        });
        assert!(gone, "the command outlived the supervisor: pid {pid}");
        sup_cleanup(&sock);
    }
}
