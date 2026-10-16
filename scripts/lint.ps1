# Lint: clippy across the workspace with warnings denied.

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

if (-not $env:CEF_PATH) {
    $env:CEF_PATH = Join-Path $AgentGoRoot "third_party\cef-dist"
}

Push-Location $AgentGoRoot
try {
    cargo clippy --workspace --locked --all-targets -- -D warnings
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
