# Build the workspace. Debug by default; -Release for the release build.
# Sets CEF_PATH to the repo-local pinned CEF distribution when present so
# the cef-rs sys crate resolves it deterministically.
# Pass -Runtime to compile the native browser executable as well.

param(
    [switch]$Release,
    [switch]$Runtime,
    [switch]$Sandbox
)

if ($Sandbox -and -not $Runtime) {
    throw "-Sandbox requires -Runtime"
}

. "$PSScriptRoot\env.ps1"
Ensure-CargoOnPath | Out-Null

# Prefer Visual Studio's signed Ninja distribution for the CEF C++ wrapper.
# A Python-installed ninja.exe on this machine is blocked by application
# control, even though it appears on PATH.
$vsNinjaCandidates = @(
    "$env:ProgramFiles\Microsoft Visual Studio\18\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
    "$env:ProgramFiles\Microsoft Visual Studio\18\Professional\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
    "$env:ProgramFiles\Microsoft Visual Studio\18\Enterprise\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
    "$env:ProgramFiles\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
    "$env:ProgramFiles\Microsoft Visual Studio\2022\Professional\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja",
    "$env:ProgramFiles\Microsoft Visual Studio\2022\Enterprise\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
)
$vsNinja = $vsNinjaCandidates |
    Where-Object { Test-Path (Join-Path $_ "ninja.exe") } |
    Select-Object -First 1
if ($vsNinja) {
    $env:PATH = "$vsNinja;$env:PATH"
}

if (-not $env:CEF_PATH) {
    $env:CEF_PATH = Join-Path $AgentGoRoot "third_party\cef-dist"
}

# Keep the manifest synchronized as part of every build.
& (Join-Path $PSScriptRoot "regen-manifest.ps1")

Push-Location $AgentGoRoot
try {
    if ($Runtime) {
        if ($Sandbox) {
            # The official CEF Windows sandbox shape is a client DLL loaded by
            # bootstrap.exe. Keep it separate from the direct development EXE.
            $args_ = @("build", "-p", "agentgo-app", "--lib", "--features", "sandbox", "--locked")
        } else {
            $args_ = @("build", "-p", "agentgo-app", "--bin", "agentgo", "--features", "runtime", "--locked")
        }
    } else {
        $args_ = @("build", "--workspace", "--locked")
    }
    if ($Release) { $args_ += "--release" }
    if ($Runtime) {
        # `agentgo-tool` (diagnostics, manifest, quiet-run) must not link CEF,
        # so it is built without the runtime feature, in its own invocation.
        $tool = @("build", "-p", "agentgo-app", "--bin", "agentgo-tool", "--bin", "agentgo-browser", "--locked")
        if ($Release) { $tool += "--release" }
        & cargo @tool
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    & cargo @args_
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($Sandbox) {
        $profile = if ($Release) { "release" } else { "debug" }
        $cefRoot = Get-ChildItem -LiteralPath (Join-Path $AgentGoRoot "third_party\cef-dist") -Recurse -File -Filter "bootstrap.exe" |
            Select-Object -First 1 -ExpandProperty DirectoryName
        if (-not $cefRoot) { throw "CEF bootstrap.exe was not found under third_party\cef-dist" }
        $bundle = Join-Path $AgentGoRoot "target\agentgo-sandbox\$profile"
        New-Item -ItemType Directory -Force -Path $bundle | Out-Null
        Copy-Item -LiteralPath (Join-Path $AgentGoRoot "target\$profile\agentgo_app.dll") -Destination (Join-Path $bundle "agentgo.dll") -Force
        $pdb = Join-Path $AgentGoRoot "target\$profile\agentgo_app.pdb"
        if (Test-Path -LiteralPath $pdb) { Copy-Item -LiteralPath $pdb -Destination (Join-Path $bundle "agentgo.pdb") -Force }
        Copy-Item -LiteralPath (Join-Path $cefRoot "bootstrap.exe") -Destination (Join-Path $bundle "agentgo.exe") -Force
        $version = (Select-String -Path (Join-Path $AgentGoRoot "Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
        & (Join-Path $PSScriptRoot "stamp-exe.ps1") -Exe (Join-Path $bundle "agentgo.exe") -Version $version
        # The bootstrap exe only runs the browser; it hands `call`, `mcp`, `version`
        # and the other commands to agentgo-tool.exe, and the Start menu shortcut
        # starts agentgo-browser.exe.
        foreach ($tool in @("agentgo-tool.exe", "agentgo-browser.exe")) {
            Copy-Item -LiteralPath (Join-Path $AgentGoRoot "target\$profile\$tool") -Destination $bundle -Force
        }
        Get-ChildItem -LiteralPath $cefRoot -File | Where-Object { $_.Extension -ne ".exe" } | Copy-Item -Destination $bundle -Force
        # Copying onto an existing locales folder would nest a second one inside it.
        $bundleLocales = Join-Path $bundle "locales"
        if (Test-Path -LiteralPath $bundleLocales) { Remove-Item -LiteralPath $bundleLocales -Recurse -Force }
        Copy-Item -LiteralPath (Join-Path $cefRoot "locales") -Destination $bundleLocales -Recurse -Force
        $bundleUi = Join-Path $bundle "ui"
        New-Item -ItemType Directory -Force -Path $bundleUi | Out-Null
        Get-ChildItem -LiteralPath (Join-Path $AgentGoRoot "ui") -Force |
            Copy-Item -Destination $bundleUi -Recurse -Force
        # Resources the runtime looks for beside the executable (as in the package).
        foreach ($dir in @("sdk", "skills")) {
            Copy-Item -LiteralPath (Join-Path $AgentGoRoot $dir) -Destination $bundle -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path (Join-Path $bundle "schemas") | Out-Null
        Copy-Item -LiteralPath (Join-Path $AgentGoRoot "schemas\capability-manifest.json") -Destination (Join-Path $bundle "schemas") -Force
        Write-Host "[sandbox] packaged bootstrap runtime: $bundle\agentgo.exe" -ForegroundColor Green
    } else {
        # Without -Runtime there is no agentgo.exe (it needs the runtime feature).
        $exe = if ($Runtime) { "agentgo.exe" } else { "agentgo-tool.exe" }
        $kind = if ($Release) { "release" } else { "debug" }
        Write-Host "[build] $kind binary: target\$kind\$exe" -ForegroundColor Green
    }
} finally {
    Pop-Location
}
