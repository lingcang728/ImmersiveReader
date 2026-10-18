#Requires -Version 7.0
<#
.SYNOPSIS
  Android real-device QA for ImmersiveReader (task-7).

.DESCRIPTION
  Installs a QA APK with adb and records honest on-device evidence:
    - device model / Android / SDK / screen,
    - APK SHA-256 + install result + `pm path`,
    - cold-start `am start -W` (Status/WaitTime/TotalTime, 3 runs),
    - logcat scan for FATAL EXCEPTION / AndroidRuntime / ANR,
    - `dumpsys meminfo <pkg>` TOTAL PSS (after launch and after opening a fixture),
    - `dumpsys gfxinfo <pkg> framestats` + janky-frame percentage while scrolling,
    - `adb exec-out screencap -p` screenshots,
    - best-effort VIEW intents for the synthetic fixtures pushed to
      /sdcard/Download/IR-QA/ (the script reports what actually happened; it
      never assumes the intent worked).

  When no device is connected the script prints an explicit SKIP and exits 2.
  Every number in the summary comes from real adb output; failed parses are
  reported as UNVERIFIED, never filled in.

.PARAMETER ApkPath
  APK to install. Required unless -SkipInstall is used.

.EXAMPLE
  pwsh -File .\scripts\qa\mobile\android-device-check.ps1 -ApkPath .\release\ImmersiveReader_1.3.0_android-universal-release.apk

.EXAMPLE
  # device already has the QA build installed
  pwsh -File .\scripts\qa\mobile\android-device-check.ps1 -SkipInstall

.NOTES
  Exit codes: 0 = measurements captured, 1 = install/launch failed,
  2 = SKIP (no device / unusable environment), 3 = script error.
#>
[CmdletBinding()]
param(
    [string]$ApkPath = "",
    [string]$PackageName = "com.lingcang.immersivereading",
    [string]$Serial = "",
    [string]$OutDir = "",
    [string]$FixturesDir = "",
    [string]$AdbPath = "",
    [int]$LaunchTimeoutSec = 30,
    [int]$ColdStartRuns = 3,
    [switch]$SkipInstall,
    [switch]$UninstallFirst,
    [switch]$SkipGfx,
    [switch]$SkipIntent
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot "docs\audit\2026-10-18\device" }
if (-not $FixturesDir) { $FixturesDir = Join-Path $RepoRoot "docs\audit\2026-10-18\fixtures" }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$Summary = [ordered]@{
    generatedAt      = (Get-Date).ToString("o")
    device           = $null
    apk              = $null
    install          = $null
    coldStart        = @()
    logcat           = $null
    memory           = @()
    gfx              = $null
    screenshots      = @()
    intent           = $null
    unverified       = @()
    skip             = $null
}

function Write-Step([string]$Message) {
    Write-Host ""
    Write-Host "== $Message" -ForegroundColor Cyan
}

function Save-Text([string]$Name, [string]$Content) {
    $path = Join-Path $OutDir $Name
    Set-Content -Path $path -Value $Content -Encoding UTF8
    return $path
}

function Resolve-Adb {
    if ($AdbPath -and (Test-Path $AdbPath)) { return (Resolve-Path $AdbPath).Path }
    $candidates = @()
    $onPath = Get-Command adb.exe -ErrorAction SilentlyContinue
    if ($onPath) { $candidates += $onPath.Source }
    $env:ANDROID_HOME | ForEach-Object {
        if ($_) { $candidates += (Join-Path $_ "platform-tools\adb.exe") }
    }
    $candidates += "G:\build_cache\android-sdk\platform-tools\adb.exe"
    $candidates += (Join-Path $env:LOCALAPPDATA "Android\Sdk\platform-tools\adb.exe")
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) { return (Resolve-Path $candidate).Path }
    }
    return $null
}

