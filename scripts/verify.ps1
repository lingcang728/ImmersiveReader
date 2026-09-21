[CmdletBinding()]
param(
    # Promote every report-only audit (fmt/audit/PSSA findings, missing optional
    # tools) into a hard failure — the mode CI/release machines should run.
    [switch]$Strict,
    # Also run the miri-harness crate (apps/desktop/src-tauri/miri-harness)
    # under `cargo +nightly miri test`. Off by default: it needs a nightly
    # toolchain with the miri component, which most machines lack. An explicit
    # request that cannot run fails loudly instead of silently skipping.
    [switch]$WithMiri
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'common.ps1')

# Every gate that ran report-only or was skipped lands here so the final
# output can never read as a stronger pass than what actually ran.
$script:AdvisoryFindings = [System.Collections.Generic.List[string]]::new()

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
    # build gate, not an audit gate. -Strict promotes findings into failures.
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][scriptblock]$Command
    )

    Write-Output "[verify] $Label (report-only)"
    $global:LASTEXITCODE = $null
    try {
        & $Command
        if ($null -ne $LASTEXITCODE -and $LASTEXITCODE -ne 0) {
            $script:AdvisoryFindings.Add($Label)
            if ($Strict) {
                throw "[verify] $Label 报告了问题（退出码 $LASTEXITCODE），-Strict 模式下视为失败。"
            }
            Write-Warning "[verify] $Label 报告了问题（退出码 $LASTEXITCODE），详见上方输出；不阻断构建门槛。"
        }
    } catch {
        $script:AdvisoryFindings.Add($Label)
        if ($Strict) {
            throw
        }
        Write-Warning "[verify] $Label 运行失败：$($_.Exception.Message)"
    }
}

function Skip-Gate {
    # Record a skipped/missing-tool gate so it surfaces in the final summary;
    # under -Strict a missing required tool is a failure.
    param([Parameter(Mandatory)][string]$Label)
    $script:AdvisoryFindings.Add("$Label（跳过）")
    if ($Strict) {
        throw "[verify] $Label 所需工具缺失，-Strict 模式下视为失败。"
    }
    Write-Warning "[verify] $Label 所需工具未安装，已跳过。"
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
    # The release workflow also reads docs/release/<version>/ — a fourth leg
    # the version trio alone does not cover. runtime-parts.json legitimately
    # appears only after the bundle is packed, so this stays a warning.
    $releaseDir = Join-Path $root "docs\release\$desktopVersion"
    if (-not (Test-Path -LiteralPath $releaseDir -PathType Container)) {
        $script:AdvisoryFindings.Add("docs\release\$desktopVersion 缺失")
        Write-Warning "[verify] docs\release\$desktopVersion\ 不存在——推送 v$desktopVersion 前必须补齐 RELEASE_NOTES.md 与 runtime-parts.json（见 docs\release\CHECKLIST.md）。"
    } elseif (-not (Test-Path -LiteralPath (Join-Path $releaseDir 'runtime-parts.json') -PathType Leaf)) {
        $script:AdvisoryFindings.Add("docs\release\$desktopVersion\runtime-parts.json 缺失")
        Write-Warning "[verify] docs\release\$desktopVersion\runtime-parts.json 缺失——发布前用 scripts\pack-runtime-bundle.ps1 生成。"
    }
    Write-Output "[verify] version fields aligned at $desktopVersion"
}

