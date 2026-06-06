# Build Windows release (installer + package)
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root
$env:CARGO_TARGET_DIR = Join-Path $Root "target"

Write-Host "Building release binaries..."
cargo build --release --bins
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "Generating peers.enc..."
$releaseData = Join-Path $env:LOCALAPPDATA "DigitalBritishPound\DBC\data"
if ($env:DBC_RELEASE_DATA_DIR) { $releaseData = $env:DBC_RELEASE_DATA_DIR }
if (Test-Path (Join-Path $releaseData "peer_key")) {
    cargo run --release --bin gen-peers-enc -- --data-dir $releaseData
} else {
    cargo run --release --bin gen-peers-enc
}
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Pkg = Join-Path $Root "release-package"
Copy-Item -Force (Join-Path $Root "target\release\dbc-node.exe") $Pkg
Copy-Item -Force (Join-Path $Root "target\release\dbc-ui.exe") $Pkg
Copy-Item -Force (Join-Path $Root "docs\USER_GUIDE.md") (Join-Path $Pkg "USER_GUIDE.txt")

$makensis = @(
    "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
    "$env:ProgramFiles\NSIS\makensis.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($makensis) {
    & $makensis (Join-Path $Root "installer\dbc-installer.nsi")
    Copy-Item -Force (Join-Path $Root "dbc-installer.exe") $Pkg
}

cmd /c (Join-Path $Root "scripts\update_sha256sum.cmd")
Write-Host "Done. Installer: $(Join-Path $Root 'dbc-installer.exe')"
