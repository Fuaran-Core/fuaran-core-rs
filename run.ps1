#Requires -Version 7.0
# fuaran-core-rs: the one-command entry point.
# cargo fmt --check -> cargo clippy -> cargo build -> cargo test -> a no_std build for
# thumbv7em-none-eabihf (Cortex-M4F).
#
# `cargo test` includes the conformance legs (tests/conformance.rs), which read Core's vectors from a
# Core checkout: FUARAN_CORE_DIR, or `fuaran-core/` beside this repository, at the commit
# core-pin.json names. They FAIL when no checkout is found rather than skip, because a skipped leg
# certifies nothing.
#
# The no_std build is part of the contract, so a missing target FAILS here, naming the remedy;
# -SkipEmbedded is the deliberate opt-out.
# Switches: -SkipFormat / -SkipBuild / -SkipTests / -SkipEmbedded for fast iteration.
[CmdletBinding()]
param(
    [switch]$SkipFormat,
    [switch]$SkipBuild,
    [switch]$SkipTests,
    [switch]$SkipEmbedded
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Resolve-Tool {
    param([string]$Name)
    $cmd = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $cmd) {
        $home_ = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME ".cargo" }
        $candidate = Join-Path (Join-Path $home_ "bin") ($Name + $(if ($IsWindows) { ".exe" } else { "" }))
        if (Test-Path $candidate) { return $candidate }
        throw "The Rust toolchain is required but '$Name' was not found on PATH or in the cargo home."
    }
    return $cmd.Source
}

function Invoke-Stage {
    param([string]$Name, [scriptblock]$Body)
    Write-Host "==> $Name"
    $global:LASTEXITCODE = 0
    & $Body
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAILED: $Name (exit $LASTEXITCODE)"
        exit $LASTEXITCODE
    }
}

$cargo = Resolve-Tool "cargo"

if (-not $SkipFormat) { Invoke-Stage "cargo fmt --check" { & $cargo fmt --all -- --check } }
if (-not $SkipBuild) {
    Invoke-Stage "cargo clippy" { & $cargo clippy --all-features --all-targets -- -D warnings }
    Invoke-Stage "cargo build" { & $cargo build --all-features }
}
if (-not $SkipTests) { Invoke-Stage "cargo test" { & $cargo test --all-features } }

if (-not $SkipEmbedded) {
    $target = "thumbv7em-none-eabihf"
    $rustup = $null
    try { $rustup = Resolve-Tool "rustup" } catch { }
    $installed = if ($rustup) { & $rustup target list --installed } else { @() }
    if ($installed -contains $target) {
        Invoke-Stage "cargo build --no-default-features --target $target" { & $cargo build --no-default-features --target $target }
    }
    else {
        Write-Host "FAILED: the no_std build needs the $target target (rustup target add $target), or pass -SkipEmbedded"
        exit 1
    }
}

Write-Host "fuaran-core-rs: green"
exit 0