function Assert-NoLegacyRuntimeReferences {
    $legacyRoots = @(
        'C:\Users\15pro\Desktop\MyProject\MMbook',
        'C:\Users\15pro\Desktop\MyProject\Zhihu_packer',
        'C:\Users\15pro\Desktop\MyProject\PodcastTranscriber'
    )
    # Whole tracked tree, not just apps/packages/tools: install/migration
    # scripts, NSIS hooks, CI YAML, JSON configs and docs are exactly where a
    # retired absolute path would hide. Text extensions only — Select-String
    # must open real files.
    $sourceFiles = @(& git -C $root ls-files |
        Where-Object { $_ -match '\.(rs|ts|tsx|svelte|js|mjs|cjs|py|ps1|psm1|nsi|json|ya?ml|toml|md|txt|cfg|ini|html|css)$' } |
        # This file necessarily contains the legacy path literals it scans
        # for — exclude it or the check would always match itself.
        Where-Object { $_ -ne 'scripts/verify.ps1' } |
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
    foreach ($relative in @(
        'apps\desktop\node_modules',
        'tools\zhihu-packer\node_modules',
        'tools\podcast-transcriber\.venv',
        # runtime trees are junction candidates too: PS 5.1 Remove-Item -Recurse
        # and robocopy /MIR both follow them into the target volume.
        'runtime',
        'runtime.__staging__',
        'runtime.__previous__'
    )) {
        $path = Join-Path $root $relative
        if (-not (Test-Path -LiteralPath $path)) {
            continue
        }
        $item = Get-Item -Force -LiteralPath $path
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "依赖/运行时目录仍是 Junction：$path"
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
        Remove-DirectoryTree -Path $path
    }
}

$root = Get-RepoRoot
$npm = Require-Command -Name 'npm.cmd'
$cargo = Require-Command -Name 'cargo.exe'
$python = Get-PodcastPython
$node = Require-Command -Name 'node.exe'
# node --test over .ts files relies on built-in type stripping, unflagged since
# Node 22.18; zhihu-packer's node:sqlite needs >=22.5. Gate the dev machine
# the same way prepare-runtime gates the vendored copy.
$nodeVersionText = [string](& $node --version 2>&1 | Select-Object -First 1)
if ($nodeVersionText -notmatch 'v?(\d+\.\d+\.\d+)' -or [version]$Matches[1] -lt [version]'22.18.0') {
    throw "node.exe 版本过旧：$nodeVersionText（contracts/zhihu 测试与 engines 均要求 Node >= 22.18）"
}
$contractsLocalTsc = Join-Path $root 'packages\contracts\node_modules\.bin\tsc.cmd'
$typescript = if (Test-Path -LiteralPath $contractsLocalTsc -PathType Leaf) {
    $contractsLocalTsc
} else {
    Join-Path $root 'apps\desktop\node_modules\.bin\tsc.cmd'
}
if (-not (Test-Path -LiteralPath $typescript -PathType Leaf)) {
    throw '未找到可用的 TypeScript 编译器；请先在 packages/contracts 或 apps/desktop 执行 npm ci。'
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
    Invoke-Checked 'desktop Rust tests' { & $cargo test --locked --manifest-path src-tauri\Cargo.toml }
    Invoke-Checked 'desktop Rust check' { & $cargo check --locked --manifest-path src-tauri\Cargo.toml }
    Invoke-Checked 'desktop Rust clippy' {
        & $cargo clippy --locked --manifest-path src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
    }
    # The tree is rustfmt-clean; keep it that way. cargo-fmt is a rustup
    # component — absence is a skipped gate, not a failure (unless -Strict).
    if (Get-Command cargo-fmt.exe -ErrorAction SilentlyContinue) {
        Invoke-Checked 'desktop Rust fmt --check' {
            & $cargo fmt --manifest-path src-tauri\Cargo.toml -- --check
        }
    } else {
        Skip-Gate 'cargo fmt --check'
    }
    $cargoAudit = Get-Command cargo-audit.exe -ErrorAction SilentlyContinue
    if ($cargoAudit) {
        Invoke-Optional 'cargo audit (Rust deps)' {
            Push-Location src-tauri
            try { & $cargoAudit.Source audit } finally { Pop-Location }
        }
    } else {
        Skip-Gate 'cargo audit'
    }
    Invoke-Optional 'npm audit (apps/desktop)' { & $npm audit }
    # Opt-in undefined-behaviour gate for atomic_file.rs (the cfg_attr(miri,
    # ignore) marks make sense only when this actually runs).
    if ($WithMiri) {
        Invoke-Checked 'atomic_file Miri harness' {
            & $cargo +nightly miri test --manifest-path src-tauri\miri-harness\Cargo.toml
        }
    }
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
    Invoke-Optional 'PSScriptAnalyzer' {
        $findings = @(Invoke-ScriptAnalyzer -Path (Join-Path $root 'scripts') -Recurse -Severity 'Warning', 'Error')
        if ($findings.Count -gt 0) {
            $findings | Select-Object -First 20 | ForEach-Object {
                Write-Output "  [pssa] $($_.ScriptName):$($_.Line) $($_.RuleName) $($_.Message)"
            }
            # Surface through $LASTEXITCODE so Invoke-Optional/-Strict can see it.
            $global:LASTEXITCODE = 1
        }
    }
} else {
    Skip-Gate 'PSScriptAnalyzer'
}

# Pester unit tests for the script surface itself (syntax parsing, shared
# helpers, junction-safe deletion). Optional locally — machines without the
# module just skip; -Strict treats a missing Pester as a failure.
$scriptTests = Join-Path $PSScriptRoot 'tests'
if ((Test-Path -LiteralPath $scriptTests -PathType Container) -and
    (Get-Module -ListAvailable Pester | Select-Object -First 1)) {
    Invoke-Checked 'PowerShell script tests (Pester)' {
        $pesterResult = Invoke-Pester -Path $scriptTests -PassThru
        if ($pesterResult.FailedCount -gt 0) {
            throw "Pester 失败 $($pesterResult.FailedCount) 项"
        }
    }
} else {
    Skip-Gate 'Pester script tests'
}

# Managed-runtime integrity gate: hard check when the vendored runtime is
# provisioned, warn-only skip on a dev checkout that lacks it.
$runtimeManifest = Join-Path $root 'runtime\manifest.json'
if (Test-Path -LiteralPath $runtimeManifest -PathType Leaf) {
    Invoke-Checked 'managed runtime manifest' {
        & (Join-Path $PSScriptRoot 'verify-runtime.ps1')
    }
} else {
    # Environmental, not a finding: CI/dev checkouts have no vendored runtime,
    # so this stays warn-only even under -Strict.
    Write-Warning '[verify] runtime\manifest.json 不存在——该检出未包含受管运行时，跳过完整性校验（可运行 scripts\prepare-runtime.ps1 生成）。'
}

# Final honesty pass: a green line must show which advisory gates were skipped
# or reported findings, so "all checks passed" can't be over-read.
if ($script:AdvisoryFindings.Count -gt 0) {
    Write-Output "[verify] report-only/跳过的门禁：$($script:AdvisoryFindings -join '；')"
}
Write-Output '[verify] all checks passed'
