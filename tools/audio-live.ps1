# Runs the live audio tests that CHANGE the real audio state of this machine:
# default output and input switched to another connected device and back, volume +/-1 %,
# mute on and off. Everything is restored at the end, also when a test fails.
#
#   tools\audio-live.ps1
#
# Only run it after a change of the audio engine, telling whoever uses the machine (sound may
# move to another device or be cut for about a second). The ordinary gates
# (tools\check.ps1) never change the audio state.
#
# Exit code 0 only if every test passed. Keep this file ASCII-only (Windows PowerShell 5.1
# reads BOM-less scripts as ANSI).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host 'Live audio tests: the audio state of this machine changes briefly and is then restored.'
$global:LASTEXITCODE = 0
cargo test -p bv-audio --test live --locked -- --ignored --test-threads=1 --nocapture
if ($LASTEXITCODE -ne 0) {
    Write-Host ''
    Write-Host 'AUDIO LIVE TESTS FAILED'
    exit 1
}
Write-Host ''
Write-Host 'AUDIO LIVE TESTS PASSED'
exit 0
