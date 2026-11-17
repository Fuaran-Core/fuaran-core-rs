#Requires -Version 7.0
# fuaran-core-rs: the one-command entry point.
# cargo fmt --check -> cargo clippy -> cargo build -> cargo test (and the premise probes in release, the capability and
# query legs again without `std`, the allocation table in debug and in release on both profiles) -> a no_std build for
# thumbv7em-none-eabihf (Cortex-M4F) -> the same build, tests and no_std build again at the minimum Rust version
# Cargo.toml declares (`rust-version`) -> (opt-in) the machine-checked proofs under verification/.
#
# `cargo test` includes the conformance legs (tests/conformance.rs), which read Core's vectors from a
# Core checkout: FUARAN_CORE_DIR, or `fuaran-core/` beside this repository, at the commit
# core-pin.json names. They FAIL when no checkout is found rather than skip, because a skipped leg
# certifies nothing.
#
# The no_std build is part of the contract, so a missing target FAILS here, naming the remedy;
# -SkipEmbedded is the deliberate opt-out.
#
# The minimum (DECISIONS.md D15): the format check and the lints are the newest tools' and run on the default toolchain
# only; every other leg runs again at the minimum, through rustup (`cargo +<toolchain>`), when that toolchain is
# installed. When it is not, the run names it as missing with the commands that install it, and does not run its legs:
# nothing is installed from here. -MinimumOnly runs the minimum's legs alone and FAILS when the toolchain or its
# embedded target is missing (CI's minimum job, so a green job is one whose legs ran); -SkipMinimum leaves them to that
# job (CI's stable job).
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
    [switch]$SkipMinimum,
    [switch]$MinimumOnly,
    [switch]$Verify
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if ($SkipMinimum -and $MinimumOnly) { throw "-SkipMinimum and -MinimumOnly exclude each other." }

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
$rustc = Resolve-Tool "rustc"
$rustup = $null
try { $rustup = Resolve-Tool "rustup" } catch { }
$target = "thumbv7em-none-eabihf"

# The build, the tests and the no_std build, on one toolchain: $On is @() for the default toolchain, or
# @("+<toolchain>") for the minimum; $Label names it in each stage. The allocation table runs in debug and in release,
# hosted and without `std`, printing its rows (DECISIONS.md D12, D14.8, D15).
function Invoke-Legs {
    param([string[]]$On, [string]$Label, [bool]$Embedded)
    Invoke-Stage "rustc --version$Label" { & $rustc @On --version }
    if (-not $SkipBuild) {
        Invoke-Stage "cargo build$Label" { & $cargo @On build --all-features }
    }
    if (-not $SkipTests) {
        Invoke-Stage "cargo test$Label" { & $cargo @On test --all-features }
        # The depth premise is claimed for both profiles (DECISIONS.md D10): the reader's frames differ between them.
        Invoke-Stage "cargo test --release (premises)$Label" { & $cargo @On test --release --all-features --test premises }
        # The capability law, document and gate vectors and the query grant vectors are claimed for both profiles
        # (DECISIONS.md D11, D13, D14): run again on the library built as the embedded build builds it (no `std`), with
        # the query module's own tests. They run on this host; the target build below only builds.
        Invoke-Stage "cargo test --no-default-features (capability and query legs)$Label" { & $cargo @On test --no-default-features --test capability --test capability_gate --test query_grant --test query }
        # The decision path, a capability's and a read's, at zero allocations once the registry is built (D12, D14):
        # every row asserted, on both profiles, in debug and in release.
        Invoke-Stage "allocation table, without std, debug$Label" { & $cargo @On test --no-default-features --test capability_alloc -- --nocapture }
        Invoke-Stage "allocation table, hosted, release$Label" { & $cargo @On test --release --all-features --test capability_alloc -- --nocapture }
        Invoke-Stage "allocation table, without std, release$Label" { & $cargo @On test --release --no-default-features --test capability_alloc -- --nocapture }
    }
    if ($Embedded) {
        Invoke-Stage "cargo build --no-default-features --target $target$Label" { & $cargo @On build --no-default-features --target $target }
    }
}

