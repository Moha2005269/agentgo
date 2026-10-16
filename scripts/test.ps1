# Run the full test suite: unit + integration tests, then the manifest
# lockstep guard, then the protocol conformance harness.

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

if (-not $env:CEF_PATH) {
    $env:CEF_PATH = Join-Path $AgentGoRoot "third_party\cef-dist"
}

& (Join-Path $PSScriptRoot "regen-manifest.ps1")

Push-Location $AgentGoRoot
try {
    cargo test --workspace --locked
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

# Protocol conformance harness (fixtures + envelope/framing checks).
& (Join-Path $PSScriptRoot "test-protocol-conformance.ps1")
exit $LASTEXITCODE