function Invoke-Adb {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [switch]$AllowFailure
    )
    $adbArgs = @()
    if ($Serial) { $adbArgs += @("-s", $Serial) }
    $adbArgs += $Arguments
    $output = & $script:Adb @adbArgs 2>&1
    $exit = $LASTEXITCODE
    $text = ($output | Out-String).Trim()
    if ($exit -ne 0 -and -not $AllowFailure) {
        throw "adb $($Arguments -join ' ') failed (exit $exit): $text"
    }
    return [pscustomobject]@{ ExitCode = $exit; Output = $text }
}

function Invoke-Screencap([string]$Name) {
    $target = Join-Path $OutDir $Name
    if (Test-Path $target) { Remove-Item $target -Force }
    # Use cmd.exe redirection: PowerShell text redirection can corrupt binary PNG.
    $adb = $script:Adb
    $serialArg = if ($Serial) { "-s $Serial " } else { "" }
    $cmd = '"' + $adb + '" ' + $serialArg + 'exec-out screencap -p > "' + $target + '"'
    & cmd.exe /c $cmd | Out-Null
    if ((Test-Path $target) -and ((Get-Item $target).Length -gt 1000)) {
        $script:Summary.screenshots += [ordered]@{
            name = $Name
            path = $target.Substring($RepoRoot.Length + 1)
            bytes = (Get-Item $target).Length
        }
        Write-Host "  screenshot: $Name ($((Get-Item $target).Length) B)"
        return $target
    }
    Write-Host "  screenshot FAILED: $Name (adb/screencap did not produce a PNG)"
    return $null
}

function Get-CurrentFocus {
    $dumpsys = Invoke-Adb -Arguments @("shell", "dumpsys", "window", "windows") -AllowFailure
    $match = [regex]::Match($dumpsys.Output, 'mCurrentFocus=[^\r\n]*')
    if ($match.Success) { return $match.Value }
    return "UNVERIFIED (mCurrentFocus not found in dumpsys window)"
}

# ---------------------------------------------------------------------------
Write-Step "Environment"
$script:Adb = Resolve-Adb
if (-not $script:Adb) {
    Write-Host "SKIP: adb not found. Install Android platform-tools or pass -AdbPath." -ForegroundColor Yellow
    Write-Host "      No device evidence was measured. This is a SKIP, not a pass."
    exit 2
}
Write-Host "adb: $script:Adb"
$adbVersion = Invoke-Adb -Arguments @("version") -AllowFailure
Write-Host ($adbVersion.Output -split "`n" | Select-Object -First 2 | Out-String).Trim()

# Wireless debugging: when -Serial is host:port and not yet attached, connect once.
if ($Serial -and $Serial -match ':\d+$') {
    $pre = Invoke-Adb -Arguments @("devices") -AllowFailure   # no -s yet: Serial not attached
    if ($pre.Output -notmatch [regex]::Escape($Serial)) {
        Write-Host "adb connect $Serial"
        Invoke-Adb -Arguments @("connect", $Serial) -AllowFailure | Out-Null
        Start-Sleep -Seconds 2
    }
}
$devicesOut = Invoke-Adb -Arguments @("devices", "-l") -AllowFailure
Write-Host $devicesOut.Output
$deviceLines = @($devicesOut.Output -split "`r?`n" | Where-Object { $_ -match '\sdevice\s' -and $_ -notmatch '^List of devices' })
if ($Serial) {
    $serialRow = @($deviceLines | Where-Object { $_ -match [regex]::Escape($Serial) })
    if ($serialRow.Count -eq 0) {
        Write-Host ""
        Write-Host "SKIP: requested device '$Serial' is not attached (adb devices shows no match)." -ForegroundColor Yellow
        Write-Host "      For wireless debugging: enable it on the phone, then pass -Serial host:port."
        $Summary.skip = "requested device not attached: $Serial"
        $Summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "device-summary.json") -Encoding UTF8
        exit 2
    }
}
if ($deviceLines.Count -eq 0) {
    Write-Host ""
    Write-Host "SKIP: no adb device connected (adb devices shows no 'device' entry)." -ForegroundColor Yellow
    Write-Host "      Connect the OPPO Find X9 over USB, enable USB debugging, accept the RSA prompt,"
    Write-Host "      then re-run this script. Nothing was measured and nothing is claimed." -ForegroundColor Yellow
    $Summary.skip = "no adb device connected"
    $Summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "device-summary.json") -Encoding UTF8
    exit 2
}
if ($deviceLines.Count -gt 1 -and -not $Serial) {
    Write-Host "ERROR: multiple devices connected; pass -Serial <serial>." -ForegroundColor Red
    Write-Host ($deviceLines -join "`n")
    exit 2
}
if (-not $Serial) {
    $Serial = ($deviceLines[0] -split '\s+')[0]
    Write-Host "Selected device: $Serial"
}

