# verify-runtime-bundle.ps1 — the consumer half ADR-001 requires and the
# release flow was missing: prove that a set of runtime-bundle volumes really
# is THIS tag's runtime, not a stale one re-uploaded onto a new draft.
#
#   parts (.001..) → per-part sha256 + size vs runtime-parts.json
#                 → concatenate → bundle sha256 vs manifest
#                 → extract → verify-runtime.ps1 (self-integrity)
#                 → app-code files hash-compared against repo sources
#                   (same five pairs install-latest.ps1 asserts at ship time)
#
# Usage:
#   pwsh scripts\verify-runtime-bundle.ps1 -PartsDirectory .\output\runtime -Version 1.2.1
#   # or against a downloaded release asset set:
#   pwsh scripts\verify-runtime-bundle.ps1 -PartsDirectory <dir with .001..> -PartsManifest <runtime-parts.json>

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PartsDirectory,
    [string]$PartsManifest = '',
    [string]$Version = '',
    [string]$WorkDirectory = '',
    [switch]$KeepExtracted
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot
$PartsDirectory = [IO.Path]::GetFullPath($PartsDirectory)
if (-not $Version) {
    $Version = [string](Get-Content -LiteralPath (Join-Path $root 'apps\desktop\package.json') -Raw | ConvertFrom-Json).version
}
if (-not $PartsManifest) {
    $PartsManifest = Join-Path $root "docs\release\$Version\runtime-parts.json"
}
if (-not (Test-Path -LiteralPath $PartsManifest -PathType Leaf)) {
    throw "runtime-parts.json 不存在：$PartsManifest"
}
$expected = Get-Content -LiteralPath $PartsManifest -Raw | ConvertFrom-Json

# Step 1: every declared volume exists with the exact size and sha256.
$partPaths = @()
foreach ($part in @($expected.parts)) {
    $path = Join-Path $PartsDirectory ([string]$part.name)
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "runtime 分卷缺失：$($part.name)"
    }
    $item = Get-Item -LiteralPath $path
    if ([int64]$item.Length -ne [int64]$part.size) {
        throw "runtime 分卷大小不匹配：$($part.name)（期望 $($part.size)，实际 $($item.Length)）"
    }
    $sha = (Get-FileSha256Hex -Path $path).ToLowerInvariant()
    if ($sha -cne ([string]$part.sha256).ToLowerInvariant()) {
        throw "runtime 分卷 SHA-256 不匹配：$($part.name)"
    }
    $partPaths += $path
    Write-Output "[bundle-verify] part ok $($part.name)"
}
$expectedTotal = (@($expected.parts) | Measure-Object -Property size -Sum).Sum
if ([int64]$expectedTotal -ne [int64]$expected.bundle.size) {
    throw 'runtime-parts.json 分卷总大小与 bundle.size 不一致。'
}

