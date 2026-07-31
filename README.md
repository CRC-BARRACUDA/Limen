# Limen

*English · [Українська](README.ua.md)*

**A modular tool for security, analysis and audit.**

Limen is a modular security, analysis &amp; audit toolkit — a portable core that
turns small, single-purpose modules into a unified console. Instead of one
monolithic tool, you compose *capabilities* (endpoint queries, system
operations, …), each shipped as an independent module in whatever language fits.
Limen discovers them, brokers calls between them over one contract, manages their
install and trust from GitHub, and renders the UI each module draws for itself.

Put very simply: Limen is an engine that brings modules together in one place and
lets them interact with one another.

---

## What it is

- **Portable / USB-first.** The app's base directory is the executable's own
  folder (override with `$LIMEN_HOME`). Modules, SDKs, settings, trust store, and
  bundled interpreters all live next to the binary — nothing touches `~`.
- **Module system, two transports, one contract.** Every module speaks the same
  JSON-RPC contract over one of:
  1. **stdio subprocess** (default) — any language (Python / JS / Lua / Go …)
     "just runs";
  2. **native C-ABI** (`libloading`, opt-in, trusted) — compiled `.so` / `.dll`
     / `.dylib` for Rust / C / Go.
- **Capability broker.** Modules declare the capabilities they *provide* and
  *require* in a `limen.toml` manifest. A module calls
  `host.call(capability, method, params)` and the core routes it to whichever
  module provides that capability. The core builds a semver dependency DAG,
  topo-sorts it, and rejects cycles.
- **GitHub package manager.** `limen-cli add owner/repo@version` reads the manifest
  from a tagged release, resolves the cross-repo dependency graph, downloads,
  verifies a sha256 digest against a lockfile, and installs. Native modules pull
  a prebuilt platform binary from the release assets.
- **Permissions & consent.** Manifests declare sensitive permissions (admin,
  network, subprocess, filesystem, elevated methods). Elevated methods prompt
  for consent at invocation time; granted trust is digest-pinned in `trust.json`.
- **Self-update.** On launch the app checks GitHub releases in the background and
  offers an in-app update if a newer version is published.
- **Multi-language UI.** The app ships in English and Ukrainian — taken from the
  OS locale on first run, switchable in Settings. Modules translate their own
  views through the SDK, so their text follows the same setting.

## Layout

```
src/
  limen-proto      the JSON-RPC contract + manifest types
  limen-host       runtime, module loader, capability broker, services for modules
  limen-core       the engine + on-disk conventions (paths, config, setup, update)
  limen-registry   GitHub package manager (install / lock / trust / verify)
  limen-cli        command-line frontend   (binary: limen-cli)
  limen-gui        egui/eframe desktop app  (binary: Limen)
  limen-sdk-rust   Rust module SDK (incl. a UI builder)
sdk/               injected module SDKs for Python / JS / Lua
modules/           first-party / demo modules (each is its own git repo)
scripts/           packaging
```

Core crates keep their `.rs` files directly under `src/<crate>/` (each `Cargo.toml`
sets explicit `[lib]`/`[[bin]]` paths).

## Building & running

Rust stable toolchain required. Development happens on **debug** builds:

```bash
# GUI
cargo run -p limen-gui

# CLI
cargo run -p limen-cli -- <command>
```

### Running without a 3D accelerator

The GUI draws its window with OpenGL and needs **version 2.0 or newer**. On a virtual machine,
a remote session, or a PC with no 3D driver, Windows only offers a software OpenGL 1.1 — so
`Limen` reports that it cannot start and exits.

