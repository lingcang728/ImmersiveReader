param(
    [switch]$ShortcutMode,
    [switch]$NoPause,
    [switch]$NoOpenFolder,
    [string]$OpenFolder = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir
$Inbox = Join-Path $Root "input"
$FinalOut = Join-Path $Root "output"
$Logs = Join-Path $Root "work\logs"
$Python = Join-Path $Root ".venv\Scripts\python.exe"
$ConfigPath = Join-Path $Root "config.json"
$SetupScript = Join-Path $ScriptDir "setup.ps1"

function Pause-IfNeeded {
    param([bool]$ShouldPause)
    if ($ShouldPause) {
        Read-Host "按 Enter 关闭窗口"
    }
}

function Resolve-ProjectPath {
    param([string]$PathValue, [string]$Fallback)
    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $Fallback
    }
    if ([System.IO.Path]::IsPathRooted($PathValue)) {
        return $PathValue
    }
    return Join-Path $Root $PathValue
}

function Find-WinGetExe {
    param([string]$Name)
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }

    $wingetPackages = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
    if (Test-Path $wingetPackages) {
        $match = Get-ChildItem -Path $wingetPackages -Recurse -Filter "$Name.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($match) {
            $env:PATH = "$($match.DirectoryName);$env:PATH"
            return $match.FullName
        }
    }
    return $null
}

Set-Location $Root

Write-Host ""
Write-Host "Podcast 转写工厂"
Write-Host "工作区: $Root"
Write-Host "扫描 input: $Inbox"
Write-Host "最终成品 Markdown: $FinalOut"
Write-Host ""

if (-not (Test-Path $Python)) {
    Write-Host "未找到虚拟环境，正在按 requirements.txt 重建..." -ForegroundColor Yellow
    if (-not (Test-Path $SetupScript)) {
        Write-Host "未找到安装脚本: $SetupScript" -ForegroundColor Red
        Pause-IfNeeded (-not ($ShortcutMode -or $NoPause))
        exit 2
    }
    & $SetupScript
    if (-not (Test-Path $Python)) {
        Write-Host "虚拟环境创建失败: $Python" -ForegroundColor Red
        Pause-IfNeeded (-not ($ShortcutMode -or $NoPause))
        exit 2
    }
}

$ffmpeg = Find-WinGetExe "ffmpeg"
$ffprobe = Find-WinGetExe "ffprobe"
if (-not $ffmpeg -or -not $ffprobe) {
    Write-Host "未找到 ffmpeg/ffprobe。请先安装 FFmpeg，或重新运行初始化流程。" -ForegroundColor Red
    Write-Host "日志目录: $Logs"
    Pause-IfNeeded (-not ($ShortcutMode -or $NoPause))
    exit 3
}

$env:VIRTUAL_ENV = Join-Path $Root ".venv"
$Config = $null
if (Test-Path $ConfigPath) {
    try {
        $Config = Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
    }
    catch {
        Write-Host "读取 config.json 失败，将使用默认运行参数: $($_.Exception.Message)" -ForegroundColor Yellow
    }
}

$ThreadLimit = 4
if ($Config -and $Config.omp_num_threads) {
    $ThreadLimit = [int]$Config.omp_num_threads
}
$BatchSize = 8
$VadFilter = $true
$ParallelFiles = 1
if ($Config -and $Config.asr) {
    if ($Config.asr.batch_size) { $BatchSize = [int]$Config.asr.batch_size }
    if ($null -ne $Config.asr.vad_filter) { $VadFilter = [bool]$Config.asr.vad_filter }
}
if ($Config -and $Config.pipeline -and $Config.pipeline.max_parallel_audio_files) {
    $ParallelFiles = [int]$Config.pipeline.max_parallel_audio_files
}
$Priority = "BelowNormal"
if ($Config -and $Config.process_priority) {
    $Priority = [string]$Config.process_priority
}
$OpenFolderAfterRun = $true
$PauseOnExit = -not ($ShortcutMode -or $NoPause)
if ($Config -and $Config.launcher) {
    if ($null -ne $Config.launcher.open_folder_after_run) {
        $OpenFolderAfterRun = [bool]$Config.launcher.open_folder_after_run
    }
    if ([string]::IsNullOrWhiteSpace($OpenFolder) -and $Config.launcher.open_folder) {
        $OpenFolder = [string]$Config.launcher.open_folder
    }
    if (-not ($ShortcutMode -or $NoPause) -and $Config.launcher.pause_on_exit) {
        $PauseOnExit = ([string]$Config.launcher.pause_on_exit).ToLowerInvariant() -ne "never"
    }
}
if ($NoOpenFolder) {
    $OpenFolderAfterRun = $false
}
$FolderToOpen = Resolve-ProjectPath $OpenFolder $FinalOut

