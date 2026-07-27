#!/usr/bin/env pwsh
# Build a release and package Windows distribution artifacts into .\dist.
#
# Produces, for the host architecture:
#   dist\Limen-<ver>-windows-<arch>.7z           distribution archive (GUI + CLI + LICENSE)
#   dist\Limen-<ver>-windows-<arch>.7z.sha256    its checksum (sha256sum format)
#   dist\Limen.exe                                raw GUI binary (for the in-app self-update)
#
# The in-app self-update renames a raw .exe over the running executable, so it
# needs Limen.exe published as its own release asset - the .7z is for manual
# download only (mirrors the note in package-linux.sh / README).
#
# Requires 7-Zip (7z.exe) on PATH or in a common install location.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Repo root = parent of this script's dir.
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Host architecture, normalized to Rust's naming (x86_64 / aarch64).
$arch = switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    'x86'   { 'i686' }
    default { $env:PROCESSOR_ARCHITECTURE.ToLower() }
}

# Version = first `version = "..."` line in Cargo.toml (the [workspace.package] one).
$verLine = (Select-String -Path 'Cargo.toml' -Pattern '^version' | Select-Object -First 1).Line
$ver = [regex]::Match($verLine, '"([^"]+)"').Groups[1].Value
if (-not $ver) { throw "could not read version from Cargo.toml" }
$stem = "Limen-$ver-windows-$arch"

# Resolve 7-Zip: PATH first, then common install locations, then the registry.
$sevenZipCmd = Get-Command 7z -ErrorAction SilentlyContinue
$sevenZip = if ($sevenZipCmd) { $sevenZipCmd.Source } else { $null }
if (-not $sevenZip) {
    $cands = @(
        "$env:ProgramFiles\7-Zip\7z.exe",
        "${env:ProgramFiles(x86)}\7-Zip\7z.exe",
        'C:\Programs\7-Zip\7z.exe',
        "$env:LOCALAPPDATA\Programs\7-Zip\7z.exe"
    )
    try { $rp = (Get-ItemProperty 'HKLM:\SOFTWARE\7-Zip' -ErrorAction Stop).Path; if ($rp) { $cands += (Join-Path $rp '7z.exe') } } catch {}
    $sevenZip = $cands | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $sevenZip) { throw "7-Zip (7z.exe) not found - install it (winget install 7zip.7zip) or add it to PATH" }

# Resolve cargo: prefer PATH, fall back to the default rustup install location so
# the script runs even in a shell that hasn't picked up cargo's PATH entry yet.
$cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
$cargo = if ($cargoCmd) { $cargoCmd.Source } else { $null }
if (-not $cargo) {
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $fallback) { $cargo = $fallback } else { throw "cargo not found on PATH or at $fallback" }
}

Write-Host ">> building release (Limen + limen-cli)"
& $cargo build --release -p limen-gui -p limen-cli
if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }

$dist  = Join-Path $root 'dist'
$stage = Join-Path $dist ".stage\$stem"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Write-Host ">> staging artifacts"
# Windows keeps debug info in a separate .pdb, so the release .exe is already
# lean - no strip step needed (unlike Linux).
Copy-Item 'target\release\Limen.exe'     (Join-Path $stage 'Limen.exe')
Copy-Item 'target\release\limen-cli.exe' (Join-Path $stage 'limen-cli.exe')
Copy-Item 'LICENSE'                      (Join-Path $stage 'LICENSE')

Write-Host ">> writing $stem.7z"
$archive = Join-Path $dist "$stem.7z"
# Archive the stem dir itself so the .7z contains a top-level <stem>\ folder,
# matching the Linux tarball layout. -mx=9 = max compression.
& $sevenZip a -t7z -mx=9 -bso0 -bsp0 $archive $stage | Out-Null
if ($LASTEXITCODE -ne 0) { throw "7z failed (exit $LASTEXITCODE)" }

# Also drop the raw GUI binary next to the archive for the self-updater's release asset.
Copy-Item 'target\release\Limen.exe' (Join-Path $dist 'Limen.exe')

Remove-Item -Recurse -Force (Join-Path $dist '.stage')

Write-Host ">> checksum"
$hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower()
# sha256sum format: "<hash>  <filename>" (two spaces), so `sha256sum -c` can verify it.
"$hash  $stem.7z" | Out-File -FilePath "$archive.sha256" -Encoding ascii -NoNewline

Write-Host ""
Write-Host "artifacts in dist\:"
Get-ChildItem $dist | Select-Object Name, @{n='Size';e={ '{0,8:N0}' -f $_.Length }}, LastWriteTime
