# Runs the @labview acceptance scenarios against the LabVIEW in a Windows image.
# Usage: labview-acceptance.ps1 -Image <image>   (on the runner; starts the container)
#        labview-acceptance.ps1 -InContainer   (inside it; run by the line above)
# The tests are built at C:\src\lvpm and mounted at the same path, because
# CARGO_MANIFEST_DIR and CARGO_BIN_EXE_lvpm are fixed at compile time.
param([string] $Image, [switch] $InContainer)
$ErrorActionPreference = 'Stop'
# Measured on a LabVIEW 2026 machine with LVPM_TEST_LABVIEW=1 and
# LVPM_TEST_ONLY_LABVIEW=1 (2026-10-26): 29 run, 4 @unsafe never run.
$expected = 29
$src = 'C:\src\lvpm'

if (-not $InContainer) {
    # The step summary goes to out\, which the workflow copies out afterwards.
    New-Item -ItemType Directory -Force out | Out-Null
    docker run --rm --isolation=process --entrypoint powershell -e GITHUB_STEP_SUMMARY=C:\out\summary.md `
        -v "${src}:${src}" -v "${PSScriptRoot}:C:\e2e" -v "${PWD}\out:C:\out" $Image `
        -NoProfile -ExecutionPolicy Bypass -File C:\e2e\labview-acceptance.ps1 -InContainer
    exit $LASTEXITCODE
}

function Fail([string] $message, [string] $log = '') {
    Write-Host "::error::$message"
    if ($env:GITHUB_STEP_SUMMARY) { Add-Content $env:GITHUB_STEP_SUMMARY "$message`n`n``````n$log`n``````" }
    exit 1
}

# The scenario that installs into the LabVIEW installation never starts a
# LabVIEW itself; in this throwaway container it is safe to start it here.
$labview = 'C:\Program Files\National Instruments\LabVIEW 2026'
$ini = Join-Path $labview 'LabVIEW.ini'
$port = 3363
if ((Test-Path $ini) -and ((Get-Content -Raw $ini) -match '(?m)^server\.tcp\.port\s*=\s*"?(\d+)')) { $port = [int]$Matches[1] }
Start-Process (Join-Path $labview 'LabVIEW.exe')
$deadline = (Get-Date).AddMinutes(5)
while (-not (Test-NetConnection 127.0.0.1 -Port $port -InformationLevel Quiet -WarningAction SilentlyContinue)) {
    if ((Get-Date) -gt $deadline) { Fail "infrastructure: LabVIEW did not answer VI Server on port $port within 5 minutes" }
    Start-Sleep 5
}

$binary = Get-ChildItem "$src\target\debug\deps\acceptance-*.exe" | Select-Object -First 1
if (-not $binary) { Fail "no acceptance test binary under $src\target\debug\deps" }
$env:LVPM_TEST_LABVIEW = '1'; $env:LVPM_TEST_ONLY_LABVIEW = '1'
$ErrorActionPreference = 'Continue'
$log = (& $binary.FullName 2>&1 | ForEach-Object { "$_" } | Out-String)
$status = $LASTEXITCODE
Write-Host $log

if ($status -ne 0) { Fail "the @labview scenarios failed (exit $status)" $log }
if ($log -match '@labview scenarios not run') { Fail 'some @labview scenarios were not run' $log }
if ($log -notmatch "(?m)^$expected @labview scenarios run") { Fail "expected $expected @labview scenarios to run" $log }
if ($log -notmatch '@unsafe scenarios never run') { Fail 'expected the @unsafe scenarios to be reported as never run' $log }
Write-Host "$expected @labview scenarios passed against LabVIEW 2026"
