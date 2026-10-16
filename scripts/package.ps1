# Builds a portable AgentGo folder and zip: the browser, its command-line tool,
# the Chromium runtime files, the bundled interface and the agent skill.
# The folder runs from anywhere; nothing is installed and nothing is written
# outside %LOCALAPPDATA%\AgentGo. Removing AgentGo = deleting the folder
# (and that data folder, if the profile should go too).
#
#   powershell -File scripts\package.ps1            (from the debug build)
#   powershell -File scripts\package.ps1 -Release   (from the release build)
#   ... -Sign       sign AgentGo's executables first (scripts\sign.ps1; the
#                   certificate comes from AGENTGO_SIGN_THUMBPRINT or AGENTGO_SIGN_PFX)
#   ... -UpdateBase https://example.com/agentgo/   address the zip will be served
#                   from, written into update-manifest.json for browser.update_check
#
# Build first: scripts\build.ps1 -Runtime [-Release].

param([switch]$Release, [switch]$Sign, [string]$UpdateBase = "", [switch]$Unsandboxed)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\env.ps1"

$profile_ = if ($Release) { "release" } else { "debug" }
$tools = Join-Path $AgentGoRoot "target\$profile_"
# The shipped shape is the sandboxed one: CEF bootstrap.exe + agentgo.dll, so renderers
# run at untrusted integrity. -Unsandboxed packages the plain development exe instead.
$source = if ($Unsandboxed) { $tools } else { Join-Path $AgentGoRoot "target\agentgo-sandbox\$profile_" }
if (-not (Test-Path (Join-Path $source "agentgo.exe"))) {
    throw "no build in $source; run scripts\build.ps1 -Runtime$(if (-not $Unsandboxed) { ' -Sandbox' })$(if ($Release) { ' -Release' }) first"
}
$version = (Select-String -Path (Join-Path $AgentGoRoot "Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$name = "AgentGo-$version-win64$(if (-not $Release) { '-debug' })$(if ($Unsandboxed) { '-nosandbox' })"
$dist = Join-Path $AgentGoRoot "dist"
$out = Join-Path $dist $name
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null

# Chromium runtime files that must sit next to the executable.
$runtime = @(
    "agentgo.exe",
    "libcef.dll", "chrome_elf.dll", "d3dcompiler_47.dll", "dxcompiler.dll", "dxil.dll",
    "libEGL.dll", "libGLESv2.dll", "vk_swiftshader.dll", "vk_swiftshader_icd.json", "vulkan-1.dll",
    "icudtl.dat", "v8_context_snapshot.bin",
    "chrome_100_percent.pak", "chrome_200_percent.pak", "resources.pak"
)
if (-not $Unsandboxed) { $runtime += "agentgo.dll" }
foreach ($tool in @("agentgo-tool.exe", "agentgo-browser.exe")) {
    $path = Join-Path $tools $tool
    if (-not (Test-Path $path)) { throw "missing $tool in $tools; run scripts\build.ps1 -Runtime$(if ($Release) { ' -Release' }) first" }
    Copy-Item $path $out
}
foreach ($file in $runtime) {
    $path = Join-Path $source $file
    if (-not (Test-Path $path)) { throw "missing runtime file $file in $source" }
    Copy-Item $path $out
}
Copy-Item -Recurse (Join-Path $source "locales") (Join-Path $out "locales")
Copy-Item -Recurse (Join-Path $AgentGoRoot "ui") (Join-Path $out "ui")
Copy-Item -Recurse (Join-Path $AgentGoRoot "skills") (Join-Path $out "skills")
Copy-Item -Recurse (Join-Path $AgentGoRoot "sdk") (Join-Path $out "sdk")
New-Item -ItemType Directory -Force (Join-Path $out "schemas") | Out-Null
Copy-Item (Join-Path $AgentGoRoot "schemas\capability-manifest.json") (Join-Path $out "schemas")
foreach ($doc in @("README.md", "CHANGELOG.md", "LICENSE", "docs\threat-model.md")) {
    $path = Join-Path $AgentGoRoot $doc
    if (Test-Path $path) { Copy-Item $path $out }
}
Copy-Item (Join-Path $PSScriptRoot "Install-AgentGo.ps1") $out
# Notices for the ad blocker's engine and filter lists (MPL-2.0, GPLv3/CC BY-SA).
$notices = Join-Path $out "licenses"
New-Item -ItemType Directory -Force $notices | Out-Null
foreach ($notice in @("adblock-rust-LICENSE-MPL-2.0", "EasyList-NOTICE.txt")) {
    Copy-Item (Join-Path $AgentGoRoot "third_party\licenses\$notice") $notices
}
$credits = Join-Path $source "CREDITS.html"
if (Test-Path $credits) { Copy-Item $credits (Join-Path $out "THIRD-PARTY-CREDITS.html") }

# A record of what exactly was packaged.
$commit = (& git -C $AgentGoRoot rev-parse HEAD 2>$null)
$dirty = [bool](& git -C $AgentGoRoot status --porcelain 2>$null)
$pins = Get-Content (Join-Path $PSScriptRoot "pins.json") -Raw | ConvertFrom-Json
[ordered]@{
    name = "AgentGo"; version = $version; profile = $profile_
    commit = "$commit"; uncommittedChanges = $dirty
    builtAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    rustc = "$(& rustc --version)"; pins = $pins
} | ConvertTo-Json -Depth 6 | Out-File -Encoding utf8 (Join-Path $out "BUILD-INFO.json")

if ($Sign) {
    & (Join-Path $PSScriptRoot "sign.ps1") -Folder $out
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
}

# Checksums of every file, then of the zip.
$sums = Get-ChildItem -Recurse -File $out | Sort-Object FullName | ForEach-Object {
    "{0}  {1}" -f (Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLower(), $_.FullName.Substring($out.Length + 1).Replace('\', '/')
}
$sums | Out-File -Encoding ascii (Join-Path $out "SHA256SUMS.txt")

$zip = "$out.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $out -DestinationPath $zip
$zipHash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
"$zipHash  $name.zip" | Out-File -Encoding ascii "$zip.sha256"

# What browser.update_check reads (the setting updateManifest points at it).
[ordered]@{
    version = $version
    url = "$UpdateBase$name.zip"
    sha256 = $zipHash
    notes = "AgentGo $version"
} | ConvertTo-Json | Out-File -Encoding utf8 (Join-Path $dist "update-manifest.json")
# browser.update_check ignores a manifest without a valid Ed25519 signature.
node (Join-Path $PSScriptRoot "dev\sign-update.js") sign (Join-Path $dist "update-manifest.json")
if ($LASTEXITCODE -ne 0) { Write-Warning "update manifest left UNSIGNED (no key); browsers will refuse it" }

$size = [math]::Round((Get-Item $zip).Length / 1MB, 1)
Write-Host "[package] $out"
Write-Host "[package] $zip ($size MB) sha256 $zipHash"
