//! Spawning a child without a console window flashing on Windows.

use std::process::Command;

use limen_proto::proc::*;

/// A command that prints a known token, per platform. Args are added *after*
/// `no_console()` at the call site, so it also exercises chaining.
fn echo() -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", "echo"]);
        c
    }
    #[cfg(not(windows))]
    {
        Command::new("echo")
    }
}

#[test]
fn no_console_is_chainable() {
    // `no_console()` must return `&mut Command` (the same builder), so calls
    // can be chained before and after it — here we configure an arg *after*.
    let mut cmd = echo();
    let before = &cmd as *const Command;
    let after = cmd.no_console() as *const Command;
    assert!(
        std::ptr::eq(before, after),
        "no_console must return the same Command it was called on"
    );
}

#[test]
fn command_still_runs_with_no_console() {
    // On Windows this actually sets CREATE_NO_WINDOW; the child must still run
    // and produce its output (the flag suppresses the console, nothing else).
    let out = echo()
        .no_console()
        .arg("limen-ok")
        .output()
        .expect("spawning an echo configured after no_console() should succeed");
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("limen-ok"),
        "child output should carry the token"
    );
}
