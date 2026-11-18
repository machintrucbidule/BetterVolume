# Exercises updates against a local imitation of the release service: builds a
# copy of BetterVolume carrying a newer version number, serves its two release files from
# the loopback address the way GitHub's release list does, and starts a copy of the
# current version pointed at it (environment variable BETTERVOLUME_UPDATE_SERVER).
#
#   tools\update-test.ps1 [-Mode portable|installed|serve] [-Version 0.1.1] [-Port 8123]
#                         [-Seconds 0]
#
#   portable   (default) a portable copy of the current version, in
#              target\update-test\portable, with its settings beside it: it offers the
#              newer version and, on "Install", replaces its own file and restarts.
#   installed  the installed copy (it must exist): on "Install" it downloads the newer
#              setup file, which updates the installation and starts it.
#   serve      only the imitation.
#
# The running BetterVolume, if any, is asked to quit first (exit message, as the
# installer does). The imitation stops after -Seconds (0 = when Enter is pressed). The
# current release files must exist (run tools\check.ps1 first). The newer copy is the
# same code with another number (BETTERVOLUME_VERSION), built like a release build
# (build-machine paths remapped).
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).
param(
    [ValidateSet('portable', 'installed', 'serve')]
    [string]$Mode = 'portable',
    [string]$Version,
    [int]$Port = 8123,
    [int]$Seconds = 0
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$match = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"(\d+)\.(\d+)\.(\d+)([^"]*)"')
if (-not $match.Success) { Write-Host 'workspace version not found in Cargo.toml'; exit 1 }
$current = "$($match.Groups[1].Value).$($match.Groups[2].Value).$($match.Groups[3].Value)$($match.Groups[4].Value)"
if (-not $Version) {
    $Version = "$($match.Groups[1].Value).$($match.Groups[2].Value).$([int]$match.Groups[3].Value + 1)"
}

$release = Join-Path $root 'target\release'
$helper = Join-Path $release 'bv_taskbar_helper.dll'
$packageTool = Join-Path $release 'examples\package.exe'
$serverTool = Join-Path $release 'examples\release-server.exe'
$currentPortable = Join-Path $root "target\package\BetterVolume-$current-portable.exe"
foreach ($file in $helper, $packageTool, $serverTool, $currentPortable) {
    if (-not (Test-Path $file)) { Write-Host "missing $file (run tools\check.ps1 first)"; exit 1 }
}

$work = Join-Path $root 'target\update-test'
$served = Join-Path $work 'server'
New-Item -ItemType Directory -Force $work, $served | Out-Null
Get-ChildItem $served -Filter '*.exe' | Remove-Item

# --- The newer version ------------------------------------------------------------------
Write-Host "building BetterVolume $Version (current: $current)"
$profileDir = [Environment]::GetFolderPath('UserProfile')
$flags = @("--remap-path-prefix=$profileDir=~")
if ($env:CARGO_HOME) { $flags += "--remap-path-prefix=$($env:CARGO_HOME)=cargo-home" }
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$previousVersion = $env:BETTERVOLUME_VERSION
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [string][char]0x1F
$env:BETTERVOLUME_VERSION = $Version
try {
    cargo build --release --locked -p bv-app --bin BetterVolume --target-dir (Join-Path $work 'build')
    if ($LASTEXITCODE -ne 0) { Write-Host 'build failed'; exit 1 }
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    $env:BETTERVOLUME_VERSION = $previousVersion
}
$newer = Join-Path $work 'build\release\BetterVolume.exe'
foreach ($kind in 'setup', 'portable') {
    $file = Join-Path $served "BetterVolume-$Version-$kind.exe"
    & $packageTool $kind $newer $helper $file
    if ($LASTEXITCODE -ne 0) { Write-Host "cannot make $file"; exit 1 }
}

# --- The imitation of the release service -----------------------------------------------
$log = Join-Path $work 'server.log'
$server = Start-Process $serverTool -ArgumentList '--port', "$Port", '--folder', "`"$served`"" `
    -PassThru -WindowStyle Hidden -RedirectStandardOutput $log
Start-Sleep -Milliseconds 500
if ($server.HasExited) { Write-Host "the imitation could not start (port $Port in use?)"; exit 1 }
$address = "http://127.0.0.1:$Port"
Write-Host "imitation of the release service: $address (requests in $log)"

Add-Type -Namespace BvUpdateTest -Name Native -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")]
public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern uint RegisterWindowMessageW(string name);
'@

# The running BetterVolume quits as from its menu (a second one would only open its panel).
function Stop-Running {
    $window = [BvUpdateTest.Native]::FindWindowW('BetterVolume.MessageWindow', $null)
    if ($window -eq [IntPtr]::Zero) { return $true }
    $message = [BvUpdateTest.Native]::RegisterWindowMessageW('BetterVolume.Exit')
    [BvUpdateTest.Native]::PostMessageW($window, $message, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    for ($i = 0; $i -lt 50; $i++) {
        Start-Sleep -Milliseconds 200
        if ([BvUpdateTest.Native]::FindWindowW('BetterVolume.MessageWindow', $null) -eq [IntPtr]::Zero) {
            return $true
        }
    }
    return $false
}

try {
    if ($Mode -ne 'serve') {
        if (-not (Stop-Running)) { Write-Host 'the running BetterVolume did not quit'; exit 1 }
        $previousServer = $env:BETTERVOLUME_UPDATE_SERVER
        $env:BETTERVOLUME_UPDATE_SERVER = $address
        try {
            if ($Mode -eq 'portable') {
                $folder = Join-Path $work 'portable'
                if (Test-Path $folder) { Remove-Item -Recurse -Force $folder }
                New-Item -ItemType Directory -Force $folder | Out-Null
                $exe = Join-Path $folder "BetterVolume-$current-portable.exe"
                Copy-Item $currentPortable $exe
                # Past its first launch, the question about the Windows volume icon answered,
                # and the current version recorded (so the newer one says it was updated).
                $settings = "version = 1`n[interface]`nfirst_run_done = true`nvolume_icon_question_done = true`n[updates]`nlast_version = `"$current`"`n"
                [IO.File]::WriteAllText((Join-Path $folder 'settings.toml'), $settings)
            } else {
                $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\BetterVolume'
                $location = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).InstallLocation
                if (-not $location) { Write-Host 'BetterVolume is not installed'; exit 1 }
                $exe = Join-Path $location 'BetterVolume.exe'
            }
            Start-Process $exe | Out-Null
            Write-Host "started $exe"
        } finally {
            $env:BETTERVOLUME_UPDATE_SERVER = $previousServer
        }
        Write-Host 'BetterVolume checks about 2 minutes after it starts (or at once with'
        Write-Host '"Check for updates" on the About page of its settings).'
    }
    if ($Seconds -gt 0) {
        Start-Sleep -Seconds $Seconds
    } else {
        Read-Host 'Press Enter to stop the imitation' | Out-Null
    }
} finally {
    if (-not $server.HasExited) { $server.Kill() }
    Write-Host 'imitation stopped'
    # The installed copy (updated or not) inherited the address of the imitation: it is
    # started again without it.
    if ($Mode -eq 'installed' -and $exe -and (Stop-Running)) {
        Start-Process $exe | Out-Null
        Write-Host "started $exe again, without the imitation"
    }
}
exit 0
