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
    $managed = Join-Path (Get-RepoRoot) 'runtime\podcast\python\python.exe'
    if (Test-Path -LiteralPath $managed) {
        return $managed
    }
    $candidate = Join-Path (Get-RepoRoot) 'tools\podcast-transcriber\.venv\Scripts\python.exe'
    if (Test-Path -LiteralPath $candidate) {
        return $candidate
    }
    $py = Require-Command -Name 'py'
    return $py
}
