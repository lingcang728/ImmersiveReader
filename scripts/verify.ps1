$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][scriptblock]$Command
    )

    Write-Output "[verify] $Label"
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Label 失败，退出码 $LASTEXITCODE"
    }
}

function Assert-NoLegacyRuntimeReferences {
    $legacyRoots = @(
        'C:\Users\15pro\Desktop\MyProject\MMbook',
        'C:\Users\15pro\Desktop\MyProject\Zhihu_packer',
        'C:\Users\15pro\Desktop\MyProject\PodcastTranscriber'
    )
    $sourceFiles = & git -C $root ls-files -- apps packages tools |
        Where-Object { $_ -match '\.(rs|ts|svelte|py|ps1)$' } |
        ForEach-Object { Join-Path $root $_ }
    if ($LASTEXITCODE -ne 0) {
        throw "无法读取 Git 源文件清单，退出码 $LASTEXITCODE"
    }
    foreach ($legacy in $legacyRoots) {
        $matches = $sourceFiles | Select-String -SimpleMatch -Pattern $legacy
        if ($matches) {
            throw "产品源码仍引用旧项目路径：$legacy"
        }
    }
    foreach ($relative in @('apps\desktop\node_modules', 'tools\zhihu-packer\node_modules', 'tools\podcast-transcriber\.venv')) {
        $path = Join-Path $root $relative
        if (-not (Test-Path -LiteralPath $path)) {
            continue
        }
        $item = Get-Item -Force -LiteralPath $path
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "依赖目录仍是 Junction：$path"
        }
    }
    Write-Output '[verify] no legacy runtime paths or junctions'
}

function Remove-FreshGeneratedDirectory {
    param([Parameter(Mandatory)][string]$RelativePath)

    $path = Join-Path $root $RelativePath
    $resolvedRoot = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    $resolvedPath = [IO.Path]::GetFullPath($path)
    if (-not $resolvedPath.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "生成物路径越界：$RelativePath"
    }
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}

$root = Get-RepoRoot
$npm = Require-Command -Name 'npm.cmd'
$cargo = Require-Command -Name 'cargo.exe'
$python = Get-PodcastPython
Assert-NoLegacyRuntimeReferences
Invoke-Checked 'contract schema parity' { & $python $root\scripts\verify_contract_parity.py }

Push-Location (Join-Path $root 'packages\contracts')
try {
    Remove-FreshGeneratedDirectory 'packages\contracts\dist'
    Invoke-Checked 'contracts tests' { node --test tests/*.test.ts }
    Invoke-Checked 'contracts build' { tsc -p tsconfig.json }
} finally {
    Pop-Location
}

$ruff = Get-Command ruff.exe -ErrorAction SilentlyContinue
if (-not $ruff) {
    throw '未找到本机 ruff.exe（请复用全局安装，勿在项目内重复安装）'
}

Push-Location (Join-Path $root 'apps\desktop')
try {
    Invoke-Checked 'desktop tests' { & $npm test }
    Invoke-Checked 'desktop Svelte check' { & $npm run check }
    Invoke-Checked 'desktop Rust tests' { & $cargo test --manifest-path src-tauri\Cargo.toml }
    Invoke-Checked 'desktop Rust check' { & $cargo check --manifest-path src-tauri\Cargo.toml }
    Invoke-Checked 'desktop Rust clippy' {
        & $cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
    }
} finally {
    Pop-Location
}

Push-Location (Join-Path $root 'tools\zhihu-packer')
try {
    Remove-FreshGeneratedDirectory 'tools\zhihu-packer\dist'
    Invoke-Checked 'Zhihu tests' { & $npm test }
    Invoke-Checked 'Zhihu TypeScript build' { & $npm run build }
    Invoke-Checked 'Zhihu Reader compile' { & $npm run compile-reader }
} finally {
    Pop-Location
}

Push-Location (Join-Path $root 'tools\podcast-transcriber')
try {
    Invoke-Checked 'Podcast Ruff' { & $ruff.Source check scripts tests }
    if ((Split-Path -Leaf $python) -ieq 'py.exe') {
        Invoke-Checked 'Podcast tests' { & $python -3 -m pytest -q }
        Invoke-Checked 'Podcast quick validation' { & $python -3 scripts\quick_validate.py }
    } else {
        Invoke-Checked 'Podcast tests' { & $python -m pytest -q }
        Invoke-Checked 'Podcast quick validation' { & $python scripts\quick_validate.py }
    }
} finally {
    Pop-Location
}

Write-Output '[verify] all checks passed'
