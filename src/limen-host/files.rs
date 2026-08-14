//! Asking the OS to pick a file, and to open one.

use limen_proto::{RpcError};
use serde_json::{json, Value};


/// Show a native "open file" dialog on the host and return the chosen path as
/// `{ "path": "..." }`, or `Null` if the user cancelled. Shells out to the
/// platform's dialog (no extra dependency), matching how `host.open` works.
pub(crate) fn host_pick_file() -> Value {
    match pick_file_native() {
        Some(path) if !path.is_empty() => json!({ "path": path }),
        _ => Value::Null,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn pick_file_native() -> Option<String> {
    let out = std::process::Command::new("zenity")
        .args(["--file-selection", "--title=Choose a file"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // non-zero on cancel
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(target_os = "macos")]
pub(crate) fn pick_file_native() -> Option<String> {
    let out = std::process::Command::new("osascript")
        .args(["-e", "POSIX path of (choose file with prompt \"Choose a file\")"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // user cancelled
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(target_os = "windows")]
pub(crate) fn pick_file_native() -> Option<String> {
    use limen_proto::NoConsole;
    // STA is required for the WinForms dialog; write only the path to stdout.
    let ps = "Add-Type -AssemblyName System.Windows.Forms; \
              $d = New-Object System.Windows.Forms.OpenFileDialog; \
              $d.Filter = 'Images (*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.webp)|*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.webp|All files (*.*)|*.*'; \
              if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { [Console]::Out.Write($d.FileName) }";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-STA", "-Command", ps])
        .no_console()
        .output()
        .ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!p.is_empty()).then_some(p)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) fn pick_file_native() -> Option<String> {
    None
}

/// Open something in the OS on a module's behalf (e.g. the devices module's
/// "Open path" / "Registry" / "Device Manager"). `params`:
/// `{ "target": "path"|"url"|"registry"|"device_manager", "value": "..." }`.
/// Best-effort and fire-and-forget — a launch failure is not a module error.
pub(crate) fn host_open(params: Value) -> std::result::Result<Value, RpcError> {
    let target = params.get("target").and_then(Value::as_str).unwrap_or("path");
    let value = params.get("value").and_then(Value::as_str).unwrap_or("");
    open_target(target, value);
    Ok(Value::Null)
}

#[cfg(target_os = "linux")]
pub(crate) fn open_target(target: &str, value: &str) {
    use std::process::Command;
    // Registry / Device Manager are Windows-only; a path or URL opens in the
    // desktop's default handler (file manager for a directory).
    if matches!(target, "registry" | "device_manager") || value.is_empty() {
        return;
    }
    match target {
        // No portable "select this file" across file managers, so settle for
        // opening the containing directory.
        "reveal" => {
            let dir = std::path::Path::new(value).parent().unwrap_or(std::path::Path::new(value));
            let _ = Command::new("xdg-open").arg(dir).spawn();
        }
        // xdg-open already routes text files to the configured editor.
        _ => {
            let _ = Command::new("xdg-open").arg(value).spawn();
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn open_target(target: &str, value: &str) {
    use std::process::Command;
    if matches!(target, "registry" | "device_manager") || value.is_empty() {
        return;
    }
    match target {
        // -R reveals the file in Finder rather than opening it.
        "reveal" => {
            let _ = Command::new("open").args(["-R", value]).spawn();
        }
        // -t forces the default *text editor* instead of the file's handler.
        "edit" => {
            let _ = Command::new("open").args(["-t", value]).spawn();
        }
        _ => {
            let _ = Command::new("open").arg(value).spawn();
        }
    }
}

/// Launch something through the shell, optionally asking for elevation.
///
/// `CreateProcess` (what `std::process::Command` uses) cannot elevate: launching
/// a program whose manifest demands admin — `regedit` — fails outright with
/// `ERROR_ELEVATION_REQUIRED` (740) and, because these launches are
/// fire-and-forget, the user sees nothing happen at all. `ShellExecuteW` is the
/// API that can raise the UAC prompt, so elevation is *requested* up front via
/// the `runas` verb rather than the launch silently failing.
#[cfg(target_os = "windows")]
pub(crate) fn shell_exec(verb: Option<&str>, file: &str, params: Option<&str>) {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    const SW_SHOWNORMAL: i32 = 1;
    let verb_w = verb.map(wide);
    let file_w = wide(file);
    let params_w = params.map(wide);
    let ptr = |o: &Option<Vec<u16>>| o.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
    // SAFETY: every pointer is either null or a NUL-terminated UTF-16 buffer
    // that outlives the call; a null hwnd/directory means "no owner window" and
    // "inherit the working directory", both valid here.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            ptr(&verb_w),
            file_w.as_ptr(),
            ptr(&params_w),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn open_target(target: &str, value: &str) {
    use limen_proto::NoConsole;
    use std::process::Command;
    match target {
        // A device instance id opens that device's own properties dialog; with
        // no id, fall back to the Device Manager console.
        "device_manager" => {
            if value.is_empty() {
                shell_exec(None, "devmgmt.msc", None);
            } else {
                // Empty /MachineName means the local machine.
                let args = format!(
                    "devmgr.dll,DeviceProperties_RunDLL /MachineName \"\" /DeviceID \"{value}\""
                );
                shell_exec(None, "rundll32.exe", Some(&args));
            }
        }
        // regedit reopens at its stored LastKey — set it, then launch regedit.
        // Writing LastKey is HKCU, so it needs no elevation; regedit itself does.
        "registry" => {
            if !value.is_empty() {
                let _ = Command::new("reg")
                    .args([
                        "add",
                        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Applets\Regedit",
                        "/v", "LastKey", "/t", "REG_SZ", "/d", value, "/f",
                    ])
                    .no_console()
                    .spawn()
                    .and_then(|mut c| c.wait());
            }
            shell_exec(Some("runas"), "regedit.exe", None);
        }
        _ if value.is_empty() => {}
        "url" => {
            let _ = Command::new("cmd").args(["/C", "start", "", value]).no_console().spawn();
        }
        // Open Explorer with the item itself selected, rather than just opening
        // its folder. `/select,<path>` must arrive as ONE argument with the path
        // quoted inside it — Rust's own argument quoting produces a form
        // explorer rejects, so the switch is passed raw.
        "reveal" => {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("explorer")
                .raw_arg(format!("/select,\"{value}\""))
                .no_console()
                .spawn();
        }
        // Show a file's contents as text, whatever its extension says. Notepad
        // is guaranteed present; the default handler would run a .bat, not open it.
        "edit" => {
            let _ = Command::new("notepad.exe").arg(value).no_console().spawn();
        }
        // A filesystem path → open in Explorer.
        _ => {
            let _ = Command::new("explorer").arg(value).spawn();
        }
    }
}
