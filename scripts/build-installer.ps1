# Builds AgentGo Setup (installer/) and writes two installers to dist\:
#
#   AgentGo-Setup.exe        small; downloads the latest GitHub release and
#                            checks it against the published SHA-256
#   AgentGo-Setup-full.exe   the browser package appended, works offline
#
#   powershell -File scripts\build-installer.ps1 [-Zip dist\AgentGo-<v>-win64.zip]
#
# The appended package ends with a 48-byte trailer read by the installer:
# sha256 (32 bytes) | package length (8, little-endian) | "AGPAYLD1".

param([string]$Zip)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

Push-Location (Join-Path $AgentGoRoot "installer")
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { cargo build --release }
    if ($LASTEXITCODE -ne 0) { throw "installer build failed" }
} finally {
    Pop-Location
}

$dist = Join-Path $AgentGoRoot "dist"
New-Item -ItemType Directory -Force $dist | Out-Null
$stub = Join-Path $AgentGoRoot "installer\target\release\agentgo-setup.exe"
$web = Join-Path $dist "AgentGo-Setup.exe"
Copy-Item $stub $web -Force
Write-Host "[installer] $web"

if (-not $Zip) {
    $Zip = Get-ChildItem $dist -Filter "AgentGo-*-win64.zip" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if ($Zip -and (Test-Path $Zip)) {
    $full = Join-Path $dist "AgentGo-Setup-full.exe"
    $hash = [System.Security.Cryptography.SHA256]::Create()
    $zipStream = [System.IO.File]::OpenRead((Resolve-Path $Zip))
    try { $digest = $hash.ComputeHash($zipStream); $length = $zipStream.Length } finally { $zipStream.Close() }
    $out = [System.IO.File]::Create($full)
    try {
        foreach ($part in @($stub, (Resolve-Path $Zip).Path)) {
            $in = [System.IO.File]::OpenRead($part)
            try { $in.CopyTo($out) } finally { $in.Close() }
        }
        $out.Write($digest, 0, 32)
        $out.Write([BitConverter]::GetBytes([UInt64]$length), 0, 8)
        $magic = [System.Text.Encoding]::ASCII.GetBytes("AGPAYLD1")
        $out.Write($magic, 0, 8)
    } finally {
        $out.Close()
    }
    foreach ($file in @($web, $full)) {
        $sum = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower()
        "$sum  $(Split-Path $file -Leaf)" | Out-File -Encoding ascii "$file.sha256"
    }
    Write-Host "[installer] $full (package $([math]::Round($length / 1MB)) MB)"
} else {
    Write-Host "[installer] no package zip found; only the web installer was written" -ForegroundColor Yellow
}
