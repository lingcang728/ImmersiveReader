# common.ps1 — shared helpers dot-sourced by the other scripts in this folder
# (verify.ps1, start.ps1, prepare-runtime.ps1, verify-runtime.ps1, migrate-*).
# It is not a standalone entry point; run scripts\verify.ps1 for the build gate.
# scripts\qa\* are manual Playwright QA harnesses — they are intentionally NOT
# wired into verify.ps1 (they need a live app/browser session; run by hand).

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:RepoRoot = Split-Path -Parent $PSScriptRoot

function Get-RepoRoot {
    return $script:RepoRoot
}

function Require-Command {
    param([Parameter(Mandatory)][string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $command) {
        throw "缺少必需工具：$Name。请先检查全局安装，不要在项目内重复安装。"
    }
    return $command.Source
}

function Get-PodcastPython {
    # Resolution order: the managed vendored runtime first (what production
    # ships), then the tool's dev venv, and only as a last resort the system
    # `py` launcher — which is unpinned and may resolve to a different Python
    # than the one the podcast tool was validated against.
    $managed = Join-Path (Get-RepoRoot) 'runtime\podcast\python\python.exe'
    if (Test-Path -LiteralPath $managed) {
        return $managed
    }
    $candidate = Join-Path (Get-RepoRoot) 'tools\podcast-transcriber\.venv\Scripts\python.exe'
    if (Test-Path -LiteralPath $candidate) {
        return $candidate
    }
    Write-Warning '未找到托管 Python（runtime\podcast\python）或开发 venv（tools\podcast-transcriber\.venv），回退到系统 py 启动器；请确认其解析到受支持的 Python 3 版本。'
    $py = Require-Command -Name 'py'
    return $py
}

function Get-FileSha256Hex {
    param([Parameter(Mandatory)][string]$Path)
    # Get-FileHash is the fast path, but some Windows PowerShell 5.1 images
    # lack it — fall back to raw .NET so scripts run under either shell.
    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    }
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $algorithm = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '')
        } finally {
            $algorithm.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}
