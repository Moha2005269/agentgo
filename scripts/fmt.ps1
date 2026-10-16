# Format check/fix. -Fix applies rustfmt.

param([switch]$Fix)

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

Push-Location $AgentGoRoot
try {
    if ($Fix) {
        cargo fmt --all
    } else {
        cargo fmt --all --check
    }
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
