[CmdletBinding()]
param(
    [string]$PodcastSource = '',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

# Same sanitization as migrate-v3-production-data.ps1: a legacy config may carry
# a plaintext api_key, and it must never land on disk in the managed location.
function Remove-SecretProperties {
    param($Value)
    if ($null -eq $Value -or $Value -is [string] -or $Value.GetType().IsPrimitive) { return }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [pscustomobject]) {
        foreach ($item in $Value) { Remove-SecretProperties -Value $item }
        return
    }
    $properties = @($Value.PSObject.Properties)
    foreach ($property in $properties) {
        if ($property.Name -match '^(?i:api_?key|deepseek_?api_?key)$') {
            $Value.PSObject.Properties.Remove($property.Name)
        } else {
            Remove-SecretProperties -Value $property.Value
        }
    }
}

$root = Get-RepoRoot
$runtime = Join-Path $root 'runtime\podcast'
# Matches storage.rs/worker.rs: the managed Podcast data root is Data\Podcast
# (config.json at its root); the old top-level "podcast" dir was pre-v3.
$target = Join-Path $env:LOCALAPPDATA 'ImmersiveReader\Data\Podcast'
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

# The worker owns input/output/work under its per-task CACHE root — nothing
# reads them under the data root, so no directories are pre-created here.

if ((Test-Path -LiteralPath $targetConfig) -and -not $Force) {
    Write-Output '[migration] managed Podcast config already exists; kept unchanged'
} else {
    $source = if ($sourceConfig -and (Test-Path -LiteralPath $sourceConfig)) {
        $sourceConfig
    } else {
        Join-Path $runtime 'app\config.example.json'
    }
    $raw = Get-Content -Raw -LiteralPath $source
    $configToWrite = $raw | ConvertFrom-Json
    $hadSecret = $raw -match '(?i)"(?:api_?key|deepseek_?api_?key)"\s*:\s*"[^"\s][^"]*"'
    Remove-SecretProperties -Value $configToWrite
    $temporary = "$targetConfig.tmp"
    $configToWrite | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $temporary -Encoding utf8
    Move-Item -LiteralPath $temporary -Destination $targetConfig -Force
    if ((Get-Content -Raw -LiteralPath $targetConfig) -match '(?i)"(?:api_?key|deepseek_?api_?key)"') {
        throw '[migration] managed Podcast config still contains a key field'
    }
    if ($hadSecret) {
        Write-Warning '[migration] api_key was stripped from the migrated Podcast config; re-enter it in app settings (stored in Windows Credential Manager).'
    }
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
        # Legacy queue/output dirs are preserved under LegacyOutput (same
        # convention as migrate-v3-production-data.ps1) — the live worker reads
        # only its per-task cache root, never a top-level input/output here.
        & robocopy $source (Join-Path $target "LegacyOutput\$name") /E /COPY:DAT /DCOPY:DAT /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
        if ($LASTEXITCODE -gt 7) {
            throw "Podcast $name 数据迁移失败（robocopy $LASTEXITCODE）"
        }
    }
}

$config = Get-Item -LiteralPath $targetConfig
Write-Output "[migration] managed config bytes=$($config.Length)"
