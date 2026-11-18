# Makes the two files of a release (FR-130) from the release build:
#
#   target\package\BetterVolume-<version>-setup.exe     (installer)
#   target\package\BetterVolume-<version>-portable.exe  (portable single file)
#
#   tools\package.ps1
#
# Both are target\release\BetterVolume.exe followed by target\release\bv_taskbar_helper.dll
# and a trailer (layout in crates\bv-core\src\package.rs, written by the bv-core example
# "package", so the format has one definition). The release build must exist and must
# have been made by tools\check.ps1 (build-machine paths remapped). Each file is
# read back after it is written.
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

$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
if (-not $match.Success) { Write-Host 'workspace version not found in Cargo.toml'; exit 1 }
$version = $match.Groups[1].Value

$tool = Join-Path $root 'target\release\examples\package.exe'
if (-not (Test-Path $tool)) { Write-Host "missing $tool (run tools\check.ps1 first)"; exit 1 }

$out = Join-Path $root 'target\package'
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter 'BetterVolume-*.exe' | Remove-Item

$bad = 0
foreach ($kind in 'setup', 'portable') {
    $file = Join-Path $out "BetterVolume-$version-$kind.exe"
    & $tool $kind $program $helper $file
    if ($LASTEXITCODE -ne 0) { $bad = 1; continue }
    $read = & $tool verify $file
    if ($LASTEXITCODE -ne 0 -or -not ("$read".StartsWith("$kind "))) {
        Write-Host "$file does not read back as a $kind file"
        $bad = 1
        continue
    }
    $size = [math]::Round((Get-Item $file).Length / 1MB, 3)
    Write-Host "$([IO.Path]::GetFileName($file)): $size MB ($read)"
}
exit $bad
