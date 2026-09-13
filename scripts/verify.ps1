$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][scriptblock]$Command
    )

    Write-Output "[verify] $Label"
    # Reset before invoking: a block that runs no native command would
    # otherwise inherit a stale code from the previous step, and $null must
    # read as success rather than rendering an empty "退出码 " in the message.
    $global:LASTEXITCODE = $null
    & $Command
    $exitCode = $LASTEXITCODE
    if ($null -ne $exitCode -and $exitCode -ne 0) {
        throw "$Label 失败，退出码 $exitCode"
    }
}

function Invoke-Optional {
    # Report-only audit step: findings warn but never fail — verify.ps1 stays a
    # build gate, not an audit gate.
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][scriptblock]$Command
    )

    Write-Output "[verify] $Label (report-only)"
    $global:LASTEXITCODE = $null
    try {
        & $Command
        if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
            Write-Warning "[verify] $Label 报告了问题（退出码 $LASTEXITCODE），详见上方输出；不阻断构建门槛。"
        }
    } catch {
        Write-Warning "[verify] $Label 运行失败：$($_.Exception.Message)"
    }
}

function Assert-VersionConsistency {
    # release.yml asserts the same trio right before `tauri build`; catching a
    # drifted Cargo.toml/tauri.conf.json version here is much cheaper.
    $desktopVersion = [string](Get-Content -LiteralPath (Join-Path $root 'apps\desktop\package.json') -Raw | ConvertFrom-Json).version
    $tauriVersion = [string](Get-Content -LiteralPath (Join-Path $root 'apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
    $cargoToml = Get-Content -LiteralPath (Join-Path $root 'apps\desktop\src-tauri\Cargo.toml') -Raw
    $packageSection = [regex]::Match($cargoToml, '(?ms)^\[package\](.*?)(?=^\[)')
    if (-not $packageSection.Success) { throw 'Cargo.toml is missing a [package] section.' }
    $cargoMatch = [regex]::Match($packageSection.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"')
    if (-not $cargoMatch.Success) { throw 'Cargo.toml [package] is missing a version.' }
    $cargoVersion = $cargoMatch.Groups[1].Value
    if ($desktopVersion -cne $tauriVersion -or $desktopVersion -cne $cargoVersion) {
        throw "版本不一致：package.json=$desktopVersion tauri.conf.json=$tauriVersion Cargo.toml=$cargoVersion"
    }
    Write-Output "[verify] version fields aligned at $desktopVersion"
}

function Assert-NoLegacyRuntimeReferences {
    $legacyRoots = @(
        'C:\Users\15pro\Desktop\MyProject\MMbook',
        'C:\Users\15pro\Desktop\MyProject\Zhihu_packer',
        'C:\Users\15pro\Desktop\MyProject\PodcastTranscriber'
    )
    $sourceFiles = @(& git -C $root ls-files -- apps packages tools |
        Where-Object { $_ -match '\.(rs|ts|svelte|py|ps1)$' } |
        ForEach-Object { Join-Path $root $_ } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf })
    if ($LASTEXITCODE -ne 0) {
        throw "无法读取 Git 源文件清单，退出码 $LASTEXITCODE"
    }
    foreach ($legacy in $legacyRoots) {
        # -LiteralPath makes Select-String open each file and scan its contents;
        # piping the path strings would only search the paths themselves.
        $matches = Select-String -LiteralPath $sourceFiles -SimpleMatch -Pattern $legacy
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
$typescript = Join-Path $root 'apps\desktop\node_modules\.bin\tsc.cmd'
if (-not (Test-Path -LiteralPath $typescript -PathType Leaf)) {
    throw '未找到 apps/desktop 已安装的 TypeScript 编译器；请先在该目录执行 npm ci。'
}
Assert-NoLegacyRuntimeReferences
Assert-VersionConsistency
# Remove the built dist before parity: on a node without TS type-stripping
# the parity harness falls back to `dist/index.js`, and a stale build would
# be tested silently instead of the current source.
Remove-FreshGeneratedDirectory 'packages\contracts\dist'
$parityScript = Join-Path $root 'scripts\verify_contract_parity.py'
if ((Split-Path -Leaf $python) -ieq 'py.exe') {
    Invoke-Checked 'contract schema parity' { & $python -3 $parityScript }
} else {
    Invoke-Checked 'contract schema parity' { & $python $parityScript }
}

Push-Location (Join-Path $root 'packages\contracts')
try {
    Invoke-Checked 'contracts tests' { node --test tests/*.test.ts }
    Invoke-Checked 'contracts build' { & $typescript -p tsconfig.json }
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
    # rustfmt is optional tooling: run the check when it is on PATH, but keep it
    # report-only — the tree is not yet rustfmt-clean; promote to Invoke-Checked
    # once a formatting pass lands.
    if (Get-Command cargo-fmt.exe -ErrorAction SilentlyContinue) {
        Invoke-Optional 'desktop Rust fmt --check' {
            & $cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
        }
    } else {
        Write-Warning '[verify] cargo-fmt 未安装，跳过 fmt --check。'
    }
    $cargoAudit = Get-Command cargo-audit.exe -ErrorAction SilentlyContinue
    if ($cargoAudit) {
        Invoke-Optional 'cargo audit (Rust deps)' {
            Push-Location src-tauri
            try { & $cargoAudit.Source audit } finally { Pop-Location }
        }
    } else {
        Write-Warning '[verify] cargo-audit 未安装，跳过 Rust 依赖审计。'
    }
    Invoke-Optional 'npm audit (apps/desktop)' { & $npm audit }
} finally {
    Pop-Location
}

Push-Location (Join-Path $root 'tools\zhihu-packer')
try {
    Remove-FreshGeneratedDirectory 'tools\zhihu-packer\dist'
    Invoke-Checked 'Zhihu Reader compile (fresh)' { & $npm run compile-reader }
    Invoke-Checked 'Zhihu tests' { & $npm test }
    Invoke-Checked 'Zhihu TypeScript build' { & $npm run build }
    Invoke-Checked 'Zhihu Reader compile' { & $npm run compile-reader }
    Invoke-Optional 'npm audit (tools/zhihu-packer)' { & $npm audit }
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

$pssa = Get-Module -ListAvailable PSScriptAnalyzer | Select-Object -First 1
if ($pssa) {
    Write-Output '[verify] PSScriptAnalyzer (report-only)'
    try {
        $findings = @(Invoke-ScriptAnalyzer -Path (Join-Path $root 'scripts') -Recurse -Severity 'Warning', 'Error')
        if ($findings.Count -gt 0) {
            Write-Warning "[verify] PSScriptAnalyzer 报告 $($findings.Count) 项（scripts\ 目录），不阻断构建门槛。"
            $findings | Select-Object -First 20 | ForEach-Object {
                Write-Output "  [pssa] $($_.ScriptName):$($_.Line) $($_.RuleName) $($_.Message)"
            }
        }
    } catch {
        Write-Warning "[verify] PSScriptAnalyzer 运行失败：$($_.Exception.Message)"
    }
} else {
    Write-Warning '[verify] PSScriptAnalyzer 未安装，跳过脚本静态检查。'
}

# Managed-runtime integrity gate: hard check when the vendored runtime is
# provisioned, warn-only skip on a dev checkout that lacks it.
$runtimeManifest = Join-Path $root 'runtime\manifest.json'
if (Test-Path -LiteralPath $runtimeManifest -PathType Leaf) {
    Invoke-Checked 'managed runtime manifest' {
        & (Join-Path $PSScriptRoot 'verify-runtime.ps1')
    }
} else {
    Write-Warning '[verify] runtime\manifest.json 不存在——该检出未包含受管运行时，跳过完整性校验（可运行 scripts\prepare-runtime.ps1 生成）。'
}

Write-Output '[verify] all checks passed'
