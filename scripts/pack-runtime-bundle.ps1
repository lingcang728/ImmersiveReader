# pack-runtime-bundle.ps1 — produce the release runtime bundle end to end:
#   runtime\ → verify-runtime → runtime-bundle.zip → split volumes →
#   docs\release\<version>\runtime-parts.json
# This is the missing producer half of the release flow: release.yml only
# validates already-uploaded volumes against that manifest, so the manifest
# must be generated from the SAME verified runtime tree the bundle archives.
# The volumes it writes are the assets to upload to the draft GitHub Release.
#
# Usage:
#   pwsh scripts\pack-runtime-bundle.ps1                      # pack into output\runtime
#   pwsh scripts\pack-runtime-bundle.ps1 -KeepBundle          # keep the merged zip too

[CmdletBinding()]
param(
    [string]$RuntimeRoot = '',
    [string]$OutputDirectory = '',
    [string]$Version = '',
    # GitHub Release assets cap at 2 GiB; 1.75 GiB matches the volumes shipped
    # with 1.2.0 and leaves headroom for the uploader.
    [int64]$PartBytes = 1879048192,
    [switch]$KeepBundle
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot
if (-not $RuntimeRoot) { $RuntimeRoot = Join-Path $root 'runtime' }
$RuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot)
if (-not (Test-Path -LiteralPath (Join-Path $RuntimeRoot 'manifest.json') -PathType Leaf)) {
    throw "受管运行时未就绪：$RuntimeRoot 缺少 manifest.json（先运行 scripts\prepare-runtime.ps1）。"
}
Assert-NotReparsePoint -Path $RuntimeRoot -Label 'runtime 源目录'
if (-not $Version) {
    $Version = [string](Get-Content -LiteralPath (Join-Path $root 'apps\desktop\package.json') -Raw | ConvertFrom-Json).version
}
if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw "版本号格式异常：$Version" }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $root 'output\runtime' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$partsManifestPath = Join-Path $root "docs\release\$Version\runtime-parts.json"

# Gate 1: the tree being packed must match its own manifest first — packing a
# corrupted runtime and recording its hashes would self-consistently ship rot.
Write-Output '[bundle] verifying managed runtime manifest before packing'
& (Join-Path $PSScriptRoot 'verify-runtime.ps1') -RuntimeRoot $RuntimeRoot

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$bundlePath = Join-Path $OutputDirectory 'runtime-bundle.zip'
Remove-Item -LiteralPath $bundlePath -Force -ErrorAction SilentlyContinue
Get-ChildItem -LiteralPath $OutputDirectory -File -Filter 'runtime-bundle.zip.*' -ErrorAction SilentlyContinue |
    Remove-Item -Force

# Gate 2: compress. .NET ZipArchive writes ZIP64 as needed, which Compress-
# Archive on Windows PowerShell 5.1 does not reliably do past 4 GiB. The
# archive holds the runtime contents flat (zhihu\, podcast\, packages\,
# manifest.json at zip root) — docs\runtime-acquisition.md documents this
# layout, and verify-runtime-bundle.ps1 accepts either flat or runtime\-rooted.
Write-Output "[bundle] compressing $RuntimeRoot -> $bundlePath"
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory(
    $RuntimeRoot, $bundlePath,
    [System.IO.Compression.CompressionLevel]::Optimal, $false)

# Gate 3: the archive must contain every manifest-listed critical file. A
# torn enumeration here would otherwise ship a bundle that verifies its own
# manifest.json yet lacks the files the manifest describes — we saw exactly
# that once (dist\reader\{core,modes,ui} skipped wholesale), so assert it.
$bundleZip = [System.IO.Compression.ZipFile]::OpenRead($bundlePath)
try {
    $archived = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    # .NET Framework (Windows PowerShell 5.1) writes entry names with '\'
    # separators while .NET Core uses '/' — normalize before comparing with
    # the manifest's forward-slash paths.
    foreach ($zipEntry in $bundleZip.Entries) { [void]$archived.Add(($zipEntry.FullName -replace '\\', '/')) }
    $bundleManifest = Get-Content -LiteralPath (Join-Path $RuntimeRoot 'manifest.json') -Raw | ConvertFrom-Json
    $missing = @(
        @($bundleManifest.entries) |
            Where-Object { -not $archived.Contains([string]$_.path) } |
            Select-Object -First 10 -ExpandProperty path
    )
    if ($missing.Count -gt 0) {
        throw "bundle 缺少 manifest 所列文件（打包时源树可能被并发改动）：$($missing -join '；')……请重跑打包。"
    }
    Write-Output "[bundle] archive contains all $($bundleManifest.entryCount) manifest entries"
} finally {
    $bundleZip.Dispose()
}

$bundleItem = Get-Item -LiteralPath $bundlePath
$bundleSha = (Get-FileSha256Hex -Path $bundlePath).ToLowerInvariant()
Write-Output "[bundle] $($bundleItem.Length) bytes, sha256 $bundleSha"

# Split into numbered volumes via a bounded read loop — plain byte ranges, no
# format awareness needed.
Write-Output "[bundle] splitting into $PartBytes-byte volumes"
$parts = [System.Collections.Generic.List[object]]::new()
$stream = [IO.File]::OpenRead($bundlePath)
try {
    $index = 1
    $buffer = New-Object byte[] (8MB)
    while ($true) {
        $partName = 'runtime-bundle.zip.{0:d3}' -f $index
        $partPath = Join-Path $OutputDirectory $partName
        $partStream = [IO.File]::Create($partPath)
        $partSize = [int64]0
        try {
            while ($partSize -lt $PartBytes) {
                $want = [int][Math]::Min($buffer.Length, $PartBytes - $partSize)
                $read = $stream.Read($buffer, 0, $want)
                if ($read -le 0) { break }
                $partStream.Write($buffer, 0, $read)
                $partSize += $read
            }
        } finally {
            $partStream.Dispose()
        }
        if ($partSize -eq 0) {
            Remove-Item -LiteralPath $partPath -Force
            break
        }
        $parts.Add([ordered]@{
            name = $partName
            size = $partSize
            sha256 = (Get-FileSha256Hex -Path $partPath).ToLowerInvariant()
        })
        Write-Output "[bundle]   $partName — $partSize bytes"
        $index += 1
        if ($partSize -lt $PartBytes) { break }  # short write = EOF reached
    }
} finally {
    $stream.Dispose()
}

$partsManifest = [ordered]@{
    bundle = [ordered]@{
        name = 'runtime-bundle.zip'
        size = $bundleItem.Length
        sha256 = $bundleSha
    }
    parts = @($parts)
}
New-Item -ItemType Directory -Path (Split-Path -Parent $partsManifestPath) -Force | Out-Null
$partsManifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $partsManifestPath -Encoding UTF8
Write-Output "[bundle] wrote $partsManifestPath"

if (-not $KeepBundle) {
    Remove-Item -LiteralPath $bundlePath -Force
    Write-Output '[bundle] merged zip removed (pass -KeepBundle to retain it)'
}

Write-Output "[bundle] done: $($parts.Count) volume(s) in $OutputDirectory; upload them to the v$Version draft release, then run scripts\verify-runtime-bundle.ps1 against the uploaded set."
