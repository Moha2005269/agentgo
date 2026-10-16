# AgentGo deterministic dependency bootstrap.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1                 # all stages
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage toolchain
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage cef
#   powershell -ExecutionPolicy Bypass -File scripts\bootstrap.ps1 -Stage fetch
#
# Stages:
#   toolchain — install rustup + the pinned Rust toolchain if missing (idempotent).
#   cef       — download the pinned CEF binary distribution, verify size+sha1(+sha256
#               when recorded), extract, and lay it out exactly as cef-rs expects.
#   fetch     — cargo fetch (deterministic via the committed Cargo.lock).
#   all       — toolchain, cef, fetch (default).
#
# Every download is verified against scripts/pins.json before extraction.
# A changed upstream fails the bootstrap loudly; it never silently uses a
# different CEF build.

param(
    [ValidateSet("all", "toolchain", "cef", "fetch")]
    [string]$Stage = "all",
    # Override the CEF storage root (default: <repo>\third_party\cef-dist).
    [string]$CefRoot
)

. "$PSScriptRoot\env.ps1"

$pins = Get-AgentGoPins
$runToolchain = ($Stage -eq "all" -or $Stage -eq "toolchain")
$runCef = ($Stage -eq "all" -or $Stage -eq "cef")
$runFetch = ($Stage -eq "all" -or $Stage -eq "fetch")

# ---------------------------------------------------------------- toolchain
if ($runToolchain) {
    Write-Host "[bootstrap] stage: toolchain (pin: $($pins.rust.toolchain))" -ForegroundColor Cyan
    $cargoBin = Ensure-CargoOnPath
    $rustup = Join-Path $cargoBin "rustup.exe"
    $cargo = Join-Path $cargoBin "cargo.exe"

    if (-not (Test-Path $rustup)) {
        Write-Host "[bootstrap] rustup missing; installing from static.rust-lang.org" -ForegroundColor Yellow
        $tmp = Join-Path $env:TEMP "rustup-init.exe"
        Invoke-WebRequest -Uri "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" -OutFile $tmp -UseBasicParsing
        & $tmp -y --default-toolchain "stable-x86_64-pc-windows-msvc" --profile minimal | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "rustup installation failed" }
    }

    # The repository's rust-toolchain.toml pins the exact version; any
    # cargo invocation from inside the repo installs it automatically.
    Push-Location $AgentGoRoot
    try {
        $v = (& $cargo "version") -join ""
        if ($LASTEXITCODE -ne 0) { throw "cargo failed inside the repo (rust-toolchain.toml pin)" }
        Write-Host "[bootstrap] $v"
    } finally {
        Pop-Location
    }

    foreach ($component in $pins.rust.components) {
        & $rustup "component" "add" $component 2>$null | Out-Null
    }
    Write-Host "[bootstrap] toolchain ok" -ForegroundColor Green
}


