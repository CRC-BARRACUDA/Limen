//! The `limen.toml` manifest schema.
//!
//! Every module ships a `limen.toml` declaring who it is, how to launch it, the
//! capabilities it **provides**, and the capabilities it **requires** from other
//! modules. The host reads these to build the dependency graph and to wire the
//! capability broker, so a module never needs to know *how* its dependencies are
//! implemented — only which capabilities it needs.
//!
//! ```toml
//! [module]
//! name = "usb"
//! version = "0.1.0"
//! language = "python"     # python | lua | js | native
//! entry = "main.py"
//! abi = "rpc"             # rpc (default, over stdio) | native (in-process C-ABI)
//!
//! [provides]
//! capabilities = ["usb.enumerate"]
//!
//! [requires.capabilities]
//! "crowdstrike.rtr" = ">=1.0"
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// The implementation language of a module. Determines how the host launches it
/// (which interpreter, or run the native binary/library directly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Lua,
    Js,
    /// A compiled artifact (Rust, C, Go, …) — a self-contained executable when
    /// `abi = "rpc"`, or a dynamic library when `abi = "native"`.
    Native,
}

/// How the host talks to the module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Abi {
    /// JSON-RPC over stdio in a child process (the safe, language-agnostic
    /// default — the only transport wired up in Phase 1).
    #[default]
    Rpc,
    /// In-process dynamic library exposing Limen's stable C ABI (Phase 2).
    Native,
}

/// The `[module]` table.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleMeta {
    pub name: String,
    pub version: String,
    pub language: Language,
    /// Path to the entry point, relative to the manifest's directory (a script
    /// for interpreted languages, a binary/library name for `native`).
    pub entry: String,
    #[serde(default)]
    pub abi: Abi,
    /// A pretty display name for the module list (the default/English one;
    /// localizable via `locales/<lang>.toml` `[module] title`). Falls back to
    /// `name` (the identifier) when absent.
    #[serde(default)]
    pub display_name: Option<String>,
    /// One-line human description (shown in the module list).
    #[serde(default)]
    pub description: Option<String>,
    /// Module authors (shown in the module list).
    #[serde(default)]
    pub authors: Vec<String>,
    /// Free-form tags for grouping and filtering in the module manager, e.g.
    /// `["security", "inventory", "ukraine"]`. Lowercase single words by
    /// convention; the manager matches them in its search box and lets the user
    /// click one to filter by it.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The module's GitHub repo (`owner/repo` or a full URL), for the
    /// "view on GitHub" action in the module list.
    #[serde(default)]
    pub repo: Option<String>,
}

/// The `[provides]` table.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Provides {
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// The `[requires]` table. `capabilities` maps a capability name to a semver
/// requirement string (e.g. `">=1.0"`). This is *runtime* wiring — the host
/// matches it against whatever modules are installed.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Requires {
    #[serde(default)]
    pub capabilities: BTreeMap<String, String>,
}

/// A `[dependencies]` entry: which *package* to install so a required capability
/// is present. Distinct from `[requires.capabilities]` (runtime): this is what
/// the GitHub manager resolves and downloads. Either a short git ref string
/// (`"owner/repo@1.2.0"`) or a detailed table.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Shorthand git ref: `"owner/repo"` or `"owner/repo@version"`.
    Short(String),
    Detailed(DetailedDep),
}

