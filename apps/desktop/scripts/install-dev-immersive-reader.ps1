[CmdletBinding()]
param(
  [switch]$Build,
  [switch]$ValidateOnly,
  [switch]$NoShortcuts
)

$ErrorActionPreference = "Stop"
$desktopRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$monorepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$installDir = Join-Path $monorepoRoot ".dev-install"
$sourceRuntime = Join-Path $monorepoRoot "runtime"
$targetRuntime = Join-Path $installDir "runtime"
$devExecutable = Join-Path $installDir "immersive-reader-dev.exe"

function Invoke-CheckedCommand {
  param([string]$FilePath, [string[]]$Arguments)
  & $FilePath @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$FilePath failed with exit code $LASTEXITCODE"
  }
}

function Get-CargoTargetDirectory {
  $fallback = Join-Path $desktopRoot "src-tauri\target"
  try {
    $metadataJson = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $desktopRoot "src-tauri\Cargo.toml")
    if ($LASTEXITCODE -ne 0) { return $fallback }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    if ($metadata.target_directory) { return [string]$metadata.target_directory }
  } catch {
    return $fallback
  }
  return $fallback
}

function Test-RuntimeJunction {
  if (-not (Test-Path -LiteralPath $targetRuntime)) { return $false }
  $item = Get-Item -LiteralPath $targetRuntime -Force
  if ($item.LinkType -ne "Junction") { return $false }
  $resolvedTarget = [IO.Path]::GetFullPath([string]$item.Target)
  return $resolvedTarget -eq [IO.Path]::GetFullPath($sourceRuntime)
}

if (-not (Test-Path -LiteralPath (Join-Path $sourceRuntime "manifest.json"))) {
  throw "Managed runtime is missing or incomplete: $sourceRuntime"
}

if ($ValidateOnly) {
  [ordered]@{
    installDir = $installDir
    executable = $devExecutable
    runtimeSource = $sourceRuntime
    runtimeJunctionValid = Test-RuntimeJunction
    productionExecutable = Join-Path $monorepoRoot "immersive-reader.exe"
    registersMarkdownAssociations = $false
  } | ConvertTo-Json
  exit 0
}

Set-Location $desktopRoot
if ($Build) {
  Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments @(
    "run", "tauri", "build", "--", "--no-bundle", "--config", "src-tauri/tauri.dev.conf.json"
  )
}

$sourceExecutable = Join-Path (Get-CargoTargetDirectory) "release\immersive-reader.exe"
if (-not (Test-Path -LiteralPath $sourceExecutable)) {
  throw "Development executable was not built: $sourceExecutable"
}

New-Item -ItemType Directory -Path $installDir -Force | Out-Null
if (Test-Path -LiteralPath $targetRuntime) {
  if (-not (Test-RuntimeJunction)) {
    throw "Refusing to replace an unexpected development runtime path: $targetRuntime"
  }
} else {
  New-Item -ItemType Junction -Path $targetRuntime -Target $sourceRuntime | Out-Null
}

$temporaryExecutable = "$devExecutable.tmp"
Copy-Item -LiteralPath $sourceExecutable -Destination $temporaryExecutable -Force
Move-Item -LiteralPath $temporaryExecutable -Destination $devExecutable -Force

if (-not $NoShortcuts) {
  $shell = New-Object -ComObject WScript.Shell
  foreach ($shortcutPath in @(
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "沉浸阅读（开发版）.lnk"),
    (Join-Path ([Environment]::GetFolderPath("Programs")) "沉浸阅读（开发版）.lnk")
  )) {
    $shortcutDir = Split-Path -Parent $shortcutPath
    if (-not (Test-Path -LiteralPath $shortcutDir)) {
      New-Item -ItemType Directory -Path $shortcutDir -Force | Out-Null
    }
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $devExecutable
    $shortcut.WorkingDirectory = $installDir
    $shortcut.IconLocation = "$devExecutable,0"
    $shortcut.Save()
  }
}

$installed = Get-Item -LiteralPath $devExecutable
$hash = Get-FileHash -LiteralPath $devExecutable -Algorithm SHA256
[ordered]@{
  executable = $installed.FullName
  timestamp = $installed.LastWriteTime.ToString("yyyy-MM-dd HH:mm:ss")
  sha256 = $hash.Hash
  runtime = $sourceRuntime
  registersMarkdownAssociations = $false
} | ConvertTo-Json
