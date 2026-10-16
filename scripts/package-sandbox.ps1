# AgentGo CEF sandbox package helper.
# Keep this as the discoverable one-command entry point; build.ps1 owns the
# actual pinned Cargo/package logic so normal and sandbox builds cannot drift.
param([switch]$Release)

$args_ = @('-Runtime', '-Sandbox')
if ($Release) { $args_ += '-Release' }
& (Join-Path $PSScriptRoot 'build.ps1') @args_
exit $LASTEXITCODE
