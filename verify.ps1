#Requires -Version 7.0
# The gate: a pass-through to run.ps1, so the gate and the one-command entry point cannot diverge.
$ErrorActionPreference = "Stop"
$global:LASTEXITCODE = 0
& (Join-Path $PSScriptRoot "run.ps1") @args
exit $LASTEXITCODE
