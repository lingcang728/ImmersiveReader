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

function Test-ReparsePoint {
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $false }
    return ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
}

function Assert-NotReparsePoint {
    # Junction/symlink roots break recursive-delete and mirror-copy safety: PS
    # 5.1 Remove-Item -Recurse follows junctions into the TARGET tree, and
    # robocopy /MIR can mirror-delete the target's contents.
    param(
        [Parameter(Mandatory)][string]$Path,
        [string]$Label = $Path
    )
    if (Test-ReparsePoint -Path $Path) {
        throw "路径是 Junction/Symlink，拒绝在其上递归操作：$Label"
    }
}

function Remove-DirectoryTree {
    # Recursively delete a directory, but never follow a reparse point: on PS
    # 5.1 `Remove-Item -Recurse` on a junction deletes the linked target's
    # contents. A reparse point gets its link removed only; real directories
    # get the normal recursive delete.
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        [IO.Directory]::Delete($item.FullName, $false)
        return
    }
    Remove-Item -LiteralPath $Path -Recurse -Force
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
