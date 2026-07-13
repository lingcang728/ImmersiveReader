param(
    [ValidateSet('desktop', 'verify')]
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
    'verify' {
        & (Join-Path $PSScriptRoot 'verify.ps1')
        exit $LASTEXITCODE
    }
}
