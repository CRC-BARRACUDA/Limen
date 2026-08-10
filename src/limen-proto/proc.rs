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
