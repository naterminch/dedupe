#!/usr/bin/env pwsh
# Builds dedupe and copies the resulting binary to the project root
# (the folder that contains this script), next to Cargo.toml:
#
#   .\build.ps1           release build -> .\dedupe.exe
#   .\build.ps1 -Debug    debug build   -> same, from the debug profile
#
# dedupe.exe = one binary for everything: running it with no argument
# opens the desktop app; any argument or flag runs a CLI scan.
param(
    [switch]$Debug
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

Push-Location $root
try {
    # The build writes straight to this console, with nothing redirected.
    # That is what keeps cargo's live progress: given a pipe or a file it
    # falls back to one plain line per crate. Its bars and "Compiling …"
    # lines are stderr, so Windows PowerShell 5.1 paints them red; the text
    # is intact and PowerShell 7 shows them plainly. Only the exit code
    # decides success, and cargo has already printed why if it failed.
    $buildArgs = @("build")
    if (-not $Debug) {
        $buildArgs += "--release"
    }
    & $cargoPath @buildArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }

    # Copy the built binary to the project root (never the user profile root).
    $rootFull = [System.IO.Path]::GetFullPath($root)
    foreach ($name in @("dedupe")) {
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
