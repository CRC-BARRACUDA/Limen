//! Language-runtime discovery for scripted modules.
//!
//! Limen is portable: it prefers a **bundled** interpreter shipped next to the
//! executable (under `<base>/runtimes/<lang>/`) so users don't have to install
//! Python/Lua/Node themselves. If there's no bundled interpreter it falls back to
//! one on the system `PATH`; if neither exists the runtime is *missing* and the
//! GUI offers Quick Setup to download it.
//!
//! Paths are platform-aware (Windows vs. Unix).

use std::path::{Path, PathBuf};

use limen_proto::Language;

/// A scripted-language runtime Limen can launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Python,
    Lua,
    Js,
}

impl Runtime {
    /// The scripted runtime a module language needs (native modules need none).
    pub fn for_language(language: Language) -> Option<Runtime> {
        match language {
            Language::Python => Some(Runtime::Python),
            Language::Lua => Some(Runtime::Lua),
            Language::Js => Some(Runtime::Js),
            Language::Native => None,
        }
    }

    /// Directory name under `<base>/runtimes/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            Runtime::Python => "python",
            Runtime::Lua => "lua",
            Runtime::Js => "node",
        }
    }

    /// Human-readable name.
    pub fn display(self) -> &'static str {
        match self {
            Runtime::Python => "Python",
            Runtime::Lua => "Lua",
            Runtime::Js => "JavaScript (Node)",
        }
    }

    /// The command name to look for on the system `PATH`.
    pub fn system_cmd(self) -> &'static str {
        match self {
            Runtime::Python => "python3",
            Runtime::Lua => "lua",
            Runtime::Js => "node",
        }
    }

    /// Path of the bundled interpreter *relative to* `<base>/runtimes/<dir>/`,
    /// per platform (Windows layout vs. Unix layout).
    fn bundled_rel(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Runtime::Python, true) => "python.exe",
            (Runtime::Python, false) => "bin/python3",
            (Runtime::Lua, true) => "lua.exe",
            (Runtime::Lua, false) => "lua",
            (Runtime::Js, true) => "node.exe",
            (Runtime::Js, false) => "bin/node",
        }
    }

    /// All runtimes, for enumeration.
    pub fn all() -> [Runtime; 3] {
        [Runtime::Python, Runtime::Lua, Runtime::Js]
    }
}

/// Where a runtime is available from.
#[derive(Debug, Clone)]
pub enum RuntimeStatus {
    /// Bundled next to the binary (the portable path).
    Bundled(PathBuf),
    /// Found on the system PATH (uses the user's own install).
    System(PathBuf),
    /// Not available — needs Quick Setup.
    Missing,
}

impl RuntimeStatus {
    pub fn is_available(&self) -> bool {
        !matches!(self, RuntimeStatus::Missing)
    }

    /// The launch command for this runtime, if available.
    pub fn command(&self) -> Option<String> {
        match self {
            RuntimeStatus::Bundled(p) | RuntimeStatus::System(p) => {
                Some(p.to_string_lossy().into_owned())
            }
            RuntimeStatus::Missing => None,
        }
    }
}

/// `<base>/runtimes` — where bundled interpreters live.
pub fn runtimes_dir(base: &Path) -> PathBuf {
    base.join("runtimes")
}

/// The bundled interpreter for `rt`, if present next to the binary.
pub fn bundled(base: &Path, rt: Runtime) -> Option<PathBuf> {
    let p = runtimes_dir(base).join(rt.dir_name()).join(rt.bundled_rel());
    p.exists().then_some(p)
}

/// Find `cmd` on the system `PATH` (adds `.exe` on Windows).
pub fn on_path(cmd: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{cmd}.exe")
    } else {
        cmd.to_string()
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(&exe))
        .find(|c| c.is_file())
}

/// Resolve where `rt` comes from: bundled first, then system PATH.
pub fn status(base: &Path, rt: Runtime) -> RuntimeStatus {
    if let Some(p) = bundled(base, rt) {
        return RuntimeStatus::Bundled(p);
    }
    if let Some(p) = on_path(rt.system_cmd()) {
        return RuntimeStatus::System(p);
    }
    RuntimeStatus::Missing
}

/// The launch command for `rt` (bundled-first), or `None` if missing.
pub fn resolve(base: &Path, rt: Runtime) -> Option<String> {
    status(base, rt).command()
}

// --------------------------------------------------------------------------- //
// Quick Setup — download a portable interpreter next to the binary.
// --------------------------------------------------------------------------- //

