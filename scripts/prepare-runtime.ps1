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
$zhihuSource = Join-Path $root 'tools\zhihu-packer'
$podcastAppSource = Join-Path $root 'tools\podcast-transcriber'
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

if ($RefreshApps) {
    foreach ($requiredRuntime in @(
        (Join-Path $runtime 'zhihu\node\node.exe'),
        (Join-Path $runtime 'podcast\python\python.exe'),
        (Join-Path $runtime 'podcast\models')
    )) {
        Require-Path -Path $requiredRuntime
    }
    Copy-Tree -Source $zhihuSource -Destination (Join-Path $runtime 'zhihu\app') `
        -ExcludeDirectories @('.git', '.browser-profile', '.obscura-profile') `
        -ExcludeFiles @('*.log', '*.db', '*.db-*')
    Copy-Tree -Source $podcastAppSource -Destination (Join-Path $runtime 'podcast\app') `
        -ExcludeDirectories @('.git', '.venv', 'models', 'input', 'output', 'work', '.pytest_cache', '__pycache__') `
        -ExcludeFiles @('config.json', '*.log', '*.pyc')
    Write-Output '[runtime] application code refreshed without rebuilding large assets'
    exit 0
}

$required = @(
    (Join-Path $zhihuSource 'dist\server.js'),
    (Join-Path $zhihuSource 'node_modules'),
    (Join-Path $podcastAppSource 'scripts\run_with_gui.py'),
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
$fullRuntime = [IO.Path]::GetFullPath($runtime)
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

$hashTargets = @(
    (Join-Path $zhihuRuntime 'node\node.exe'),
    (Join-Path $zhihuRuntime 'chromium\msedge.exe'),
    (Join-Path $podcastRuntime 'python\python.exe'),
    (Join-Path $podcastRuntime 'ffmpeg\ffmpeg.exe'),
    (Join-Path $podcastRuntime 'ffmpeg\ffprobe.exe')
) + @(Get-ChildItem -LiteralPath (Join-Path $podcastRuntime 'models') -File -Recurse | Select-Object -ExpandProperty FullName)

$manifest = $hashTargets | ForEach-Object {
    $item = Get-Item -LiteralPath $_
    [ordered]@{
        path = $item.FullName.Substring($stagingRuntime.Length).TrimStart('\\')
        bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash
    }
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $stagingRuntime 'manifest.json') -Encoding UTF8
if (Test-Path -LiteralPath $fullRuntime) {
    Remove-Item -LiteralPath $fullRuntime -Recurse -Force
}
Move-Item -LiteralPath $fullStagingRuntime -Destination $fullRuntime
Write-Output "[runtime] prepared $runtime"
