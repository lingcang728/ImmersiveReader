[CmdletBinding()]
param(
    [switch]$ValidateOnly,
    [switch]$RefreshApps,
    [string]$PodcastSource = '',
    [string]$PythonRoot = 'G:\python',
    [string]$EdgeRoot = 'C:\Program Files (x86)\Microsoft\Edge\Application'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot
$runtime = Join-Path $root 'runtime'
$fullRuntime = [IO.Path]::GetFullPath($runtime)
$zhihuSource = Join-Path $root 'tools\zhihu-packer'
$podcastAppSource = Join-Path $root 'tools\podcast-transcriber'
$contractsSource = Join-Path $root 'packages\contracts'
# site-packages bootstrap order: an explicit -PodcastSource is honored as-is;
# otherwise the tool's own dev venv (the real upstream source) wins, then — to
# keep refreshes working — the already-provisioned runtime. Without the venv
# fallback a clean machine dead-ended: building runtime required runtime.
$podcastSitePackagesCandidates = @()
if ($PodcastSource) {
    $podcastSitePackagesCandidates += Join-Path $PodcastSource '.venv\Lib\site-packages'
} else {
    $podcastSitePackagesCandidates += Join-Path $podcastAppSource '.venv\Lib\site-packages'
    $podcastSitePackagesCandidates += Join-Path $runtime 'podcast\python\Lib\site-packages'
}
$podcastSitePackages = [string]($podcastSitePackagesCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1)
$podcastModels = if ($PodcastSource) {
    Join-Path $PodcastSource 'models'
} else {
    Join-Path $runtime 'podcast\models'
}
$node = Require-Command -Name 'node.exe'
$npm = Require-Command -Name 'npm.cmd'
$ffmpeg = Require-Command -Name 'ffmpeg.exe'
$ffprobe = Require-Command -Name 'ffprobe.exe'

function Require-Path {
    param([Parameter(Mandatory)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "缺少运行时来源：$Path"
    }
}

function Copy-Tree {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [string[]]$ExcludeDirectories = @(),
        [string[]]$ExcludeFiles = @()
    )

    $arguments = @($Source, $Destination, '/E', '/COPY:DAT', '/DCOPY:DAT', '/R:2', '/W:1', '/NFL', '/NDL', '/NJH', '/NJS', '/NP')
    if ($ExcludeDirectories.Count -gt 0) {
        $arguments += '/XD'
        $arguments += $ExcludeDirectories
    }
    if ($ExcludeFiles.Count -gt 0) {
        $arguments += '/XF'
        $arguments += $ExcludeFiles
    }
    & robocopy @arguments
    if ($LASTEXITCODE -gt 7) {
        throw "复制运行时失败：$Source -> $Destination（robocopy $LASTEXITCODE）"
    }
}

# Directories/files that must never reach the vendored zhihu sidecar app:
# src/ and tests/ are compile-time only (the sidecar runs dist\server.js),
# tools/ is a manual utility, output/ is dev-generated content, and the
# profile/cache dirs are machine-local state (see TOOL_LOCAL_STATE_DIRS in
# tools/zhihu-packer/src/runtime-paths.ts — keep both lists in sync).
$ZhihuAppExcludedDirectories = @(
    '.git', 'src', 'tests', 'tools', 'output',
    '.browser-profile', '.obscura-profile', '.browser-cache'
)
$ZhihuAppExcludedFiles = @('*.log', '*.db', '*.db-*')

function Prune-DevDependencies {
    # The runtime copy of node_modules still contains devDependencies — strip
    # them so the bundle ships production deps only (tsx/typescript/esbuild/
    # jsdom/marked/pinyin-pro are compile-time; nothing in the sidecar chain
    # imports them). npm prune removes packages without touching the network.
    param([Parameter(Mandatory)][string]$AppDir)

    Push-Location $AppDir
    try {
        & $npm prune --omit=dev | Write-Output
        if ($LASTEXITCODE -ne 0) {
            throw "npm prune --omit=dev 失败：$AppDir（退出码 $LASTEXITCODE）"
        }
    } finally {
        Pop-Location
    }
}

function Reset-AppDestination {
    param([Parameter(Mandatory)][string]$Destination)

    $fullDestination = [IO.Path]::GetFullPath($Destination)
    $allowedRoot = $fullRuntime.TrimEnd('\') + [IO.Path]::DirectorySeparatorChar
    if (-not $fullDestination.StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝清理受管运行时外的应用目录：$fullDestination"
    }
    Remove-DirectoryTree -Path $fullDestination
}

function Assert-RuntimeNotInUse {
    # Sidecars/workers keep runtime files open; deleting or swapping the
    # runtime underneath them tears the install. Fail fast instead of killing
    # them silently — callers must close the app/dev session first.
    $prefix = $fullRuntime.TrimEnd('\') + [IO.Path]::DirectorySeparatorChar
    $running = @()
    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        $processPath = $null
        try { $processPath = $process.Path } catch { $processPath = $null }
        if ($processPath -and
            (([IO.Path]::GetFullPath($processPath) + '\').StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase))) {
            $running += $process
        }
    }
    if ($running.Count -gt 0) {
        $names = ($running | ForEach-Object { "$($_.ProcessName) (PID $($_.Id))" }) -join ', '
        throw "受管运行时正被以下进程占用，请先关闭沉浸阅读/开发会话后重试：$names"
    }
}

function Assert-ZhihuNode {
    param(
        [Parameter(Mandatory)][string]$NodeExe,
        [Parameter(Mandatory)][string]$Label
    )
    # tools\zhihu-packer\src\db.ts imports node:sqlite (DatabaseSync), which only
    # exists in Node >= 22.5.0 — assert both the version and that the module
    # actually loads so a torn or too-old vendored copy fails at build time.
    $versionText = [string](& $NodeExe --version 2>&1 | Select-Object -First 1)
    if ($versionText -notmatch 'v?(\d+\.\d+\.\d+)') {
        throw "$Label node.exe 无法报告版本（可能损坏）：$NodeExe"
    }
    $version = [version]$Matches[1]
    if ($version -lt [version]'22.5.0') {
        throw "$Label node.exe $version 过旧：zhihu-packer 需要 node:sqlite（Node >= 22.5.0）：$NodeExe"
    }
    & $NodeExe -e "require('node:sqlite')" 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "$Label node.exe 缺少可用的 node:sqlite 模块：$NodeExe"
    }
    Write-Output "[runtime] $Label node.exe $version (node:sqlite ok)"
}

function Assert-MediaTool {
    param(
        [Parameter(Mandatory)][string]$Exe,
        [Parameter(Mandatory)][string]$Label
    )
    # ffmpeg/ffprobe must run and report a modern version — a binary that
    # cannot execute means the vendored copy is corrupt.
    $line = [string](& $Exe -version 2>&1 | Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($line) -or $line -notmatch 'version (\d+(?:\.\d+)+)') {
        throw "$Label 无法报告版本（可能损坏）：$Exe"
    }
    if ([version]$Matches[1] -lt [version]'5.0') {
        throw "$Label 版本过旧（要求 >= 5.0）：$Exe"
    }
    Write-Output "[runtime] $Label $line"
}

function Assert-PythonRuntime {
    param(
        [Parameter(Mandatory)][string]$PythonExe,
        [Parameter(Mandatory)][string]$Label
    )
    $line = [string](& $PythonExe --version 2>&1 | Select-Object -First 1)
    if ($line -notmatch 'Python (\d+\.\d+\.\d+)') {
        throw "$Label python.exe 无法报告版本（可能损坏）：$PythonExe"
    }
    if ([version]$Matches[1] -lt [version]'3.9.0') {
        throw "$Label python.exe 版本过旧（要求 >= 3.9）：$PythonExe"
    }
    Write-Output "[runtime] $Label $line"
}

function Get-ExeVersionLine {
    # Best-effort version string: run the tool's version flag, else fall back
    # to the PE product version so provenance never blocks on a missing flag.
    param(
        [Parameter(Mandatory)][string]$Exe,
        [string]$VersionArg = '--version'
    )
    $line = [string](& $Exe $VersionArg 2>&1 | Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($line) -or $LASTEXITCODE -ne 0) {
        $line = [string](Get-Item -LiteralPath $Exe).VersionInfo.ProductVersion
    }
    $global:LASTEXITCODE = 0
    return $line.Trim()
}

function Get-RuntimeComponents {
    # Provenance record for manifest.json: which exact binaries went in, where
    # they came from, and their versions — a path/bytes/sha256 manifest alone
    # can never answer "what version is inside this runtime".
    param(
        [Parameter(Mandatory)][string]$NodeSourceExe,
        [Parameter(Mandatory)][string]$PythonSourceExe,
        [Parameter(Mandatory)][string]$FfmpegSourceExe,
        [Parameter(Mandatory)][string]$FfprobeSourceExe,
        [Parameter(Mandatory)][string]$ChromiumSourceExe,
        [Parameter(Mandatory)][string]$ModelsSource
    )
    $components = [ordered]@{}
    foreach ($spec in @(
        @{ key = 'node'; exe = $NodeSourceExe; arg = '--version' },
        @{ key = 'python'; exe = $PythonSourceExe; arg = '--version' },
        @{ key = 'ffmpeg'; exe = $FfmpegSourceExe; arg = '-version' },
        @{ key = 'ffprobe'; exe = $FfprobeSourceExe; arg = '-version' }
    )) {
        $components[$spec.key] = [ordered]@{
            source = [string]$spec.exe
            version = Get-ExeVersionLine -Exe $spec.exe -VersionArg $spec.arg
            sha256 = (Get-FileSha256Hex -Path $spec.exe)
        }
    }
    # Never spawn msedge.exe for a version query — the PE product version is
    # equally authoritative and cannot flash a browser window.
    $components['chromium'] = [ordered]@{
        source = [string]$ChromiumSourceExe
        version = [string](Get-Item -LiteralPath $ChromiumSourceExe).VersionInfo.ProductVersion
        sha256 = (Get-FileSha256Hex -Path $ChromiumSourceExe)
    }
    $modelFiles = @(Get-ChildItem -LiteralPath $ModelsSource -File -Recurse -ErrorAction SilentlyContinue)
    $components['models'] = [ordered]@{
        source = [string]$ModelsSource
        files = $modelFiles.Count
        bytes = [long](($modelFiles | Measure-Object Length -Sum).Sum)
    }
    return $components
}

function Assert-PreservedRuntimeEntries {
    # -RefreshApps rewrites manifest.json to describe the NEW state — which
    # would bless a corrupted vendored binary into a green manifest. Before
    # touching anything, verify the entries RefreshApps does not regenerate
    # (vendored node/chromium/python/ffmpeg/models) against the old manifest.
    param([Parameter(Mandatory)][string]$RuntimeRoot)

    $manifestPath = Join-Path $RuntimeRoot 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        Write-Warning '[runtime] 现有 manifest.json 缺失，无法做 RefreshApps 前置完整性校验。'
        return
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $refreshedRoots = @('zhihu/app/', 'podcast/app/', 'packages/contracts/')
    $checked = 0
    foreach ($entry in @($manifest.entries)) {
        $relative = [string]$entry.path
        if ($refreshedRoots | Where-Object { $relative.StartsWith($_, [StringComparison]::OrdinalIgnoreCase) }) {
            continue
        }
        $path = Join-Path $RuntimeRoot ($relative -replace '/', '\')
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "RefreshApps 前置校验：受管文件缺失：$relative"
        }
        $item = Get-Item -LiteralPath $path
        if ([int64]$item.Length -ne [int64]$entry.bytes -or
            (Get-FileSha256Hex -Path $path) -ine [string]$entry.sha256) {
            throw "RefreshApps 前置校验：受管文件已损坏或被改写：$relative（先修复 runtime，再刷新应用代码）"
        }
        $checked += 1
    }
    Write-Output "[runtime] preserved entries verified against existing manifest: $checked"
}

function Get-CriticalRuntimeFiles {
    param([Parameter(Mandatory)][string]$RuntimeRoot)

    $codeRoots = @(
        (Join-Path $RuntimeRoot 'zhihu\app'),
        (Join-Path $RuntimeRoot 'podcast\app'),
        (Join-Path $RuntimeRoot 'packages\contracts')
    )
    $binaryRoots = @(
        (Join-Path $RuntimeRoot 'zhihu\node'),
        (Join-Path $RuntimeRoot 'zhihu\chromium'),
        (Join-Path $RuntimeRoot 'podcast\python'),
        (Join-Path $RuntimeRoot 'podcast\ffmpeg')
    )
    $files = @()
    foreach ($codeRoot in $codeRoots) {
        if (Test-Path -LiteralPath $codeRoot) {
            $files += Get-ChildItem -LiteralPath $codeRoot -File -Recurse
        }
    }
    foreach ($binaryRoot in $binaryRoots) {
        if (Test-Path -LiteralPath $binaryRoot) {
            $files += Get-ChildItem -LiteralPath $binaryRoot -File -Recurse |
                Where-Object { $_.Extension.ToLowerInvariant() -in @('.exe', '.dll', '.pyd', '.pak', '.bin', '.dat') }
        }
    }
    $modelRoot = Join-Path $RuntimeRoot 'podcast\models'
    if (Test-Path -LiteralPath $modelRoot) {
        $files += Get-ChildItem -LiteralPath $modelRoot -File -Recurse
    }
    $files | Sort-Object -Property FullName -Unique
}

function Write-CriticalRuntimeManifest {
    param(
        [Parameter(Mandatory)][string]$RuntimeRoot,
        # Component provenance (version/source/sha256 per vendored tool) —
        # additive field; verify-runtime.ps1 only requires schemaVersion >= 2.
        $Components = $null
    )

    $fullRuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot).TrimEnd('\')
    $entries = @(Get-CriticalRuntimeFiles -RuntimeRoot $fullRuntimeRoot | ForEach-Object {
        $item = Get-Item -LiteralPath $_.FullName
        [ordered]@{
            path = $item.FullName.Substring($fullRuntimeRoot.Length).TrimStart('\').Replace('\', '/')
            bytes = $item.Length
            sha256 = (Get-FileSha256Hex -Path $item.FullName)
        }
    })
    if ($entries.Count -eq 0) {
        throw "没有找到受管运行时 critical 文件：$fullRuntimeRoot"
    }
    $manifest = [ordered]@{
        schemaVersion = 2
        generatedAt = (Get-Date).ToUniversalTime().ToString('o')
        entryCount = $entries.Count
        entries = $entries
    }
    if ($null -ne $Components) {
        $manifest['components'] = $Components
    }
    $manifestPath = Join-Path $fullRuntimeRoot 'manifest.json'
    $temporaryPath = "$manifestPath.tmp"
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $temporaryPath -Encoding UTF8
    Move-Item -LiteralPath $temporaryPath -Destination $manifestPath -Force
    Write-Output "[runtime] critical manifest contains $($entries.Count) entries"
}

if ($RefreshApps) {
    # RefreshApps mirrors the built sidecar/contracts dist into the runtime —
    # assert the sources exist so a clean checkout fails loudly instead of
    # shipping a runtime without the zhihu sidecar entry point.
    foreach ($requiredRuntime in @(
        (Join-Path $runtime 'zhihu\node\node.exe'),
        (Join-Path $runtime 'podcast\python\python.exe'),
        (Join-Path $runtime 'podcast\ffmpeg\ffmpeg.exe'),
        (Join-Path $runtime 'podcast\models'),
        (Join-Path $zhihuSource 'dist\server.js'),
        (Join-Path $zhihuSource 'node_modules'),
        (Join-Path $contractsSource 'dist\index.js')
    )) {
        Require-Path -Path $requiredRuntime
    }
    # Vendored binaries are refreshed into installs unchanged — they must run
    # and meet minimum versions, not merely exist.
    Assert-ZhihuNode -NodeExe (Join-Path $runtime 'zhihu\node\node.exe') -Label 'vendored'
    Assert-PythonRuntime -PythonExe (Join-Path $runtime 'podcast\python\python.exe') -Label 'vendored'
    Assert-MediaTool -Exe (Join-Path $runtime 'podcast\ffmpeg\ffmpeg.exe') -Label 'vendored ffmpeg'
    Assert-RuntimeNotInUse
    # The manifest is about to be rewritten to the post-refresh state — prove
    # the parts RefreshApps keeps (binaries, models) are still intact first,
    # or a corrupted runtime would be blessed by its own new manifest.
    Assert-PreservedRuntimeEntries -RuntimeRoot $runtime
    $zhihuApp = Join-Path $runtime 'zhihu\app'
    $podcastApp = Join-Path $runtime 'podcast\app'
    $contractsRuntime = Join-Path $runtime 'packages\contracts'
    Reset-AppDestination -Destination $zhihuApp
    Reset-AppDestination -Destination $podcastApp
    Reset-AppDestination -Destination $contractsRuntime
    Copy-Tree -Source $zhihuSource -Destination $zhihuApp `
        -ExcludeDirectories $ZhihuAppExcludedDirectories `
        -ExcludeFiles $ZhihuAppExcludedFiles
    Prune-DevDependencies -AppDir $zhihuApp
    Copy-Tree -Source $podcastAppSource -Destination $podcastApp `
        -ExcludeDirectories @('.git', '.venv', 'models', 'input', 'output', 'work', '.pytest_cache', '__pycache__') `
        -ExcludeFiles @('config.json', '*.log', '*.pyc')
    Copy-Tree -Source $contractsSource -Destination $contractsRuntime `
        -ExcludeDirectories @('.git', 'node_modules')
    $refreshComponents = Get-RuntimeComponents `
        -NodeSourceExe (Join-Path $runtime 'zhihu\node\node.exe') `
        -PythonSourceExe (Join-Path $runtime 'podcast\python\python.exe') `
        -FfmpegSourceExe (Join-Path $runtime 'podcast\ffmpeg\ffmpeg.exe') `
        -FfprobeSourceExe (Join-Path $runtime 'podcast\ffmpeg\ffprobe.exe') `
        -ChromiumSourceExe (Join-Path $runtime 'zhihu\chromium\msedge.exe') `
        -ModelsSource (Join-Path $runtime 'podcast\models')
    Write-CriticalRuntimeManifest -RuntimeRoot $runtime -Components $refreshComponents
    Write-Output '[runtime] application code refreshed without rebuilding large assets'
    exit 0
}

if (-not $podcastSitePackages) {
    if ($PodcastSource) {
        throw "-PodcastSource 缺少 site-packages：$(Join-Path $PodcastSource '.venv\Lib\site-packages')（需要 .venv 布局的旧安装/开发目录）。"
    }
    throw "找不到 Podcast site-packages 来源。请为 tools\podcast-transcriber 创建 .venv（pip install -r requirements.txt）、传入 -PodcastSource <旧安装目录>，或先恢复现有 runtime。"
}
if ($podcastSitePackages -ieq (Join-Path $runtime 'podcast\python\Lib\site-packages')) {
    Write-Warning '[runtime] site-packages 复用自现有 runtime；如需干净自举请改用 tools\podcast-transcriber\.venv 或 -PodcastSource。'
}
if (-not (Test-Path -LiteralPath $podcastModels)) {
    throw "缺少 Podcast 模型目录：$podcastModels（模型不在仓库内；传 -PodcastSource 或先恢复现有 runtime）。"
}
$required = @(
    (Join-Path $zhihuSource 'dist\server.js'),
    (Join-Path $zhihuSource 'node_modules'),
    (Join-Path $contractsSource 'dist\index.js'),
    (Join-Path $podcastAppSource 'scripts\sidecar_server.py'),
    $podcastSitePackages,
    $podcastModels,
    (Join-Path $PythonRoot 'python.exe'),
    (Join-Path $PythonRoot 'Lib'),
    (Join-Path $EdgeRoot 'msedge.exe'),
    $node,
    $ffmpeg,
    $ffprobe
)
$required | ForEach-Object { Require-Path -Path $_ }

# Assert the tool versions being vendored before anything is copied: the
# zhihu sidecar needs node:sqlite (Node >= 22.5.0) and a binary that cannot
# execute here would ship broken.
Assert-ZhihuNode -NodeExe $node -Label 'source'
Assert-PythonRuntime -PythonExe (Join-Path $PythonRoot 'python.exe') -Label 'source'
Assert-MediaTool -Exe $ffmpeg -Label 'source ffmpeg'
Assert-MediaTool -Exe $ffprobe -Label 'source ffprobe'

if ($ValidateOnly) {
    Write-Output '[runtime] all reusable sources are available'
    $required | ForEach-Object { Write-Output "[runtime] $_" }
    exit 0
}

$stagingRuntime = "$runtime.__staging__"
$previousRuntime = "$runtime.__previous__"
$fullStagingRuntime = [IO.Path]::GetFullPath($stagingRuntime)
$fullPreviousRuntime = [IO.Path]::GetFullPath($previousRuntime)
$fullRoot = [IO.Path]::GetFullPath($root) + [IO.Path]::DirectorySeparatorChar
if (-not $fullRuntime.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase) -or
    -not $fullStagingRuntime.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase) -or
    -not $fullPreviousRuntime.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝清理工作区外的运行时目录：$fullRuntime"
}
foreach ($runtimeDir in @($fullRuntime, $fullStagingRuntime, $fullPreviousRuntime)) {
    Assert-NotReparsePoint -Path $runtimeDir -Label "runtime 目录"
}
Assert-RuntimeNotInUse
Remove-DirectoryTree -Path $fullStagingRuntime
Remove-DirectoryTree -Path $fullPreviousRuntime

$zhihuRuntime = Join-Path $stagingRuntime 'zhihu'
$podcastRuntime = Join-Path $stagingRuntime 'podcast'

New-Item -ItemType Directory -Path (Join-Path $zhihuRuntime 'node') -Force | Out-Null
Copy-Item -LiteralPath $node -Destination (Join-Path $zhihuRuntime 'node\node.exe')
Copy-Tree -Source $zhihuSource -Destination (Join-Path $zhihuRuntime 'app') `
    -ExcludeDirectories $ZhihuAppExcludedDirectories `
    -ExcludeFiles $ZhihuAppExcludedFiles
Prune-DevDependencies -AppDir (Join-Path $zhihuRuntime 'app')
Copy-Tree -Source $EdgeRoot -Destination (Join-Path $zhihuRuntime 'chromium')
Copy-Tree -Source $contractsSource -Destination (Join-Path $stagingRuntime 'packages\contracts') `
    -ExcludeDirectories @('.git', 'node_modules')

Copy-Tree -Source $podcastAppSource -Destination (Join-Path $podcastRuntime 'app') `
    -ExcludeDirectories @('.git', '.venv', 'models', 'input', 'output', 'work', '.pytest_cache', '__pycache__') `
    -ExcludeFiles @('config.json', '*.log', '*.pyc')
Copy-Tree -Source $PythonRoot -Destination (Join-Path $podcastRuntime 'python') `
    -ExcludeDirectories @((Join-Path $PythonRoot 'Lib\site-packages'), (Join-Path $PythonRoot 'Scripts'), (Join-Path $PythonRoot 'Doc'), (Join-Path $PythonRoot 'testing'))
Copy-Tree -Source $podcastSitePackages `
    -Destination (Join-Path $podcastRuntime 'python\Lib\site-packages')
New-Item -ItemType Directory -Path (Join-Path $podcastRuntime 'ffmpeg') -Force | Out-Null
Copy-Item -LiteralPath $ffmpeg -Destination (Join-Path $podcastRuntime 'ffmpeg\ffmpeg.exe')
Copy-Item -LiteralPath $ffprobe -Destination (Join-Path $podcastRuntime 'ffmpeg\ffprobe.exe')
Copy-Tree -Source $podcastModels -Destination (Join-Path $podcastRuntime 'models')

# The staged binaries must actually run before the swap: a truncated copy
# would otherwise only fail at first launch, after the previous runtime was
# already replaced.
Assert-ZhihuNode -NodeExe (Join-Path $stagingRuntime 'zhihu\node\node.exe') -Label 'vendored'
Assert-PythonRuntime -PythonExe (Join-Path $stagingRuntime 'podcast\python\python.exe') -Label 'vendored'
Assert-MediaTool -Exe (Join-Path $stagingRuntime 'podcast\ffmpeg\ffmpeg.exe') -Label 'vendored ffmpeg'
Assert-MediaTool -Exe (Join-Path $stagingRuntime 'podcast\ffmpeg\ffprobe.exe') -Label 'vendored ffprobe'

Write-CriticalRuntimeManifest -RuntimeRoot $stagingRuntime -Components (Get-RuntimeComponents `
    -NodeSourceExe $node `
    -PythonSourceExe (Join-Path $PythonRoot 'python.exe') `
    -FfmpegSourceExe $ffmpeg `
    -FfprobeSourceExe $ffprobe `
    -ChromiumSourceExe (Join-Path $EdgeRoot 'msedge.exe') `
    -ModelsSource $podcastModels)

# Swap staging in only after it is fully built and manifest-verified: move the
# old runtime aside first so a failed swap can be rolled back, then delete the
# previous tree. Never delete the live runtime before staging is ready.
Assert-RuntimeNotInUse
$hadCurrent = Test-Path -LiteralPath $fullRuntime
if ($hadCurrent) {
    Move-Item -LiteralPath $fullRuntime -Destination $fullPreviousRuntime
}
try {
    Move-Item -LiteralPath $fullStagingRuntime -Destination $fullRuntime
} catch {
    if ($hadCurrent -and
        (Test-Path -LiteralPath $fullPreviousRuntime) -and
        -not (Test-Path -LiteralPath $fullRuntime)) {
        Move-Item -LiteralPath $fullPreviousRuntime -Destination $fullRuntime
    }
    throw
}
if (Test-Path -LiteralPath $fullPreviousRuntime) {
    try {
        # Remove-DirectoryTree refuses to follow a reparse point — it removes
        # only the link, never the junction target's contents.
        Remove-DirectoryTree -Path $fullPreviousRuntime
    } catch {
        Write-Warning "旧运行时目录清理失败（可手动删除）：$fullPreviousRuntime — $($_.Exception.Message)"
    }
}
Write-Output "[runtime] prepared $runtime"
