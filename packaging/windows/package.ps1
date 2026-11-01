param(
    [ValidateSet("amd64", "arm64")]
    [string]$Arch = $(if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "amd64" })
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$versionMatch = Select-String -Path (Join-Path $root "Cargo.toml") `
    -Pattern '^version = "(.+)"' | Select-Object -First 1
if (-not $versionMatch) {
    throw "Cannot read the version."
}
$version = $versionMatch.Matches[0].Groups[1].Value

$wintunVersion = "0.14.1"
$wintunHash = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51"
$cache = Join-Path $root "target\wintun"
$zip = Join-Path $cache "wintun-$wintunVersion.zip"
$unpacked = Join-Path $cache "wintun-$wintunVersion"

New-Item -ItemType Directory -Force $cache | Out-Null
if (-not (Test-Path $zip)) {
    Invoke-WebRequest "https://www.wintun.net/builds/wintun-$wintunVersion.zip" -OutFile $zip
}
$actualHash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $wintunHash) {
    throw "The Wintun file is not correct."
}
$dll = Join-Path $unpacked "wintun\bin\$Arch\wintun.dll"
Expand-Archive -Path $zip -DestinationPath $unpacked -Force
if (-not (Test-Path $dll)) {
    throw "The Wintun file has no $Arch DLL."
}

Push-Location $root
try {
    cargo build --release --locked -p fiend-desktop -p fiend-daemon -p fiend-cli
    if ($LASTEXITCODE -ne 0) {
        throw "The build failed."
    }
} finally {
    Pop-Location
}

$stage = Join-Path $root "dist\fiend-$version-windows-$Arch"
if (Test-Path $stage) {
    Remove-Item -Recurse -Force $stage
}
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item (Join-Path $root "target\release\fiend-desktop.exe") (Join-Path $stage "Fiend.exe")
Copy-Item (Join-Path $root "target\release\fiend-daemon.exe") (Join-Path $stage "fiend-daemon.exe")
Copy-Item (Join-Path $root "target\release\fiend.exe") (Join-Path $stage "fiend-cli.exe")
Copy-Item $dll (Join-Path $stage "wintun.dll")
Copy-Item (Join-Path $unpacked "wintun\LICENSE.txt") (Join-Path $stage "wintun-LICENSE.txt")

$package = Join-Path $root "dist\fiend-$version-windows-$Arch.zip"
if (Test-Path $package) {
    Remove-Item $package
}
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $package
Write-Host "Package: $package"