/// A downloadable portable interpreter for one runtime + platform.
struct Source {
    url: &'static str,
    /// The archive's top-level directory, if it must be renamed to `dir_name()`.
    /// `None` means the archive already extracts to `dir_name()`.
    top_dir: Option<&'static str>,
}

/// The download source for `rt` on the current OS/arch, if one is configured.
///
/// NOTE: only linux-x86_64 Python is verified. Windows/macOS and Node/Lua entries
/// need real-world testing; add/adjust here as they're confirmed.
fn source(rt: Runtime) -> Option<Source> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (rt, os, arch) {
        // python-build-standalone `install_only` extracts to `python/bin/python3`.
        (Runtime::Python, "linux", "x86_64") => Some(Source {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20240814/cpython-3.12.5+20240814-x86_64-unknown-linux-gnu-install_only.tar.gz",
            top_dir: None,
        }),
        (Runtime::Python, "windows", "x86_64") => Some(Source {
            url: "https://github.com/astral-sh/python-build-standalone/releases/download/20240814/cpython-3.12.5+20240814-x86_64-pc-windows-msvc-shared-install_only.tar.gz",
            top_dir: None,
        }),
        // Node official dist extracts to `node-vX-<os>-<arch>/bin/node`.
        (Runtime::Js, "linux", "x86_64") => Some(Source {
            url: "https://nodejs.org/dist/v20.17.0/node-v20.17.0-linux-x64.tar.xz",
            top_dir: Some("node-v20.17.0-linux-x64"),
        }),
        // Lua has no canonical portable binary distribution — source TBD.
        _ => None,
    }
}

/// Whether Quick Setup can download `rt` on this platform.
pub fn can_install(rt: Runtime) -> bool {
    source(rt).is_some()
}

/// Download and install the portable interpreter for `rt` into
/// `<base>/runtimes/<dir>/`. Replaces any existing bundled copy. Uses `curl` and
/// `tar`; no extra Rust dependencies.
pub fn install(base: &Path, rt: Runtime) -> Result<(), String> {
    let src = source(rt).ok_or_else(|| {
        format!(
            "no download available for {} on {}/{}",
            rt.display(),
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let runtimes = runtimes_dir(base);
    std::fs::create_dir_all(&runtimes).map_err(|e| format!("creating runtimes dir: {e}"))?;

    // Download to a temp file next to the runtimes dir.
    let archive = runtimes.join(format!(".{}-download", rt.dir_name()));
    let status = std::process::Command::new("curl")
        .args(["-sSL", "-o"])
        .arg(&archive)
        .arg(src.url)
        .status()
        .map_err(|e| format!("running curl (is it installed?): {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&archive);
        return Err(format!("download failed (curl exit {status})"));
    }

    // Extract into the runtimes dir (tar auto-detects gz/xz).
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&runtimes)
        .status()
        .map_err(|e| format!("running tar (is it installed?): {e}"))?;
    let _ = std::fs::remove_file(&archive);
    if !status.success() {
        return Err(format!("extraction failed (tar exit {status})"));
    }

    // Normalize the extracted directory to `<dir_name>`.
    let dest = runtimes.join(rt.dir_name());
    if let Some(top) = src.top_dir {
        let extracted = runtimes.join(top);
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&extracted, &dest)
            .map_err(|e| format!("placing interpreter: {e}"))?;
    }

    // Verify it's now discoverable.
    if bundled(base, rt).is_none() {
        return Err(format!(
            "installed {} but no interpreter at the expected path",
            rt.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_bundled_is_system_or_missing() {
        // An empty base has no bundled interpreter, so status must come from PATH
        // (System) or be Missing — never Bundled.
        let base = std::env::temp_dir().join("limen-rt-none");
        assert!(!matches!(
            status(&base, Runtime::Python),
            RuntimeStatus::Bundled(_)
        ));
    }

    /// Network + ~30MB download. Run explicitly:
    ///   cargo test -p limen-host --  --ignored quick_setup_python
    #[test]
    #[ignore]
    fn quick_setup_python() {
        let base = std::env::temp_dir().join(format!("limen-rt-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        install(&base, Runtime::Python).expect("install python");

        // Now it should resolve as Bundled and actually run.
        let cmd = match status(&base, Runtime::Python) {
            RuntimeStatus::Bundled(p) => p,
            other => panic!("expected Bundled, got {other:?}"),
        };
        let out = std::process::Command::new(&cmd)
            .args(["-c", "print(1+1)"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2");

        std::fs::remove_dir_all(&base).ok();
    }
}