# ---------------------------------------------------------------------------
Write-Step "Device info"
$props = [ordered]@{}
foreach ($prop in @(
        "ro.product.brand", "ro.product.model", "ro.product.device",
        "ro.build.version.release", "ro.build.version.sdk", "ro.build.display.id"
    )) {
    $value = (Invoke-Adb -Arguments @("shell", "getprop", $prop) -AllowFailure).Output.Trim()
    $props[$prop] = $value
}
$wmSize = (Invoke-Adb -Arguments @("shell", "wm", "size") -AllowFailure).Output.Trim()
$wmDensity = (Invoke-Adb -Arguments @("shell", "wm", "density") -AllowFailure).Output.Trim()
$Summary.device = [ordered]@{
    serial = $Serial
    brand = $props["ro.product.brand"]
    model = $props["ro.product.model"]
    device = $props["ro.product.device"]
    androidRelease = $props["ro.build.version.release"]
    sdk = $props["ro.build.version.sdk"]
    buildId = $props["ro.build.display.id"]
    wmSize = $wmSize
    wmDensity = $wmDensity
}
Write-Host "  $($Summary.device.brand) $($Summary.device.model) / Android $($Summary.device.androidRelease) (SDK $($Summary.device.sdk))"
Write-Host "  $wmSize / $wmDensity"

# ---------------------------------------------------------------------------
if (-not $SkipInstall) {
    Write-Step "Install APK"
    if (-not $ApkPath -or -not (Test-Path $ApkPath)) {
        Write-Host "ERROR: -ApkPath not provided or file missing: '$ApkPath'" -ForegroundColor Red
        exit 3
    }
    $apk = Get-Item $ApkPath
    $apkHash = (Get-FileHash -Path $apk.FullName -Algorithm SHA256).Hash
    $Summary.apk = [ordered]@{
        path = $apk.FullName
        bytes = $apk.Length
        sha256 = $apkHash
        lastWriteTime = $apk.LastWriteTime.ToString("o")
    }
    Write-Host "  APK: $($apk.Length) B  SHA-256 $apkHash"
    if ($UninstallFirst) {
        # Release signing key changed vs 1.2.1 debug APK: install -r over a
        # differently signed package fails with INSTALL_FAILED_UPDATE_INCOMPATIBLE.
        $uninstall = Invoke-Adb -Arguments @("uninstall", $PackageName) -AllowFailure
        Save-Text "uninstall-previous.txt" $uninstall.Output | Out-Null
        Write-Host "  uninstall previous package: exit=$($uninstall.ExitCode) $($uninstall.Output)"
    }
    $install = Invoke-Adb -Arguments @("install", "-r", "-d", $apk.FullName) -AllowFailure
    Save-Text "install.txt" $install.Output | Out-Null
    $S = $Summary
    $S.install = [ordered]@{ exitCode = $install.ExitCode; output = $install.Output }
    if ($install.ExitCode -ne 0 -or $install.Output -notmatch 'Success') {
        Write-Host "FAIL: adb install did not report Success." -ForegroundColor Red
        $S.unverified += "install"
        $S | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "device-summary.json") -Encoding UTF8
        exit 1
    }
    Write-Host "  install: Success"
} else {
    Write-Step "Install APK (skipped: -SkipInstall)"
    $Summary.install = [ordered]@{ skipped = $true }
}

