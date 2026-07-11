param(
  [Parameter(Mandatory = $false)]
  [string]$Url = "",

  [Parameter(Mandatory = $true)]
  [string]$ExitEndpoint,

  [Parameter(Mandatory = $false)]
  [string]$OpenEndpoint = ""
)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$notifyIcon = New-Object System.Windows.Forms.NotifyIcon
$notifyIcon.Text = "Podcast Transcriber"
$iconPath = Join-Path $PSScriptRoot "..\assets\icon.ico"
if (Test-Path $iconPath) {
  $notifyIcon.Icon = New-Object System.Drawing.Icon($iconPath)
} else {
  $notifyIcon.Icon = [System.Drawing.SystemIcons]::Application
}
$notifyIcon.Visible = $true

$menu = New-Object System.Windows.Forms.ContextMenuStrip
$openItem = $menu.Items.Add("打开窗口")
$exitItem = $menu.Items.Add("退出并清理")

$openAction = {
  if (-not [string]::IsNullOrWhiteSpace($OpenEndpoint)) {
    try {
      Invoke-RestMethod `
        -Method Post `
        -Uri $OpenEndpoint `
        -ContentType "application/json" `
        -Body '{"action":"show"}' `
        -TimeoutSec 5 | Out-Null
      return
    } catch {
      # Fall back to the URL below if the GUI service cannot handle the request.
    }
  }
  if (-not [string]::IsNullOrWhiteSpace($Url)) {
    Start-Process $Url
  }
}

$openItem.add_Click($openAction)
$notifyIcon.add_DoubleClick($openAction)

$exitItem.add_Click({
  try {
    Invoke-RestMethod `
      -Method Post `
      -Uri $ExitEndpoint `
      -ContentType "application/json" `
      -Body '{"action":"quit"}' `
      -TimeoutSec 10 | Out-Null
  } catch {
    # The GUI service may already be gone; the tray should still close.
  }

  $notifyIcon.Visible = $false
  $notifyIcon.Dispose()
  [System.Windows.Forms.Application]::Exit()
})

$notifyIcon.ContextMenuStrip = $menu

try {
  [System.Windows.Forms.Application]::Run()
} finally {
  $notifyIcon.Visible = $false
  $notifyIcon.Dispose()
}
