param(
    [ValidateSet('desktop', 'zhihu', 'podcast', 'verify')]
    [string]$Action = 'desktop'
)

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot

switch ($Action) {
    'desktop' {
        $npm = Require-Command -Name 'npm.cmd'
        $env:IMMERSIVE_RUNTIME_ROOT = Join-Path $root 'runtime'
        & $npm --prefix (Join-Path $root 'apps\desktop') run tauri dev
        exit $LASTEXITCODE
    }
    'zhihu' {
        $npm = Require-Command -Name 'npm.cmd'
        $project = Join-Path $root 'tools\zhihu-packer'
        $library = Join-Path $env:USERPROFILE 'Documents\沉浸阅读\Library'
        $runtime = Join-Path $env:LOCALAPPDATA 'ImmersiveReader\zhihu'
        $env:IMMERSIVE_LIBRARY_ROOT = $library
        $env:IMMERSIVE_ZHIHU_OUTPUT = Join-Path $library '知乎'
        $env:IMMERSIVE_ZHIHU_DB = Join-Path $runtime 'zhihu-packer.db'
        $env:IMMERSIVE_ZHIHU_PROFILE = Join-Path $runtime 'browser-profile'
        $env:IMMERSIVE_CHROMIUM_EXECUTABLE = Join-Path $root 'runtime\zhihu\chromium\msedge.exe'
        Start-Process -FilePath $npm -ArgumentList @('run', 'web') -WorkingDirectory $project -WindowStyle Hidden
        Write-Output '知乎归档控制台正在后台启动：http://127.0.0.1:3000'
    }
    'podcast' {
        $python = Get-PodcastPython
        $project = Join-Path $root 'tools\podcast-transcriber'
        $scriptPath = Join-Path $project 'scripts\run_with_gui.py'
        $env:IMMERSIVE_PODCAST_DATA_ROOT = Join-Path $env:LOCALAPPDATA 'ImmersiveReader\podcast'
        $env:IMMERSIVE_PODCAST_MODEL_ROOT = Join-Path $root 'runtime\podcast\models'
        $env:IMMERSIVE_PODCAST_PYTHON = $python
        $env:PATH = (Join-Path $root 'runtime\podcast\ffmpeg') + ';' + $env:PATH
        $arguments = if ((Split-Path -Leaf $python) -ieq 'py.exe') {
            @('-3', $scriptPath)
        } else {
            @($scriptPath)
        }
        Start-Process -FilePath $python -ArgumentList $arguments -WorkingDirectory $project -WindowStyle Hidden
        Write-Output '播客转写窗口正在启动。'
    }
    'verify' {
        & (Join-Path $PSScriptRoot 'verify.ps1')
        exit $LASTEXITCODE
    }
}
