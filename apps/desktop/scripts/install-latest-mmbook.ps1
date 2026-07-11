[CmdletBinding()]
param(
  [switch]$Build,
  [switch]$NoShortcuts,
  [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "MMbook")
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $repoRoot

function Invoke-CheckedCommand {
  param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath,
    [string[]]$Arguments = @()
  )

  & $FilePath @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "$FilePath $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
  }
}

if ($Build) {
  $localBuildConfig = Join-Path ([System.IO.Path]::GetTempPath()) "mmbook-tauri-local-build.json"
  Set-Content -LiteralPath $localBuildConfig -Encoding UTF8 -Value '{"bundle":{"createUpdaterArtifacts":false}}'
  try {
    Invoke-CheckedCommand -FilePath "npm" -Arguments @(
      "run",
      "tauri",
      "build",
      "--",
      "--no-sign",
      "--config",
      $localBuildConfig
    )
  } finally {
    Remove-Item -LiteralPath $localBuildConfig -Force -ErrorAction SilentlyContinue
  }
}

function Get-CargoTargetDirectory {
  $fallback = Join-Path $repoRoot "src-tauri\target"
  try {
    $metadataJson = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $repoRoot "src-tauri\Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
      return $fallback
    }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    if ($metadata.target_directory) {
      return [string]$metadata.target_directory
    }
  } catch {
    return $fallback
  }
  return $fallback
}

$bundleDir = Join-Path (Get-CargoTargetDirectory) "release\bundle\nsis"
if (-not (Test-Path $bundleDir)) {
  throw "NSIS bundle directory not found: $bundleDir. Run npm run tauri build first."
}

$installer = Get-ChildItem -Path $bundleDir -Filter "MMbook_*_x64-setup.exe" -File |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1

if (-not $installer) {
  throw "No MMbook NSIS installer found in $bundleDir. Run npm run tauri build first."
}

Write-Host "Installing latest MMbook package:"
Write-Host "  $($installer.FullName)"
Write-Host "  timestamp: $($installer.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
Write-Host "  sha256: $((Get-FileHash -Algorithm SHA256 -LiteralPath $installer.FullName).Hash)"

$process = Start-Process -FilePath $installer.FullName -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
if ($process.ExitCode -ne 0) {
  throw "Installer failed with exit code $($process.ExitCode)"
}

$installedExe = Join-Path $InstallDir "mmbook.exe"
if (-not (Test-Path $installedExe)) {
  throw "Installed MMbook executable not found: $installedExe"
}

function Set-DefaultValue {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [Parameter(Mandatory = $true)]
    [string]$Value
  )

  if (-not (Test-Path $Path)) {
    New-Item -Path $Path -Force | Out-Null
  }
  Set-Item -Path $Path -Value $Value
}

function Register-MarkdownProgId {
  param(
    [Parameter(Mandatory = $true)]
    [string]$Extension,
    [Parameter(Mandatory = $true)]
    [string]$ProgId,
    [Parameter(Mandatory = $true)]
    [string]$OpenCommand
  )

  Set-DefaultValue -Path "HKCU:\Software\Classes\$Extension" -Value $ProgId
  Set-DefaultValue -Path "HKCU:\Software\Classes\$ProgId" -Value "Markdown Document"
  Set-DefaultValue -Path "HKCU:\Software\Classes\$ProgId\shell\open\command" -Value $OpenCommand
}

$openCommand = "`"$installedExe`" `"%1`""
Register-MarkdownProgId -Extension ".md" -ProgId "md" -OpenCommand $openCommand
Register-MarkdownProgId -Extension ".markdown" -ProgId "markdown" -OpenCommand $openCommand

foreach ($extension in @(".md", ".markdown")) {
  $userChoicePath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\UserChoice"
  if (Test-Path $userChoicePath) {
    $userChoice = Get-ItemProperty -Path $userChoicePath -ErrorAction SilentlyContinue
    if ($userChoice.ProgId -and $userChoice.ProgId -notin @("md", "markdown")) {
      Write-Warning "$extension UserChoice is $($userChoice.ProgId). Windows may keep using that app until you change Default apps manually."
    }
  }
}

if (-not $NoShortcuts) {
  $shell = New-Object -ComObject WScript.Shell
  $shortcutPaths = @(
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "MMbook.lnk"),
    (Join-Path ([Environment]::GetFolderPath("Programs")) "MMbook.lnk")
  )

  foreach ($shortcutPath in $shortcutPaths) {
    $shortcutDir = Split-Path -Parent $shortcutPath
    if (-not (Test-Path $shortcutDir)) {
      New-Item -ItemType Directory -Path $shortcutDir -Force | Out-Null
    }

    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $installedExe
    $shortcut.WorkingDirectory = $InstallDir
    $shortcut.IconLocation = "$installedExe,0"
    $shortcut.Save()
  }
}

$installedFile = Get-Item -LiteralPath $installedExe
$installedHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe

Write-Host "MMbook installed and registered:"
Write-Host "  exe: $installedExe"
Write-Host "  timestamp: $($installedFile.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
Write-Host "  product version: $($installedFile.VersionInfo.ProductVersion)"
Write-Host "  sha256: $($installedHash.Hash)"
Write-Host "  open command: $openCommand"
