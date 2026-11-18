# Smoke-tests the release executable and measures its footprint against the budget
# (docs/ARCHITECTURE.md section 2, mirrored in tools/budget.json).
#
#   tools\measure.ps1 [-Exe <path>] [-Broker <path>] [-IdleSeconds 60]
#
# What it does: starts the app, checks that it is up, lets it idle, measures idle CPU and
# private memory, checks that a second launch exits and leaves the first one running
# (FR-001), asks the app to quit and checks that it exits cleanly.
# Exit code 0 = everything within budget; 1 = a check failed or a budget is exceeded.
#
# The budget is defined for a 60 s idle period. A shorter -IdleSeconds is accepted for quick
# iterations but the result is then explicitly reported as NOT CONFORMING and must never be
# recorded as the measurement of a change.
#
# It also opens the flyout six times (as a second launch would ask) to
# measure its open latency. The "flyout used" state is measured in a second
# run with Home Assistant on: the application connects to the test broker on the loopback
# address (-Broker, built by tools\check.ps1), must publish its entities, uses the flyout
# once, idles, and must tell the broker it is offline when it quits.
#
# The application runs with data folders of its own (variable
# BETTERVOLUME_DATA_DIR, removed at the end): the user's settings are never read or
# changed, and every run starts from the same state. A short first run, with an empty
# folder, checks that the very first launch opens the settings window and remembers it
# (FR-071); the measured run starts from settings where that first launch is done.
#
# Updates are turned off for these runs (BETTERVOLUME_UPDATE_SERVER=off):
# a release build would otherwise ask the release service once the first check is due,
# and the figures would depend on the network. tools\update-test.ps1 exercises updates.
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).
param(
    [string]$Exe,
    [string]$Broker,
    [int]$IdleSeconds = 60
)

$ErrorActionPreference = 'Stop'
$conformingIdleSeconds = 60
if ($IdleSeconds -lt 1) {
    Write-Host 'FAIL: -IdleSeconds must be at least 1'
    exit 1
}
$root = Split-Path -Parent $PSScriptRoot
if (-not $Exe) { $Exe = Join-Path $root 'target\release\BetterVolume.exe' }
if (-not $Broker) { $Broker = Join-Path $root 'target\release\examples\test-broker.exe' }
$windowClass = 'BetterVolume.MessageWindow'
$WM_CLOSE = 0x0010
$WM_KEYDOWN = 0x0100
$VK_ESCAPE = 0x1B
$flyoutClass = 'BetterVolume.Flyout'
$settingsClass = 'BetterVolume.Settings'

if (-not (Test-Path $Exe)) {
    Write-Host "FAIL: $Exe not found. Build it first: cargo build --release"
    exit 1
}
$Exe = (Resolve-Path $Exe).Path
$budget = Get-Content (Join-Path $PSScriptRoot 'budget.json') -Raw | ConvertFrom-Json

Add-Type -Namespace BvMeasure -Name Native -MemberDefinition @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")]
public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
[DllImport("user32.dll")]
public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[StructLayout(LayoutKind.Sequential)]
public struct NOTIFYICONIDENTIFIER { public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem; }
[StructLayout(LayoutKind.Sequential)]
public struct RECT { public int Left, Top, Right, Bottom; }
[DllImport("shell32.dll")]
public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT rect);
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern uint RegisterWindowMessageW(string name);
[DllImport("user32.dll")]
public static extern bool IsWindowVisible(IntPtr h);
[DllImport("user32.dll")]
public static extern bool GetWindowRect(IntPtr h, out RECT rect);
[DllImport("user32.dll")]
public static extern bool IsWindow(IntPtr h);
'@

# True when the taskbar knows the tray icon (identifier 1) of the given message window.
function Test-TrayIcon([IntPtr]$window) {
    $id = New-Object BvMeasure.Native+NOTIFYICONIDENTIFIER
    $id.cbSize = [Runtime.InteropServices.Marshal]::SizeOf([type][BvMeasure.Native+NOTIFYICONIDENTIFIER])
    $id.hWnd = $window
    $id.uID = 1
    $rect = New-Object BvMeasure.Native+RECT
    [BvMeasure.Native]::Shell_NotifyIconGetRect([ref]$id, [ref]$rect) -ge 0
}

function Find-AppWindow {
    [BvMeasure.Native]::FindWindowW($windowClass, $null)
}

