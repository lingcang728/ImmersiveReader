[CmdletBinding()]
param(
  [switch]$Build,
  [switch]$RegisterMarkdownAssociations,
  [switch]$NoShortcuts,
  # Default: monorepo root (easy to find and delete with the project).
  [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"
$desktopRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
# apps/desktop/scripts -> ImmersiveReader monorepo root
$monorepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not $InstallDir) {
  $InstallDir = $monorepoRoot
}
$InstallDir = [System.IO.Path]::GetFullPath($InstallDir)
Set-Location $desktopRoot

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
  } catch { return $fallback }
  return $fallback
}

if ($Build) {
  Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments @("run", "tauri", "build", "--", "--no-sign", "--bundles", "nsis")
}

$bundleDir = Join-Path (Get-CargoTargetDirectory) "release\bundle\nsis"
$installer = Get-ChildItem -LiteralPath $bundleDir -Filter "沉浸阅读_*_x64-setup.exe" -File |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if (-not $installer) {
  throw "No 沉浸阅读 NSIS installer found in $bundleDir"
}

if (-not (Test-Path -LiteralPath $InstallDir)) {
  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$sourceRuntime = Join-Path $monorepoRoot 'runtime'
if (-not (Test-Path -LiteralPath (Join-Path $sourceRuntime 'manifest.json'))) {
  throw "Managed runtime is missing. Run scripts\prepare-runtime.ps1 first."
}
$targetRuntime = Join-Path $InstallDir 'runtime'
if ([System.IO.Path]::GetFullPath($sourceRuntime) -ne [System.IO.Path]::GetFullPath($targetRuntime)) {
  & robocopy $sourceRuntime $targetRuntime /MIR /COPY:DAT /DCOPY:DAT /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
  if ($LASTEXITCODE -gt 7) { throw "Runtime install failed with robocopy exit code $LASTEXITCODE" }
}

$installerHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installer.FullName
Write-Host "Installing $($installer.FullName)"
Write-Host "Installer SHA-256: $($installerHash.Hash)"
Write-Host "Install directory: $InstallDir"
# NSIS: /D must be last and must not be quoted, even with spaces.
$nsisArgs = "/S /D=$InstallDir"
$process = Start-Process -FilePath $installer.FullName -ArgumentList $nsisArgs -Wait -PassThru -WindowStyle Hidden
if ($process.ExitCode -ne 0) { throw "Installer failed with exit code $($process.ExitCode)" }

$installedExe = Join-Path $InstallDir "immersive-reader.exe"
if (-not (Test-Path -LiteralPath $installedExe)) {
  throw "Installed executable not found: $installedExe"
}
foreach ($required in @(
  'runtime\zhihu\node\node.exe',
  'runtime\zhihu\chromium\msedge.exe',
  'runtime\podcast\python\python.exe',
  'runtime\podcast\ffmpeg\ffmpeg.exe',
  'runtime\podcast\models'
)) {
  if (-not (Test-Path -LiteralPath (Join-Path $InstallDir $required))) {
    throw "Installed runtime is incomplete: $required"
  }
}

if ($RegisterMarkdownAssociations) {
  $progId = "ImmersiveReader.Markdown"
  $openCommand = "`"$installedExe`" `"%1`""
  foreach ($extension in @(".md", ".markdown")) {
    New-Item -Path "HKCU:\Software\Classes\$extension" -Force | Out-Null
    Set-Item -Path "HKCU:\Software\Classes\$extension" -Value $progId
  }
  New-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Force | Out-Null
  Set-Item -Path "HKCU:\Software\Classes\$progId" -Value "Markdown Document"
  Set-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Value $openCommand
  Write-Host "Markdown associations registered after validation."
} else {
  Write-Host "Markdown associations were intentionally left unchanged."
}

if (-not $NoShortcuts) {
  $shell = New-Object -ComObject WScript.Shell
  foreach ($shortcutPath in @(
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "沉浸阅读.lnk"),
    (Join-Path ([Environment]::GetFolderPath("Programs")) "沉浸阅读.lnk")
  )) {
    $shortcutDir = Split-Path -Parent $shortcutPath
    if (-not (Test-Path -LiteralPath $shortcutDir)) { New-Item -ItemType Directory -Path $shortcutDir -Force | Out-Null }
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $installedExe
    $shortcut.WorkingDirectory = $InstallDir
    $shortcut.IconLocation = "$installedExe,0"
    $shortcut.Save()
  }
}

$installed = Get-Item -LiteralPath $installedExe
$installedHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe
Write-Host "Installed EXE: $installedExe"
Write-Host "Timestamp: $($installed.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
Write-Host "Product version: $($installed.VersionInfo.ProductVersion)"
Write-Host "SHA-256: $($installedHash.Hash)"