# Step 2: reconstruct the bundle and verify the WHOLE-archive hash — matching
# every part individually is not the same as matching the bundle (this is the
# field release.yml's digest check cannot cover).
$work = if ($WorkDirectory) {
    [IO.Path]::GetFullPath($WorkDirectory)
} else {
    Join-Path ([IO.Path]::GetTempPath()) ("immersive-bundle-verify-" + [guid]::NewGuid().ToString('N'))
}
New-Item -ItemType Directory -Path $work -Force | Out-Null
$bundlePath = Join-Path $work ([string]$expected.bundle.name)
$extractRoot = Join-Path $work 'extracted'
try {
    Write-Output '[bundle-verify] concatenating volumes'
    $outStream = [IO.File]::Create($bundlePath)
    try {
        $buffer = New-Object byte[] (8MB)
        foreach ($partPath in $partPaths) {
            $inStream = [IO.File]::OpenRead($partPath)
            try {
                while (($read = $inStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $outStream.Write($buffer, 0, $read)
                }
            } finally {
                $inStream.Dispose()
            }
        }
    } finally {
        $outStream.Dispose()
    }
    $bundleItem = Get-Item -LiteralPath $bundlePath
    if ([int64]$bundleItem.Length -ne [int64]$expected.bundle.size) {
        throw "合并后 bundle 大小不匹配：$($bundleItem.Length) != $($expected.bundle.size)"
    }
    $bundleSha = (Get-FileSha256Hex -Path $bundlePath).ToLowerInvariant()
    if ($bundleSha -cne ([string]$expected.bundle.sha256).ToLowerInvariant()) {
        throw "合并后 bundle SHA-256 不匹配：$bundleSha != $($expected.bundle.sha256)"
    }
    Write-Output "[bundle-verify] bundle sha256 ok ($bundleSha)"

    # Step 3: extract and run the managed-runtime self-integrity gate.
    Write-Output '[bundle-verify] extracting bundle'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($bundlePath, $extractRoot)
    $runtimeRoot = if (Test-Path -LiteralPath (Join-Path $extractRoot 'runtime\manifest.json') -PathType Leaf) {
        Join-Path $extractRoot 'runtime'
    } elseif (Test-Path -LiteralPath (Join-Path $extractRoot 'manifest.json') -PathType Leaf) {
        $extractRoot
    } else {
        throw '解压结果缺少 runtime manifest.json（既不是 runtime\ 顶层也不是平铺布局）。'
    }
    & (Join-Path $PSScriptRoot 'verify-runtime.ps1') -RuntimeRoot $runtimeRoot

    # Step 4: the bundle's application code must equal this checkout's built
    # output — the check release.yml cannot do from GitHub digests alone.
    $pairs = @(
        @{ Name = 'Reader template'; Source = 'tools\zhihu-packer\dist\reader-template.html'; Runtime = 'zhihu\app\dist\reader-template.html' },
        @{ Name = 'Zhihu sidecar server'; Source = 'tools\zhihu-packer\dist\server.js'; Runtime = 'zhihu\app\dist\server.js' },
        @{ Name = 'Contracts runtime'; Source = 'packages\contracts\dist\index.js'; Runtime = 'packages\contracts\dist\index.js' },
        @{ Name = 'Podcast final markdown generator'; Source = 'tools\podcast-transcriber\scripts\polish_interview_markdown.py'; Runtime = 'podcast\app\scripts\polish_interview_markdown.py' },
        @{ Name = 'Podcast language classifier'; Source = 'tools\podcast-transcriber\scripts\podcast_transcriber\language.py'; Runtime = 'podcast\app\scripts\podcast_transcriber\language.py' }
    )
    foreach ($pair in $pairs) {
        $sourcePath = Join-Path $root $pair.Source
        $runtimePath = Join-Path $runtimeRoot $pair.Runtime
        if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
            throw "缺少源码构建产物：$($pair.Source)（先构建 contracts 与 zhihu-packer 再验证 bundle）。"
        }
        if (-not (Test-Path -LiteralPath $runtimePath -PathType Leaf)) {
            throw "bundle 缺少应用文件：$($pair.Runtime)"
        }
        if ((Get-FileSha256Hex -Path $sourcePath) -ine (Get-FileSha256Hex -Path $runtimePath)) {
            throw "bundle 应用代码与当前源码不一致：$($pair.Name)——分卷可能来自旧版本。"
        }
        Write-Output "[bundle-verify] source==bundle $($pair.Name)"
    }

    Write-Output "[bundle-verify] verified: $($partPaths.Count) part(s), bundle sha256, manifest integrity, app-code parity."
} finally {
    Remove-Item -LiteralPath $bundlePath -Force -ErrorAction SilentlyContinue
    if ($KeepExtracted) {
        Write-Output "[bundle-verify] extracted tree kept at $extractRoot"
    } else {
        Remove-DirectoryTree -Path $extractRoot
        if (-not $WorkDirectory) { Remove-DirectoryTree -Path $work }
    }
}
