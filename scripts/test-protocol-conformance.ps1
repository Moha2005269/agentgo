# Protocol conformance harness.
#
# Drives the pinned protocol contract through the built `agentgo` binary
# and validates the schema files against the envelope fixtures. Milestone
# 0 scope: envelope/framing fixtures round-trip through the binary's
# version/doctor/manifest JSON outputs, and every schema file parses.
# Milestone 3 extends this with live stdio request/response sessions.

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

$failures = 0

function Assert-True($condition, $name) {
    if ($condition) {
        Write-Host "  [ok] $name" -ForegroundColor Green
    } else {
        Write-Host "  [FAIL] $name" -ForegroundColor Red
        $script:failures++
    }
}

Write-Host "[conformance] schema files parse as JSON"
foreach ($schema in @("protocol.schema.json", "policy.schema.json", "skill.schema.json", "capability-manifest.schema.json")) {
    $path = Join-Path $AgentGoRoot "schemas\$schema"
    try {
        Get-Content $path -Raw | ConvertFrom-Json | Out-Null
        Assert-True $true "$schema parses"
    } catch {
        Assert-True $false "$schema parses ($($_.Exception.Message))"
    }
}

Write-Host "[conformance] manifest parses and reports all sections"
$manifest = Get-Content (Join-Path $AgentGoRoot "schemas\capability-manifest.json") -Raw | ConvertFrom-Json
Assert-True ($manifest.sections.Count -ge 32) "manifest covers all inventory sections (got $($manifest.sections.Count))"
Assert-True ($manifest.counts.p0 -gt 400) "P0 surface preserved (got $($manifest.counts.p0))"
Assert-True ($manifest.counts.total -eq ($manifest.counts.p0 + $manifest.counts.p1 + $manifest.counts.p2)) "counts add up"

Write-Host "[conformance] binary reports pinned versions as JSON"
Push-Location $AgentGoRoot
try {
    $json = cargo run -q -p agentgo-app --bin agentgo-tool -- version --json | Out-String
    $v = $json | ConvertFrom-Json
    Assert-True ($null -ne $v.cef) "version JSON carries CEF pin"
    Assert-True ($v.cef -like "152.0.5+*") "CEF pin is 152.0.5"
    Assert-True ($v.chromium -eq "152.0.7977.54") "Chromium pin is 152.0.7977.54"
    Assert-True ($v.protocol -eq "0.1.0") "protocol version is 0.1.0"
    Assert-True ($v.nativeLaneImplemented -eq $false) "native lane honestly unimplemented"
} finally {
    Pop-Location
}

if ($failures -gt 0) {
    Write-Host "[conformance] FAILED with $failures failure(s)" -ForegroundColor Red
    exit 1
}
Write-Host "[conformance] all checks passed" -ForegroundColor Green
exit 0