/// The table form of a dependency. Exactly one of `repo` (GitHub) or `path`
/// (local, resolved relative to the depending module) should be set.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DetailedDep {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// The `[permissions]` table — what a module declares it needs to do. The host
/// surfaces these for consent before running untrusted (e.g. GitHub-sourced)
/// code, given that a module can execute scripts across a fleet.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Permissions {
    /// Hosts/domains the module may reach over the network.
    #[serde(default)]
    pub network: Vec<String>,
    /// Filesystem paths the module may touch.
    #[serde(default)]
    pub filesystem: Vec<String>,
    /// May execute scripts on managed fleet hosts.
    #[serde(default)]
    pub run_hosts: bool,
    /// May spawn local subprocesses.
    #[serde(default)]
    pub subprocess: bool,
    /// The whole module requires elevated / administrator privileges (every
    /// method is gated). For finer control, use `elevated_methods` instead.
    #[serde(default)]
    pub admin: bool,
    /// Specific method names that require the user's approval before they run
    /// (e.g. `["restart_service"]`). The rest of the module runs freely; the
    /// consent prompt only appears when one of these is actually invoked.
    #[serde(default)]
    pub elevated_methods: Vec<String>,
    /// Some of the module's methods may need elevated / administrator privileges
    /// **on the host running Limen** to work fully (e.g. reading protected system
    /// state). This is purely a heads-up the UI surfaces — it does not gate,
    /// prompt, or list which methods. (Scripts run on *remote* fleet hosts via
    /// RTR are not this: they run elevated on the remote machine, not the host.)
    #[serde(default)]
    pub may_require_admin: bool,
    /// May ask the host to run a command with administrator / root privileges,
    /// via `host.elevate`.
    ///
    /// Declared separately from [`subprocess`](Self::subprocess) because it is a
    /// different thing to consent to: a module that may spawn processes runs
    /// them as you, while this one asks the operating system to run something as
    /// root. The host refuses `host.elevate` unless this is set, so a module
    /// cannot reach for it without having said so where the user can read it.
    #[serde(default)]
    pub elevate: bool,
}

impl Permissions {
    /// Whether any declared permission is security-relevant (worth consenting to).
    pub fn sensitive(&self) -> bool {
        self.admin
            || self.elevate
            || self.run_hosts
            || self.subprocess
            || !self.elevated_methods.is_empty()
            || !self.network.is_empty()
            || !self.filesystem.is_empty()
    }

    /// Whether invoking `method` needs the user's approval: the whole module is
    /// admin-gated, or the method is explicitly listed as elevated.
    pub fn method_needs_consent(&self, method: &str) -> bool {
        self.admin || self.elevated_methods.iter().any(|m| m == method)
    }

    /// Human-readable descriptions of what the module is asking for. The most
    /// sensitive (admin) is listed first.
    pub fn summary(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.admin {
            out.push("administrator privileges".to_string());
        }
        if self.run_hosts {
            out.push("execute on fleet hosts".to_string());
        }
        if self.subprocess {
            out.push("spawn local processes".to_string());
        }
        if self.elevate {
            // Worded so it reads as what it is on the consent screen: the module
            // does not become root, it asks the OS to run something as root.
            out.push("run commands as administrator".to_string());
        }
        if !self.network.is_empty() {
            out.push(format!("network: {}", self.network.join(", ")));
        }
        if !self.filesystem.is_empty() {
            out.push(format!("filesystem: {}", self.filesystem.join(", ")));
        }
        out
    }
}

/// A parsed `limen.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub module: ModuleMeta,
    #[serde(default)]
    pub provides: Provides,
    #[serde(default)]
    pub requires: Requires,
    /// Capabilities this module can *optionally* use if a provider is present —
    /// soft dependencies. Absent providers never block the module; it simply
    /// lights up the extra feature when one is loaded. Same shape as `requires`
    /// (`[optional.capabilities]`).
    #[serde(default)]
    pub optional: Requires,
    /// Packages to install (the manager's download graph). Keyed by module name.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DepSpec>,
    /// What the module is permitted to do (surfaced for consent).
    #[serde(default)]
    pub permissions: Permissions,
}

impl Manifest {
    pub fn from_toml_str(s: &str) -> anyhow::Result<Self> {
        toml::from_str(s).context("parsing limen.toml")
    }

    /// Read and parse `<dir>/limen.toml`.
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("limen.toml");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_toml_str(&text)
    }
}

/// A `[module]` field (`title` / `description`) translated for `lang`, read from
/// `<dir>/locales/<lang>.toml`. `None` for `"en"` (the manifest's own language)
/// or when no such file/key exists — the caller then keeps the manifest default.
fn localized_module_field(dir: &Path, lang: &str, field: &str) -> Option<String> {
    if lang == "en" {
        return None;
    }
    let text = std::fs::read_to_string(dir.join("locales").join(format!("{lang}.toml"))).ok()?;
    let val: toml::Value = toml::from_str(&text).ok()?;
    val.get("module")?.get(field)?.as_str().map(str::to_string)
}

