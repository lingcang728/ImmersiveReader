[CmdletBinding()]
param(
    [string]$RuntimeRoot = '',
    [string]$ManifestPath = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot
if (-not $RuntimeRoot) {
    $RuntimeRoot = Join-Path $root 'runtime'
}
$RuntimeRoot = [IO.Path]::GetFullPath($RuntimeRoot).TrimEnd('\')
if (-not $ManifestPath) {
    $ManifestPath = Join-Path $RuntimeRoot 'manifest.json'
}
$ManifestPath = [IO.Path]::GetFullPath($ManifestPath)

if (-not (Test-Path -LiteralPath $ManifestPath)) {
    throw "Managed runtime manifest is missing: $ManifestPath"
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ([int]$manifest.schemaVersion -lt 2) {
    throw "Managed runtime manifest is stale; schemaVersion 2 is required"
}
$entries = @($manifest.entries)
if ($entries.Count -eq 0 -or $entries.Count -ne [int]$manifest.entryCount) {
    throw "Managed runtime manifest entry count is invalid"
}

$seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$rootPrefix = $RuntimeRoot + [IO.Path]::DirectorySeparatorChar
foreach ($entry in $entries) {
    $relative = [string]$entry.path
    if ([IO.Path]::IsPathRooted($relative) -or $relative.Contains('..') -or $relative.Contains('\')) {
        throw "Managed runtime manifest path is unsafe: $relative"
    }
    if (-not $seen.Add($relative)) {
        throw "Managed runtime manifest contains duplicate path: $relative"
    }
    $path = [IO.Path]::GetFullPath((Join-Path $RuntimeRoot ($relative -replace '/', '\')))
    if (-not $path.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Managed runtime manifest path escapes runtime root: $relative"
    }
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Managed runtime critical file is missing: $relative"
    }
    $item = Get-Item -LiteralPath $path
    if ([int64]$item.Length -ne [int64]$entry.bytes) {
        throw "Managed runtime size mismatch: $relative"
    }
    $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
    if ($hash -ine [string]$entry.sha256) {
        throw "Managed runtime SHA-256 mismatch: $relative"
    }
}
Write-Output "[runtime] verified $($entries.Count) critical entries in $RuntimeRoot"