$pmPath = Invoke-Adb -Arguments @("shell", "pm", "path", $PackageName) -AllowFailure
if ($pmPath.Output -notmatch 'package:') {
    Write-Host "FAIL: $PackageName is not installed on the device." -ForegroundColor Red
    $Summary.unverified += "package-installed"
    $Summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "device-summary.json") -Encoding UTF8
    exit 1
}
Write-Host "  pm path: $($pmPath.Output.Split("`n")[0])"
$Summary.install["pmPath"] = $pmPath.Output

$pkgDump = Invoke-Adb -Arguments @("shell", "dumpsys", "package", $PackageName) -AllowFailure
$versionMatch = [regex]::Match($pkgDump.Output, 'versionName=([^\s]+)')
$codeMatch = [regex]::Match($pkgDump.Output, 'versionCode=(\d+)')
if ($versionMatch.Success) { $Summary.install["versionName"] = $versionMatch.Groups[1].Value }
if ($codeMatch.Success) { $Summary.install["versionCode"] = $codeMatch.Groups[1].Value }
Write-Host "  versionName=$($Summary.install['versionName']) versionCode=$($Summary.install['versionCode'])"

# Resolve launcher activity for a deterministic cold start.
$resolve = Invoke-Adb -Arguments @(
    "shell", "cmd", "package", "resolve-activity", "--brief",
    "-c", "android.intent.category.LAUNCHER", $PackageName
) -AllowFailure
$launcherActivity = ($resolve.Output -split "`r?`n" | Where-Object { $_ -match '/' } | Select-Object -Last 1)
if (-not $launcherActivity) { $launcherActivity = "$PackageName/.MainActivity" }
$launcherActivity = $launcherActivity.Trim()
Write-Host "  launcher: $launcherActivity"

# ---------------------------------------------------------------------------
Write-Step "Cold start (am start -W x$ColdStartRuns)"
$S = $Summary
for ($run = 1; $run -le $ColdStartRuns; $run++) {
    Invoke-Adb -Arguments @("shell", "am", "force-stop", $PackageName) -AllowFailure | Out-Null
    Start-Sleep -Milliseconds 800
    Invoke-Adb -Arguments @("logcat", "-c") -AllowFailure | Out-Null
    $start = Invoke-Adb -Arguments @(
        "shell", "am", "start", "-W", "-n", $launcherActivity
    ) -AllowFailure
    Save-Text ("cold-start-run{0}.txt" -f $run) $start.Output | Out-Null
    $status = [regex]::Match($start.Output, 'Status:\s*(\w+)').Groups[1].Value
    $totalMatch = [regex]::Match($start.Output, 'TotalTime:\s*(\d+)')
    $waitMatch = [regex]::Match($start.Output, 'WaitTime:\s*(\d+)')
    $entry = [ordered]@{
        run = $run
        status = if ($status) { $status } else { "UNVERIFIED" }
        totalTimeMs = if ($totalMatch.Success) { [int]$totalMatch.Groups[1].Value } else { $null }
        waitTimeMs = if ($waitMatch.Success) { [int]$waitMatch.Groups[1].Value } else { $null }
    }
    if (-not $totalMatch.Success) { $S.unverified += "cold-start-totalTime-$run" }
    $S.coldStart += $entry
    Write-Host ("  run {0}: status={1} TotalTime={2} WaitTime={3}" -f $run, $entry.status, $entry.totalTimeMs, $entry.waitTimeMs)
    Start-Sleep -Seconds 2
}
$totals = @($S.coldStart | ForEach-Object { $_.totalTimeMs } | Where-Object { $_ -ne $null })
if ($totals.Count -gt 0) {
    $sorted = @($totals | Sort-Object)
    $median = if ($sorted.Count % 2 -eq 1) { $sorted[[int][math]::Floor($sorted.Count / 2)] } else { [int](($sorted[$sorted.Count / 2 - 1] + $sorted[$sorted.Count / 2]) / 2) }
    Write-Host "  median TotalTime: $median ms over $($sorted.Count) runs"
    $S.coldStartMedianTotalTimeMs = $median
}

# Keep the app foregrounded for the remaining measurements.
Start-Sleep -Seconds 2
Invoke-Screencap "01-launch.png" | Out-Null

# ---------------------------------------------------------------------------
Write-Step "Logcat scan"
$logcat = Invoke-Adb -Arguments @("logcat", "-d", "-v", "threadtime") -AllowFailure
Save-Text "logcat-launch.txt" $logcat.Output | Out-Null
$fatal = @($logcat.Output -split "`r?`n" | Where-Object { $_ -match 'FATAL EXCEPTION|AndroidRuntime|ANR in |Force finishing|Process: ' + [regex]::Escape($PackageName) })
$crashSignatures = @($fatal | Where-Object { $_ -match 'FATAL EXCEPTION|ANR in |Force finishing' })
$S.logcat = [ordered]@{
    lines = @($logcat.Output -split "`r?`n").Count
    fatalOrAnr = $crashSignatures.Count
    firstMatches = @($crashSignatures | Select-Object -First 12)
    logFile = "logcat-launch.txt"
}
Write-Host "  logcat lines: $($S.logcat.lines); FATAL/ANR hits: $($S.logcat.fatalOrAnr)"
if ($S.logcat.fatalOrAnr -gt 0) {
    Write-Host "  !!! crash/ANR signatures found — see logcat-launch.txt" -ForegroundColor Red
}

# ---------------------------------------------------------------------------
Write-Step "Memory: dumpsys meminfo (after launch)"
$memAfterLaunch = Invoke-Adb -Arguments @("shell", "dumpsys", "meminfo", $PackageName) -AllowFailure
Save-Text "meminfo-launch.txt" $memAfterLaunch.Output | Out-Null
$pss = [regex]::Match($memAfterLaunch.Output, 'TOTAL PSS:\s*(\d+)')
$totalPss = [regex]::Match($memAfterLaunch.Output, '(?m)^\s*TOTAL\s+(\d+)')
$memEntry = [ordered]@{
    phase = "after-launch"
    totalPssKb = if ($pss.Success) { [int]$pss.Groups[1].Value } elseif ($totalPss.Success) { [int]$totalPss.Groups[1].Value } else { $null }
    rawFile = "meminfo-launch.txt"
}
if ($memEntry.totalPssKb -eq $null) { $S.unverified += "meminfo-after-launch" }
$S.memory += $memEntry
Write-Host "  TOTAL PSS: $($memEntry.totalPssKb) KB"

# ---------------------------------------------------------------------------
Write-Step "Fixtures + content:// VIEW intents + reading metrics"
$fixtureStatus = [ordered]@{
    pushed = $false
    mdIntent = "not attempted"
    epubIntent = "not attempted"
    currentFocus = $null
    backToShelf = "not attempted"
}
if ($SkipIntent) {
    $fixtureStatus["skipped"] = $true
    Write-Host "  skipped (-SkipIntent)"
} elseif (-not (Test-Path $FixturesDir)) {
    $fixtureStatus["mdIntent"] = "fixtures dir missing: $FixturesDir"
    $S.unverified += "fixtures"
    Write-Host "  fixtures dir missing: $FixturesDir"
} else {
    $remoteDir = "/sdcard/Download/IR-QA"
    Invoke-Adb -Arguments @("shell", "mkdir", "-p", $remoteDir) -AllowFailure | Out-Null
    $pushLog = @()
    foreach ($name in @("sample-notes.md", "sample-book-epub.epub", "sample-library")) {
        $local = Join-Path $FixturesDir $name
        if (-not (Test-Path $local)) { $pushLog += "MISSING $local"; continue }
        $push = Invoke-Adb -Arguments @("push", $local, "$remoteDir/") -AllowFailure
        $pushLog += "$name exit=$($push.ExitCode) $($push.Output)"
    }
    $pushLog -join "`n" | Set-Content (Join-Path $OutDir "push-fixtures.txt") -Encoding UTF8
    $fixtureStatus["pushed"] = -not ($pushLog -match "exit=[^0]")
    Write-Host "  push fixtures: $($fixtureStatus['pushed'])"

    # Android 7+ blocks app->app file:// intents; the adb shell stays exempt but
    # Android 17 routes file:// VIEW to a chooser. Use the DocumentsProvider
    # content:// URI + explicit component + read grant (verified on PLJ110).
    $mdContentUri = "content://com.android.externalstorage.documents/document/primary%3ADownload%2FIR-QA%2Fsample-notes.md"
    $epubContentUri = "content://com.android.externalstorage.documents/document/primary%3ADownload%2FIR-QA%2Fsample-book-epub.epub"

    Invoke-Adb -Arguments @("shell", "am", "force-stop", $PackageName) -AllowFailure | Out-Null
    Start-Sleep -Milliseconds 900
    Invoke-Adb -Arguments @("logcat", "-c") -AllowFailure | Out-Null
    $mdIntentResult = Invoke-Adb -Arguments @(
        "shell", "am", "start", "-W",
        "-n", "$PackageName/.MainActivity",
        "-a", "android.intent.action.VIEW",
        "-d", $mdContentUri,
        "-t", "text/markdown",
        "--grant-read-uri-permission"
    ) -AllowFailure
    Save-Text "intent-md.txt" $mdIntentResult.Output | Out-Null
    Start-Sleep -Seconds 6
    $fixtureStatus.currentFocus = (Get-CurrentFocus)
    $fixtureStatus.mdIntent = if ($mdIntentResult.Output -match "Error|Exception") {
        "ERROR: $($mdIntentResult.Output)"
    } elseif ($mdIntentResult.Output -match "Status:\s*ok") {
        "delivered (verify 02-reader-md.png)"
    } else {
        "UNVERIFIED: $($mdIntentResult.Output)"
    }
    if (-not ($mdIntentResult.Output -match "Status:\s*ok")) { $S.unverified += "md-intent" }
    Write-Host "  md intent: $($fixtureStatus.mdIntent)"
    Invoke-Screencap "02-reader-md.png" | Out-Null

    # Reading metrics: reset frame stats, then three real scroll gestures.
    Invoke-Adb -Arguments @("shell", "dumpsys", "gfxinfo", $PackageName, "reset") -AllowFailure | Out-Null
    $wm = $wmSize -match "(\d+)x(\d+)"
    if ($wm) {
        $w = [int]$Matches[1]; $h = [int]$Matches[2]
        $x = [int]($w / 2); $y0 = [int]($h * 0.72); $y1 = [int]($h * 0.22)
        for ($i = 0; $i -lt 3; $i++) {
            Invoke-Adb -Arguments @("shell", "input", "swipe", $x, $y0, $x, $y1, "150") -AllowFailure | Out-Null
            Start-Sleep -Milliseconds 450
        }
    }
    Start-Sleep -Milliseconds 700
    Invoke-Screencap "03-reader-scrolled.png" | Out-Null
    $gfxReader = Invoke-Adb -Arguments @("shell", "dumpsys", "gfxinfo", $PackageName) -AllowFailure
    Save-Text "gfxinfo-reader.txt" $gfxReader.Output | Out-Null
    $totalFrames = [regex]::Match($gfxReader.Output, "Total frames rendered:\s*(\d+)")
    $janky = [regex]::Match($gfxReader.Output, "Janky frames:\s*(\d+)\s*\(([\d.]+)%\)")
    $p50 = [regex]::Match($gfxReader.Output, "50th percentile:\s*(\d+)ms")
    $p90 = [regex]::Match($gfxReader.Output, "90th percentile:\s*(\d+)ms")
    $p99 = [regex]::Match($gfxReader.Output, "99th percentile:\s*(\d+)ms")
    $S.gfx = [ordered]@{
        scenario = "reader scroll x3 (content:// external open)"
        totalFrames = if ($totalFrames.Success) { [int]$totalFrames.Groups[1].Value } else { $null }
        jankyFrames = if ($janky.Success) { [int]$janky.Groups[1].Value } else { $null }
        jankyPercent = if ($janky.Success) { [double]$janky.Groups[2].Value } else { $null }
        p50ms = if ($p50.Success) { [int]$p50.Groups[1].Value } else { $null }
        p90ms = if ($p90.Success) { [int]$p90.Groups[1].Value } else { $null }
        p99ms = if ($p99.Success) { [int]$p99.Groups[1].Value } else { $null }
        rawFile = "gfxinfo-reader.txt"
    }
    if (-not $janky.Success) { $S.unverified += "gfxinfo-reader" }
    Write-Host "  reader gfx: frames=$($S.gfx.totalFrames) janky=$($S.gfx.jankyFrames) ($($S.gfx.jankyPercent)%) p50=$($S.gfx.p50ms)ms"
    $frameStats = Invoke-Adb -Arguments @("shell", "dumpsys", "gfxinfo", $PackageName, "framestats") -AllowFailure
    Save-Text "gfxinfo-framestats.csv" $frameStats.Output | Out-Null

    # PSS while a document is open.
    $memAfterReader = Invoke-Adb -Arguments @("shell", "dumpsys", "meminfo", $PackageName) -AllowFailure
    Save-Text "meminfo-reader.txt" $memAfterReader.Output | Out-Null
    $pss2 = [regex]::Match($memAfterReader.Output, "TOTAL PSS:\s*(\d+)")
    $total2 = [regex]::Match($memAfterReader.Output, "(?m)^\s*TOTAL\s+(\d+)")
    $memReader = [ordered]@{
        phase = "after-reader-open+scroll"
        totalPssKb = if ($pss2.Success) { [int]$pss2.Groups[1].Value } elseif ($total2.Success) { [int]$total2.Groups[1].Value } else { $null }
        rawFile = "meminfo-reader.txt"
    }
    if ($memReader.totalPssKb -eq $null) { $S.unverified += "meminfo-after-reader" }
    $S.memory += $memReader
    Write-Host "  TOTAL PSS while reading: $($memReader.totalPssKb) KB"

    # BACK must return to the shelf without killing the app (shelf-open path).
    Invoke-Adb -Arguments @("shell", "input", "keyevent", "KEYCODE_BACK") -AllowFailure | Out-Null
    Start-Sleep -Seconds 2
    $alive = (Invoke-Adb -Arguments @("shell", "pidof", $PackageName) -AllowFailure).Output
    $fixtureStatus.backToShelf = if ($alive) { "app alive after BACK; verify 04-back-to-shelf.png" } else { "UNVERIFIED: process gone after BACK" }
    Write-Host "  BACK: $($fixtureStatus.backToShelf)"
    Invoke-Screencap "04-back-to-shelf.png" | Out-Null

    # Light/dark screenshots (app follows OS scheme; toggle system uiMode).
    Invoke-Adb -Arguments @("shell", "cmd", "uimode", "night", "no") -AllowFailure | Out-Null
    Start-Sleep -Seconds 2
    Invoke-Screencap "05-shelf-light.png" | Out-Null
    Invoke-Adb -Arguments @("shell", "cmd", "uimode", "night", "yes") -AllowFailure | Out-Null
    Start-Sleep -Seconds 2

    # EPUB external open.
    $epubIntentResult = Invoke-Adb -Arguments @(
        "shell", "am", "start", "-W",
        "-n", "$PackageName/.MainActivity",
        "-a", "android.intent.action.VIEW",
        "-d", $epubContentUri,
        "-t", "application/epub+zip",
        "--grant-read-uri-permission"
    ) -AllowFailure
    Save-Text "intent-epub.txt" $epubIntentResult.Output | Out-Null
    Start-Sleep -Seconds 6
    $fixtureStatus.epubIntent = if ($epubIntentResult.Output -match "Error|Exception") {
        "ERROR: $($epubIntentResult.Output)"
    } elseif ($epubIntentResult.Output -match "Status:\s*ok") {
        "delivered (verify 08-epub-import.png)"
    } else {
        "UNVERIFIED: $($epubIntentResult.Output)"
    }
    if (-not ($epubIntentResult.Output -match "Status:\s*ok")) { $S.unverified += "epub-intent" }
    Write-Host "  epub intent: $($fixtureStatus.epubIntent)"
    Invoke-Screencap "08-epub-import.png" | Out-Null

    $logcatFlow = Invoke-Adb -Arguments @("logcat", "-d", "-v", "threadtime") -AllowFailure
    Save-Text "logcat-reader-flow.txt" $logcatFlow.Output | Out-Null
    $flowCrashes = @($logcatFlow.Output -split "`r?`n" | Where-Object { $_ -match "FATAL EXCEPTION|ANR in " })
    if ($flowCrashes.Count -gt 0) {
        Write-Host "  !!! crash/ANR during reader flow: $($flowCrashes.Count) hits" -ForegroundColor Red
        $S.logcat["readerFlowCrashHits"] = $flowCrashes.Count
    }
}
$S.intent = $fixtureStatus

