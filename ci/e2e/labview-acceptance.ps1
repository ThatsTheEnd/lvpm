# Runs the @labview acceptance scenarios against the LabVIEW in a Windows image.
# Usage: labview-acceptance.ps1 -Image <image> -Year <year>   (on the runner; starts the container)
#        labview-acceptance.ps1 -Year <year> -InContainer     (inside it; run by the line above)
# The tests are built at C:\src\lvpm and mounted at the same path, because
# CARGO_MANIFEST_DIR and CARGO_BIN_EXE_lvpm are fixed at compile time.
param([string] $Image, [Parameter(Mandatory)] [int] $Year, [switch] $InContainer)
$ErrorActionPreference = 'Stop'
# Measured on a LabVIEW 2026 machine with LVPM_TEST_LABVIEW=1 and
# LVPM_TEST_ONLY_LABVIEW=1 (2026-10-25): 29 run, 4 @unsafe never run.
$expected = 29
$src = 'C:\src\lvpm'

if (-not $InContainer) {
    # The step summary goes to out\, which the workflow copies out afterwards.
    New-Item -ItemType Directory -Force out | Out-Null
    docker run --rm --isolation=process --entrypoint powershell -e GITHUB_STEP_SUMMARY=C:\out\summary.md `
        -v "${src}:${src}" -v "${PSScriptRoot}:C:\e2e:ro" -v "${PWD}\out:C:\out" $Image `
        -NoProfile -ExecutionPolicy Bypass -File C:\e2e\labview-acceptance.ps1 -Year $Year -InContainer
    exit $LASTEXITCODE
}

function Fail([string] $message, [string] $log = '') {
    Write-Host "::error::$message"
    # A step summary is capped at 1 MiB: keep the tail, where the failure and the counts are.
    $tail = ($log -split "`n" | Select-Object -Last 200) -join "`n"
    if ($env:GITHUB_STEP_SUMMARY) { Add-Content $env:GITHUB_STEP_SUMMARY "$message`n`n``````n$tail`n``````" }
    exit 1
}

# The plan leaves starting LabVIEW to the scenarios. This script starts one up
# front on purpose, so a slow first start is not charged to the first scenario;
# the container is thrown away afterwards.
$labview = "C:\Program Files\National Instruments\LabVIEW $Year"
$ini = Join-Path $labview 'LabVIEW.ini'
# shortcut: the same fallback as lvpm's vi_server_port (3363), so this waits
# where lvpm connects; 2026 itself defaults to 3364, so an image whose
# LabVIEW.ini sets no port fails here. Fix both together if that happens.
$port = 3363
if ((Test-Path $ini) -and ((Get-Content -Raw $ini) -match '(?m)^server\.tcp\.port\s*=\s*"?(\d+)')) { $port = [int]$Matches[1] }
if (-not (Test-Path (Join-Path $labview 'LabVIEW.exe'))) { Fail "infrastructure: no LabVIEW.exe under $labview" }
Start-Process (Join-Path $labview 'LabVIEW.exe')
$deadline = (Get-Date).AddMinutes(5)
while (-not (Test-NetConnection 127.0.0.1 -Port $port -InformationLevel Quiet -WarningAction SilentlyContinue)) {
    # By name, not by the started process, in case LabVIEW.exe hands off and
    # exits (not measured in the image; a renamed child would fail here).
    if (-not (Get-Process LabVIEW -ErrorAction SilentlyContinue)) { Fail "infrastructure: LabVIEW exited before VI Server answered on port $port" }
    if ((Get-Date) -gt $deadline) { Fail "infrastructure: LabVIEW did not answer VI Server on port $port within 5 minutes" }
    Start-Sleep 5
}

$binary = Get-ChildItem "$src\target\debug\deps\acceptance-*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
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
Write-Host "$expected @labview scenarios passed against LabVIEW $Year"
