# Sign a release of a module with the team's key, for Limen to check on install.
# scripts/sign-module.sh, for Windows: the same statement, the same signatures.
#
#   scripts\sign-module.ps1 <module-repo> [-Ref REF] [-Out DIR] [FILE...]
#
#   <module-repo>  the module's git repository (a working copy of it)
#   -Ref REF       the commit that is released (default: HEAD) — the one the
#                  release's tag points at, and the tip of the default branch
#   -Out DIR       where the signatures go (default: <module-repo>\dist)
#   FILE...        the release's other files to sign as they are: a native
#                  module's library or archive for each platform
#
# Writes:
#   limen-module.asc   the signature of the module's statement — one line:
#                      its name, version and the digest of its files
#   <FILE>.sig         one per FILE
#
# Upload them to the module's GitHub release for that version, beside its other
# files:   gh release upload v<version> <out>\limen-module.asc <out>\*.sig
#
# The statement is worked out from an export of REF — the files exactly as the
# repository has them, which is what Limen downloads — never from the working
# copy, which has files of its own (a config.json, build output) and, on
# Windows, line endings of its own.
#
# Signing again later — with a new key, say — is running this again and
# uploading over the old signatures: the module itself is not touched.
#
# The key has to be in this machine's keyring (Gpg4win's: the one Kleopatra
# shows, not the one Git Bash's own gpg keeps); gpg asks for its passphrase.
# $env:LIMEN_SIGN_KEY names another key than the team's.
#
# Needs git, gpg (Gpg4win) and tar (Windows' own) on PATH, and a built limen-cli.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)][string]$Repo,
    [string]$Ref = 'HEAD',
    [string]$Out = '',
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Files = @()
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# A native program, with what it writes to stderr thrown away, and its exit
# code. Through cmd: Windows PowerShell 5.1 turns a native program's redirected
# stderr into an error, which under 'Stop' ends the script. $Stdout, when
# given, is the file its output goes to — byte for byte, which PowerShell's own
# `>` does not do (it writes UTF-16).
function Invoke-Quiet {
    param([string]$Program, [string[]]$Arguments, [string]$Stdout = 'nul')
    $line = '"' + $Program + '"'
    foreach ($a in $Arguments) { $line += ' "' + $a + '"' }
    # The outer pair of quotes is cmd's own: it takes one off each end.
    & cmd /d /c "`"$line > `"$Stdout`" 2>nul`""
    return $LASTEXITCODE
}

function Stop-With([string]$Message, [int]$Code = 2) {
    [Console]::Error.WriteLine($Message)
    exit $Code
}

$limenRoot = Split-Path -Parent $PSScriptRoot
$signKey = if ($env:LIMEN_SIGN_KEY) { $env:LIMEN_SIGN_KEY } else { '8D151FD231907502BBFA528E140ABA21244518B8' }

foreach ($tool in 'git', 'gpg', 'tar') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Stop-With "$tool not found on PATH" }
}

if (-not (Test-Path -LiteralPath $Repo -PathType Container)) { Stop-With "no such directory: $Repo" }
$Repo = (Resolve-Path -LiteralPath $Repo).Path
if ((Invoke-Quiet 'git' @('-C', $Repo, 'rev-parse', '--git-dir')) -ne 0) { Stop-With "$Repo is not a git repository" }
$commit = (& git -C $Repo rev-parse --verify "$Ref^{commit}")
if ($LASTEXITCODE -ne 0 -or -not $commit) { Stop-With "no such commit: $Ref" }
$commit = "$commit".Trim()
$head = "$(& git -C $Repo rev-parse HEAD)".Trim()

if (-not $Out) { $Out = Join-Path $Repo 'dist' }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path -LiteralPath $Out).Path

foreach ($f in $Files) {
    if (-not (Test-Path -LiteralPath $f -PathType Leaf)) { Stop-With "no such file: $f" }
}
$Files = @($Files | ForEach-Object { (Resolve-Path -LiteralPath $_).Path })

