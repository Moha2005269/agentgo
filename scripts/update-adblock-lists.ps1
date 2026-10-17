# Refreshes the filter lists that are compiled into AgentGo and rebuilds the
# engine snapshot (crates\agentgo-adblock\data). Run it before a release, look
# at the diff, then rebuild. The browser itself never downloads these lists.
#
#   powershell -File scripts\update-adblock-lists.ps1

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\env.ps1"

$data = Join-Path $AgentGoRoot "crates\agentgo-adblock\data"
$lists = Join-Path $data "lists"
New-Item -ItemType Directory -Force $lists | Out-Null

$sources = [ordered]@{
    "easylist"           = "https://easylist.to/easylist/easylist.txt"
    "easyprivacy"        = "https://easylist.to/easylist/easyprivacy.txt"
    "ublock-filters"     = "https://ublockorigin.github.io/uAssets/filters/filters.min.txt"
    "ublock-privacy"     = "https://ublockorigin.github.io/uAssets/filters/privacy.min.txt"
    "ublock-badware"     = "https://ublockorigin.github.io/uAssets/filters/badware.min.txt"
    "ublock-unbreak"     = "https://ublockorigin.github.io/uAssets/filters/unbreak.min.txt"
    "ublock-quick-fixes" = "https://ublockorigin.github.io/uAssets/filters/quick-fixes.min.txt"
    "adguard-base"       = "https://filters.adtidy.org/extension/ublock/filters/2_without_easylist.txt"
    "adguard-social"     = "https://filters.adtidy.org/extension/ublock/filters/4.txt"
    "adguard-dns"        = "https://adguardteam.github.io/AdGuardSDNSFilter/Filters/filter.txt"
    "peter-lowe"         = "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext"
    "urlhaus"            = "https://malware-filter.gitlab.io/malware-filter/urlhaus-filter-agh-online.txt"
    "hagezi-pro"         = "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/adblock/pro.plus.txt"
    "adguard-cookie"     = "https://filters.adtidy.org/extension/ublock/filters/18.txt"
    "adguard-mobile"     = "https://filters.adtidy.org/extension/ublock/filters/11.txt"
    "ubo-cookies"        = "https://ublockorigin.github.io/uAssets/filters/annoyances-cookies.txt"
    "easylist-cookie"    = "https://secure.fanboy.co.nz/fanboy-cookiemonster_ubo.txt"
    # Not a blocking list: the source of the dangerous-site check (threats.txt).
    "phishing"           = "https://malware-filter.gitlab.io/malware-filter/phishing-filter-agh.txt"
}
foreach ($name in $sources.Keys) {
    $tmp = Join-Path $env:TEMP "agentgo-$name.txt"
    Invoke-WebRequest -Uri $sources[$name] -OutFile $tmp -UseBasicParsing

    # A captive portal or an error page must never replace a list.
    $length = (Get-Item $tmp).Length
    $rules = (Select-String -Path $tmp -Pattern '^(\|\||@@|##)' -List -ErrorAction SilentlyContinue)
    if ($length -lt 20KB -or -not $rules) {
        throw "$name from $($sources[$name]) does not look like a filter list ($length bytes)"
    }
    Copy-Item $tmp (Join-Path $lists "$name.txt") -Force
    Write-Host "[adblock] $name.txt updated, $([math]::Round($length / 1KB)) KB"
}

# Scriptlets and redirect resources (Brave's build of uBlock Origin's).
$res = Join-Path $env:TEMP "agentgo-resources.json"
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/brave/adblock-resources/master/dist/resources.json" -OutFile $res -UseBasicParsing
if ((Get-Item $res).Length -lt 100KB) { throw "resources.json does not look right" }
Copy-Item $res (Join-Path $data "resources.json") -Force

Push-Location $AgentGoRoot
try {
    # Known malware and phishing hosts for the dangerous-site check.
    & node (Join-Path $PSScriptRoot "dev\build-threats.js")
    if ($LASTEXITCODE -ne 0) { throw "building threats.txt failed" }
    & cargo run --release -p agentgo-adblock --example build_snapshot
    if ($LASTEXITCODE -ne 0) { throw "building the snapshot failed" }
} finally { Pop-Location }
Write-Host "[adblock] rebuild to compile the new snapshot in: scripts\build.ps1 -Runtime -Release" -ForegroundColor Green
