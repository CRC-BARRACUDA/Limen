//! Keep spawned child processes from flashing a console window on Windows.
//!
//! The GUI (`Limen.exe`) is built as a Windows GUI-subsystem binary, so it has no
//! console of its own. But when it spawns a **console-subsystem** child (`curl`,
//! `git`, `tar`, a Python module, PowerShell for toasts, …) Windows allocates a
//! fresh console for that child — which flashes on screen for a moment. Setting
//! the `CREATE_NO_WINDOW` creation flag suppresses that console. No-op elsewhere.

use std::process::Command;

/// Extension for [`Command`] that hides the child's console window on Windows.
pub trait NoConsole {
    /// On Windows, spawn the child with `CREATE_NO_WINDOW` so no console window
    /// appears. A no-op on other platforms. Chainable like the other `Command`
    /// builder methods.
    fn no_console(&mut self) -> &mut Self;
}

impl NoConsole for Command {
    #[cfg(windows)]
    fn no_console(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        // winbase.h CREATE_NO_WINDOW — run without allocating a console.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }

    #[cfg(not(windows))]
    fn no_console(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
