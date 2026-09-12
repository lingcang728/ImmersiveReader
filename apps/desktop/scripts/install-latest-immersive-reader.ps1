[CmdletBinding()]
param(
  [switch]$Build,
  [switch]$SignUpdater,
  [switch]$RegisterMarkdownAssociations,
  [switch]$OpenDefaultAppsSettings,
  [switch]$NoShortcuts,
  # Rebuild Start Menu / Search / Default Apps icons without running NSIS.
  [switch]$RepairShellIdentity,
  # Default: %LOCALAPPDATA%\Programs\ImmersiveReader — a per-user install root
  # outside the source tree. Installing into the monorepo root let
  # robocopy /MIR and `git clean -xdf` tear the live production install.
  [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"
$desktopRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
# apps/desktop/scripts -> ImmersiveReader monorepo root
$monorepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not $InstallDir) {
  if (-not $env:LOCALAPPDATA) {
    throw "LOCALAPPDATA is unavailable; pass -InstallDir explicitly."
  }
  $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\ImmersiveReader"
}
$InstallDir = [System.IO.Path]::GetFullPath($InstallDir)
$monorepoPrefix = $monorepoRoot.TrimEnd('\') + '\'
if ($InstallDir -ieq $monorepoRoot -or
    $InstallDir.StartsWith($monorepoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
  Write-Warning "InstallDir is inside the source tree ($InstallDir); runtime rebuilds and 'git clean -xdf' can destroy the installed app. Prefer an external per-user directory."
}
$sourceRuntime = Join-Path $monorepoRoot 'runtime'
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

function Get-QuotedIconLocation {
  param(
    [Parameter(Mandatory)][string]$InstalledExe,
    [Parameter(Mandatory)][string]$InstalledIco
  )
  if ($InstalledIco -eq $InstalledExe) { return "$InstalledExe,0" }
  return "`"$InstalledIco`",0"
}

function Get-ShortcutIconLocation {
  param(
    [Parameter(Mandatory)][string]$InstalledExe,
    [Parameter(Mandatory)][string]$InstalledIco
  )
  if ($InstalledIco -eq $InstalledExe) { return "$InstalledExe,0" }
  return "$InstalledIco,0"
}

function Set-ShortcutAppUserModelId {
  param(
    [Parameter(Mandatory)][string]$ShortcutPath,
    [Parameter(Mandatory)][string]$AppUserModelId
  )
  if (-not ("ImmersiveReader.ShortcutAppUserModelId" -as [type])) {
    Add-Type -Language CSharp -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;

namespace ImmersiveReader {
  public static class ShortcutAppUserModelId {
    public static void Set(string shortcutPath, string appId) {
      var link = (IShellLinkW)new CShellLink();
      ((IPersistFile)link).Load(shortcutPath, 2);
      var store = (IPropertyStore)link;
      var key = new PROPERTYKEY(new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), 5);
      var pv = new PROPVARIANT(appId);
      try {
        Marshal.ThrowExceptionForHR(store.SetValue(ref key, pv));
        Marshal.ThrowExceptionForHR(store.Commit());
        ((IPersistFile)link).Save(shortcutPath, true);
      } finally {
        pv.Dispose();
        Marshal.ReleaseComObject(store);
        Marshal.ReleaseComObject(link);
      }
    }

    [ComImport, Guid("00021401-0000-0000-C000-000000000046")]
    private class CShellLink {}

    [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("000214F9-0000-0000-C000-000000000046")]
    private interface IShellLinkW {
      void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszFile, int cchMaxPath, IntPtr pfd, int fFlags);
      void GetIDList(out IntPtr ppidl);
      void SetIDList(IntPtr pidl);
      void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszName, int cchMaxName);
      void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string pszName);
      void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszDir, int cchMaxPath);
      void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string pszDir);
      void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszArgs, int cchMaxPath);
      void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string pszArgs);
      void GetHotkey(out short pwHotkey);
      void SetHotkey(short wHotkey);
      void GetShowCmd(out int piShowCmd);
      void SetShowCmd(int iShowCmd);
      void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder pszIconPath, int cchIconPath, out int piIcon);
      void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string pszIconPath, int iIcon);
      void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string pszPathRel, int dwReserved);
      void Resolve(IntPtr hwnd, int fFlags);
      void SetPath([MarshalAs(UnmanagedType.LPWStr)] string pszFile);
    }

    [ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
    private interface IPropertyStore {
      int GetCount(out uint cProps);
      int GetAt(uint iProp, out PROPERTYKEY pkey);
      int GetValue(ref PROPERTYKEY key, [Out] PROPVARIANT pv);
      int SetValue(ref PROPERTYKEY key, [In] PROPVARIANT pv);
      int Commit();
    }

    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    private struct PROPERTYKEY {
      public Guid fmtid;
      public uint pid;
      public PROPERTYKEY(Guid fmtid, uint pid) { this.fmtid = fmtid; this.pid = pid; }
    }

    [StructLayout(LayoutKind.Explicit)]
    private sealed class PROPVARIANT : IDisposable {
      [FieldOffset(0)] ushort vt;
      [FieldOffset(8)] IntPtr pointerValue;
      public PROPVARIANT(string value) {
        vt = 31;
        pointerValue = Marshal.StringToCoTaskMemUni(value);
      }
      public void Dispose() {
        PropVariantClear(this);
        GC.SuppressFinalize(this);
      }
      [DllImport("ole32.dll")]
      private static extern int PropVariantClear([In, Out] PROPVARIANT pvar);
    }
  }
}
"@
  }
  [ImmersiveReader.ShortcutAppUserModelId]::Set($ShortcutPath, $AppUserModelId)
}

function Deploy-ImmersiveReaderShellIcon {
  param(
    [Parameter(Mandatory)][string]$DesktopRoot,
    [Parameter(Mandatory)][string]$InstallDir,
    [Parameter(Mandatory)][string]$InstalledExe
  )
  $bundledIco = Join-Path $DesktopRoot "src-tauri\icons\icon.ico"
  $installedIco = Join-Path $InstallDir "immersive-reader.ico"
  if (Test-Path -LiteralPath $bundledIco) {
    Copy-Item -LiteralPath $bundledIco -Destination $installedIco -Force
    Write-Host "Deployed shell icon: $installedIco"
    return $installedIco
  }
  Write-Warning "Bundled icon.ico missing; falling back to EXE icon resource."
  return $InstalledExe
}

function Update-ImmersiveReaderShellIdentity {
  param(
    [Parameter(Mandatory)][string]$InstallDir,
    [Parameter(Mandatory)][string]$InstalledExe,
    [Parameter(Mandatory)][string]$InstalledIco,
    [switch]$NoShortcuts
  )
  $registeredName = "沉浸阅读"
  $iconLocation = Get-QuotedIconLocation -InstalledExe $InstalledExe -InstalledIco $InstalledIco
  $shortcutIcon = Get-ShortcutIconLocation -InstalledExe $InstalledExe -InstalledIco $InstalledIco
  $openCommand = "`"$InstalledExe`" `"%1`""

  $appKey = "HKCU:\Software\Classes\Applications\immersive-reader.exe"
  New-Item -Path "$appKey\DefaultIcon" -Force | Out-Null
  New-Item -Path "$appKey\shell\open\command" -Force | Out-Null
  New-ItemProperty -Path $appKey -Name "FriendlyAppName" -Value $registeredName -PropertyType String -Force | Out-Null
  Set-Item -Path "$appKey\DefaultIcon" -Value $iconLocation
  Set-Item -Path "$appKey\shell\open\command" -Value $openCommand

  $appPathKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\immersive-reader.exe"
  New-Item -Path $appPathKey -Force | Out-Null
  Set-Item -Path $appPathKey -Value $InstalledExe
  New-ItemProperty -Path $appPathKey -Name "Path" -Value $InstallDir -PropertyType String -Force | Out-Null

  $capabilitiesPath = "HKCU:\Software\ImmersiveReader\Capabilities"
  if (Test-Path -LiteralPath $capabilitiesPath) {
    New-ItemProperty -Path $capabilitiesPath -Name "ApplicationIcon" -Value $iconLocation -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $capabilitiesPath -Name "ApplicationName" -Value $registeredName -PropertyType String -Force | Out-Null
  }

  foreach ($progId in @("ImmersiveReader.Markdown", "md")) {
    $defaultIconPath = "HKCU:\Software\Classes\$progId\DefaultIcon"
    if (Test-Path -LiteralPath $defaultIconPath) {
      Set-Item -Path $defaultIconPath -Value $iconLocation
    }
    $commandPath = "HKCU:\Software\Classes\$progId\shell\open\command"
    if (Test-Path -LiteralPath $commandPath) {
      Set-Item -Path $commandPath -Value $openCommand
    }
  }

  $uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\沉浸阅读"
  if (Test-Path -LiteralPath $uninstallKey) {
    $uninstallExe = Join-Path $InstallDir "uninstall.exe"
    New-ItemProperty -Path $uninstallKey -Name "DisplayIcon" -Value $iconLocation -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $uninstallKey -Name "InstallLocation" -Value $InstallDir -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $uninstallKey -Name "DisplayName" -Value $registeredName -PropertyType String -Force | Out-Null
    if (Test-Path -LiteralPath $uninstallExe) {
      New-ItemProperty -Path $uninstallKey -Name "UninstallString" -Value "`"$uninstallExe`"" -PropertyType String -Force | Out-Null
    }
  }

  if (-not $NoShortcuts) {
    $shell = New-Object -ComObject WScript.Shell
    foreach ($shortcutPath in @(
      (Join-Path ([Environment]::GetFolderPath("Desktop")) "沉浸阅读.lnk"),
      (Join-Path ([Environment]::GetFolderPath("Programs")) "沉浸阅读.lnk")
    )) {
      $shortcutDir = Split-Path -Parent $shortcutPath
      if (-not (Test-Path -LiteralPath $shortcutDir)) {
        New-Item -ItemType Directory -Path $shortcutDir -Force | Out-Null
      }
      if (Test-Path -LiteralPath $shortcutPath) {
        [System.IO.File]::Delete($shortcutPath)
      }
      $shortcut = $shell.CreateShortcut($shortcutPath)
      $shortcut.TargetPath = $InstalledExe
      $shortcut.WorkingDirectory = $InstallDir
      $shortcut.IconLocation = $shortcutIcon
      $shortcut.Description = $registeredName
      $shortcut.Save()
      [void][System.Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
      Set-ShortcutAppUserModelId -ShortcutPath $shortcutPath -AppUserModelId "com.lingcang.immersivereading"
      Write-Host "Wrote shortcut: $shortcutPath -> $shortcutIcon"
    }
  }

  try {
    Add-Type -Namespace ImmersiveReader -Name ShellNotify -MemberDefinition @"
      [System.Runtime.InteropServices.DllImport("shell32.dll")]
      public static extern void SHChangeNotify(int wEventId, uint uFlags, System.IntPtr dwItem1, System.IntPtr dwItem2);
      [System.Runtime.InteropServices.DllImport("shell32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, EntryPoint="SHChangeNotify")]
      public static extern void SHChangeNotifyPath(int wEventId, uint uFlags, string dwItem1, string dwItem2);
"@ -ErrorAction SilentlyContinue
    # SHCNE_ASSOCCHANGED = 0x08000000, SHCNF_IDLIST = 0x0000
    [ImmersiveReader.ShellNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
    $startMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "沉浸阅读.lnk"
    if (Test-Path -LiteralPath $startMenu) {
      # SHCNE_UPDATEITEM = 0x00002000, SHCNF_PATHW = 0x0005
      [ImmersiveReader.ShellNotify]::SHChangeNotifyPath(0x00002000, 0x0005, $startMenu, $null)
    }
    Write-Host "Notified Windows shell of icon/association changes."
  } catch {
    Write-Warning "Shell association notify skipped: $($_.Exception.Message)"
  }

  $iconFile = if ($iconLocation -match '^"([^"]+)"') { $Matches[1] } else { ($iconLocation -split ',', 2)[0] }
  if (-not (Test-Path -LiteralPath $iconFile)) {
    throw "Shell icon target is missing: $iconFile"
  }
}

function Get-ProcessesUnderDirectory {
  param([Parameter(Mandatory)][string]$Root)

  $prefix = [System.IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
  $found = @()
  foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
    $processPath = $null
    try { $processPath = $process.Path } catch { $processPath = $null }
    if ($processPath -and
        ([System.IO.Path]::GetFullPath($processPath) + '\').StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
      $found += $process
    }
  }
  return ,$found
}

function Stop-ProcessesUnderDirectory {
  param([Parameter(Mandatory)][string]$Root)

  # The app's close handler flushes state before prevent_close hides the
  # window to the tray, so give GUI processes a graceful window first, then
  # force-kill: sidecars/workers keep runtime files open and a robocopy /MIR
  # or NSIS overwrite against them produces a torn install.
  $running = @(Get-ProcessesUnderDirectory -Root $Root)
  if ($running.Count -eq 0) { return }
  $names = ($running | ForEach-Object { "$($_.ProcessName) (PID $($_.Id))" }) -join ', '
  Write-Warning "Stopping processes under $Root before replacing files: $names"
  foreach ($process in $running) {
    try { [void]$process.CloseMainWindow() } catch { }
  }
  Start-Sleep -Seconds 3
  $running | Stop-Process -Force -ErrorAction SilentlyContinue
  $deadline = (Get-Date).AddSeconds(15)
  while ((Get-Date) -lt $deadline) {
    $running = @(Get-ProcessesUnderDirectory -Root $Root)
    if ($running.Count -eq 0) { return }
    Start-Sleep -Milliseconds 500
  }
  $names = ($running | ForEach-Object { "$($_.ProcessName) (PID $($_.Id))" }) -join ', '
  throw "Processes are still running under $Root; close Immersive Reader and retry: $names"
}

function Assert-RuntimeAppHashes {
  $zhihuAppTemplate = Join-Path $monorepoRoot "runtime\zhihu\app\dist\reader-template.html"
  $zhihuSourceTemplate = Join-Path $monorepoRoot "tools\zhihu-packer\dist\reader-template.html"
  $zhihuAppServer = Join-Path $monorepoRoot "runtime\zhihu\app\dist\server.js"
  $zhihuSourceServer = Join-Path $monorepoRoot "tools\zhihu-packer\dist\server.js"
  $contractsAppIndex = Join-Path $monorepoRoot "runtime\packages\contracts\dist\index.js"
  $contractsSourceIndex = Join-Path $monorepoRoot "packages\contracts\dist\index.js"
  $podcastAppPolish = Join-Path $monorepoRoot "runtime\podcast\app\scripts\polish_interview_markdown.py"
  $podcastSourcePolish = Join-Path $monorepoRoot "tools\podcast-transcriber\scripts\polish_interview_markdown.py"
  $podcastAppLanguage = Join-Path $monorepoRoot "runtime\podcast\app\scripts\podcast_transcriber\language.py"
  $podcastSourceLanguage = Join-Path $monorepoRoot "tools\podcast-transcriber\scripts\podcast_transcriber\language.py"
  foreach ($pair in @(
    @{ Name = "Reader template"; Source = $zhihuSourceTemplate; Runtime = $zhihuAppTemplate },
    @{ Name = "Zhihu sidecar server"; Source = $zhihuSourceServer; Runtime = $zhihuAppServer },
    @{ Name = "Contracts runtime"; Source = $contractsSourceIndex; Runtime = $contractsAppIndex },
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

if ($RepairShellIdentity) {
  $installedExe = Join-Path $InstallDir "immersive-reader.exe"
  if (-not (Test-Path -LiteralPath $installedExe)) {
    throw "Installed executable not found: $installedExe"
  }
  $installedIco = Deploy-ImmersiveReaderShellIcon -DesktopRoot $desktopRoot -InstallDir $InstallDir -InstalledExe $installedExe
  Update-ImmersiveReaderShellIdentity -InstallDir $InstallDir -InstalledExe $installedExe -InstalledIco $installedIco -NoShortcuts:$NoShortcuts
  $installed = Get-Item -LiteralPath $installedExe
  $installedHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe
  Write-Host "Repaired shell identity for $installedExe"
  Write-Host "Timestamp: $($installed.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
  Write-Host "Product version: $($installed.VersionInfo.ProductVersion)"
  Write-Host "SHA-256: $($installedHash.Hash)"
  return
}

if ($Build) {
  # Clean checkouts ship neither the zhihu sidecar dist\server.js nor the
  # contracts dist\ — prepare-runtime -RefreshApps mirrors them into the
  # managed runtime, so build them first or the installed tools come out
  # broken (require_runtime fails on the missing sidecar entry point).
  $zhihuDist = Join-Path $monorepoRoot "tools\zhihu-packer\dist"
  if (Test-Path -LiteralPath $zhihuDist) {
    Remove-Item -LiteralPath $zhihuDist -Recurse -Force
  }
  $contractsDist = Join-Path $monorepoRoot "packages\contracts\dist"
  if (Test-Path -LiteralPath $contractsDist) {
    Remove-Item -LiteralPath $contractsDist -Recurse -Force
  }
  Push-Location (Join-Path $monorepoRoot "tools\zhihu-packer")
  try {
    Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments @("run", "build")
    # Compile continuous-reader template after tsc so dist holds both outputs.
    Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments @("run", "compile-reader")
  } finally {
    Pop-Location
  }
  # contracts has no local toolchain; reuse the desktop app's tsc like verify.ps1.
  $contractsTsc = Join-Path $monorepoRoot "apps\desktop\node_modules\.bin\tsc.cmd"
  if (-not (Test-Path -LiteralPath $contractsTsc -PathType Leaf)) {
    throw "apps\desktop TypeScript compiler not found; run npm ci in apps\desktop first."
  }
  Push-Location (Join-Path $monorepoRoot "packages\contracts")
  try {
    Invoke-CheckedCommand -FilePath $contractsTsc -Arguments @("-p", "tsconfig.json")
  } finally {
    Pop-Location
  }
  # RefreshApps deletes code inside the repo runtime in place — stop any
  # sidecar/worker still running from it first.
  Stop-ProcessesUnderDirectory -Root $sourceRuntime
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
  $tauriBuildArguments = @("run", "tauri", "build", "--", "--bundles", "nsis")
  if (-not $SignUpdater) {
    $tauriBuildArguments += "--no-sign"
  }
  Invoke-CheckedCommand -FilePath "npm.cmd" -Arguments $tauriBuildArguments
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

if (-not (Test-Path -LiteralPath (Join-Path $sourceRuntime 'manifest.json'))) {
  throw "Managed runtime is missing. Run scripts\prepare-runtime.ps1 first."
}
# Stop the installed app and any sidecars/workers before touching InstallDir:
# robocopy /MIR and the NSIS overwrite both tear a running install.
Stop-ProcessesUnderDirectory -Root $InstallDir
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
  'runtime\zhihu\app\dist\server.js',
  'runtime\zhihu\app\dist\reader-template.html',
  'runtime\packages\contracts\dist\index.js',
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

# Deploy a standalone BMP+AND alpha ICO next to the EXE so Explorer / shortcuts
# can use true transparency (PE-embedded icons often show black corners).
$installedIco = Deploy-ImmersiveReaderShellIcon -DesktopRoot $desktopRoot -InstallDir $InstallDir -InstalledExe $installedExe
$iconLocation = Get-QuotedIconLocation -InstalledExe $installedExe -InstalledIco $installedIco

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
  Set-Item -Path "HKCU:\Software\Classes\$progId\DefaultIcon" -Value $iconLocation
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
    Set-Item -Path "HKCU:\Software\Classes\md\DefaultIcon" -Value $iconLocation
    Write-Host "Migrated the legacy md Markdown handler/icon to $installedExe."
  }

  New-Item -Path "$capabilitiesPath\FileAssociations" -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationName" -Value $registeredName -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationDescription" -Value "本地长文阅读、知乎归档和播客转写工具。" -PropertyType String -Force | Out-Null
  New-ItemProperty -Path $capabilitiesPath -Name "ApplicationIcon" -Value $iconLocation -PropertyType String -Force | Out-Null
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

Update-ImmersiveReaderShellIdentity -InstallDir $InstallDir -InstalledExe $installedExe -InstalledIco $installedIco -NoShortcuts:$NoShortcuts

$installed = Get-Item -LiteralPath $installedExe
$installedHash = Get-FileHash -Algorithm SHA256 -LiteralPath $installedExe
Write-Host "Installed EXE: $installedExe"
Write-Host "Timestamp: $($installed.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss'))"
Write-Host "Product version: $($installed.VersionInfo.ProductVersion)"
Write-Host "SHA-256: $($installedHash.Hash)"
