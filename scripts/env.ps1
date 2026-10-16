# AgentGo build environment helpers. Sourced by the other scripts.
# Requires PowerShell 5.1+ (powershell.exe) or PowerShell 7.

$ErrorActionPreference = "Stop"

# Locate the repository root relative to this script.
$Script:AgentGoRoot = Split-Path -Parent $PSScriptRoot

function Get-AgentGoPins {
    $pinsPath = Join-Path $AgentGoRoot "scripts\pins.json"
    if (-not (Test-Path $pinsPath)) {
        throw "pins.json missing at $pinsPath"
    }
    return (Get-Content $pinsPath -Raw | ConvertFrom-Json)
}

function Get-FileSha1Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $sha1 = [System.Security.Cryptography.SHA1]::Create()
    try {
        ($sha1.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }) -join ""
    } finally {
        $sha1.Dispose()
    }
}

function Get-FileSha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        ($sha256.ComputeHash($bytes) | ForEach-Object { $_.ToString("x2") }) -join ""
    } finally {
        $sha256.Dispose()
    }
}

function Ensure-CargoOnPath {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $cargoBin = Join-Path $cargoHome "bin"
    if ((Test-Path (Join-Path $cargoBin "cargo.exe")) -and -not ($env:PATH -like "*$cargoBin*")) {
        $env:PATH = "$cargoBin;$env:PATH"
    }
    return $cargoBin
}
