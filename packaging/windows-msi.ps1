# Build the per-user MSI from the release DLL, LICENSE, and README.
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
    '-pdbtype' 'none' `
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

# A 32 KiB stub plus a sidecar cab means the cabinet was not embedded.
# A valid MSI can be smaller than the uncompressed DLL because the cabinet is compressed.
$externalCabs = @()
foreach ($dir in @([string]$root, (Join-Path $root 'dist'))) {
    $externalCabs += @(Get-ChildItem -LiteralPath $dir -Filter 'cab*.cab' -ErrorAction SilentlyContinue)
}
if ($externalCabs) {
    $names = ($externalCabs | ForEach-Object { $_.FullName }) -join ', '
    throw "WiX left an external cabinet ($names); the MSI did not embed it"
}

$msiLength = (Get-Item -LiteralPath $out).Length
$minEmbedded = 200000
if ($msiLength -lt $minEmbedded) {
    throw "MSI $msiLength bytes looks like an empty stub; cabinet was probably not embedded"
}

foreach ($dir in @($root, (Join-Path $root 'dist'))) {
    Get-ChildItem -LiteralPath $dir -Filter '*.wixpdb' -ErrorAction SilentlyContinue |
        Remove-Item -Force
}

Remove-Item -LiteralPath $payload -Recurse -Force
Write-Output $out
