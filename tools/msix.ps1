# Makes the Microsoft Store package (FR-134) from the release build:
#
#   target\package\BetterVolume-<version>.msix
#
#   tools\msix.ps1
#
# The package holds target\release\BetterVolume.exe, target\release\bv_taskbar_helper.dll,
# the images of packaging\store\Assets (tools\store-assets.ps1) indexed in resources.pri,
# and packaging\store\AppxManifest.xml with the workspace version followed by ".0". It is
# not signed: the Store signs it when it is published. The release build must exist and
# must have been made by tools\check.ps1. Needs the Windows SDK (makeappx, makepri), which
# the C++ build tools of the Rust toolchain install.
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$program = Join-Path $root 'target\release\BetterVolume.exe'
$helper = Join-Path $root 'target\release\bv_taskbar_helper.dll'
foreach ($file in $program, $helper) {
    if (-not (Test-Path $file)) { Write-Host "missing $file (run tools\check.ps1 first)"; exit 1 }
}

$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$match = [regex]::Match($cargo, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
if (-not $match.Success) { Write-Host 'workspace version not found in Cargo.toml'; exit 1 }
$version = $match.Groups[1].Value
# The Store wants four numbers, the last one 0; a preview (2.2.0-beta.1) has no package.
if ($version -notmatch '^\d+\.\d+\.\d+$') { Write-Host "no Store package for version $version"; exit 0 }

# The newest Windows SDK that has both tools.
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$sdk = Get-ChildItem $kits -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64' } |
    Where-Object { (Test-Path (Join-Path $_ 'makeappx.exe')) -and (Test-Path (Join-Path $_ 'makepri.exe')) } |
    Select-Object -First 1
if (-not $sdk) { Write-Host 'Windows SDK not found (makeappx.exe, makepri.exe)'; exit 1 }
$makeappx = Join-Path $sdk 'makeappx.exe'
$makepri = Join-Path $sdk 'makepri.exe'

$work = Join-Path $root 'target\msix'
$layout = Join-Path $work 'layout'
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory (Join-Path $layout 'Assets') | Out-Null
Copy-Item $program, $helper $layout
Copy-Item (Join-Path $root 'packaging\store\Assets\*.png') (Join-Path $layout 'Assets')
$manifest = (Get-Content (Join-Path $root 'packaging\store\AppxManifest.xml') -Raw).Replace('$VERSION$', "$version.0")
[IO.File]::WriteAllText((Join-Path $layout 'AppxManifest.xml'), $manifest, [Text.UTF8Encoding]::new($false))

# The images' qualifiers (scale, target size) only work through resources.pri.
$config = Join-Path $work 'priconfig.xml'
& $makepri createconfig /cf $config /dq en-US /pv 10.0.0 /o | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'makepri createconfig failed'; exit 1 }
& $makepri new /pr $layout /cf $config /mn (Join-Path $layout 'AppxManifest.xml') /of (Join-Path $layout 'resources.pri') /o | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'makepri new failed'; exit 1 }

$out = Join-Path $root 'target\package'
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter 'BetterVolume-*.msix' | Remove-Item
$package = Join-Path $out "BetterVolume-$version.msix"
& $makeappx pack /d $layout /p $package /o | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host 'makeappx pack failed'; exit 1 }

# Read back: the package lists the program, the helper and the manifest's version.
$check = Join-Path $work 'check'
& $makeappx unpack /p $package /d $check /o | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "$package does not unpack"; exit 1 }
foreach ($name in 'BetterVolume.exe', 'bv_taskbar_helper.dll', 'resources.pri', 'AppxManifest.xml') {
    if (-not (Test-Path (Join-Path $check $name))) { Write-Host "$package lacks $name"; exit 1 }
}
if ((Get-FileHash (Join-Path $check 'BetterVolume.exe')).Hash -ne (Get-FileHash $program).Hash) {
    Write-Host "$package does not hold this build's program"; exit 1
}
if (-not ((Get-Content (Join-Path $check 'AppxManifest.xml') -Raw).Contains("Version=""$version.0"""))) {
    Write-Host "$package does not carry version $version.0"; exit 1
}
$size = [math]::Round((Get-Item $package).Length / 1MB, 3)
Write-Host "$([IO.Path]::GetFileName($package)): $size MB (version $version.0)"
exit 0
