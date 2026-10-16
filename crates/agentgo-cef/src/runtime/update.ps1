# AgentGo update swap. Waits for the browser process to exit, then replaces
# the installed files with a staged version, keeping the old ones for
# rollback. Run by browser.update_stage (apply) and browser.update_rollback.
#   update.ps1 -WaitPid <pid> -Install <dir> -Staged <dir> -Backup <dir> [-Relaunch <exe args>]
param(
  [int]$WaitPid = 0,
  [Parameter(Mandatory = $true)][string]$Install,
  [Parameter(Mandatory = $true)][string]$Staged,
  [Parameter(Mandatory = $true)][string]$Backup,
  [string]$Relaunch = ''
)
$ErrorActionPreference = 'Stop'
if ($WaitPid -gt 0) { try { Wait-Process -Id $WaitPid -Timeout 120 -ErrorAction Stop } catch {} }
Start-Sleep -Milliseconds 300
$log = Join-Path $Install 'updates\swap.log'
function Note($text) { Add-Content -LiteralPath $log -Value ("{0:o} {1}" -f (Get-Date), $text) }
try {
  if (-not (Test-Path -LiteralPath $Staged)) { throw "staged folder $Staged is missing" }
  # The previous version, minus the updates folder itself.
  if (Test-Path -LiteralPath $Backup) { Remove-Item -LiteralPath $Backup -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $Backup | Out-Null
  Get-ChildItem -LiteralPath $Install -Force | Where-Object { $_.Name -ne 'updates' } | ForEach-Object {
    Move-Item -LiteralPath $_.FullName -Destination (Join-Path $Backup $_.Name) -Force
  }
  try {
    Get-ChildItem -LiteralPath $Staged -Force | ForEach-Object {
      Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $Install $_.Name) -Recurse -Force
    }
  } catch {
    # A half copy is worse than the old version: put it back.
    Get-ChildItem -LiteralPath $Install -Force | Where-Object { $_.Name -ne 'updates' } | Remove-Item -Recurse -Force
    Get-ChildItem -LiteralPath $Backup -Force | ForEach-Object { Move-Item -LiteralPath $_.FullName -Destination (Join-Path $Install $_.Name) -Force }
    throw
  }
  Remove-Item -LiteralPath (Join-Path $Install 'updates\pending.json') -Force -ErrorAction SilentlyContinue
  Set-Content -LiteralPath (Join-Path $Install 'updates\applied.json') -Value (@{ from = $Staged; backup = $Backup; at = (Get-Date).ToString('o') } | ConvertTo-Json -Compress)
  Note "swapped in $Staged (previous kept in $Backup)"
  if ($Relaunch) {
    # The windowless launcher when present, so no console window opens.
    $exe = Join-Path $Install 'agentgo-browser.exe'
    if (-not (Test-Path -LiteralPath $exe)) { $exe = Join-Path $Install 'agentgo.exe' }
    if (Test-Path -LiteralPath $exe) { Start-Process -FilePath $exe -ArgumentList $Relaunch }
  }
  exit 0
} catch {
  Note "swap failed: $($_.Exception.Message)"
  exit 1
}
