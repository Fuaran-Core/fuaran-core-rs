#Requires -Version 7.0
# fuaran-core-rs: the one-command entry point.
# cargo fmt --check -> cargo build -> cargo test -> a no_std build for thumbv7em-none-eabihf.
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
if (-not $SkipBuild) { Invoke-Stage "cargo build" { & $cargo build --all-features } }
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
        Write-Host "SKIPPED: no_std build ($target not installed; rustup target add $target)"
    }
}

Write-Host "fuaran-core-rs: green"
exit 0