$SitePackages = Join-Path $env:VIRTUAL_ENV "Lib\site-packages"
$NvidiaDllDirs = @(
    (Join-Path $SitePackages "nvidia\cuda_runtime\bin"),
    (Join-Path $SitePackages "nvidia\cuda_nvrtc\bin"),
    (Join-Path $SitePackages "nvidia\cublas\bin"),
    (Join-Path $SitePackages "nvidia\cudnn\bin"),
    (Join-Path $SitePackages "ctranslate2")
) | Where-Object { Test-Path $_ }
$env:PATH = "$(Join-Path $env:VIRTUAL_ENV 'Scripts');$($NvidiaDllDirs -join ';');$env:PATH"
$env:PYTHONUTF8 = "1"
$env:OMP_NUM_THREADS = "$ThreadLimit"
$env:MKL_NUM_THREADS = "$ThreadLimit"
$env:OPENBLAS_NUM_THREADS = "$ThreadLimit"
$env:NUMEXPR_NUM_THREADS = "$ThreadLimit"
$env:CT2_USE_EXPERIMENTAL_PACKED_GEMM = "1"
$env:PODCAST_TRANSCRIBER_NO_OPEN_OUTPUT = "1"

if ($NvidiaDllDirs.Count -gt 0) {
    Write-Host "NVIDIA GPU DLL 路径已加入本次进程 PATH:"
    foreach ($dir in $NvidiaDllDirs) { Write-Host "  $dir" }
    Write-Host ""
}

try {
    Write-Host "运行配置: GPU batch=$BatchSize, VAD=$VadFilter, parallel files=$ParallelFiles, CPU threads=$ThreadLimit, priority=$Priority"
    Write-Host ""

    $ArgumentList = @((Join-Path $Root "scripts\transcribe_podcasts.py"), "--force", "--no-open-output")
    $Process = Start-Process -FilePath $Python -ArgumentList $ArgumentList -WorkingDirectory $Root -NoNewWindow -PassThru
    Start-Sleep -Milliseconds 300
    try { $Process.PriorityClass = $Priority } catch { Write-Host "设置进程优先级失败: $($_.Exception.Message)" -ForegroundColor Yellow }
    if ($Config -and $null -ne $Config.process_affinity_mask) {
        try { $Process.ProcessorAffinity = [IntPtr]([int64]$Config.process_affinity_mask) }
        catch { Write-Host "设置 CPU affinity 失败: $($_.Exception.Message)" -ForegroundColor Yellow }
    }
    $Process.WaitForExit()
    $exitCode = $Process.ExitCode
}
catch {
    Write-Host "运行失败: $($_.Exception.Message)" -ForegroundColor Red
    $exitCode = 1
}

Write-Host ""
Write-Host "最终成品 Markdown 请到这里查看: $FinalOut"
if ($exitCode -eq 0) {
    Write-Host "本次临时产物和历史记录已清理。"
}
else {
    Write-Host "如果出错，请看日志目录: $Logs"
}
Write-Host ""
if ($OpenFolderAfterRun -and (Test-Path $FolderToOpen)) {
    try {
        Start-Process -FilePath "explorer.exe" -ArgumentList @($FolderToOpen)
        Write-Host "已打开文件夹: $FolderToOpen"
    }
    catch {
        Write-Host "打开文件夹失败: $($_.Exception.Message)" -ForegroundColor Yellow
    }
}
Pause-IfNeeded $PauseOnExit
exit $exitCode
