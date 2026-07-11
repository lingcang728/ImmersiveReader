param(
    [switch]$DryRun,
    [Parameter(Mandatory)][string]$SourceRoot,
    [string]$LibraryRoot = (Join-Path $env:USERPROFILE 'Documents\沉浸阅读\Library'),
    [string]$RuntimeRoot = (Join-Path $env:LOCALAPPDATA 'ImmersiveReader\zhihu')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'common.ps1')

function Get-Hash {
    param([Parameter(Mandatory)][string]$Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Test-IncludedArchiveFile {
    param([Parameter(Mandatory)][System.IO.FileInfo]$File)
    return $File.Name -notin @('reader.html', 'universal-reader.html', 'manifest.json', '.reading.json')
}

function Get-SourceFiles {
    param([Parameter(Mandatory)][string]$Root)
    if (-not (Test-Path -LiteralPath $Root)) {
        return @()
    }
    return @(Get-ChildItem -LiteralPath $Root -Recurse -File | Where-Object { Test-IncludedArchiveFile -File $_ })
}

function Copy-TreeSafely {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][bool]$Preview
    )

    $files = Get-SourceFiles -Root $Source
    $conflicts = [System.Collections.Generic.List[string]]::new()
    $pending = [System.Collections.Generic.List[object]]::new()
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($Source.TrimEnd('\\').Length).TrimStart('\\')
        $target = Join-Path $Destination $relative
        if (Test-Path -LiteralPath $target) {
            if ((Get-Hash -Path $file.FullName) -ne (Get-Hash -Path $target)) {
                $conflicts.Add($relative)
            }
        } else {
            $pending.Add([pscustomobject]@{ Source = $file.FullName; Target = $target; Relative = $relative })
        }
    }
    if ($conflicts.Count -gt 0) {
        throw "目标存在不同内容，迁移已停止：$($conflicts -join ', ')"
    }
    if (-not $Preview) {
        foreach ($item in $pending) {
            $parent = Split-Path -Parent $item.Target
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
            Copy-Item -LiteralPath $item.Source -Destination $item.Target
        }
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($Source.TrimEnd('\\').Length).TrimStart('\\')
            $target = Join-Path $Destination $relative
            if (-not (Test-Path -LiteralPath $target)) {
                throw "迁移后缺少文件：$relative"
            }
            if ((Get-Hash -Path $file.FullName) -ne (Get-Hash -Path $target)) {
                throw "迁移后哈希不一致：$relative"
            }
        }
    }
    return [pscustomobject]@{
        Source = $Source
        Destination = $Destination
        Total = $files.Count
        Pending = $pending.Count
        Existing = $files.Count - $pending.Count
        Conflicts = $conflicts.Count
    }
}

function Copy-SingleFileSafely {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][bool]$Preview
    )
    if (-not (Test-Path -LiteralPath $Source)) {
        throw "源文件不存在：$Source"
    }
    if (Test-Path -LiteralPath $Destination) {
        if ((Get-Hash -Path $Source) -ne (Get-Hash -Path $Destination)) {
            throw "目标文件已存在且内容不同：$Destination"
        }
        return 'existing'
    }
    if (-not $Preview) {
        New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
        Copy-Item -LiteralPath $Source -Destination $Destination
        if ((Get-Hash -Path $Source) -ne (Get-Hash -Path $Destination)) {
            throw "复制后哈希不一致：$Destination"
        }
    }
    return 'pending'
}

if (-not (Test-Path -LiteralPath $SourceRoot)) {
    throw "知乎源项目不存在：$SourceRoot"
}

$sourceOutput = Join-Path $SourceRoot 'output'
$targetOutput = Join-Path $LibraryRoot '知乎'
$sourceDb = Join-Path $SourceRoot 'zhihu-packer.db'
$targetDb = Join-Path $RuntimeRoot 'zhihu-packer.db'
$sourceProfile = Join-Path $SourceRoot '.browser-profile'
$targetProfile = Join-Path $RuntimeRoot 'browser-profile'
$reports = [System.Collections.Generic.List[object]]::new()

foreach ($book in Get-ChildItem -LiteralPath $sourceOutput -Directory) {
    $reports.Add((Copy-TreeSafely -Source $book.FullName -Destination (Join-Path $targetOutput $book.Name) -Preview $DryRun.IsPresent))
}

$dbState = Copy-SingleFileSafely -Source $sourceDb -Destination $targetDb -Preview $DryRun.IsPresent
if (Test-Path -LiteralPath $sourceProfile) {
    $reports.Add((Copy-TreeSafely -Source $sourceProfile -Destination $targetProfile -Preview $DryRun.IsPresent))
}

$root = Get-RepoRoot
$npm = Require-Command -Name 'npm.cmd'
$env:IMMERSIVE_ZHIHU_OUTPUT = if ($DryRun) { $sourceOutput } else { $targetOutput }
$env:IMMERSIVE_ZHIHU_DB = if ($DryRun) { $sourceDb } else { $targetDb }
$arguments = @('--prefix', (Join-Path $root 'tools\zhihu-packer'), 'run', 'build-manifests')
if ($DryRun) {
    $arguments += @('--', '--dry-run')
}
& $npm @arguments
if ($LASTEXITCODE -ne 0) {
    throw "manifest 生成检查失败，退出码 $LASTEXITCODE"
}

if (-not $DryRun) {
    & $npm --prefix (Join-Path $root 'tools\zhihu-packer') run build-reader:from-manifests
    if ($LASTEXITCODE -ne 0) {
        throw "Reader 构建失败，退出码 $LASTEXITCODE"
    }
}

$report = [ordered]@{
    timestamp = (Get-Date).ToString('o')
    dryRun = $DryRun.IsPresent
    sourceRoot = $SourceRoot
    libraryRoot = $LibraryRoot
    runtimeRoot = $RuntimeRoot
    database = $dbState
    trees = @($reports)
}
$report | ConvertTo-Json -Depth 6

if (-not $DryRun) {
    $reportDir = Join-Path $root 'artifacts\migration'
    New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $reportDir 'latest.json') -Encoding UTF8
}
