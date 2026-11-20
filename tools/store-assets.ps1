# Regenerates the images of the Microsoft Store package (packaging\store\Assets) from the
# two drawings of the application icon (assets\icon): the small drawing for images of
# 32 px and less, the full one above, as for the .ico (tools\icon.ps1).
#
#   tools\store-assets.ps1
#
# Images (names follow the package resource rules; tools\msix.ps1 indexes them):
#   Square44x44Logo.scale-*            app list and taskbar, per display scaling
#   Square44x44Logo.targetsize-*       the same at exact sizes, plain and "unplated"
#                                      (no tile plate: the taskbar and Start use these)
#   Square150x150Logo.scale-*          the medium tile, the icon at two thirds of it
#   StoreLogo.scale-*                  the package's logo
# Only run when a drawing changes; the images are committed.
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$icons = Join-Path $root 'assets\icon'
$out = Join-Path $root 'packaging\store\Assets'

$edge = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'Microsoft Edge not found' }

function DataUri([string]$file) {
    'data:image/svg+xml;base64,' + [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $icons $file)))
}
$small = DataUri 'BetterVolume-small.svg'
$large = DataUri 'BetterVolume.svg'

# Every image: file name, image size, icon size (centred).
$images = @()
foreach ($scale in 100, 125, 150, 200, 400) {
    $f = $scale / 100.0
    $s44 = [int][math]::Round(44 * $f)
    $images += , @("Square44x44Logo.scale-$scale.png", $s44, $s44)
    $s150 = [int][math]::Round(150 * $f)
    $images += , @("Square150x150Logo.scale-$scale.png", $s150, [int][math]::Round($s150 * 2 / 3))
    $s50 = [int][math]::Round(50 * $f)
    $images += , @("StoreLogo.scale-$scale.png", $s50, $s50)
}
foreach ($size in 16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256) {
    foreach ($suffix in '', '_altform-unplated', '_altform-lightunplated') {
        $images += , @("Square44x44Logo.targetsize-$size$suffix.png", $size, $size)
    }
}

# One page per image size row: images side by side at known places.
$gap = 8
$x = 0
$places = @()
$html = ''
$height = 0
foreach ($image in $images) {
    $size = $image[1]
    $icon = $image[2]
    $src = if ($icon -le 32) { $small } else { $large }
    $offset = [int][math]::Floor(($size - $icon) / 2)
    $html += "<img src=""$src"" style=""position:absolute;left:$($x + $offset)px;top:${offset}px;width:${icon}px;height:${icon}px"">"
    $places += , @($image[0], $x, $size)
    $x += $size + $gap
    if ($size -gt $height) { $height = $size }
}
$page = "<!doctype html><html><head><style>html,body{margin:0;background:transparent}</style></head><body>$html</body></html>"

$work = Join-Path ([IO.Path]::GetTempPath()) ('bv-store-assets-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $work | Out-Null
try {
    $file = Join-Path $work 'assets.html'
    $shot = Join-Path $work 'assets.png'
    [IO.File]::WriteAllText($file, $page, [Text.UTF8Encoding]::new($false))
    $arguments = @(
        '--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1',
        '--default-background-color=00000000', "--user-data-dir=$(Join-Path $work 'profile')",
        "--window-size=$x,$height", "--screenshot=$shot", ([Uri]$file).AbsoluteUri
    )
    $process = Start-Process -FilePath $edge -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if (-not (Test-Path $shot)) { throw "Edge produced no image (exit code $($process.ExitCode))" }

    New-Item -ItemType Directory -Force $out | Out-Null
    Get-ChildItem $out -Filter '*.png' | Remove-Item
    Add-Type -AssemblyName System.Drawing
    $sheet = [Drawing.Bitmap]::FromFile($shot)
    try {
        foreach ($place in $places) {
            $size = $place[2]
            $bitmap = $sheet.Clone([Drawing.Rectangle]::new($place[1], 0, $size, $size), [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $bitmap.Save((Join-Path $out $place[0]), [Drawing.Imaging.ImageFormat]::Png)
            $bitmap.Dispose()
        }
    } finally {
        $sheet.Dispose()
    }
    Write-Host "wrote $($places.Count) images to $out"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