To render in software instead, download `mesa3d-<version>-release-msvc.7z` from
[pal1000/mesa-dist-win](https://github.com/pal1000/mesa-dist-win/releases) and copy these three
files out of its `x64/` folder into the same folder as `Limen.exe`:

| File | Why |
|---|---|
| `opengl32.dll` | the driver entry point Windows looks up |
| `libgallium_wgl.dll` | the renderer itself (this one is ~59 MB) |
| `dxil.dll` | shader compilation for Mesa's default Windows driver |

Limen then starts normally and everything works — rendering just runs on the CPU, so it is
slower than a real GPU. All three files are needed; with `dxil.dll` missing the app fails
silently.

> ⚠️ **Only do this on a machine that has no 3D acceleration.** These files *replace* the system
> OpenGL driver for whatever sits next to them, so putting them beside `Limen.exe` on a PC with a
> working GPU will **disable** hardware acceleration and make it slower.

`limen-cli` does all the same work with no GPU at all, and is unaffected.

### Release + packaging

Both scripts read the version from `[workspace.package]` in `Cargo.toml`, build
`Limen` + `limen-cli` in release, and write to `dist/` for the build machine's
architecture.

**Linux**

```bash
./scripts/package-linux.sh
```

| Artifact | What |
|---|---|
| `Limen-<ver>-linux-<arch>.tar.gz` | Distribution tarball — `Limen`, `limen-cli`, `LICENSE`, `JetBrainsMono-OFL.txt` (stripped) |
| `Limen-<ver>-linux-<arch>.tar.gz.sha256` | Checksum |

**Windows**

```powershell
.\scripts\package-windows.ps1
```

Needs 7-Zip (`winget install 7zip.7zip`) — on `PATH`, in a usual install
location, or registered in `HKLM\SOFTWARE\7-Zip`. There is no strip step:
Windows keeps debug info in a separate `.pdb`, so the release `.exe` is already
lean.

| Artifact | What |
|---|---|
| `Limen-<ver>-windows-<arch>.7z` | Distribution archive — `Limen.exe`, `limen-cli.exe`, `LICENSE`, `JetBrainsMono-OFL.txt` |
| `Limen-<ver>-windows-<arch>.7z.sha256` | Checksum |
| `Limen-<ver>-windows-<arch>.exe` | Raw GUI binary, for the in-app self-update |

> **Naming release assets matters.** The self-updater picks the first asset whose
> filename contains **both** the OS and arch tokens (`linux`/`windows`,
> `x86_64`/`aarch64`). It prefers a raw binary — renamed straight over the
> running executable — and otherwise falls back to an archive it extracts the
> executable from (`.tar.gz` / `.tgz` / `.tar` / `.zip` / `.7z`). Keep the
> `<ver>-<os>-<arch>` stem: a bare `Limen.exe` carries no tokens and is
> invisible to the updater.

## CLI

```
limen-cli modules              list installed modules and their capabilities
limen-cli describe <cap>       show a capability provider's self-description
limen-cli run <cap> <method>   invoke a capability method (--params JSON, --target ID)
limen-cli add <ref>            install a module + deps (owner/repo[@ver], or a local path)
limen-cli list                 list installed modules from the lockfile
limen-cli update [name]        re-fetch and reinstall modules (all, or one by name)
limen-cli remove <name>        uninstall a module
limen-cli permissions          show every module's declared permissions and trust status
limen-cli trust <name>         approve a module (pins its current content digest)
limen-cli untrust <name>       revoke approval
limen-cli verify               sha256 tamper-check installed modules vs the lockfile
limen-cli demo                 run the built-in cross-language, cross-transport demo
```

## Writing a module

A module is a directory with a `limen.toml` manifest plus an entry point. Minimal
manifest:

```toml
[module]
name = "local-devices"                # the identifier other modules resolve
display_name = "Local Devices"        # pretty name on the card (falls back to name)
description = "Enumerates locally attached devices."
version = "0.1.0"
language = "native"          # or "python" | "js" | "lua"
entry = "local_devices"      # binary/script/lib entry
abi = "rpc"                  # rpc (default, over stdio) | native (in-process C-ABI)
repo = "CRC-BARRACUDA/limen-local-devices"

[provides]
capabilities = ["devices.local"]

[permissions]
subprocess = true
```

`display_name` and `description` are what the module card shows, in the installed
list and in the available-modules list alike. Both are the **English defaults** —
add a `locales/<lang>.toml` next to the manifest with `[module] title` /
`description` to override them per language:

```toml
# locales/uk.toml
[module]
title = "Локальні пристрої"
description = "Перелічує локально під'єднані пристрої."
```

Modules get an SDK injected at spawn time (Python / JS / Lua) or link the Rust
SDK. All expose the same surface — `host.call` / `emit` / `subscribe` / `about`
/ `log` — plus a UI builder (`Window` / `Label` / `Text` / `Select` / `Button`
/ `Row` / `Separator` / `Table`) whose returned view spec the GUI renders. A
module drawing its own UI returns that spec from a `ui` method; actions that
return a new view spec re-render in place.

Modules are distributed as their own GitHub repos (`CRC-BARRACUDA/limen-<name>`)
and are **not** part of this workspace.

## License

Limen is free software: you may redistribute it and/or modify it under the terms
of the **GNU General Public License, version 3 or later**. See [LICENSE](LICENSE).

---

<p align="center"><sub>Powered by <b>CRC BARRACUDA</b></sub></p>
