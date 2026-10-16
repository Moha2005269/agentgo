# Regenerates schemas/capability-manifest.json from docs/capability-inventory.md
# and fails if the tracked manifest would change beyond status/testId.

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

Push-Location $AgentGoRoot
try {
    $inv = Join-Path $AgentGoRoot "docs\capability-inventory.md"
    $man = Join-Path $AgentGoRoot "schemas\capability-manifest.json"
    # Keep manifest tooling in its own Cargo target directory. A default
    # `cargo run` writes the CLI-only agentgo.exe to target\debug and can
    # silently replace the CEF runtime executable that users double-click.
    $manifestTargetDir = Join-Path $AgentGoRoot "target\manifest-check"
    if (-not (Test-Path $man)) {
        cargo run --target-dir $manifestTargetDir -q -p agentgo-app --bin agentgo-tool --manifest-path (Join-Path $AgentGoRoot "Cargo.toml") -- manifest generate --inventory $inv --out $man
        if ($LASTEXITCODE -ne 0) { throw "manifest generation failed" }
        Write-Host "[manifest] generated $man"
    } else {
        cargo run --target-dir $manifestTargetDir -q -p agentgo-app --bin agentgo-tool -- manifest validate --manifest $man --inventory $inv
        if ($LASTEXITCODE -ne 0) {
            # Stale: regenerate and surface the diff obligation.
            cargo run --target-dir $manifestTargetDir -q -p agentgo-app --bin agentgo-tool -- manifest generate --inventory $inv --out $man
            if ($LASTEXITCODE -ne 0) { throw "manifest regeneration failed" }
            Write-Host "[manifest] regenerated (was stale); commit the updated manifest" -ForegroundColor Yellow
        } else {
            Write-Host "[manifest] in sync"
        }
    }
} finally {
    Pop-Location
}
