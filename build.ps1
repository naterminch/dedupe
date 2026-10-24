#!/usr/bin/env pwsh
# Builds dedupe and copies the resulting binaries to the project root
# (the folder that contains this script), next to Cargo.toml:
#
#   .\build.ps1           release build -> .\dedupe.exe + .\dedupe-gui.exe
#   .\build.ps1 -Debug    debug build   -> same, from the debug profile
#   .\build.ps1 -NoCache  ignore the kache build cache for this build
#
# dedupe.exe     = console binary (CLI; opens the GUI when run without a path)
# dedupe-gui.exe = GUI-only binary (no console window ever)
param(
    [switch]$Debug,
    [switch]$NoCache
)

# Project root = the directory containing this script. Everything below is
# anchored to this path; we never write to $HOME or the user profile.
$root = $PSScriptRoot
if (-not $root) {
    throw "could not determine the script directory (project root)"
}
$profile = if ($Debug) { "debug" } else { "release" }

# Resolve cargo: trust PATH first, fall back to the standard rustup location
# (covers shells started before rustup was installed / added to PATH).
$cargoPath = $null
$cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCmd) {
    $cargoPath = $cargoCmd.Source
} else {
    $rustupCargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path -LiteralPath $rustupCargo) {
        $cargoPath = $rustupCargo
    }
}
if (-not $cargoPath) {
    throw "cargo not found on PATH (install via https://rustup.rs)"
}

# kache is configured as Cargo's rustc-wrapper, and Cargo resolves the wrapper
# by name. A shell started before kache was installed has a stale PATH, which
# would fail the build with "could not execute wrapper". Add the usual install
# locations for this process only; -NoCache opts the build out entirely.
$kacheMissing = $false
if ($NoCache) {
    $env:KACHE_DISABLED = "1"
} elseif (-not (Get-Command kache -ErrorAction SilentlyContinue)) {
    $kacheDir = $null
    foreach ($candidate in @(
            (Join-Path $env:USERPROFILE ".local\bin"),
            (Join-Path $env:USERPROFILE ".cargo\bin")
        )) {
        if (Test-Path -LiteralPath (Join-Path $candidate "kache.exe")) {
            $kacheDir = $candidate
            break
        }
    }
    if ($kacheDir) {
        $env:PATH = "$kacheDir;$env:PATH"
    } else {
        $kacheMissing = $true
    }
}

Push-Location $root
try {
    # Cargo prints progress on stderr, which PS 5.1 surfaces as noisy error
    # records, so stderr goes to a log inside the project and is printed only
    # when the build fails. Real failures are caught via $LASTEXITCODE.
    # Create target/ first: PowerShell cannot redirect into a missing
    # directory, and a fresh clone has no target/ yet.
    $targetDir = Join-Path $root "target"
    if (-not (Test-Path -LiteralPath $targetDir)) {
        New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
    }
    $errLog = Join-Path $targetDir "build-stderr.log"
    if ($Debug) {
        & $cargoPath build 2> $errLog
    } else {
        & $cargoPath build --release 2> $errLog
    }
    if ($LASTEXITCODE -ne 0) {
        $log = if (Test-Path -LiteralPath $errLog) { Get-Content $errLog -Raw } else { "" }
        if ($log) { Write-Host $log }
        if ($kacheMissing -and $log -match 'rustc.wrapper|kache') {
            Write-Host "hint: Cargo is configured to use the kache wrapper, but kache is not on PATH."
            Write-Host "      Add it to PATH, or rerun with .\build.ps1 -NoCache to bypass the cache."
        }
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    Remove-Item -LiteralPath $errLog -ErrorAction SilentlyContinue

    # Copy the built binaries to the project root (never the user profile root).
    $rootFull = [System.IO.Path]::GetFullPath($root)
    foreach ($name in @("dedupe", "dedupe-gui")) {
        $source = Join-Path $root "target/$profile/$name.exe"
        $dest = Join-Path $root "$name.exe"
        Copy-Item -LiteralPath $source -Destination $dest -Force
        Write-Host "copied  $source"
        Write-Host "    to  $dest  (project root: $rootFull)"
    }
}
finally {
    Pop-Location
}
