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
$podcastSitePackages = if ($PodcastSource) {
    Join-Path $PodcastSource '.venv\Lib\site-packages'
} else {
    Join-Path $runtime 'podcast\python\Lib\site-packages'
}
$podcastModels = if ($PodcastSource) {
    Join-Path $PodcastSource 'models'
} else {
    Join-Path $runtime 'podcast\models'
}
$node = Require-Command -Name 'node.exe'
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

function Reset-AppDestination {
    param([Parameter(Mandatory)][string]$Destination)

    $fullDestination = [IO.Path]::GetFullPath($Destination)
    $allowedRoot = $fullRuntime.TrimEnd('\') + [IO.Path]::DirectorySeparatorChar
    if (-not $fullDestination.StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "拒绝清理受管运行时外的应用目录：$fullDestination"
    }
    if (Test-Path -LiteralPath $fullDestination) {
        Remove-Item -LiteralPath $fullDestination -Recurse -Force
    }
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
    param([Parameter(Mandatory)][string]$RuntimeRoot)

    $fullRuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot).TrimEnd('\')
    $entries = @(Get-CriticalRuntimeFiles -RuntimeRoot $fullRuntimeRoot | ForEach-Object {
        $item = Get-Item -LiteralPath $_.FullName
        [ordered]@{
            path = $item.FullName.Substring($fullRuntimeRoot.Length).TrimStart('\').Replace('\', '/')
            bytes = $item.Length
            sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash
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
    $manifestPath = Join-Path $fullRuntimeRoot 'manifest.json'
    $temporaryPath = "$manifestPath.tmp"
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $temporaryPath -Encoding UTF8
    Move-Item -LiteralPath $temporaryPath -Destination $manifestPath -Force
    Write-Output "[runtime] critical manifest contains $($entries.Count) entries"
}

if ($RefreshApps) {
    foreach ($requiredRuntime in @(
        (Join-Path $runtime 'zhihu\node\node.exe'),
        (Join-Path $runtime 'podcast\python\python.exe'),
        (Join-Path $runtime 'podcast\models'),
        (Join-Path $contractsSource 'dist\index.js')
    )) {
        Require-Path -Path $requiredRuntime
    }
    $zhihuApp = Join-Path $runtime 'zhihu\app'
    $podcastApp = Join-Path $runtime 'podcast\app'
    $contractsRuntime = Join-Path $runtime 'packages\contracts'
    Reset-AppDestination -Destination $zhihuApp
    Reset-AppDestination -Destination $podcastApp
    Reset-AppDestination -Destination $contractsRuntime
    Copy-Tree -Source $zhihuSource -Destination $zhihuApp `
        -ExcludeDirectories @('.git', '.browser-profile', '.obscura-profile') `
        -ExcludeFiles @('*.log', '*.db', '*.db-*')
    Copy-Tree -Source $podcastAppSource -Destination $podcastApp `
        -ExcludeDirectories @('.git', '.venv', 'models', 'input', 'output', 'work', '.pytest_cache', '__pycache__') `
        -ExcludeFiles @('config.json', '*.log', '*.pyc')
    Copy-Tree -Source $contractsSource -Destination $contractsRuntime `
        -ExcludeDirectories @('.git', 'node_modules')
    Write-CriticalRuntimeManifest -RuntimeRoot $runtime
    Write-Output '[runtime] application code refreshed without rebuilding large assets'
    exit 0
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

if ($ValidateOnly) {
    Write-Output '[runtime] all reusable sources are available'
    $required | ForEach-Object { Write-Output "[runtime] $_" }
    exit 0
}

$stagingRuntime = "$runtime.__staging__"
$fullStagingRuntime = [IO.Path]::GetFullPath($stagingRuntime)
$fullRoot = [IO.Path]::GetFullPath($root) + [IO.Path]::DirectorySeparatorChar
if (-not $fullRuntime.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase) -or
    -not $fullStagingRuntime.StartsWith($fullRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝清理工作区外的运行时目录：$fullRuntime"
}
if (Test-Path -LiteralPath $fullStagingRuntime) {
    Remove-Item -LiteralPath $fullStagingRuntime -Recurse -Force
}

$zhihuRuntime = Join-Path $stagingRuntime 'zhihu'
$podcastRuntime = Join-Path $stagingRuntime 'podcast'

New-Item -ItemType Directory -Path (Join-Path $zhihuRuntime 'node') -Force | Out-Null
Copy-Item -LiteralPath $node -Destination (Join-Path $zhihuRuntime 'node\node.exe')
Copy-Tree -Source $zhihuSource -Destination (Join-Path $zhihuRuntime 'app') `
    -ExcludeDirectories @('.git', '.browser-profile', '.obscura-profile') `
    -ExcludeFiles @('*.log', '*.db', '*.db-*')
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

Write-CriticalRuntimeManifest -RuntimeRoot $stagingRuntime
if (Test-Path -LiteralPath $fullRuntime) {
    Remove-Item -LiteralPath $fullRuntime -Recurse -Force
}
Move-Item -LiteralPath $fullStagingRuntime -Destination $fullRuntime
Write-Output "[runtime] prepared $runtime"
