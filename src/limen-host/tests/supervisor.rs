//! The supervisor: an elevated command dies with the app that started it.

use limen_host::*;

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
