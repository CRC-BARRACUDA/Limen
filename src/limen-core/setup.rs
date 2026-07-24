//! Runtime setup status — which scripted interpreters are available, for the
//! GUI's Quick Setup flow.

use limen_host::runtimes::{self, Runtime, RuntimeStatus};

use crate::paths;

/// Where a runtime comes from (a frontend-friendly flattening of
/// [`RuntimeStatus`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    /// Bundled next to the binary (portable).
    Bundled,
    /// Provided by the user's system PATH.
    System,
    /// Not available — needs Quick Setup.
    Missing,
}

/// A runtime and its current availability.
#[derive(Debug, Clone)]
pub struct RuntimeInfo {
    pub runtime: Runtime,
    pub display: String,
    pub state: RuntimeState,
    /// Resolved interpreter path/command, if available.
    pub location: Option<String>,
}

fn info(rt: Runtime) -> RuntimeInfo {
    let status = runtimes::status(&paths::home(), rt);
    let state = match status {
        RuntimeStatus::Bundled(_) => RuntimeState::Bundled,
        RuntimeStatus::System(_) => RuntimeState::System,
        RuntimeStatus::Missing => RuntimeState::Missing,
    };
    RuntimeInfo {
        runtime: rt,
        display: rt.display().to_string(),
        state,
        location: status.command(),
    }
}

/// Availability of all three scripted runtimes.
pub fn runtime_report() -> Vec<RuntimeInfo> {
    Runtime::all().into_iter().map(info).collect()
}

/// Whether Quick Setup can download `rt` on this platform.
pub fn can_install(rt: Runtime) -> bool {
    runtimes::can_install(rt)
}

/// Download + install the portable interpreter for `rt` next to the binary.
/// Blocking (network); run off the UI thread.
pub fn install_runtime(rt: Runtime) -> Result<(), String> {
    runtimes::install(&paths::home(), rt)
}