# limen-cli works the statement out, exactly as Limen will on install.
# The newer of the release and debug builds: an old one may not know how.
$cli = Join-Path $limenRoot 'target\release\limen-cli.exe'
$debug = Join-Path $limenRoot 'target\debug\limen-cli.exe'
if (-not (Test-Path -LiteralPath $cli) -or
    ((Test-Path -LiteralPath $debug) -and (Get-Item -LiteralPath $debug).LastWriteTime -gt (Get-Item -LiteralPath $cli).LastWriteTime)) {
    $cli = $debug
}
if (-not (Test-Path -LiteralPath $cli) -or (Invoke-Quiet $cli @('module-statement', '--help')) -ne 0) {
    Stop-With 'build limen-cli first: cargo build -p limen-cli'
}

if ((Invoke-Quiet 'gpg' @('--list-secret-keys', $signKey)) -ne 0) {
    Stop-With "the signing key $signKey is not in this machine's keyring"
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("limen-sign-" + [Guid]::NewGuid().ToString('N'))
$exportDir = Join-Path $work 'export'
$tarball = Join-Path $work 'export.tar'
$statement = Join-Path $work 'statement'
New-Item -ItemType Directory -Force -Path $exportDir | Out-Null
try {
    # As GitHub's own archive of the commit: no line endings rewritten on the
    # way. To a file and not down a pipe — PowerShell's pipe is text, and would
    # rewrite the archive itself.
    & git -C $Repo -c core.autocrlf=false archive --format=tar -o $tarball $commit
    if ($LASTEXITCODE -ne 0) { Stop-With "git could not export $commit" 1 }
    & tar -xf $tarball -C $exportDir
    if ($LASTEXITCODE -ne 0) { Stop-With "tar could not unpack the export of $commit" 1 }

    if ((Invoke-Quiet $cli @('module-statement', $exportDir) -Stdout $statement) -ne 0) {
        # Again, aloud: what went wrong is on stderr.
        & $cli module-statement $exportDir | Out-Null
        Stop-With 'limen-cli could not work the statement out' 1
    }
    $said = [IO.File]::ReadAllText($statement).Trim()
    Write-Host ">> $said"
    Write-Host "   of $($commit.Substring(0, 12)) ($Ref)"
    $dirty = (& git -C $Repo status --porcelain --untracked-files=no)
    if ($commit -ne $head -or $dirty) {
        Write-Host "   note: this is the commit's content, not the working copy's"
    }

    Write-Host ">> signing with $signKey"
    $moduleSig = Join-Path $Out 'limen-module.asc'
    & gpg --batch --yes --local-user $signKey --armor --detach-sign -o $moduleSig $statement
    if ($LASTEXITCODE -ne 0) { Stop-With 'gpg could not sign the statement' 1 }
    $written = @($moduleSig)
    $pairs = @(, @($statement, $moduleSig))
    foreach ($f in $Files) {
        $sig = Join-Path $Out ((Split-Path -Leaf $f) + '.sig')
        & gpg --batch --yes --local-user $signKey --detach-sign -o $sig $f
        if ($LASTEXITCODE -ne 0) { Stop-With "gpg could not sign $f" 1 }
        $written += $sig
        $pairs += , @($f, $sig)
    }

    # Checked the way Limen will check them: by the keys built into this build.
    # (A key of one's own is not among them: that it is not is said once, below.)
    $ok = $true
    foreach ($pair in $pairs) {
        if ($env:LIMEN_SIGN_KEY) {
            $code = Invoke-Quiet $cli @('verify-signature', $pair[0], $pair[1])
        } else {
            & $cli verify-signature $pair[0] $pair[1]
            $code = $LASTEXITCODE
        }
        if ($code -ne 0) { $ok = $false }
    }
    if (-not $ok) {
        if ($env:LIMEN_SIGN_KEY) {
            Write-Host '   (signed with LIMEN_SIGN_KEY, which is not one of the keys built into Limen)'
        } else {
            Stop-With '>> a signature does not verify against the keys built into Limen' 1
        }
    }

    $version = ($said -split '\s+')[2]
    Write-Host ''
    Write-Host 'written:'
    foreach ($w in $written) { Write-Host "  $w" }
    Write-Host 'upload them to the release of this version, for example:'
    Write-Host "  gh release upload v$version $(($written | ForEach-Object { '"' + $_ + '"' }) -join ' ')"
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

exit 0
