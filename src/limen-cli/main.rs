//! Limen CLI.
//!
//! Discovers modules (from `~/.limen/settings.json`, `--modules-dir`, or a local
//! `./modules` for development), then lists / describes / runs their
//! capabilities. Frontends stay thin: all the work lives in [`limen_core`].
//!
//!   limen modules
//!   limen describe demo.native
//!   limen run demo.native shout --params '{"name":"world"}'
//!   limen demo

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use limen_core::{paths, Config, Engine};
use limen_registry::{digest_dir, Registry, TrustStore, VerifyStatus};
use serde_json::{json, Value};

#[derive(Parser)]
#[command(name = "limen", version, about = "Limen — modular ops console")]
struct Cli {
    /// Directory to search for modules (repeatable). If given, replaces the
    /// configured search paths.
    #[arg(long = "modules-dir", global = true, value_name = "DIR")]
    modules_dir: Vec<PathBuf>,

    /// Run even if untrusted modules request sensitive permissions.
    #[arg(long, global = true)]
    allow_untrusted: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List installed modules and their capabilities.
    Modules,
    /// Show a capability provider's self-description.
    Describe {
        /// e.g. `demo.native`
        capability: String,
    },
    /// Invoke a capability method.
    Run {
        capability: String,
        method: String,
        /// JSON object of parameters.
        #[arg(long, default_value = "{}", value_name = "JSON")]
        params: String,
        /// Target host/id (repeatable); passed to the module as `params.targets`.
        #[arg(long = "target", value_name = "ID")]
        targets: Vec<String>,
    },
    /// Install a module (and its dependencies) from GitHub or a local path.
    Add {
        /// e.g. `owner/repo`, `owner/repo@1.2.0`, `./path/to/module`, `file:/abs`.
        reference: String,
    },
    /// List installed modules from the lockfile.
    List,
    /// Re-fetch and reinstall installed modules (all, or one by name).
    Update { name: Option<String> },
    /// Uninstall a module by name.
    Remove { name: String },
    /// Show each module's declared permissions and trust status.
    Permissions,
    /// Approve a module to run (pins to its current content digest).
    Trust { name: String },
    /// Revoke a module's approval.
    Untrust { name: String },
    /// Verify installed modules against the lockfile digests (tamper check).
    Verify,
    /// Run the built-in cross-language, cross-transport demo.
    Demo,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let dirs = resolve_search_dirs(&cli)?;

    match &cli.command {
        Command::Modules => {
            let engine = Engine::load(&dirs)?;
            print_modules(&engine);
        }
        Command::Describe { capability } => {
            let mut engine = Engine::load(&dirs)?;
            engine.start()?;
            let out = engine.describe(capability)?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            engine.shutdown();
        }
        Command::Run {
            capability,
            method,
            params,
            targets,
        } => {
            let mut params: Value =
                serde_json::from_str(params).context("--params must be a valid JSON object")?;
            if !targets.is_empty()
                && let Value::Object(map) = &mut params {
                    map.insert("targets".into(), json!(targets));
                }
            let mut engine = Engine::load(&dirs)?;
            consent_gate(&engine, cli.allow_untrusted)?;
            engine.start()?;
            let out = engine.run(capability, method, params)?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            engine.shutdown();
        }
        Command::Add { reference } => {
            let reg = Registry::new(paths::home());
            let report = reg.add(reference).context("installing module")?;
            println!(
                "installed {} module(s) into {}",
                report.installed.len(),
                reg.modules_dir().display()
            );
            for e in &report.installed {
                println!(
                    "  {} v{} [{}] {}",
                    e.name,
                    e.version,
                    e.source,
                    short_digest(&e.digest)
                );
            }
        }
        Command::List => {
            let reg = Registry::new(paths::home());
            let locked = reg.list()?;
            if locked.is_empty() {
                println!("no modules installed");
            } else {
                println!("{:<14} {:<9} {:<6} REFERENCE", "MODULE", "VERSION", "SOURCE");
                for e in &locked {
                    println!("{:<14} {:<9} {:<6} {}", e.name, e.version, e.source, e.reference);
                }
            }
        }
        Command::Update { name } => {
            let reg = Registry::new(paths::home());
            let report = reg.update(name.as_deref())?;
            println!("updated {} module(s)", report.installed.len());
            for e in &report.installed {
                println!("  {} v{} {}", e.name, e.version, short_digest(&e.digest));
            }
        }
        Command::Remove { name } => {
            let reg = Registry::new(paths::home());
            if reg.remove(name)? {
                println!("removed {name}");
            } else {
                println!("{name} was not installed");
            }
        }
        Command::Permissions => {
            let engine = Engine::load(&dirs)?;
            let trust = TrustStore::load(&paths::home())?;
            println!("{:<14} {:<10} PERMISSIONS", "MODULE", "STATUS");
            for spec in engine.modules() {
                let status = if !spec.permissions.sensitive() {
                    "safe"
                } else {
                    let digest = digest_dir(&spec.cwd).unwrap_or_default();
                    if trust.is_trusted(&spec.name, &digest) {
                        "trusted"
                    } else {
                        "UNTRUSTED"
                    }
                };
                let perms = spec.permissions.summary();
                let perms = if perms.is_empty() {
                    "—".to_string()
                } else {
                    perms.join("; ")
                };
                println!("{:<14} {:<10} {perms}", spec.name, status);
            }
        }
        Command::Trust { name } => {
            let engine = Engine::load(&dirs)?;
            let spec = engine
                .modules()
                .iter()
                .find(|m| &m.name == name)
                .ok_or_else(|| anyhow!("module {name} not found in the search paths"))?;
            let digest = digest_dir(&spec.cwd)?;
            let mut trust = TrustStore::load(&paths::home())?;
            trust.approve(name, &digest);
            trust.save(&paths::home())?;
            println!("trusted {name} [{}]", short_digest(&digest));
        }
        Command::Untrust { name } => {
            let mut trust = TrustStore::load(&paths::home())?;
            if trust.revoke(name) {
                trust.save(&paths::home())?;
                println!("untrusted {name}");
            } else {
                println!("{name} was not trusted");
            }
        }
        Command::Verify => {
            let reg = Registry::new(paths::home());
            let items = reg.verify()?;
            if items.is_empty() {
                println!("no installed modules to verify");
            }
            for item in &items {
                match &item.status {
                    VerifyStatus::Ok => println!("ok       {}", item.name),
                    VerifyStatus::Missing => println!("MISSING  {}", item.name),
                    VerifyStatus::Modified { .. } => println!("MODIFIED {}", item.name),
                }
            }
        }
        Command::Demo => {
            let engine = Engine::load(&dirs)?;
            consent_gate(&engine, cli.allow_untrusted)?;
            drop(engine);
            run_demo(&dirs)?;
        }
    }
    Ok(())
}

