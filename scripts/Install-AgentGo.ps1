# Installs AgentGo for the current Windows user. No administrator rights.
# Run it from the unzipped AgentGo folder:
#
#   powershell -ExecutionPolicy Bypass -File Install-AgentGo.ps1
#   powershell -ExecutionPolicy Bypass -File Install-AgentGo.ps1 -Uninstall
#
# What it does, and nothing else:
#   1. copies this folder to %LOCALAPPDATA%\Programs\AgentGo
#   2. adds that folder to your user PATH, so `agentgo` works in any terminal
#   3. creates a Start menu shortcut "AgentGo"
#   4. copies the agent skill to %USERPROFILE%\.claude\skills\agentgo
#      (skipped when you have no .claude folder)
#   5. registers the browser with Claude Code as an MCP server, when the
#      `claude` command exists
# Your profile (%LOCALAPPDATA%\AgentGo) is never touched; -Uninstall keeps it
# unless you add -RemoveProfile.

param([switch]$Uninstall, [switch]$RemoveProfile, [switch]$NoClaude)

$ErrorActionPreference = "Stop"
$target = Join-Path $env:LOCALAPPDATA "Programs\AgentGo"
$shortcut = Join-Path ([Environment]::GetFolderPath("Programs")) "AgentGo.lnk"
$skill = Join-Path $env:USERPROFILE ".claude\skills\agentgo"
$exe = Join-Path $target "agentgo.exe"

function Set-UserPath([bool]$add) {
    $parts = @([Environment]::GetEnvironmentVariable("Path", "User") -split ";" | Where-Object { $_ -and ($_.TrimEnd("\") -ne $target) })
    if ($add) { $parts += $target }
    [Environment]::SetEnvironmentVariable("Path", ($parts -join ";"), "User")
}

if ($Uninstall) {
    if (Get-Process agentgo -ErrorAction SilentlyContinue) { throw "AgentGo is running. Close it first." }
    if (-not $NoClaude -and (Get-Command claude -ErrorAction SilentlyContinue)) { & claude mcp remove --scope user agentgo 2>$null | Out-Null; & claude mcp remove agentgo 2>$null | Out-Null }
    if (-not $NoClaude -and (Get-Command droid -ErrorAction SilentlyContinue)) { & droid mcp remove agentgo 2>$null | Out-Null }
    foreach ($path in @($shortcut, $skill, $target)) { if (Test-Path $path) { Remove-Item -Recurse -Force $path } }
    Set-UserPath $false
    if ($RemoveProfile) {
        $data = Join-Path $env:LOCALAPPDATA "AgentGo"
        if (Test-Path $data) { Remove-Item -Recurse -Force $data }
        Write-Host "AgentGo and its profile were removed."
    } else {
        Write-Host "AgentGo was removed. Your profile is still in $env:LOCALAPPDATA\AgentGo (add -RemoveProfile to delete it)."
    }
    return
}

$source = $PSScriptRoot
if (-not (Test-Path (Join-Path $source "agentgo.exe"))) { throw "Run this script from the unzipped AgentGo folder (agentgo.exe is not next to it)." }
if (Get-Process agentgo -ErrorAction SilentlyContinue) { throw "AgentGo is running. Close it first." }

# Check the files against the checksums that came with them.
$sums = Join-Path $source "SHA256SUMS.txt"
if (Test-Path $sums) {
    foreach ($line in Get-Content $sums) {
        $hash, $name = $line -split "  ", 2
        if ($name -eq "SHA256SUMS.txt") { continue }
        $file = Join-Path $source $name
        if (-not (Test-Path $file) -or (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower() -ne $hash) {
            throw "The file $name is missing or was changed. Download AgentGo again."
        }
    }
}

if ((Resolve-Path $source).Path.TrimEnd("\") -ne $target) {
    if (Test-Path $target) { Remove-Item -Recurse -Force $target }
    New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
    Copy-Item -Recurse $source $target
}
Set-UserPath $true

$link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
# The windowless launcher: no console window opens with the browser.
$launcher = Join-Path $target "agentgo-browser.exe"
$link.TargetPath = if (Test-Path $launcher) { $launcher } else { $exe }
$link.WorkingDirectory = $target
$link.Description = "AgentGo Browser"
$link.Save()

if (Test-Path (Join-Path $env:USERPROFILE ".claude")) {
    if (Test-Path $skill) { Remove-Item -Recurse -Force $skill }
    New-Item -ItemType Directory -Force (Split-Path $skill) | Out-Null
    Copy-Item -Recurse (Join-Path $target "skills\agentgo") $skill
}
$claude = $false
if (-not $NoClaude -and (Get-Command claude -ErrorAction SilentlyContinue)) {
    & claude mcp remove --scope user agentgo 2>$null | Out-Null
    & claude mcp remove agentgo 2>$null | Out-Null
    & claude mcp add --scope user agentgo -- $exe mcp | Out-Null
    $claude = ($LASTEXITCODE -eq 0)
}

$droid = $false
if (-not $NoClaude -and (Get-Command droid -ErrorAction SilentlyContinue)) {
    & $exe setup droid | Out-Null
    $droid = ($LASTEXITCODE -eq 0)
}

Write-Host "AgentGo is installed in $target"
Write-Host "  Start it:        Start menu > AgentGo   (or: agentgo, in a new terminal)"
if ($droid) { Write-Host "  Droid:           registered as the MCP server 'agentgo' (full path, attaches to the open browser)." }
if ($claude) {
    Write-Host "  Claude Code:     registered as the MCP server 'agentgo'. Ask Claude to use the browser."
} else {
    Write-Host "  Connect an agent: claude mcp add --scope user agentgo -- `"$exe`" mcp"
}
Write-Host "  Remove it:       powershell -ExecutionPolicy Bypass -File `"$target\Install-AgentGo.ps1`" -Uninstall"
