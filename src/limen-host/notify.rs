//! Desktop notifications, three platforms' worth.

use limen_proto::{RpcError};
use serde_json::{Value};


pub(crate) fn host_notify(params: Value) -> std::result::Result<Value, RpcError> {
    let title = params.get("title").and_then(Value::as_str).unwrap_or("Limen");
    let body = params.get("body").and_then(Value::as_str).unwrap_or("");
    let urgency = params
        .get("urgency")
        .and_then(Value::as_str)
        .filter(|u| matches!(*u, "low" | "normal" | "critical"))
        .unwrap_or("normal");
    notify_native(title, body, urgency);
    Ok(Value::Null)
}

#[cfg(target_os = "linux")]
pub(crate) fn notify_native(title: &str, body: &str, urgency: &str) {
    // Arguments go straight to execve — no shell, so nothing in `title`/`body`
    // can be read as syntax however it is spelled.
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "Limen", "-u", urgency, title, body])
        .spawn();
}

#[cfg(target_os = "macos")]
pub(crate) fn notify_native(title: &str, body: &str, _urgency: &str) {
    // `osascript -e` takes AppleScript *source*, so the text is escaped rather
    // than merely quoted — a stray `"` would otherwise end the literal and the
    // rest would be executed as script.
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .spawn();
}

/// Give Windows an app identity of our own, so notifications are attributed to
/// **Limen** rather than to whatever process happened to raise them.
///
/// A toast from an *unregistered* AUMID is accepted and then silently dropped —
/// which is why borrowing PowerShell's registered id works at all, at the cost
/// of every module notification reading "Windows PowerShell". Registering ours
/// costs one HKCU key and makes the attribution honest. No elevation: this is
/// the user's own hive.
///
/// Done once per process, and idempotent besides. `IconUri` is only set when the
/// icon has actually been extracted (`limen-core` writes it on the app's first
/// notification); a missing file would just render no icon. Note the icon —
/// unlike the name — appears from the next sign-in, because `WpnUserService`
/// caches an app's display data when it starts.
#[cfg(target_os = "windows")]
pub(crate) fn ensure_app_id() {
    use limen_proto::proc::NoConsole;
    use std::sync::OnceLock;
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        const KEY: &str = r"HKCU\Software\Classes\AppUserModelId\Limen";
        let mut values = vec![("DisplayName", APP_ID.to_string())];
        if let Some(icon) = icon_file() {
            values.push(("IconUri", icon.to_string_lossy().into_owned()));
            values.push(("IconBackgroundColor", "00000000".to_string()));
        }
        for (name, value) in values {
            // `.output()` rather than `.status()`: reg's "The operation completed
            // successfully." would otherwise land in the debug build's console.
            let _ = std::process::Command::new("reg")
                .args(["add", KEY, "/v", name, "/t", "REG_SZ", "/d", &value, "/f"])
                .no_console()
                .output();
        }
    });
}

/// The app id Windows attributes Limen's toasts to. Matches the one
/// `limen-core` registers for update notifications, so both speak as one app.
#[cfg(target_os = "windows")]
pub(crate) const APP_ID: &str = "Limen";

/// The extracted app icon, if it is there. `limen-core` unpacks it to
/// `<base>/state/icon.png` at startup; the host only ever reads it, so a missing
/// file is not an error — it just means a toast without a picture.
#[cfg(target_os = "windows")]
pub(crate) fn icon_file() -> Option<std::path::PathBuf> {
    // Imported here rather than at the top of the file: only this function wants
    // it, and it exists only on Windows, so a file-level `use` would be an
    // unused import everywhere else.
    use crate::sdk::limen_home;
    let path = limen_home().join("state").join("icon.png");
    path.is_file().then_some(path)
}

#[cfg(target_os = "windows")]
pub(crate) fn notify_native(title: &str, body: &str, _urgency: &str) {
    use limen_proto::proc::NoConsole;
    // A WinRT ToastGeneric shown through our own registered AUMID — registered
    // first, since an unregistered id is accepted and then silently dropped.
    ensure_app_id();
    // The text lands inside an XML document inside a single-quoted PowerShell
    // string, so it has to survive both: XML entities first, then PowerShell's
    // doubled-quote escape.
    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
    fn ps(s: &str) -> String {
        s.replace('\'', "''")
    }
    // `appLogoOverride` is the thumbnail *beside the text*. It is separate from
    // the small icon in the header, which comes from the AUMID's `IconUri` and
    // only refreshes when `WpnUserService` restarts — this one is per-toast, so
    // it shows immediately. A `file:///` src needs forward slashes.
    let logo = icon_file()
        .map(|p| {
            format!(
                "<image placement=\"appLogoOverride\" src=\"file:///{}\"/>",
                xml(&p.to_string_lossy().replace('\\', "/"))
            )
        })
        .unwrap_or_default();
    let doc = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text>{}</binding></visual></toast>",
        xml(title),
        xml(body),
        logo
    );
    let script = format!(
        "[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
         [void][Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom,ContentType=WindowsRuntime];\
         $x=New-Object Windows.Data.Xml.Dom.XmlDocument;$x.LoadXml('{}');\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{}')\
         .Show((New-Object Windows.UI.Notifications.ToastNotification $x))",
        ps(&doc),
        APP_ID
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .no_console()
        .spawn();
}
