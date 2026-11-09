# Build the per-machine MSI from the release DLL, LICENSE, and README.
$ErrorActionPreference = 'Stop'

$version = $env:VERSION
$arch = $env:ARCH
$target = $env:TARGET
if (-not $version -or -not $arch -or -not $target) {
    throw 'VERSION, ARCH, and TARGET are required'
}

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$wixArch = if ($arch -eq 'arm64') { 'arm64' } else { 'x64' }
$dll = Join-Path $root "target\$target\release\muninn_coinbase_connector.dll"
if (-not (Test-Path -LiteralPath $dll)) {
    throw "missing $dll"
}

$payload = Join-Path $root 'dist\payload'
if (Test-Path -LiteralPath $payload) {
    Remove-Item -LiteralPath $payload -Recurse -Force
}
New-Item -ItemType Directory -Path $payload | Out-Null
Copy-Item -LiteralPath $dll -Destination (Join-Path $payload 'muninn_coinbase_connector.dll')
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $payload 'LICENSE')
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination (Join-Path $payload 'README.md')

dotnet tool install --global wix --version 7.0.1
if ($LASTEXITCODE -ne 0) {
    throw "dotnet tool install exited $LASTEXITCODE"
}
$wix = Join-Path $env:USERPROFILE '.dotnet\tools\wix.exe'
$payloadDefine = (Resolve-Path -LiteralPath $payload).Path.Replace('\', '/')
$out = Join-Path $root "dist\muninn-coinbase-connector-$version-windows-$arch.msi"
$wxs = Join-Path $root 'packaging\windows.wxs'
# WiX 7 refuses to build until this command accepts EULA id wix7. Switches are
# the single-dash forms from `wix build -?`; `--arch` is not a switch.
& $wix build '-acceptEula' 'wix7' '-arch' $wixArch `
    '-d' "Version=$version" `
    '-d' "Payload=$payloadDefine" `
    '-out' $out `
    $wxs
if ($LASTEXITCODE -ne 0) {
    throw "wix build exited $LASTEXITCODE"
}
if (-not (Test-Path -LiteralPath $out)) {
    throw "wix did not write $out"
}

Remove-Item -LiteralPath $payload -Recurse -Force
Write-Output $out