/// A module's card **description** translated for `lang` (falls back to the
/// manifest's default `description`).
pub fn localized_description(dir: &Path, lang: &str) -> Option<String> {
    localized_module_field(dir, lang, "description")
}

/// A module's card **display title** translated for `lang` (falls back to the
/// manifest's `display_name`, then its `name`).
pub fn localized_title(dir: &Path, lang: &str) -> Option<String> {
    localized_module_field(dir, lang, "title")
}

#[cfg(test)]
mod elevate_tests {
    use super::*;

    /// Elevation is opt-in and must be visible. A module that never mentions it
    /// cannot ask for root, and one that does has to show up on the consent
    /// screen saying so — otherwise the permission is a formality.
    #[test]
    fn elevation_is_declared_or_refused() {
        let silent: Permissions = toml::from_str("subprocess = true").unwrap();
        assert!(!silent.elevate, "not asking is the default");
        assert!(!silent.summary().iter().any(|p| p.contains("administrator")));

        let asking: Permissions = toml::from_str("subprocess = true\nelevate = true").unwrap();
        assert!(asking.elevate);
        assert!(asking.sensitive(), "it must require consent");
        assert!(
            asking
                .summary()
                .iter()
                .any(|p| p == "run commands as administrator"),
            "and say so in words the consent screen shows: {:?}",
            asking.summary()
        );
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_manifest() {
        let m = Manifest::from_toml_str(
            r#"
            [module]
            name = "usb"
            version = "0.1.0"
            language = "python"
            entry = "main.py"

            [provides]
            capabilities = ["usb.enumerate"]

            [requires.capabilities]
            "crowdstrike.rtr" = ">=1.0"
            "#,
        )
        .unwrap();

        assert_eq!(m.module.name, "usb");
        assert_eq!(m.module.language, Language::Python);
        assert_eq!(m.module.abi, Abi::Rpc); // defaulted
        assert_eq!(m.provides.capabilities, vec!["usb.enumerate"]);
        assert_eq!(m.requires.capabilities["crowdstrike.rtr"], ">=1.0");
        // No [permissions] table => nothing sensitive.
        assert!(!m.permissions.sensitive());
        // Absent `tags` is an empty list, never an error — every manifest
        // predating the field must still parse.
        assert!(m.module.tags.is_empty());
    }

    /// Tags are optional metadata the module manager groups and filters by.
    #[test]
    fn parses_tags() {
        let m = Manifest::from_toml_str(
            r#"
            [module]
            name = "usb"
            version = "0.1.0"
            language = "python"
            entry = "main.py"
            tags = ["security", "inventory"]
            "#,
        )
        .unwrap();

        assert_eq!(m.module.tags, vec!["security", "inventory"]);
    }

    #[test]
    fn parses_permissions_and_flags_sensitive() {
        let m = Manifest::from_toml_str(
            r#"
            [module]
            name = "crowdstrike"
            version = "0.1.0"
            language = "native"
            entry = "crowdstrike"
            abi = "native"

            [permissions]
            run_hosts = true
            network = ["api.crowdstrike.com"]
            "#,
        )
        .unwrap();

        assert!(m.permissions.run_hosts);
        assert!(m.permissions.sensitive());
        assert!(m.permissions.summary().iter().any(|s| s.contains("fleet hosts")));
    }

    #[test]
    fn admin_is_sensitive_and_listed_first() {
        let m = Manifest::from_toml_str(
            "[module]\nname=\"a\"\nversion=\"0.1.0\"\nlanguage=\"python\"\nentry=\"m.py\"\n\
             [permissions]\nadmin = true\n",
        )
        .unwrap();
        assert!(m.permissions.admin);
        assert!(m.permissions.sensitive());
        assert_eq!(m.permissions.summary().first().map(String::as_str), Some("administrator privileges"));
    }
}
