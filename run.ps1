#Requires -Version 7.0
# fuaran-core-rs: the one-command entry point.
# cargo fmt --check -> cargo clippy -> cargo build -> cargo test (and the premise probes in release) -> a no_std build for
# thumbv7em-none-eabihf (Cortex-M4F) -> (opt-in) the machine-checked proofs under verification/.
#
# `cargo test` includes the conformance legs (tests/conformance.rs), which read Core's vectors from a
# Core checkout: FUARAN_CORE_DIR, or `fuaran-core/` beside this repository, at the commit
# core-pin.json names. They FAIL when no checkout is found rather than skip, because a skipped leg
# certifies nothing.
#
# The no_std build is part of the contract, so a missing target FAILS here, naming the remedy;
# -SkipEmbedded is the deliberate opt-out.
#
# -Verify runs the proofs (verification/README.md): the Kani harnesses and the Creusot replay, then prints the
# replay record's identity. Each
# tool runs natively when it is on PATH, else in Docker from the Dockerfile beside it (built once);
# with neither, the leg SKIPS and names the reason. A proof that runs and does not check FAILS.
# Switches: -SkipFormat / -SkipBuild / -SkipTests / -SkipEmbedded for fast iteration.
[CmdletBinding()]
param(
    [switch]$SkipFormat,
    [switch]$SkipBuild,
    [switch]$SkipTests,
    [switch]$SkipEmbedded,
    [switch]$Verify
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

# One proof leg. $Tool is the cargo subcommand binary ("cargo-kani"); $Dir the crate under
# verification/; $Native the cargo invocation when the tool is on PATH, $InContainer the same
# work as a shell line run in the image built from $Dir's Dockerfile with the repository at /src.
# A skip is a named reason and exit 0; a tool that ran and did not prove the goals is a failure.
function Invoke-Proof {
    param([string]$Name, [string]$Tool, [string]$Dir, [string]$Image, [scriptblock]$Native, [string]$InContainer)
    if (Get-Command $Tool -CommandType Application -ErrorAction SilentlyContinue) {
        Push-Location $Dir
        try { & $Native } finally { Pop-Location }
        return
    }
    $docker = Get-Command docker -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $docker) {
        Write-Host "SKIPPED: $Name needs '$Tool' on PATH or Docker; neither was found"
        $global:LASTEXITCODE = 0
        return
    }
    $dockerfile = Split-Path -Parent (Join-Path $PSScriptRoot $Dir)
    if (-not (& $docker.Source image inspect $Image 2>$null)) {
        Write-Host "building $Image from $dockerfile/Dockerfile (once; the toolchain is pinned there)"
        & $docker.Source build -t $Image $dockerfile
        if ($LASTEXITCODE -ne 0) { return }
    }
    & $docker.Source run --rm -v "${PSScriptRoot}:/src" $Image bash -c $InContainer
}

$cargo = Resolve-Tool "cargo"

if (-not $SkipFormat) { Invoke-Stage "cargo fmt --check" { & $cargo fmt --all -- --check } }
if (-not $SkipBuild) {
    Invoke-Stage "cargo clippy" { & $cargo clippy --all-features --all-targets -- -D warnings }
    Invoke-Stage "cargo build" { & $cargo build --all-features }
}
if (-not $SkipTests) {
    Invoke-Stage "cargo test" { & $cargo test --all-features }
    # The depth premise is claimed for both profiles (DECISIONS.md D10): the reader's frames differ between them.
    Invoke-Stage "cargo test --release (premises)" { & $cargo test --release --all-features --test premises }
}

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

if ($Verify) {
    Invoke-Stage "verify: Kani harnesses (verification/kani)" {
        Invoke-Proof -Name "the Kani harnesses" -Tool "cargo-kani" -Dir "verification/kani" `
            -Image "fuaran-core-verify-kani" `
            -Native { & $cargo kani } `
            -InContainer "cd /src/verification/kani && cargo kani"
    }
    Invoke-Stage "verify: Creusot replay (verification/creusot/escape)" {
        Invoke-Proof -Name "the Creusot proof" -Tool "cargo-creusot" -Dir "verification/creusot/escape" `
            -Image "fuaran-core-verify-creusot" `
            -Native { & $cargo creusot; if ($LASTEXITCODE -eq 0) { & $cargo creusot prove --replay } } `
            -InContainer "cd /src/verification/creusot/escape && cargo creusot && cargo creusot prove --replay"
    }
    # The identity of the replay record, which the reference's claims ladder records on this proof's row and a
    # host-compliance declaration names (verification/README.md, "The replay identity"). It is a property of
    # the files, printed whether or not the leg above ran.
    Write-Host "creusot/escape replay identity: $(& pwsh -NoProfile -File verification/creusot/replay-identity.ps1)"
}

Write-Host "fuaran-core-rs: green"
exit 0