function Get-PrivateWorkingSetMb([int]$processId) {
    $row = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter "IDProcess=$processId"
    if (-not $row) { return $null }
    [math]::Round($row.WorkingSetPrivate / 1MB, 2)
}

$rows = New-Object System.Collections.Generic.List[object]
$failed = $false
function Add-Row([string]$metric, $measured, $budgetValue, $targetValue, [bool]$ok) {
    if (-not $ok) { $script:failed = $true }
    $rows.Add([pscustomobject]@{
        Metric   = $metric
        Measured = $measured
        Budget   = $budgetValue
        Target   = $targetValue
        Verdict  = $(if ($ok) { 'ok' } else { 'FAIL' })
    })
}
function Add-Skipped([string]$metric, [string]$why, $budgetValue = '', $targetValue = '') {
    $rows.Add([pscustomobject]@{ Metric = $metric; Measured = "n/a ($why)"; Budget = $budgetValue; Target = $targetValue; Verdict = 'skipped' })
}

# The app is single-instance: an instance already running would make our launch exit at once.
if ((Find-AppWindow) -ne [IntPtr]::Zero) {
    Write-Host 'FAIL: BetterVolume is already running. Quit it (tray icon > Quit) and run again.'
    exit 1
}

# --- Data folders --------------------------------------------------------------------
$previousData = $env:BETTERVOLUME_DATA_DIR
$previousUpdates = $env:BETTERVOLUME_UPDATE_SERVER
$env:BETTERVOLUME_UPDATE_SERVER = 'off'
$dataRoot = Join-Path ([IO.Path]::GetTempPath()) "bv-measure-$PID"
$firstRunData = Join-Path $dataRoot 'first-run'
$runData = Join-Path $dataRoot 'run'
if (Test-Path $dataRoot) { Remove-Item -Recurse -Force $dataRoot }
New-Item -ItemType Directory -Force $firstRunData, $runData | Out-Null
# An installation past its first launch that answered the question about the Windows volume
# icon: otherwise the settings window would open at start and the idle figures
# would include it.
$answered = "version = 1`n[interface]`nfirst_run_done = true`nvolume_icon_question_done = true`n"
[IO.File]::WriteAllText((Join-Path $runData 'settings.toml'), $answered)