if (-not $MinimumOnly) {
    if (-not $SkipFormat) { Invoke-Stage "cargo fmt --check" { & $cargo fmt --all -- --check } }
    if (-not $SkipBuild) { Invoke-Stage "cargo clippy" { & $cargo clippy --all-features --all-targets -- -D warnings } }
    $embedded = $false
    if (-not $SkipEmbedded) {
        $installed = if ($rustup) { & $rustup target list --installed } else { @() }
        if ($installed -notcontains $target) {
            Write-Host "FAILED: the no_std build needs the $target target (rustup target add $target), or pass -SkipEmbedded"
            exit 1
        }
        $embedded = $true
    }
    Invoke-Legs -On @() -Label "" -Embedded $embedded
}

# The minimum: Cargo.toml's `rust-version`, read here so this script names no version of its own. A toolchain is
# looked for among those rustup lists (`1.85`, or a patch of it such as `1.85.1`) and never installed from here.
$minimum = (Select-String -Path Cargo.toml -Pattern '^rust-version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value
if (-not $minimum) { throw "Cargo.toml declares no rust-version." }
$minimumSummary = "minimum $minimum"
$install = "rustup toolchain install $minimum --profile minimal"
$addTarget = "rustup target add $target --toolchain $minimum"
if ($SkipMinimum) {
    Write-Host "SKIPPED: the legs at the minimum Rust version $minimum (-SkipMinimum)"
    $minimumSummary += ": skipped (-SkipMinimum)"
}
elseif ($SkipBuild -and $SkipTests -and $SkipEmbedded) {
    Write-Host "SKIPPED: the legs at the minimum Rust version $minimum (-SkipBuild -SkipTests -SkipEmbedded select none)"
    $minimumSummary += ": no leg selected"
}
else {
    $global:LASTEXITCODE = 0
    $toolchain = $null
    if ($rustup) {
        $pattern = '^' + [regex]::Escape($minimum) + '(\.\d+)?-'
        $toolchain = & $rustup toolchain list | ForEach-Object { ($_ -split '\s+')[0] } | Where-Object { $_ -match $pattern } | Select-Object -First 1
    }
    if (-not $toolchain) {
        $why = if ($rustup) { "is not installed" } else { "cannot be looked for: rustup was not found" }
        Write-Host "MISSING: the minimum Rust version Cargo.toml declares ($minimum) $why, so its legs did not run. Remedy:"
        Write-Host "    $install"
        if (-not $SkipEmbedded) { Write-Host "    $addTarget" }
        if ($MinimumOnly) { Write-Host "FAILED: -MinimumOnly needs the minimum's toolchain"; exit 1 }
        $global:LASTEXITCODE = 0
        $minimumSummary += ": MISSING, its legs did not run"
    }
    else {
        $embedded = $false
        if (-not $SkipEmbedded) {
            if ((& $rustup target list --installed --toolchain $toolchain) -contains $target) { $embedded = $true }
            else {
                Write-Host "MISSING: the $target target at the minimum ($toolchain), so its no_std build will not run. Remedy:"
                Write-Host "    rustup target add $target --toolchain $toolchain"
                if ($MinimumOnly) { Write-Host "FAILED: -MinimumOnly needs the minimum's $target target"; exit 1 }
                $global:LASTEXITCODE = 0
            }
        }
        # Nothing here may install a toolchain: the one named is installed, and rustup is told not to fetch one.
        $autoInstall = $env:RUSTUP_AUTO_INSTALL
        $env:RUSTUP_AUTO_INSTALL = "0"
        try { Invoke-Legs -On @("+$toolchain") -Label " (minimum $toolchain)" -Embedded $embedded }
        finally { $env:RUSTUP_AUTO_INSTALL = $autoInstall }
        $minimumSummary += ": ran on $toolchain" + $(if (-not $SkipEmbedded -and -not $embedded) { ", its no_std build MISSING" } else { "" })
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

Write-Host "fuaran-core-rs: green ($minimumSummary)"
exit 0
