$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = Split-Path -Parent $ScriptDir
$Python = Join-Path $Root ".venv\Scripts\python.exe"
$Requirements = Join-Path $Root "requirements.txt"
$Config = Join-Path $Root "config.json"
$ExampleConfig = Join-Path $Root "config.example.json"

Set-Location $Root

if (-not (Test-Path $Requirements)) {
    throw "requirements.txt not found: $Requirements"
}

if (-not (Test-Path $Config)) {
    if (-not (Test-Path $ExampleConfig)) {
        throw "config.example.json not found: $ExampleConfig"
    }
    Copy-Item -LiteralPath $ExampleConfig -Destination $Config
    Write-Host "已从 config.example.json 生成本地 config.json。"
}

function Test-PythonVersion {
    param([string]$Command, [string[]]$Arguments = @())
    try {
        $VersionText = & $Command @Arguments -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"
        $Parts = [string]$VersionText -split "\."
        if ($Parts.Count -lt 2) { return $false }
        $Major = [int]$Parts[0]
        $Minor = [int]$Parts[1]
        return ($Major -gt 3) -or ($Major -eq 3 -and $Minor -ge 8)
    }
    catch {
        return $false
    }
}

if (-not (Test-Path $Python)) {
    $SystemPython = Get-Command py -ErrorAction SilentlyContinue
    if ($SystemPython) {
        if (-not (Test-PythonVersion "py" @("-3"))) {
            throw "Python >= 3.8 is required. Install a newer Python, then rerun this script."
        }
        & py -3 -m venv .venv
    }
    else {
        $SystemPython = Get-Command python -ErrorAction SilentlyContinue
        if (-not $SystemPython) {
            throw "Python 3 was not found. Install Python, then rerun this script."
        }
        if (-not (Test-PythonVersion "python")) {
            throw "Python >= 3.8 is required. Install a newer Python, then rerun this script."
        }
        & python -m venv .venv
    }
}

& $Python -m pip install --upgrade pip

$InstallRequirements = $Requirements
if (-not (Get-Command nvidia-smi -ErrorAction SilentlyContinue)) {
    $CpuRequirements = Join-Path $env:TEMP "podcast-transcriber-requirements-cpu.txt"
    Get-Content -LiteralPath $Requirements -Encoding UTF8 |
        Where-Object { $_ -notmatch "^\s*nvidia-" } |
        Set-Content -LiteralPath $CpuRequirements -Encoding UTF8
    $InstallRequirements = $CpuRequirements
    Write-Host "未检测到 nvidia-smi，本次安装跳过 CUDA wheel；运行时会使用 CPU fallback。"
}

& $Python -m pip install -r $InstallRequirements

New-Item -ItemType Directory -Force -Path `
    (Join-Path $Root "input"), `
    (Join-Path $Root "output"), `
    (Join-Path $Root "work"), `
    (Join-Path $Root "models") | Out-Null

Write-Host "Setup complete: $Python"
