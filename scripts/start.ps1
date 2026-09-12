param(
    [ValidateSet('desktop', 'verify')]
    [string]$Action = 'desktop'
)

. (Join-Path $PSScriptRoot 'common.ps1')

$root = Get-RepoRoot

switch ($Action) {
    'desktop' {
        $npm = Require-Command -Name 'npm.cmd'
        # Preflight: fail fast with a clear message instead of letting the app
        # boot without its managed sidecar runtimes, or letting vite die on the
        # pinned dev port (vite.config.js uses strictPort + devUrl stays 1420).
        $manifestPath = Join-Path $root 'runtime\manifest.json'
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
            throw "受管运行时未就绪：缺少 $manifestPath。请先运行 scripts\prepare-runtime.ps1 或恢复 runtime\ 目录后再启动。"
        }
        $portBusy = @(Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)
        if ($portBusy.Count -gt 0) {
            $ownerPid = $portBusy[0].OwningProcess
            $owner = Get-Process -Id $ownerPid -ErrorAction SilentlyContinue
            $ownerName = if ($owner) { $owner.ProcessName } else { 'unknown' }
            throw "端口 1420 已被占用（PID $ownerPid $ownerName）。devUrl 固定指向 http://localhost:1420，请先结束占用进程再启动。"
        }
        $env:IMMERSIVE_RUNTIME_ROOT = Join-Path $root 'runtime'
        & $npm --prefix (Join-Path $root 'apps\desktop') run tauri dev
        exit $LASTEXITCODE
    }
    'verify' {
        & (Join-Path $PSScriptRoot 'verify.ps1')
        exit $LASTEXITCODE
    }
}
