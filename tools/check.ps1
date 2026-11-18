# Runs every quality gate a change must pass, with one command.
#
#   tools\check.ps1 [-SkipMeasure] [-IdleSeconds 60]
#
# Gates, in order: formatting, lints (warnings are errors), unsafe-code policy, tests,
# release build (with build-machine paths remapped), no local path in the executable,
# release files (setup and portable, read back), smoke test + footprint measurement
# against the budget. Exit code 0 only if every gate passes. The CI workflow runs this
# same script.
#
# Only a run whose last line is exactly "CHECK PASSED" validates a change. -SkipMeasure and an
# idle period under 60 s exist for quick iterations and say so in their last line.
#
# Keep this file ASCII-only (Windows PowerShell 5.1 reads BOM-less scripts as ANSI).
param(
    [switch]$SkipMeasure,
    [int]$IdleSeconds = 60
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Invoke-Gate([string]$name, [scriptblock]$body) {
    Write-Host ''
    Write-Host "=== $name ==="
    $global:LASTEXITCODE = 0
    & $body
    if ($LASTEXITCODE -ne 0) {
        Write-Host ''
        Write-Host "CHECK FAILED at: $name"
        exit 1
    }
}

# docs/ARCHITECTURE.md section 8: no unsafe code outside bv-audio, bv-shell and
# the rendering backend of bv-ui-kit. The compiler enforces it through crate attributes;
# this gate makes sure the attributes stay, that a new crate cannot be forgotten, and that
# nobody re-allows unsafe code locally.
function Test-UnsafePolicy {
    # Crates that may use unsafe code anywhere.
    $unsafeAllowed = @('bv-audio', 'bv-shell', 'bv-taskbar-helper')
    # Crates that deny it but may re-allow it inside one folder only.
    $denyOnly = @{ 'bv-ui-kit' = 'crates\bv-ui-kit\src\backend' }
    $bad = 0
    foreach ($crate in Get-ChildItem (Join-Path $root 'crates') -Directory) {
        $name = $crate.Name
        if ($unsafeAllowed -contains $name) { continue }
        $rootFile = Join-Path $crate.FullName 'src\lib.rs'
        if (-not (Test-Path $rootFile)) { $rootFile = Join-Path $crate.FullName 'src\main.rs' }
        if (-not (Test-Path $rootFile)) {
            Write-Host "crate ${name}: no src\lib.rs or src\main.rs"
            $bad = 1
            continue
        }
        $attribute = '#![forbid(unsafe_code)]'
        if ($denyOnly.ContainsKey($name)) { $attribute = '#![deny(unsafe_code)]' }
        $manifest = (Get-Content (Join-Path $crate.FullName 'Cargo.toml') -Raw) -replace '\s', ''
        if (-not $manifest.Contains('[lints]workspace=true')) {
            Write-Host "crate ${name}: Cargo.toml must keep [lints] workspace = true"
            $bad = 1
        }
        if (-not (Get-Content $rootFile | Where-Object { $_.Trim() -eq $attribute })) {
            Write-Host "crate ${name}: missing $attribute in its root file (a crate that needs unsafe code must be added to the allowed list here and in docs/ARCHITECTURE.md section 8)"
            $bad = 1
        }
        $allowedFolder = $null
        if ($denyOnly.ContainsKey($name)) { $allowedFolder = (Join-Path $root $denyOnly[$name]) + '\' }
        foreach ($source in Get-ChildItem $crate.FullName -Recurse -Filter *.rs) {
            if ($allowedFolder -and $source.FullName.StartsWith($allowedFolder, [StringComparison]::OrdinalIgnoreCase)) { continue }
            foreach ($hit in @(Select-String -Path $source.FullName -Pattern '(allow|expect)\s*\([^)]*unsafe_code')) {
                Write-Host "crate ${name}: unsafe code re-allowed at $($source.FullName):$($hit.LineNumber)"
                $bad = 1
            }
            # Examples, tests and build scripts are crate roots of their own: the attribute
            # of src\ does not reach them, so they may not use unsafe code at all.
            $srcFolder = (Join-Path $crate.FullName 'src') + '\'
            if (-not $source.FullName.StartsWith($srcFolder, [StringComparison]::OrdinalIgnoreCase)) {
                foreach ($hit in @(Select-String -Path $source.FullName -Pattern '\bunsafe\b(?!_code)')) {
                    Write-Host "crate ${name}: unsafe code outside src at $($source.FullName):$($hit.LineNumber)"
                    $bad = 1
                }
            }
        }
    }
    if ($bad -eq 0) { Write-Host 'unsafe-code policy respected' }
    $global:LASTEXITCODE = $bad
}

# Privacy: the release executable must not carry paths of the build machine.
# Rust writes the source path of dependencies (under the Cargo home) and of the
# toolchain into the binary (panic locations). The release build remaps every path under
# the user profile to "~"; Cargo has no stable setting for this yet, so it is passed
# here, through the encoded flags variable (safe with spaces in paths). Any release build
# meant to leave this machine must be made this way (the release workflow runs this script).
function Invoke-ReleaseBuild {
    $profileDir = [Environment]::GetFolderPath('UserProfile')
    $separator = [string][char]0x1F
    $flags = @("--remap-path-prefix=$profileDir=~")
    if ($env:CARGO_HOME) { $flags += "--remap-path-prefix=$($env:CARGO_HOME)=cargo-home" }
    $previous = $env:CARGO_ENCODED_RUSTFLAGS
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join $separator
    try {
        cargo build --release --locked
        if ($LASTEXITCODE -eq 0) {
            # The test broker of the footprint measurement (never shipped), and the tool that
            # makes the release files (tools\package.ps1).
            cargo build --release --locked -p bv-mqtt --example test-broker --features test-broker
            if ($LASTEXITCODE -eq 0) { cargo build --release --locked -p bv-core --example package }
            # The local imitation of the release service (tools\update-test.ps1, never shipped).
            if ($LASTEXITCODE -eq 0) { cargo build --release --locked -p bv-app --example release-server }
        }
    } finally {
        $env:CARGO_ENCODED_RUSTFLAGS = $previous
    }
}

# The shipped binaries (the executable and the taskbar helper library, which the release
# files carry) hold no path of the user profile (UTF-8 or UTF-16).
function Test-NoLocalPaths {
    $profileDir = [Environment]::GetFolderPath('UserProfile')
    $bad = 0
    foreach ($name in 'BetterVolume.exe', 'bv_taskbar_helper.dll') {
        $bytes = [IO.File]::ReadAllBytes((Join-Path $root "target\release\$name"))
        foreach ($encoding in [Text.Encoding]::UTF8, [Text.Encoding]::Unicode) {
            $text = $encoding.GetString($bytes)
            foreach ($needle in @($profileDir, $profileDir.Replace([string][char]0x5C, '/'))) {
                if ($text.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    Write-Host "$name contains a path of the user profile ($($encoding.EncodingName))"
                    $bad = 1
                }
            }
        }
    }
    if ($bad -eq 0) { Write-Host 'the shipped binaries hold no path of the user profile' }
    $global:LASTEXITCODE = $bad
}

Invoke-Gate 'Formatting' { cargo fmt --all -- --check }
Invoke-Gate 'Lints (warnings are errors)' { cargo clippy --workspace --all-targets --locked -- -D warnings }
Invoke-Gate 'Unsafe-code policy' { Test-UnsafePolicy }
Invoke-Gate 'Tests' { cargo test --workspace --locked }
Invoke-Gate 'Release build' { Invoke-ReleaseBuild }
Invoke-Gate 'No local paths in the executable' { Test-NoLocalPaths }
Invoke-Gate 'Release files (setup and portable)' { & (Join-Path $PSScriptRoot 'package.ps1') }
if ($SkipMeasure) {
    Write-Host ''
    Write-Host '=== Smoke test and footprint: SKIPPED (-SkipMeasure) ==='
} else {
    Invoke-Gate 'Smoke test and footprint' { & (Join-Path $PSScriptRoot 'measure.ps1') -IdleSeconds $IdleSeconds }
}

Write-Host ''
if ($SkipMeasure) {
    Write-Host 'CHECK PASSED (footprint not measured)'
} elseif ($IdleSeconds -lt 60) {
    Write-Host "CHECK PASSED (footprint NOT CONFORMING: idle period $IdleSeconds s instead of 60 s)"
} else {
    Write-Host 'CHECK PASSED'
}
exit 0
