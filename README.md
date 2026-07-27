# Limen

**A modular tool for security and analysis.**

Limen is a modular security &amp; analysis toolkit — a portable host that turns
small, single-purpose modules into a unified console. Instead of one monolithic
tool, you compose *capabilities* (endpoint queries, host operations, …), each
shipped as an independent module in whatever language fits.
Limen discovers them, brokers calls between them over one contract, manages their
install and trust from GitHub, and renders the UI each module draws for itself.
It's a Rust rewrite of the original Python Limen — a native `egui` app with a
proper package manager underneath.

> ⚠️ **Development-branch software** — APIs and on-disk formats may change. Free
> software under the **GNU GPL v3 or later** (which carries its own no-warranty
> terms).

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
  `host.call(capability, method, params)` and the host routes it to whichever
  module provides that capability. The host builds a semver dependency DAG,
  topo-sorts it, and rejects cycles.
- **GitHub package manager.** `limen add owner/repo@version` reads the manifest
  from a tagged release, resolves the cross-repo dependency graph, downloads,
  verifies a sha256 digest against a lockfile, and installs. Native modules pull
  a prebuilt platform binary from the release assets.
- **Permissions & consent.** Manifests declare sensitive permissions (admin,
  network, subprocess, filesystem, elevated methods). Elevated methods prompt
  for consent at invocation time; granted trust is digest-pinned in `trust.json`.
- **Self-update.** On launch the app checks GitHub releases in the background and
  offers an in-app update if a newer version is published.

## Layout

```
src/
  limen-proto      the JSON-RPC contract + manifest types
  limen-host       runtime, module loader, capability broker, host services
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

### Release + packaging (Linux)

```bash
./scripts/package-linux.sh
```

Produces, in `dist/`, for the host architecture:

| Artifact | What |
|---|---|
| `Limen-<ver>-linux-<arch>.tar.gz` | Distribution tarball — `Limen`, `limen-cli`, `LICENSE` (stripped) |
| `Limen-<ver>-linux-<arch>.tar.gz.sha256` | Checksum |

> The in-app self-update needs a **raw binary** release asset (it renames the
> download directly over the running executable) — a `.tar.gz` is for manual
> download only.

## CLI

```
limen-cli modules              list installed modules
limen-cli describe <cap>       describe a capability / its methods
limen-cli run <cap> <method>   invoke a capability method
limen-cli add <owner/repo>     install a module (and its dependencies)
limen-cli list                 list modules available in the configured org
limen-cli update <name>        update an installed module
limen-cli remove <name>        uninstall a module
limen-cli permissions <name>   show a module's declared permissions
limen-cli trust <name>         approve a module (digest-pinned)
limen-cli untrust <name>       revoke approval
limen-cli verify               sha256 tamper-check installed modules vs the lockfile
```

## Writing a module

A module is a directory with a `limen.toml` manifest plus an entry point. Minimal
manifest:

```toml
[module]
name = "local-devices"
version = "0.1.0"
language = "native"          # or "python" | "js" | "lua"
entry = "local_devices"      # binary/script/lib entry

[provides]
capabilities = ["devices.local"]

[permissions]
subprocess = true

repo = "CRC-BARRACUDA/limen-local-devices"
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
