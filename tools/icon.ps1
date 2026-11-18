# Regenerates the application icon (assets\icon\BetterVolume.ico) from its two
# drawings: BetterVolume-small.svg for 16 to 32 px (a simplified drawing, the full one is
# unreadable there) and BetterVolume.svg for 40 px and up.
#
#   tools\icon.ps1
#
# The drawings are rendered by Microsoft Edge (present on every Windows 11) in headless
# mode, on a transparent background, at their exact pixel sizes; the PNG images are then
# packed into one .ico file (PNG entries, read by Windows since Vista). Only run when a
# drawing changes; the .ico is committed and embedded by crates\bv-app\build.rs.
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$assets = Join-Path $root 'assets\icon'
$out = Join-Path $assets 'BetterVolume.ico'

$edge = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw 'Microsoft Edge not found' }

$sizes = @(16, 20, 24, 32, 40, 48, 64, 256)
$gap = 8
function DataUri([string]$file) {
    'data:image/svg+xml;base64,' + [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $assets $file)))
}
$small = DataUri 'BetterVolume-small.svg'
$large = DataUri 'BetterVolume.svg'

# Every size side by side on one transparent page, at known places.
$x = 0
$places = @()
$images = ''
foreach ($size in $sizes) {
    $src = if ($size -le 32) { $small } else { $large }
    $images += "<img src=""$src"" style=""position:absolute;left:${x}px;top:0;width:${size}px;height:${size}px"">"
    $places += , @($size, $x)
    $x += $size + $gap
}
$page = "<!doctype html><html><head><style>html,body{margin:0;background:transparent}</style></head><body>$images</body></html>"

$work = Join-Path ([IO.Path]::GetTempPath()) ('bv-icon-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $work | Out-Null
try {
    $html = Join-Path $work 'icon.html'
    $shot = Join-Path $work 'icon.png'
    [IO.File]::WriteAllText($html, $page, [Text.UTF8Encoding]::new($false))
    $arguments = @(
        '--headless=new', '--disable-gpu', '--hide-scrollbars', '--force-device-scale-factor=1',
        '--default-background-color=00000000', "--user-data-dir=$(Join-Path $work 'profile')",
        '--window-size=800,300', "--screenshot=$shot", ([Uri]$html).AbsoluteUri
    )
    $process = Start-Process -FilePath $edge -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if (-not (Test-Path $shot)) { throw "Edge produced no image (exit code $($process.ExitCode))" }

    Add-Type -AssemblyName System.Drawing
    $sheet = [Drawing.Bitmap]::FromFile($shot)
    $entries = @()
    try {
        foreach ($place in $places) {
            $size = $place[0]
            $bitmap = $sheet.Clone([Drawing.Rectangle]::new($place[1], 0, $size, $size), [Drawing.Imaging.PixelFormat]::Format32bppArgb)
            $stream = [IO.MemoryStream]::new()
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $bitmap.Dispose()
            $entries += , @($size, $stream.ToArray())
        }
    } finally {
        $sheet.Dispose()
    }

    # ICONDIR, then one ICONDIRENTRY per image, then the PNG images.
    $file = [IO.MemoryStream]::new()
    $writer = [IO.BinaryWriter]::new($file)
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$entries.Count)
    $offset = 6 + 16 * $entries.Count
    foreach ($entry in $entries) {
        $size = $entry[0]
        $dimension = if ($size -ge 256) { 0 } else { $size }
        $writer.Write([byte]$dimension)
        $writer.Write([byte]$dimension)
        $writer.Write([byte]0)
        $writer.Write([byte]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]32)
        $writer.Write([uint32]$entry[1].Length)
        $writer.Write([uint32]$offset)
        $offset += $entry[1].Length
    }
    foreach ($entry in $entries) { $writer.Write([byte[]]$entry[1]) }
    $writer.Flush()
    [IO.File]::WriteAllBytes($out, $file.ToArray())
    Write-Host "wrote $out ($($entries.Count) sizes, $($file.Length) bytes)"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
