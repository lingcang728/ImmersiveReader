$ErrorActionPreference = 'Stop'
$desktopRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$loadedLocalSigningKey = $false

function Set-UpdaterSecretAcl {
  param([Parameter(Mandatory)][string]$Path)
  # A plaintext-adjacent secret must not be readable by other users/processes:
  # strip inheritance and leave only the current user with read access.
  try {
    $acl = Get-Acl -LiteralPath $Path
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($rule in @($acl.Access)) { [void]$acl.RemoveAccessRule($rule) }
    $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    $acl.AddAccessRule([System.Security.AccessControl.FileSystemAccessRule]::new($identity, 'Read', 'Allow'))
    Set-Acl -LiteralPath $Path -AclObject $acl
  } catch {
    Write-Warning "无法收紧签名备份文件 ACL：$Path — $($_.Exception.Message)"
  }
}

function Copy-VerifiedSecret {
  param(
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$Target
  )
  Copy-Item -LiteralPath $Source -Destination $Target -Force
  if ((Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash -ne
      (Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash) {
    throw "签名备份迁移校验失败：$Source"
  }
}

Push-Location $desktopRoot
try {
  if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY) -and
      [string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY_PATH)) {
    # The offline updater key backup must not live in Documents: that folder is
    # commonly OneDrive-synced and indexed. Prefer an explicit env override,
    # else the non-roaming per-user secrets directory.
    $backup = [string]$env:IMMERSIVE_SIGNING_BACKUP_DIR
    if ([string]::IsNullOrWhiteSpace($backup)) {
      $backup = Join-Path $env:LOCALAPPDATA 'ImmersiveReader\Secrets\updater-backup'
    }
    $legacyBackup = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'ImmersiveReader-Updater-Offline-Backup'
    $private = Join-Path $backup 'immersive-reader-updater.key'
    $privateEncrypted = "$private.dpapi"
    $passwordFile = Join-Path $backup 'immersive-reader-updater.key-password.dpapi'

    if ($legacyBackup -and
        ([IO.Path]::GetFullPath($backup) -ne [IO.Path]::GetFullPath($legacyBackup)) -and
        (Test-Path -LiteralPath $legacyBackup)) {
      $alreadyMigrated = (Test-Path -LiteralPath $private) -or
        (Test-Path -LiteralPath $privateEncrypted) -or
        (Test-Path -LiteralPath $passwordFile)
      $legacyFiles = @(@(
        'immersive-reader-updater.key',
        'immersive-reader-updater.key.dpapi',
        'immersive-reader-updater.key-password.dpapi'
      ) | ForEach-Object { Join-Path $legacyBackup $_ } | Where-Object { Test-Path -LiteralPath $_ })
      if (-not $alreadyMigrated -and $legacyFiles.Count -gt 0) {
        New-Item -ItemType Directory -Path $backup -Force | Out-Null
        foreach ($legacyFile in $legacyFiles) {
          Copy-VerifiedSecret -Source $legacyFile -Target (Join-Path $backup (Split-Path -Leaf $legacyFile))
        }
        foreach ($legacyFile in $legacyFiles) { Remove-Item -LiteralPath $legacyFile -Force }
        Write-Warning "updater 签名备份已从 Documents 迁移到 $backup；请确认 $legacyBackup 已无残留机密。"
      } elseif ($legacyFiles.Count -gt 0) {
        Write-Warning "旧版 Documents 签名备份仍存在于 $legacyBackup；新位置已有备份，请手动删除旧目录中的机密文件。"
      }
    }

    $hasKey = (Test-Path -LiteralPath $private) -or (Test-Path -LiteralPath $privateEncrypted)
    if (-not $hasKey -or -not (Test-Path -LiteralPath $passwordFile)) {
      throw '缺少 ImmersiveReader updater 离线签名备份。'
    }
    if (Test-Path -LiteralPath $privateEncrypted) {
      # Preferred: DPAPI-encrypted blob decryptable only by this user/machine.
      $keySecure = ConvertTo-SecureString ((Get-Content -LiteralPath $privateEncrypted -Raw).Trim())
      $keyCredential = [System.Management.Automation.PSCredential]::new('immersive-reader-updater-key', $keySecure)
      $env:TAURI_SIGNING_PRIVATE_KEY = $keyCredential.GetNetworkCredential().Password
      Set-UpdaterSecretAcl -Path $privateEncrypted
    } else {
      Write-Warning "updater 私钥以明文存于 $private；建议用 ConvertFrom-SecureString 改写为 immersive-reader-updater.key.dpapi（仅本机本用户可解密）。"
      $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -LiteralPath $private -Raw).Trim()
      Set-UpdaterSecretAcl -Path $private
    }
    $secure = ConvertTo-SecureString (Get-Content -LiteralPath $passwordFile -Raw)
    $credential = [System.Management.Automation.PSCredential]::new('immersive-reader-updater', $secure)
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $credential.GetNetworkCredential().Password
    Set-UpdaterSecretAcl -Path $passwordFile
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
