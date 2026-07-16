[CmdletBinding()]
param(
  [switch]$Build,
  [switch]$RegisterMarkdownAssociations,
  [switch]$OpenDefaultAppsSettings,
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

function Get-FileSha256Hex {
  param([Parameter(Mandatory)][string]$Path)
  return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Assert-RuntimeAppHashes {
  $zhihuAppTemplate = Join-Path $monorepoRoot "runtime\zhihu\app\dist\reader-template.html"
  $zhihuSourceTemplate = Join-Path $monorepoRoot "tools\zhihu-packer\dist\reader-template.html"
  $podcastAppPolish = Join-Path $monorepoRoot "runtime\podcast\app\scripts\polish_interview_markdown.py"
  $podcastSourcePolish = Join-Path $monorepoRoot "tools\podcast-transcriber\scripts\polish_interview_markdown.py"
  $podcastAppLanguage = Join-Path $monorepoRoot "runtime\podcast\app\scripts\podcast_transcriber\language.py"
  $podcastSourceLanguage = Join-Path $monorepoRoot "tools\podcast-transcriber\scripts\podcast_transcriber\language.py"
  foreach ($pair in @(
    @{ Name = "Reader template"; Source = $zhihuSourceTemplate; Runtime = $zhihuAppTemplate },
    @{ Name = "Podcast final markdown generator"; Source = $podcastSourcePolish; Runtime = $podcastAppPolish },
    @{ Name = "Podcast language classifier"; Source = $podcastSourceLanguage; Runtime = $podcastAppLanguage }
  )) {
    if (-not (Test-Path -LiteralPath $pair.Source)) {
      throw "Ship preflight missing source: $($pair.Name) ($($pair.Source))"
    }
    if (-not (Test-Path -LiteralPath $pair.Runtime)) {
      throw "Ship preflight missing managed runtime copy: $($pair.Name) ($($pair.Runtime))"
    }
    $sourceHash = Get-FileSha256Hex -Path $pair.Source
    $runtimeHash = Get-FileSha256Hex -Path $pair.Runtime
    if ($sourceHash -ne $runtimeHash) {
      throw "Managed runtime drift for $($pair.Name): source=$sourceHash runtime=$runtimeHash. Re-run prepare-runtime -RefreshApps."
    }
    Write-Host "[ship] hash ok $($pair.Name): $sourceHash"
  }
}

if ($Build) {
  # Compile continuous-reader template, refresh managed runtime app code, then package.
  Push-Location (Join-Path $monorepoRoot "tools\zhihu-packer")
  try {
    Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments @("run", "compile-reader")
  } finally {
    Pop-Location
  }
  $prepareRuntime = Join-Path $monorepoRoot "scripts\prepare-runtime.ps1"
  Invoke-CheckedCommand -FilePath "powershell.exe" -Arguments @(
    "-ExecutionPolicy", "Bypass",
    "-File", $prepareRuntime,
    "-RefreshApps"
  )
  Assert-RuntimeAppHashes
  $verifyRuntime = Join-Path $monorepoRoot "scripts\verify-runtime.ps1"
  Invoke-CheckedCommand -FilePath "powershell.exe" -Arguments @(
    "-ExecutionPolicy", "Bypass",
    "-File", $verifyRuntime,
    "-RuntimeRoot", (Join-Path $monorepoRoot "runtime")
  )
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
$verifyRuntime = Join-Path $monorepoRoot "scripts\verify-runtime.ps1"
Invoke-CheckedCommand -FilePath "powershell.exe" -Arguments @(
  "-ExecutionPolicy", "Bypass",
  "-File", $verifyRuntime,
  "-RuntimeRoot", $targetRuntime,
  "-ManifestPath", (Join-Path $targetRuntime "manifest.json")
)

if ($RegisterMarkdownAssociations) {
  $progId = "ImmersiveReader.Markdown"
  $registeredName = "沉浸阅读"
  $capabilitiesPath = "HKCU:\Software\ImmersiveReader\Capabilities"
  $openCommand = "`"$installedExe`" `"%1`""
  foreach ($extension in @(".md", ".markdown")) {
    New-Item -Path "HKCU:\Software\Classes\$extension" -Force | Out-Null
    Set-Item -Path "HKCU:\Software\Classes\$extension" -Value $progId
    New-Item -Path "HKCU:\Software\Classes\$extension\OpenWithProgids" -Force | Out-Null
    New-ItemProperty -Path "HKCU:\Software\Classes\$extension\OpenWithProgids" -Name $progId -Value "" -PropertyType String -Force | Out-Null
  }
  New-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Force | Out-Null
  New-Item -Path "HKCU:\Software\Classes\$progId\DefaultIcon" -Force | Out-Null
  Set-Item -Path "HKCU:\Software\Classes\$progId" -Value "Markdown Document"
  Set-Item -Path "HKCU:\Software\Classes\$progId\DefaultIcon" -Value "`"$installedExe`",0"
  Set-Item -Path "HKCU:\Software\Classes\$progId\shell\open\command" -Value $openCommand

  # Windows can retain a protected UserChoice that points to the old MMbook
  # ProgId (`md`). Migrate that legacy command in place so existing defaults
  # immediately open the current production executable without bypassing the
  # UserChoice protection. Always refresh DefaultIcon on `md` when it opens
  # this product so Explorer file icons match the installed EXE artwork.
  $legacyCommandPath = "HKCU:\Software\Classes\md\shell\open\command"
  $legacyCommand = (Get-ItemProperty -LiteralPath $legacyCommandPath -Name "(default)" -ErrorAction SilentlyContinue).'(default)'
  if ($legacyCommand -and $legacyCommand -match "(?i)(mmbook|immersive-reader|沉浸阅读)") {
    Set-Item -Path $legacyCommandPath -Value $openCommand
    Set-Item -Path "HKCU:\Software\Classes\md" -Value $registeredName
    New-Item -Path "HKCU:\Software\Classes\md\DefaultIcon" -Force | Out-Null
    Set-Item -Path "HKCU:\Software\Classes\md\DefaultIcon" -Value "`"$installedExe`",0"
    Write-Host "Migrated the legacy md Markdown handler/icon to $installedExe."
  }

  New-Item -Path "$capabilitiesPath\FileAssociations" -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationName" -Value $registeredName -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationDescription" -Value "本地长文阅读、知乎归档和播客转写工具。" -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationIcon" -Value "$installedExe,0" -PropertyType String -Force | Out-Null
  foreach ($extension in @(".md", ".markdown")) {
    New-ItemProperty -Path "$capabilitiesPath\FileAssociations" -Name $extension -Value $progId -PropertyType String -Force | Out-Null
  }
  New-Item -Path "HKCU:\Software\RegisteredApplications" -Force | Out-Null
  New-ItemProperty -Path "HKCU:\Software\RegisteredApplications" -Name $registeredName -Value "Software\ImmersiveReader\Capabilities" -PropertyType String -Force | Out-Null

  $userChoices = foreach ($extension in @(".md", ".markdown")) {
    $userChoicePath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$extension\UserChoice"
    $current = (Get-ItemProperty -LiteralPath $userChoicePath -Name "ProgId" -ErrorAction SilentlyContinue).ProgId
    [pscustomobject]@{ Extension = $extension; ProgId = $current }
  }
  Write-Host "Markdown handler and Default Apps capabilities registered after validation."
  foreach ($choice in $userChoices) {
    if ($choice.ProgId -and $choice.ProgId -ne $progId) {
      Write-Warning "$($choice.Extension) UserChoice remains $($choice.ProgId); Windows requires the user to change it in Default Apps."
    }
  }
  if ($OpenDefaultAppsSettings) {
    $settingsUri = "ms-settings:defaultapps?registeredAppUser=$([Uri]::EscapeDataString($registeredName))"
    Start-Process $settingsUri
    Write-Host "Opened Windows Default Apps for $registeredName."
  }
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

# Force Explorer / shell to pick up the new EXE icon and file associations
# without forging protected UserChoice hashes.
try {
  Add-Type -Namespace ImmersiveReader -Name ShellNotify -MemberDefinition @"
    [System.Runtime.InteropServices.DllImport("shell32.dll")]
    public static extern void SHChangeNotify(int wEventId, uint uFlags, System.IntPtr dwItem1, System.IntPtr dwItem2);
"@ -ErrorAction SilentlyContinue
  # SHCNE_ASSOCCHANGED = 0x08000000, SHCNF_IDLIST = 0x0000
  [ImmersiveReader.ShellNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
  Write-Host "Notified Windows shell of icon/association changes."
} catch {
  Write-Warning "Shell association notify skipped: $($_.Exception.Message)"
}

$installed = Get-Item -LiteralPath $installedExe
$installedHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe
Write-Host "Installed EXE: $installedExe"
Write-Host "Timestamp: $($installed.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
Write-Host "Product version: $($installed.VersionInfo.ProductVersion)"
Write-Host "SHA-256: $($installedHash.Hash)"