# --- First launch (FR-071) -------------------------------------------------------------
$env:BETTERVOLUME_DATA_DIR = $firstRunData
$first = Start-Process $Exe -PassThru
$firstOk = $false
$settingsMs = $null
try {
    $settingsWindow = [IntPtr]::Zero
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 5000 -and -not $first.HasExited) {
        # [NullString]: a $null string argument would reach Windows as an empty title.
        $settingsWindow = [BvMeasure.Native]::FindWindowW($settingsClass, [NullString]::Value)
        if ($settingsWindow -ne [IntPtr]::Zero -and [BvMeasure.Native]::IsWindowVisible($settingsWindow)) {
            $settingsMs = $watch.ElapsedMilliseconds
            break
        }
        Start-Sleep -Milliseconds 20
    }
    if ($null -ne $settingsMs) {
        Start-Sleep -Milliseconds 500
        # Closing the window destroys it (nothing is kept while it is closed).
        [void][BvMeasure.Native]::PostMessageW($settingsWindow, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        $closeWatch = [Diagnostics.Stopwatch]::StartNew()
        while ([BvMeasure.Native]::IsWindow($settingsWindow) -and $closeWatch.ElapsedMilliseconds -lt 3000) { Start-Sleep -Milliseconds 20 }
        $closed = -not [BvMeasure.Native]::IsWindow($settingsWindow)
        [void][BvMeasure.Native]::PostMessageW((Find-AppWindow), $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        $firstExited = $first.WaitForExit(3000)
        $saved = Join-Path $firstRunData 'settings.toml'
        # Closing the window answers the question about the Windows volume icon with
        # "Keep both": both are remembered, and nothing is hidden.
        $text = if (Test-Path $saved) { Get-Content $saved -Raw } else { '' }
        $remembered = ($text -match 'first_run_done = true') -and ($text -match 'volume_icon_question_done = true') -and ($text -match 'hide_windows_volume_icon = false')
        $firstOk = $closed -and $firstExited -and ($first.ExitCode -eq 0) -and $remembered
    }
}
finally {
    if (-not $first.HasExited) { $first.Kill() }
}
Add-Row 'First launch opens the settings window and its question, closes it, remembers it' $(if ($firstOk) { "yes ($settingsMs ms to shown)" } else { 'no' }) '' '' $firstOk

$env:BETTERVOLUME_DATA_DIR = $runData
$process = Start-Process $Exe -PassThru
try {
    # --- Starts ---------------------------------------------------------------------
    $window = [IntPtr]::Zero
    $startWatch = [Diagnostics.Stopwatch]::StartNew()
    while ($startWatch.ElapsedMilliseconds -lt 5000) {
        $window = Find-AppWindow
        if ($window -ne [IntPtr]::Zero -or $process.HasExited) { break }
        Start-Sleep -Milliseconds 50
    }
    $windowPid = 0
    if ($window -ne [IntPtr]::Zero) { [void][BvMeasure.Native]::GetWindowThreadProcessId($window, [ref]$windowPid) }
    $started = ($window -ne [IntPtr]::Zero) -and ($windowPid -eq $process.Id) -and (-not $process.HasExited)
    Add-Row 'Starts (message window up)' $(if ($started) { "$($startWatch.ElapsedMilliseconds) ms" } else { 'no' }) '' '' $started
    if (-not $started) { throw 'the application did not start' }

    # --- Tray icon ------------------------------------------------------------------
    $iconWatch = [Diagnostics.Stopwatch]::StartNew()
    $hasIcon = $false
    while ($iconWatch.ElapsedMilliseconds -lt 5000) {
        $hasIcon = Test-TrayIcon $window
        if ($hasIcon) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($hasIcon) {
        Add-Row 'Tray icon registered with the taskbar' 'yes' '' '' $true
    } elseif ($env:CI) {
        # Hosted CI machines may have no interactive taskbar to hold an icon.
        Add-Skipped 'Tray icon registered with the taskbar' 'no taskbar on this CI machine'
    } else {
        Add-Row 'Tray icon registered with the taskbar' 'no' '' '' $false
    }

    # --- Idle -----------------------------------------------------------------------
    Write-Host "Idling for $IdleSeconds s..."
    Start-Sleep -Seconds 2          # let start-up work settle before the CPU window
    $process.Refresh()
    $cpuBefore = $process.TotalProcessorTime
    Start-Sleep -Seconds $IdleSeconds
    $process.Refresh()
    if ($process.HasExited) { throw 'the application exited while idle' }
    $cpuMs = ($process.TotalProcessorTime - $cpuBefore).TotalMilliseconds
    $cpuPerMin = [math]::Round($cpuMs * 60 / $IdleSeconds, 1)
    Add-Row 'Idle CPU (ms per minute)' $cpuPerMin $budget.idle_cpu_ms_per_min.budget '' ($cpuMs -lt $budget.idle_cpu_tolerance_ms)

    $privateMb = Get-PrivateWorkingSetMb $process.Id
    $ram = $budget.idle_never_opened_private_mb
    Add-Row 'Idle private RAM, flyout never opened (MB)' $privateMb $ram.budget $ram.target (($null -ne $privateMb) -and ($privateMb -le $ram.budget))
    Add-Row 'Threads (informative)' $process.Threads.Count '' '' $true
    Add-Row 'Handles (informative)' $process.HandleCount '' '' $true

    # --- Flyout (FR-021, FR-022) ------------------------------------------------------
    # Open latency = from the request (the message a second launch sends) to the flyout
    # window being shown, i.e. the first frame of its entrance. Measured from outside the
    # process, so it includes the message delivery. The first opening creates the graphics
    # device; the next ones reuse it.
    $activate = [BvMeasure.Native]::RegisterWindowMessageW('BetterVolume.Activate')
    $openings = New-Object System.Collections.Generic.List[double]
    $firstPixels = New-Object System.Collections.Generic.List[double]
    $flyoutOk = $true
    for ($i = 0; $i -lt 6 -and $flyoutOk; $i++) {
        $watch = [Diagnostics.Stopwatch]::StartNew()
        [void][BvMeasure.Native]::PostMessageW($window, $activate, [IntPtr]::Zero, [IntPtr]::Zero)
        $flyout = [IntPtr]::Zero
        $shown = $false
        while ($watch.ElapsedMilliseconds -lt 3000) {
            $flyout = [BvMeasure.Native]::FindWindowW($flyoutClass, $null)
            if ($flyout -ne [IntPtr]::Zero -and [BvMeasure.Native]::IsWindowVisible($flyout)) { $shown = $true; break }
            # Yield the processor between checks without sleeping (a sleep would round the
            # measurement up to the 15.6 ms scheduler tick).
            [void][Threading.Thread]::Yield()
        }
        $elapsed = $watch.Elapsed.TotalMilliseconds
        if (-not $shown) { $flyoutOk = $false; break }
        $openings.Add($elapsed)
        # Informative: until the window holds visible pixels (it is shown empty, past the
        # monitor edge, then rises one position per compositor frame).
        $rect = New-Object BvMeasure.Native+RECT
        while ($watch.ElapsedMilliseconds -lt 3000) {
            if ([BvMeasure.Native]::GetWindowRect($flyout, [ref]$rect) -and $rect.Bottom -gt $rect.Top -and $rect.Right -gt $rect.Left) { break }
            [void][Threading.Thread]::Yield()
        }
        $firstPixels.Add($watch.Elapsed.TotalMilliseconds)
        # Let the entrance finish, then close with Escape and wait until it is hidden.
        Start-Sleep -Milliseconds 400
        [void][BvMeasure.Native]::PostMessageW($flyout, $WM_KEYDOWN, [IntPtr]$VK_ESCAPE, [IntPtr]::Zero)
        $closeWatch = [Diagnostics.Stopwatch]::StartNew()
        while ([BvMeasure.Native]::IsWindowVisible($flyout) -and $closeWatch.ElapsedMilliseconds -lt 3000) { Start-Sleep -Milliseconds 10 }
        if ([BvMeasure.Native]::IsWindowVisible($flyout)) { $flyoutOk = $false }
        Start-Sleep -Milliseconds 200
    }
    if ($flyoutOk -and $openings.Count -eq 6) {
        $first = [math]::Round($openings[0], 1)
        # The worst of the next openings counts.
        $next = [math]::Round(($openings | Select-Object -Skip 1 | Measure-Object -Maximum).Maximum, 1)
        $latencyOk = ($first -le $budget.open_first_ms.budget) -and ($next -le $budget.open_next_ms.budget)
        # Hosted CI machines are shared virtual machines without a graphics card: their
        # timings say nothing about a user's PC, so there the latency is only reported.
        # The budget is enforced on every local run.
        $latencyName = 'Flyout open latency, first / next worst of 5 (ms)'
        if ($env:CI -and -not $latencyOk) { $latencyName += ' (informative on CI)'; $latencyOk = $true }
        Add-Row $latencyName "$first / $next" "$($budget.open_first_ms.budget) / $($budget.open_next_ms.budget)" '' $latencyOk
        $pixelsFirst = [math]::Round($firstPixels[0], 1)
        $pixelsNext = [math]::Round(($firstPixels | Select-Object -Skip 1 | Measure-Object -Maximum).Maximum, 1)
        Add-Row 'Flyout first visible pixels, first / next worst (ms, informative)' "$pixelsFirst / $pixelsNext" '' '' $true
    } else {
        Add-Row 'Flyout opens and closes (activation, Escape)' 'no' '' '' $false
    }

    # --- Single instance (FR-001) -----------------------------------------------------
    # Measured after the flyout openings: the second launch opens the flyout of the first
    # instance, which must not count as the "first opening" above.
    $second = Start-Process $Exe -PassThru
    $secondExited = $second.WaitForExit(5000)
    if (-not $secondExited) { $second.Kill() }
    $process.Refresh()
    $single = $secondExited -and ($second.ExitCode -eq 0) -and (-not $process.HasExited)
    # The flyout it opened: close it before idling.
    $opened = $false
    $openWatch = [Diagnostics.Stopwatch]::StartNew()
    while ($openWatch.ElapsedMilliseconds -lt 3000) {
        $flyout = [BvMeasure.Native]::FindWindowW($flyoutClass, $null)
        if ($flyout -ne [IntPtr]::Zero -and [BvMeasure.Native]::IsWindowVisible($flyout)) { $opened = $true; break }
        Start-Sleep -Milliseconds 20
    }
    if ($opened) {
        Start-Sleep -Milliseconds 400
        [void][BvMeasure.Native]::PostMessageW($flyout, $WM_KEYDOWN, [IntPtr]$VK_ESCAPE, [IntPtr]::Zero)
        $closeWatch = [Diagnostics.Stopwatch]::StartNew()
        while ([BvMeasure.Native]::IsWindowVisible($flyout) -and $closeWatch.ElapsedMilliseconds -lt 3000) { Start-Sleep -Milliseconds 10 }
        $opened = -not [BvMeasure.Native]::IsWindowVisible($flyout)
    }
    $single = $single -and $opened
    Add-Row 'Second launch exits, first keeps running and opens its flyout' $(if ($single) { 'yes' } else { 'no' }) '' '' $single

    # --- Quits cleanly ----------------------------------------------------------------
    [void][BvMeasure.Native]::PostMessageW($window, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    $exited = $process.WaitForExit(3000)
    $clean = $exited -and ($process.ExitCode -eq 0)
    Add-Row 'Quits cleanly on request' $(if ($clean) { 'yes' } elseif ($exited) { "exit code $($process.ExitCode)" } else { 'no (killed)' }) '' '' $clean
}
catch {
    Write-Host "FAIL: $($_.Exception.Message)"
    $failed = $true
}
finally {
    if (-not $process.HasExited) { $process.Kill() }
}

# --- Home Assistant on (FR-118, NFR-01: "flyout used, Home Assistant on") -------------
# A second run connects to the test broker on the loopback address (no other network):
# the link must connect and publish its entities, the flyout is used once, then the
# application idles; on quit it must tell the broker it is offline.
$brokerOut = Join-Path $dataRoot 'broker.txt'
$brokerProcess = $null
$haProcess = $null
function Get-BrokerState {
    if (-not (Test-Path $brokerOut)) { return $null }
    $lines = @(Get-Content $brokerOut -ErrorAction SilentlyContinue)
    if ($lines.Count -eq 0) { return $null }
    $lines[-1]
}
try {
    if (-not (Test-Path $Broker)) { throw "$Broker not found (built by tools\check.ps1)" }
    $brokerProcess = Start-Process $Broker -ArgumentList '--port', '0' -PassThru -WindowStyle Hidden -RedirectStandardOutput $brokerOut
    $port = $null
    $portWatch = [Diagnostics.Stopwatch]::StartNew()
    while ($portWatch.ElapsedMilliseconds -lt 5000 -and $null -eq $port) {
        if (Test-Path $brokerOut) {
            $match = Select-String -Path $brokerOut -Pattern '^port (\d+)$' -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($match) { $port = [int]$match.Matches[0].Groups[1].Value }
        }
        Start-Sleep -Milliseconds 50
    }
    if ($null -eq $port) { throw 'the test broker did not start' }
    $haData = Join-Path $dataRoot 'ha'
    New-Item -ItemType Directory -Force $haData | Out-Null
    $haSettings = "version = 1`n[interface]`nfirst_run_done = true`nvolume_icon_question_done = true`n[home_assistant]`nenabled = true`nhost = `"127.0.0.1`"`nport = $port`nnode_id = `"measure`"`n"
    [IO.File]::WriteAllText((Join-Path $haData 'settings.toml'), $haSettings)
    $env:BETTERVOLUME_DATA_DIR = $haData
    $haProcess = Start-Process $Exe -PassThru
    # Connected and published: availability, 9 configurations, 6 states. A machine
    # without audio devices (hosted CI) has no device entities (their configurations are
    # empty, which keeps nothing in the broker): availability and the three media buttons.
    $expected = 16
    if ($env:CI) { $expected = 4 }
    $published = $false
    $haWatch = [Diagnostics.Stopwatch]::StartNew()
    while ($haWatch.ElapsedMilliseconds -lt 10000) {
        $state = Get-BrokerState
        if ($state -match '^connected=1 retained=(\d+) availability=online$' -and [int]$Matches[1] -ge $expected) { $published = $true; break }
        Start-Sleep -Milliseconds 50
    }
    if (-not $published) { Write-Host "test broker, last state: $(Get-BrokerState)" }
    Add-Row 'Home Assistant link connects and publishes its entities' $(if ($published) { "yes ($($haWatch.ElapsedMilliseconds) ms)" } else { 'no' }) '' '' $published
    if (-not $published) { throw 'the Home Assistant link did not publish' }

    # Use the flyout once (as a second launch would ask), then close it.
    $haWindow = Find-AppWindow
    $activate = [BvMeasure.Native]::RegisterWindowMessageW('BetterVolume.Activate')
    [void][BvMeasure.Native]::PostMessageW($haWindow, $activate, [IntPtr]::Zero, [IntPtr]::Zero)
    $flyout = [IntPtr]::Zero
    $openWatch = [Diagnostics.Stopwatch]::StartNew()
    while ($openWatch.ElapsedMilliseconds -lt 3000) {
        $flyout = [BvMeasure.Native]::FindWindowW($flyoutClass, $null)
        if ($flyout -ne [IntPtr]::Zero -and [BvMeasure.Native]::IsWindowVisible($flyout)) { break }
        Start-Sleep -Milliseconds 20
    }
    Start-Sleep -Milliseconds 400
    [void][BvMeasure.Native]::PostMessageW($flyout, $WM_KEYDOWN, [IntPtr]$VK_ESCAPE, [IntPtr]::Zero)
    $closeWatch = [Diagnostics.Stopwatch]::StartNew()
    while ([BvMeasure.Native]::IsWindowVisible($flyout) -and $closeWatch.ElapsedMilliseconds -lt 3000) { Start-Sleep -Milliseconds 10 }
    $used = -not [BvMeasure.Native]::IsWindowVisible($flyout)
    if (-not $used) { throw 'the flyout did not open and close with Home Assistant on' }

    Write-Host "Idling for $IdleSeconds s after using the flyout, Home Assistant on..."
    Start-Sleep -Seconds 2
    $haProcess.Refresh()
    $cpuBefore = $haProcess.TotalProcessorTime
    Start-Sleep -Seconds $IdleSeconds
    $haProcess.Refresh()
    if ($haProcess.HasExited) { throw 'the application exited while idle with Home Assistant on' }
    $cpuMs = ($haProcess.TotalProcessorTime - $cpuBefore).TotalMilliseconds
    $cpuPerMin = [math]::Round($cpuMs * 60 / $IdleSeconds, 1)
    Add-Row 'Idle CPU, flyout used, Home Assistant on (ms per minute)' $cpuPerMin $budget.idle_cpu_ms_per_min.budget '' ($cpuMs -lt $budget.idle_cpu_tolerance_ms)
    $usedMb = Get-PrivateWorkingSetMb $haProcess.Id
    $usedBudget = $budget.idle_used_ha_on_private_mb
    Add-Row 'Idle private RAM, flyout used, Home Assistant on (MB)' $usedMb $usedBudget.budget $usedBudget.target (($null -ne $usedMb) -and ($usedMb -le $usedBudget.budget))
    Add-Row 'Threads, Home Assistant on (informative)' $haProcess.Threads.Count '' '' $true
    $stillConnected = (Get-BrokerState) -match '^connected=1 '
    Add-Row 'Home Assistant link still connected after the idle period' $(if ($stillConnected) { 'yes' } else { 'no' }) '' '' $stillConnected

    [void][BvMeasure.Native]::PostMessageW($haWindow, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    $haExited = $haProcess.WaitForExit(5000)
    $offlineWatch = [Diagnostics.Stopwatch]::StartNew()
    $offline = $false
    while ($offlineWatch.ElapsedMilliseconds -lt 3000) {
        if ((Get-BrokerState) -match 'availability=offline$') { $offline = $true; break }
        Start-Sleep -Milliseconds 50
    }
    $haClean = $haExited -and ($haProcess.ExitCode -eq 0) -and $offline
    Add-Row 'Quits cleanly; Home Assistant sees it offline' $(if ($haClean) { 'yes' } elseif ($haExited) { "exit code $($haProcess.ExitCode), offline: $offline" } else { 'no (killed)' }) '' '' $haClean
}
catch {
    Write-Host "FAIL: $($_.Exception.Message)"
    $failed = $true
}
finally {
    if ($haProcess -and -not $haProcess.HasExited) { $haProcess.Kill() }
    if ($brokerProcess -and -not $brokerProcess.HasExited) { $brokerProcess.Kill() }
    $env:BETTERVOLUME_DATA_DIR = $previousData
    $env:BETTERVOLUME_UPDATE_SERVER = $previousUpdates
    Start-Sleep -Milliseconds 200
    Remove-Item -Recurse -Force $dataRoot -ErrorAction SilentlyContinue
}

$exeMb = [math]::Round((Get-Item $Exe).Length / 1MB, 3)
Add-Row 'Executable size (MB)' $exeMb $budget.exe_mb.budget $budget.exe_mb.target ($exeMb -le $budget.exe_mb.budget)

$rows | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
if ($failed) {
    Write-Host 'MEASURE: FAILED'
    exit 1
}
if ($IdleSeconds -lt $conformingIdleSeconds) {
    Write-Host "MEASURE: no failure, but NOT CONFORMING (idle period $IdleSeconds s instead of $conformingIdleSeconds s). Do not record."
    exit 0
}
Write-Host 'MEASURE: within budget'
exit 0
