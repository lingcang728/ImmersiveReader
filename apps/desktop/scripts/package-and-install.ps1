$ErrorActionPreference = 'Stop'
$desktopRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$loadedLocalSigningKey = $false

Push-Location $desktopRoot
try {
  if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY) -and
      [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    $backup = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'ImmersiveReader-Updater-Offline-Backup'
    $private = Join-Path $backup 'immersive-reader-updater.key'
    $passwordFile = Join-Path $backup 'immersive-reader-updater.key-password.dpapi'
    if (-not (Test-Path -LiteralPath $private) -or -not (Test-Path -LiteralPath $passwordFile)) {
      throw '缺少 ImmersiveReader updater 离线签名备份。'
    }
    $secure = ConvertTo-SecureString (Get-Content -LiteralPath $passwordFile -Raw)
    $credential = [System.Management.Automation.PSCredential]::new('immersive-reader-updater', $secure)
    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -LiteralPath $private -Raw).Trim()
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $credential.GetNetworkCredential().Password
    $loadedLocalSigningKey = $true
  }

  & (Join-Path $PSScriptRoot 'install-latest-immersive-reader.ps1') -Build -SignUpdater -RegisterMarkdownAssociations
  if ($LASTEXITCODE -ne 0) { throw '沉浸阅读生产安装失败。' }
  & (Join-Path $PSScriptRoot 'collect-release.ps1')
  if ($LASTEXITCODE -ne 0) { throw '沉浸阅读发布资产收集失败。' }
} finally {
  if ($loadedLocalSigningKey) {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
  }
  Pop-Location
}