# ---------------------------------------------------------------------- cef
if ($runCef) {
    $cef = $pins.cef
    Write-Host "[bootstrap] stage: cef (pin: $($cef.version))" -ForegroundColor Cyan

    if (-not $CefRoot) { $CefRoot = Join-Path $AgentGoRoot "third_party\cef-dist" }
    $downloads = Join-Path $AgentGoRoot "third_party\downloads"
    $cefDir = Join-Path $CefRoot "$($cef.version)\windows64"
    New-Item -ItemType Directory -Force -Path $downloads | Out-Null

    if (Test-Path (Join-Path $cefDir "libcef.dll")) {
        Write-Host "[bootstrap] cef distro already present at $cefDir (delete to re-download)"
    } else {
        $archivePath = Join-Path $downloads $cef.archiveName
        if (-not (Test-Path $archivePath)) {
            Write-Host "[bootstrap] downloading $($cef.archiveUrl)"
            $progressPreference = "SilentlyContinue"
            Invoke-WebRequest -Uri $cef.archiveUrl -OutFile $archivePath -UseBasicParsing
        }

        # Verify size.
        $size = (Get-Item $archivePath).Length
        if ($size -ne [int64]$cef.archiveSizeBytes) {
            throw "CEF archive size mismatch: expected $($cef.archiveSizeBytes), got $size"
        }
        # Verify sha1 against the pin.
        $sha1 = Get-FileSha1Hex -Path $archivePath
        if ($sha1 -ne $cef.archiveSha1) {
            throw "CEF archive sha1 mismatch: expected $($cef.archiveSha1), got $sha1"
        }
        # Verify sha256 when the pin records one.
        if ($cef.archiveSha256 -and $cef.archiveSha256 -ne "") {
            $sha256 = Get-FileSha256Hex -Path $archivePath
            if ($sha256 -ne $cef.archiveSha256) {
                throw "CEF archive sha256 mismatch: expected $($cef.archiveSha256), got $sha256"
            }
        }
        Write-Host "[bootstrap] archive verified (sha1 $sha1)"

        # Record the sha256 for future verification.
        $computed256 = Get-FileSha256Hex -Path $archivePath
        Set-Content -Path (Join-Path $downloads "verified-sha256.txt") -Value "$($cef.archiveName) $computed256"

        # Extract with bsdtar (Windows 10+ ships bz2-capable tar).
        $extractDir = Join-Path $downloads "extract-$($cef.version)"
        if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
        New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
        Write-Host "[bootstrap] extracting..."
        tar -xjf $archivePath -C $extractDir
        if ($LASTEXITCODE -ne 0) { throw "tar extraction failed" }

        $inner = Join-Path $extractDir ($cef.archiveName -replace '\.tar\.bz2$', '')
        if (-not (Test-Path $inner)) { throw "extracted folder missing: $inner" }

        # Restructure exactly like cef-rs's extract_target_archive:
        # windows64 = Release contents + Resources entries + cmake/include/...
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $cefDir) | Out-Null
        if (Test-Path $cefDir) { Remove-Item -Recurse -Force $cefDir }
        Move-Item (Join-Path $inner "Release") $cefDir

        $resources = Join-Path $inner "Resources"
        foreach ($entry in (Get-ChildItem $resources)) {
            Move-Item $entry.FullName (Join-Path $cefDir $entry.Name)
        }
        foreach ($rel in @("CMakeLists.txt", "cmake", "include", "libcef_dll")) {
            Move-Item (Join-Path $inner $rel) (Join-Path $cefDir $rel)
        }

        # cef-rs check_archive_json expects this descriptor in the dir.
        $archiveJson = @{
            type = "minimal"
            name = $cef.minimalArchive.name
            sha1 = $cef.minimalArchive.sha1
        } | ConvertTo-Json
        Set-Content -Path (Join-Path $cefDir "archive.json") -Value $archiveJson

        Remove-Item -Recurse -Force $extractDir
        Write-Host "[bootstrap] cef distro ready at $cefDir" -ForegroundColor Green
    }

    # Export CEF_PATH for subsequent builds in this session.
    $env:CEF_PATH = $CefRoot
    Write-Host "[bootstrap] CEF_PATH=$CefRoot (set for this session; scripts\build.ps1 re-sets it)"
}

# -------------------------------------------------------------------- fetch
if ($runFetch) {
    Write-Host "[bootstrap] stage: fetch (Cargo.lock is committed; updates are explicit)" -ForegroundColor Cyan
    Ensure-CargoOnPath | Out-Null
    Push-Location $AgentGoRoot
    try {
        cargo fetch --locked
        if ($LASTEXITCODE -ne 0) { throw "cargo fetch --locked failed" }
    } finally {
        Pop-Location
    }
    Write-Host "[bootstrap] fetch ok" -ForegroundColor Green
}

Write-Host "[bootstrap] done (stage: $Stage)" -ForegroundColor Green
