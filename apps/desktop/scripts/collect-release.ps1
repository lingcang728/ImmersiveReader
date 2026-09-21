param([string]$OutputDirectory = '..\..\output\release')

$ErrorActionPreference = 'Stop'
$desktopRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$repoRoot = (Resolve-Path (Join-Path $desktopRoot '..\..')).Path
$output = [System.IO.Path]::GetFullPath((Join-Path $desktopRoot $OutputDirectory))
$repoPrefix = $repoRoot.TrimEnd('\') + '\'
if (-not $output.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw '发布输出目录必须位于 ImmersiveReader 仓库内。'
}

$version = (Get-Content -LiteralPath (Join-Path $desktopRoot 'package.json') -Raw | ConvertFrom-Json).version
$tauriVersion = (Get-Content -LiteralPath (Join-Path $desktopRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
if ($version -cne $tauriVersion) { throw "版本不一致：npm=$version tauri=$tauriVersion" }
$metadata = cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $desktopRoot 'src-tauri\Cargo.toml') | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw '无法解析 Cargo target 目录。' }
# All three version declarations must agree — Cargo.toml is the one the Rust
# crate and NSIS metadata actually carry.
$cargoPackage = @($metadata.packages) | Where-Object { $_.name -eq 'immersive-reader' } | Select-Object -First 1
if ($null -eq $cargoPackage) { throw '无法从 Cargo metadata 解析 immersive-reader crate。' }
$cargoVersion = [string]$cargoPackage.version
if (-not $cargoVersion) { throw '无法从 Cargo metadata 解析 immersive-reader crate 版本。' }
if ($version -cne $cargoVersion) { throw "版本不一致：npm=$version cargo=$cargoVersion" }
$source = Join-Path $metadata.target_directory "release\bundle\nsis\沉浸阅读_${version}_x64-setup.exe"
$signaturePath = "$source.sig"
if (-not (Test-Path -LiteralPath $source) -or -not (Test-Path -LiteralPath $signaturePath)) {
  throw '缺少沉浸阅读 NSIS 安装包或 updater 签名。'
}

New-Item -ItemType Directory -Force -Path $output | Out-Null
# Only prior release artifacts may be removed — the "inside the repo" check
# above still permits pointing at a source directory; wiping every file there
# would delete tracked sources. Non-artifact files must survive.
Get-ChildItem -LiteralPath $output -File -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -like '*-setup.exe' -or $_.Name -like '*-setup.exe.sig' -or $_.Name -eq 'latest.json' } |
  Remove-Item -Force
$installerName = "ImmersiveReader_${version}_x64-setup.exe"
$installer = Join-Path $output $installerName
Copy-Item -LiteralPath $source -Destination $installer -Force
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash -cne (Get-FileHash -Algorithm SHA256 -LiteralPath $installer).Hash) {
  throw '沉浸阅读安装包复制校验失败。'
}
# Keep the raw .sig next to the installer: the release workflow verifies it
# against the tauri.conf.json updater pubkey before publishing.
Copy-Item -LiteralPath $signaturePath -Destination "$installer.sig" -Force

$signature = (Get-Content -LiteralPath $signaturePath -Raw).Trim()
if ([string]::IsNullOrWhiteSpace($signature)) { throw 'updater 签名为空。' }
# notes fallback: prefer the release notes' first content paragraph over a
# stale hard-coded string; env override still wins for ad-hoc messages.
$fallbackNotes = "沉浸阅读 $version 更新，详见 GitHub Release 说明。"
$notesFile = Join-Path $repoRoot "docs\release\$version\RELEASE_NOTES.md"
if (-not $env:IMMERSIVE_READER_RELEASE_NOTES -and (Test-Path -LiteralPath $notesFile -PathType Leaf)) {
  $paragraph = Get-Content -LiteralPath $notesFile -Encoding UTF8 |
    Where-Object { $_ -match '\S' -and $_ -notmatch '^\s*#' } |
    Select-Object -First 1
  if ($paragraph) { $fallbackNotes = $paragraph.Trim() }
}
$latest = [ordered]@{
  version = $version
  notes = if ($env:IMMERSIVE_READER_RELEASE_NOTES) { $env:IMMERSIVE_READER_RELEASE_NOTES.Trim() } else { $fallbackNotes }
  pub_date = (Get-Date).ToUniversalTime().ToString('o')
  size = (Get-Item -LiteralPath $installer).Length
  platforms = [ordered]@{
    'windows-x86_64' = [ordered]@{
      signature = $signature
      url = "https://github.com/lingcang728/ImmersiveReader/releases/download/v$version/$installerName"
      size = (Get-Item -LiteralPath $installer).Length
    }
  }
}
[System.IO.File]::WriteAllText(
  (Join-Path $output 'latest.json'),
  ($latest | ConvertTo-Json -Depth 6),
  [System.Text.UTF8Encoding]::new($false)
)
Get-ChildItem -LiteralPath $output -File | Select-Object Name,Length,LastWriteTime,@{n='SHA256';e={(Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash}}
