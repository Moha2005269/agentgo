# Signs AgentGo's own executables with Authenticode (signtool) and checks
# the result. The certificate is the owner's: a code-signing certificate in
# the Windows certificate store (thumbprint) or a .pfx file.
#
#   powershell -File scripts\sign.ps1 -Folder dist\AgentGo-0.1.0-win64 -Thumbprint <sha1>
#   powershell -File scripts\sign.ps1 -Folder <dir> -Pfx cert.pfx     (password in AGENTGO_SIGN_PASSWORD)
#   powershell -File scripts\sign.ps1 -Folder <dir> -Check            (only report signatures)
#   powershell -File scripts\sign.ps1 -Folder <dir> -WhatIf           (what would be signed)
#
# The thumbprint can also come from AGENTGO_SIGN_THUMBPRINT. Files that are
# already signed by someone else (Google's Chromium files) are left alone.

param(
    [Parameter(Mandatory = $true)][string]$Folder,
    [string]$Thumbprint = $env:AGENTGO_SIGN_THUMBPRINT,
    [string]$Pfx = $env:AGENTGO_SIGN_PFX,
    [string]$Timestamp = "http://timestamp.digicert.com",
    [switch]$Check,
    [switch]$WhatIf
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Folder)) { throw "no folder $Folder" }

function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $found = Get-ChildItem -LiteralPath $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
    if ($found) { return $found.FullName }
    return $null
}

# What AgentGo itself ships: its executables and any unsigned library.
$files = Get-ChildItem -LiteralPath $Folder -File | Where-Object { $_.Extension -in ".exe", ".dll" }
$report = foreach ($file in $files) {
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    [pscustomobject]@{
        File   = $file.Name
        Status = "$($signature.Status)"
        Signer = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { "" }
        Ours   = $file.Name -like "agentgo*" -or $signature.Status -eq "NotSigned"
    }
}

if ($Check) {
    $report | Format-Table -AutoSize | Out-String | Write-Host
    $unsigned = @($report | Where-Object { $_.Ours -and $_.Status -ne "Valid" })
    Write-Host "[sign] $($unsigned.Count) of AgentGo's files are not validly signed"
    exit ([int]($unsigned.Count -gt 0))
}

$targets = @($report | Where-Object { $_.Ours -and ($_.Status -ne "Valid" -or $_.File -like "agentgo*") })
$tool = Find-SignTool
if ($WhatIf) {
    Write-Host "[sign] signtool: $(if ($tool) { $tool } else { 'not found (install the Windows SDK signing tools)' })"
    Write-Host "[sign] would sign: $(($targets | ForEach-Object File) -join ', ')"
    Write-Host "[sign] certificate: $(if ($Thumbprint) { "store thumbprint $Thumbprint" } elseif ($Pfx) { "pfx $Pfx" } else { 'none given' })"
    exit 0
}
if (-not $tool) { throw "signtool.exe was not found; install the Windows SDK (Signing Tools for Desktop Apps)" }
if (-not $Thumbprint -and -not $Pfx) { throw "no certificate: pass -Thumbprint (certificate store) or -Pfx (with AGENTGO_SIGN_PASSWORD)" }

$common = @("sign", "/fd", "SHA256", "/tr", $Timestamp, "/td", "SHA256", "/d", "AgentGo")
$certificate = if ($Thumbprint) { @("/sha1", $Thumbprint) } else { @("/f", $Pfx) + $(if ($env:AGENTGO_SIGN_PASSWORD) { @("/p", $env:AGENTGO_SIGN_PASSWORD) } else { @() }) }
foreach ($target in $targets) {
    $path = Join-Path $Folder $target.File
    & $tool @common @certificate $path
    if ($LASTEXITCODE -ne 0) { throw "signtool failed for $($target.File)" }
    $after = Get-AuthenticodeSignature -LiteralPath $path
    if ($after.Status -ne "Valid") { throw "$($target.File) is not validly signed after signing: $($after.Status)" }
    Write-Host "[sign] signed $($target.File): $($after.SignerCertificate.Subject)"
}
