[CmdletBinding()]
param(
    [string]$PodcastSource = '',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot
$runtime = Join-Path $root 'runtime\podcast'
$target = Join-Path $env:LOCALAPPDATA 'ImmersiveReader\podcast'
$sourceConfig = if ($PodcastSource) { Join-Path $PodcastSource 'config.json' } else { '' }
$targetConfig = Join-Path $target 'config.json'

foreach ($required in @(
    (Join-Path $runtime 'python\python.exe'),
    (Join-Path $runtime 'ffmpeg\ffmpeg.exe'),
    (Join-Path $runtime 'models'),
    (Join-Path $runtime 'app\config.example.json')
)) {
    if (-not (Test-Path -LiteralPath $required)) {
        throw "受管 Podcast 运行时不完整：$required"
    }
}

foreach ($name in @('input', 'output', 'work')) {
    New-Item -ItemType Directory -Path (Join-Path $target $name) -Force | Out-Null
}

if ((Test-Path -LiteralPath $targetConfig) -and -not $Force) {
    Write-Output '[migration] managed Podcast config already exists; kept unchanged'
} else {
    $source = if ($sourceConfig -and (Test-Path -LiteralPath $sourceConfig)) {
        $sourceConfig
    } else {
        Join-Path $runtime 'app\config.example.json'
    }
    $raw = Get-Content -Raw -LiteralPath $source
    $null = $raw | ConvertFrom-Json
    $temporary = "$targetConfig.tmp"
    Copy-Item -LiteralPath $source -Destination $temporary -Force
    Move-Item -LiteralPath $temporary -Destination $targetConfig -Force
    Write-Output '[migration] Podcast config migrated without exposing its contents'
}

$configDocument = Get-Content -Raw -LiteralPath $targetConfig | ConvertFrom-Json
if ($configDocument.asr -and $configDocument.asr.model) {
    $configuredModel = [string]$configDocument.asr.model
    if ([IO.Path]::IsPathRooted($configuredModel)) {
        $configDocument.asr.model = [IO.Path]::GetFileName($configuredModel.TrimEnd('\', '/'))
        $temporary = "$targetConfig.tmp"
        $serialized = $configDocument | ConvertTo-Json -Depth 100
        [IO.File]::WriteAllText($temporary, $serialized, (New-Object Text.UTF8Encoding($false)))
        Move-Item -LiteralPath $temporary -Destination $targetConfig -Force
        Write-Output '[migration] Podcast model reference normalized for managed runtime'
    }
}

foreach ($name in @('input', 'output')) {
    if (-not $PodcastSource) { break }
    $source = Join-Path $PodcastSource $name
    if (Test-Path -LiteralPath $source) {
        & robocopy $source (Join-Path $target $name) /E /COPY:DAT /DCOPY:DAT /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
        if ($LASTEXITCODE -gt 7) {
            throw "Podcast $name 数据迁移失败（robocopy $LASTEXITCODE）"
        }
    }
}

$config = Get-Item -LiteralPath $targetConfig
Write-Output "[migration] managed config bytes=$($config.Length)"
