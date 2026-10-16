# Doctor: verifies the pinned toolchain, CEF distribution, local layout,
# and manifest sync. Thin wrapper over `agentgo doctor` so a fresh machine
# has one entry point before/after bootstrap.

param([switch]$Strict)

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

# The manifest must exist for the doctor manifest check; create it from
# the inventory when missing (first run on a fresh clone).
$man = Join-Path $AgentGoRoot "schemas\capability-manifest.json"
$inv = Join-Path $AgentGoRoot "docs\capability-inventory.md"
if (-not (Test-Path $man)) {
    & (Join-Path $PSScriptRoot "regen-manifest.ps1")
}

# Default CEF_PATH to the repo-local distro so doctor verifies the pinned
# layout even when the caller did not export it.
if (-not $env:CEF_PATH) {
    $env:CEF_PATH = Join-Path $AgentGoRoot "third_party\cef-dist"
}

Push-Location $AgentGoRoot
try {
    $env:AGENTGO_REPO = $AgentGoRoot
    $args_ = @("run", "-q", "-p", "agentgo-app", "--bin", "agentgo-tool", "--", "doctor")
    if ($Strict) { $args_ += "--strict" }
    & cargo @args_
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