Write-Step "Summary"
$S.apk = $S.apk
$S | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $OutDir "device-summary.json") -Encoding UTF8
$reportLines = @(
    "# Android 真机证据（android-device-check.ps1）",
    "",
    "- 生成时间：$($S.generatedAt)",
    "- 设备：$($S.device.brand) $($S.device.model) / Android $($S.device.androidRelease) (SDK $($S.device.sdk)) / $($S.device.wmSize) $($S.device.wmDensity)",
    "- 包名：$PackageName",
    "- APK：$($S.apk.path)（$($S.apk.bytes) B，SHA-256 ``$($S.apk.sha256)``）",
    "- 安装：exit=$($S.install.exitCode) versionName=$($S.install.versionName)",
    "- 冷启动：$(($S.coldStart | ForEach-Object { "run$($_.run) status=$($_.status) TotalTime=$($_.totalTimeMs)ms WaitTime=$($_.waitTimeMs)ms" }) -join '；')（median TotalTime=$($S.coldStartMedianTotalTimeMs)ms）",
    "- logcat FATAL/ANR 命中：$($S.logcat.fatalOrAnr)",
    "- 内存：$(($S.memory | ForEach-Object { "$($_.phase)=$($_.totalPssKb)KB" }) -join '；')",
    "- 帧统计：frames=$($S.gfx.totalFrames) janky=$($S.gfx.jankyFrames) ($($S.gfx.jankyPercent)%) p50=$($S.gfx.p50ms)ms p90=$($S.gfx.p90ms)ms p99=$($S.gfx.p99ms)ms",
    "- 截图：$(($S.screenshots | ForEach-Object { $_.name }) -join '、')",
    "- 外部打开 intent：md=$($S.intent.mdIntent)；epub=$($S.intent.epubIntent)；当前焦点=$($S.intent.currentFocus)",
    "- 未验证项：$(if ($S.unverified.Count -gt 0) { $S.unverified -join '、' } else { '无' })"
)
Save-Text "device-report.md" ($reportLines -join "`n") | Out-Null

Write-Host ""
Write-Host "Evidence written to: $OutDir" -ForegroundColor Green
Write-Host "  device-summary.json / device-report.md / logcat-*.txt / meminfo-*.txt / gfxinfo-*.txt / *.png"
if ($S.logcat.fatalOrAnr -gt 0) {
    Write-Host "WARNING: FATAL/ANR signatures were found on device — treat as FAIL until triaged." -ForegroundColor Red
    exit 1
}
exit 0