/// Refuse to run when an untrusted module requests sensitive permissions, unless
/// `--allow-untrusted` was passed. Trust is pinned to the module's content
/// digest via [`TrustStore`].
fn consent_gate(engine: &Engine, allow_untrusted: bool) -> Result<()> {
    let trust = TrustStore::load(&paths::home())?;
    let mut blocked: Vec<(String, Vec<String>)> = Vec::new();
    for spec in engine.modules() {
        if !spec.permissions.sensitive() {
            continue;
        }
        let digest = digest_dir(&spec.cwd).unwrap_or_default();
        if !trust.is_trusted(&spec.name, &digest) {
            blocked.push((spec.name.clone(), spec.permissions.summary()));
        }
    }

    if blocked.is_empty() {
        return Ok(());
    }
    if allow_untrusted {
        eprintln!(
            "[warning] running {} untrusted module(s) with sensitive permissions (--allow-untrusted)",
            blocked.len()
        );
        return Ok(());
    }

    eprintln!("Refusing to run — untrusted module(s) request sensitive permissions:");
    for (name, perms) in &blocked {
        eprintln!("  {name}: {}", perms.join("; "));
    }
    eprintln!("\nApprove with `limen trust <name>`, or re-run with --allow-untrusted.");
    bail!("blocked {} untrusted module(s)", blocked.len());
}

/// Shorten a `sha256:<hex>` digest for display.
fn short_digest(digest: &str) -> String {
    match digest.split_once(':') {
        Some((algo, hex)) => format!("{algo}:{}", &hex[..hex.len().min(12)]),
        None => digest.to_string(),
    }
}

/// Search paths: `--modules-dir` if given, else the configured paths; plus a
/// local `./modules` for in-repo development.
fn resolve_search_dirs(cli: &Cli) -> Result<Vec<PathBuf>> {
    paths::ensure_dirs();
    let mut dirs = if cli.modules_dir.is_empty() {
        Config::load()?.search_dirs()
    } else {
        cli.modules_dir.clone()
    };
    let local = PathBuf::from("modules");
    if local.is_dir() && !dirs.contains(&local) {
        dirs.push(local);
    }
    Ok(dirs)
}

fn print_modules(engine: &Engine) {
    println!("{:<14} {:<9} PROVIDES", "MODULE", "VERSION");
    for spec in engine.modules() {
        let caps = if spec.capabilities.is_empty() {
            "-".to_string()
        } else {
            spec.capabilities.join(", ")
        };
        println!("{:<14} {:<9} {caps}", spec.name, spec.version);
        for (cap, req) in &spec.requires {
            println!("{:<14} {:<9}   requires {cap} {req}", "", "");
        }
    }
}

fn run_demo(dirs: &[PathBuf]) -> Result<()> {
    let mut engine = Engine::load(dirs)?;
    engine.start()?;

    let native = engine
        .run("demo.native", "shout", json!({ "name": "Fleet" }))
        .context("invoking demo.native.shout")?;
    println!("\n=== demo.native.shout (in-process native module) ===");
    println!("{}", serde_json::to_string_pretty(&native)?);

    let out = engine
        .run("demo.consumer", "run", json!({ "name": "Limen" }))
        .context("invoking demo.consumer.run")?;
    println!("\n=== demo.consumer.run (python -> broker -> subprocess + in-process) ===");
    println!("{}", serde_json::to_string_pretty(&out)?);

    engine.shutdown();
    Ok(())
}
