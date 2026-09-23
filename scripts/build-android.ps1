<#
.SYNOPSIS
    沉浸阅读 (ImmersiveReader) Android APK 构建与打包脚本
.DESCRIPTION
    自动化检测 Android 开发环境 (JDK, Android SDK, NDK, Rust targets)，
    并调用 Tauri 2 CLI 打包输出适配 OPPO Find X9 (ARM64-v8a) 及主流安卓机型的 APK 安装包。
.PARAMETER Target
    目标架构，默认 'aarch64' (对应 OPPO Find X9 等 ARM64 设备)。可选: aarch64, armv7, x86_64, universal
.PARAMETER Release
    是否打包生产 Release 版本 (优化体积与性能并进行代码混淆)
.PARAMETER SkipToolchainCheck
    跳过 Rust target 和环境检查
#>
[CmdletBinding()]
param(
    [ValidateSet('aarch64', 'armv7', 'x86_64', 'universal')]
    [string]$Target = 'aarch64',
    [switch]$Release,
    [switch]$SkipToolchainCheck
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$desktopDir = Join-Path $repoRoot 'apps\desktop'
$srcTauriDir = Join-Path $desktopDir 'src-tauri'

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  沉浸阅读 Android APK 打包脚本 (Tauri 2 Mobile)  " -ForegroundColor Cyan
Write-Host "  适配机型: OPPO Find X9 (ColorOS 16 / Android 16)" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan

# 1. 检查 Node / npm
$npm = Get-Command 'npm.cmd' -ErrorAction SilentlyContinue
if (-not $npm) {
    throw "未找到 npm.cmd，请确保已安装 Node.js (>=22.18)。"
}

# 2. 检查 Java JDK
$java = Get-Command 'java.exe' -ErrorAction SilentlyContinue
if (-not $java -and -not $env:JAVA_HOME) {
    throw "未找到 Java 运行环境。请设置 JAVA_HOME 或将 JDK 添加到 PATH。"
}
if (-not $env:JAVA_HOME -and $java) {
    $javaBin = Split-Path -Parent $java.Source
    $candidateHome = Split-Path -Parent $javaBin
    if (Test-Path -LiteralPath (Join-Path $candidateHome 'bin\java.exe')) {
        $env:JAVA_HOME = $candidateHome
        Write-Host "自动推导 JAVA_HOME: $env:JAVA_HOME" -ForegroundColor Cyan
    }
}
if ($env:JAVA_HOME -and (Test-Path -LiteralPath $env:JAVA_HOME)) {
    Write-Host "[OK] JAVA_HOME: $env:JAVA_HOME" -ForegroundColor Green
    if ($java) {
        $javaVersionOutput = & $java.Source -version 2>&1 | Out-String
        if ($javaVersionOutput -match 'version "(\d+)') {
            $majorVersion = [int]$matches[1]
            if ($majorVersion -gt 21) {
                Write-Warning "[WARN] 当前 Java 大版本为 JDK $majorVersion。Android Gradle 插件 (AGP 8.x) 最佳兼容为 JDK 17 或 21。"
                Write-Warning "若 Gradle 报错 'Unsupported class file major version'，建议临时设置 `$env:JAVA_HOME 指向 JDK 17/21。"
            }
        }
    }
} elseif ($java) {
    Write-Host "[OK] java: $($java.Source)" -ForegroundColor Green
}

# 2.5 AGP 8.x 需要 JDK 17/21：JAVA_HOME 缺失或指向更高版本时，自动切换到 G:\build_cache 下的 JDK 21
$javaHomeExe = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\java.exe' } else { $null }
$javaHomeMajor = $null
if ($javaHomeExe -and (Test-Path -LiteralPath $javaHomeExe)) {
    $javaHomeOut = & $javaHomeExe -version 2>&1 | Out-String
    if ($javaHomeOut -match 'version "(\d+)') { $javaHomeMajor = [int]$matches[1] }
}
if (-not $env:JAVA_HOME -or -not $javaHomeMajor -or $javaHomeMajor -gt 21) {
    $jdk21 = Get-ChildItem -LiteralPath 'G:\build_cache' -Directory -Filter 'jdk-21*' -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
    if ($jdk21 -and (Test-Path -LiteralPath (Join-Path $jdk21.FullName 'bin\java.exe'))) {
        Write-Host "自动切换 JAVA_HOME -> $($jdk21.FullName)（AGP 兼容版本）" -ForegroundColor Cyan
        $env:JAVA_HOME = $jdk21.FullName
    } elseif ($javaHomeMajor -gt 21) {
        Write-Warning "[WARN] JAVA_HOME 指向 JDK $javaHomeMajor，且未在 G:\build_cache 找到 jdk-21*，Android 构建可能失败。"
    }
}

# 3. 检查 Android SDK
$sdkCandidates = @(
    $env:ANDROID_HOME,
    $env:ANDROID_SDK_ROOT,
    (Join-Path $env:LOCALAPPDATA 'Android\Sdk'),
    'C:\Android\Sdk'
)
$foundSdk = $null
foreach ($path in $sdkCandidates) {
    if ($path -and (Test-Path -LiteralPath $path)) {
        $foundSdk = $path
        break
    }
}

if ($foundSdk) {
    $env:ANDROID_HOME = $foundSdk
    $env:ANDROID_SDK_ROOT = $foundSdk
    Write-Host "[OK] ANDROID_HOME: $foundSdk" -ForegroundColor Green
} else {
    Write-Warning "[WARN] 未在系统或默认路径检测到 Android SDK。"
    Write-Warning "若尚未安装 Android SDK，请下载 Android Studio 或通过 commandlinetools 安装 SDK Platforms (API 34/35)。"
    Write-Warning "安装后请设置环境变量 ANDROID_HOME 指向 SDK 目录。"
}

# 4. 检查 Android NDK
if (-not $env:NDK_HOME -and $env:ANDROID_NDK_HOME) {
    $env:NDK_HOME = $env:ANDROID_NDK_HOME
}
if (-not $env:NDK_HOME -and $env:ANDROID_HOME) {
    $ndkDir = Join-Path $env:ANDROID_HOME 'ndk'
    if (Test-Path -LiteralPath $ndkDir) {
        $latestNdk = Get-ChildItem -Path $ndkDir -Directory | Sort-Object Name -Descending | Select-Object -First 1
        if ($latestNdk) {
            $env:NDK_HOME = $latestNdk.FullName
        }
    }
}
if ($env:NDK_HOME) {
    Write-Host "[OK] NDK_HOME: $env:NDK_HOME" -ForegroundColor Green
} else {
    Write-Warning "[WARN] 未检测到 Android NDK。Tauri 构建 C/Rust 本地库需要 NDK (建议 NDK r26b+)。"
}

# 5. 检查 Rust Android Target
if (-not $SkipToolchainCheck) {
    $rustup = Get-Command 'rustup.exe' -ErrorAction SilentlyContinue
    if ($rustup) {
        $rustTargets = @{
            'aarch64'   = @('aarch64-linux-android')
            'armv7'     = @('armv7-linux-androideabi')
            'x86_64'    = @('x86_64-linux-android')
            'universal' = @('aarch64-linux-android', 'armv7-linux-androideabi', 'x86_64-linux-android')
        }
        $targetTriples = $rustTargets[$Target]
        if ($targetTriples) {
            $installedTargets = @(& $rustup.Source target list --installed)
            foreach ($triple in $targetTriples) {
                Write-Host "检查 Rust target: $triple..."
                if ($installedTargets -notcontains $triple) {
                    Write-Host "正在为 rustup 添加 target: $triple..." -ForegroundColor Yellow
                    & $rustup.Source target add $triple
                } else {
                    Write-Host "[OK] Rust target 已就绪: $triple" -ForegroundColor Green
                }
            }
        }
    }
}

# 6. 执行前端构建校验
Write-Host "正在编译前端并校验类型..." -ForegroundColor Cyan
& $npm.Source --prefix $desktopDir run check
if ($LASTEXITCODE -ne 0) {
    throw "前端类型检查失败，请先修复前端错误。"
}
& $npm.Source --prefix $desktopDir run build
if ($LASTEXITCODE -ne 0) {
    throw "前端生产打包失败。"
}

# 7. 执行 Tauri Android 构建
$tauriCli = Join-Path $desktopDir 'node_modules\.bin\tauri.cmd'
if (-not (Test-Path -LiteralPath $tauriCli)) {
    throw "未找到 Tauri CLI。请在 apps/desktop 下运行 npm ci。"
}

Push-Location $desktopDir
try {
    $genAndroid = Join-Path $srcTauriDir 'gen\android'
    if (-not (Test-Path -LiteralPath $genAndroid)) {
        Write-Host "检测到尚未生成 Android 项目工程，正在调用 tauri android init..." -ForegroundColor Cyan
        & $tauriCli android init --ci --skip-targets-install
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "Tauri Android 项目初始化因环境缺失未能完成（需配置 ANDROID_HOME 与 NDK_HOME）。"
        }
    }

    Write-Host "正在调用 Tauri Android 构建 ($Target)..." -ForegroundColor Cyan
    $buildArgs = @('android', 'build', '--apk')
    if ($Target -ne 'universal') {
        $buildArgs += @('--target', $Target)
    }
    if ($Release) {
        # release 模式 (默认会构建 release APK)
    } else {
        $buildArgs += @('--debug')
    }

    Write-Host "执行构建命令: $tauriCli ($($buildArgs -join ' '))"
    & $tauriCli $buildArgs

    if ($LASTEXITCODE -eq 0) {
        Write-Host "==================================================" -ForegroundColor Green
        Write-Host "  APK 打包成功！" -ForegroundColor Green
        Write-Host "==================================================" -ForegroundColor Green
        
        # 查找并输出 APK 位置
        $apkSearchPath = Join-Path $srcTauriDir 'gen\android\app\build\outputs\apk'
        if (Test-Path -LiteralPath $apkSearchPath) {
            $apks = Get-ChildItem -Path $apkSearchPath -Filter '*.apk' -Recurse
            $mobileOut = Join-Path $repoRoot 'output\mobile'
            New-Item -ItemType Directory -Force -Path $mobileOut | Out-Null
            foreach ($apk in $apks) {
                $hash = (Get-FileHash -LiteralPath $apk.FullName -Algorithm SHA256).Hash
                $sizeMb = [math]::Round($apk.Length / 1MB, 2)
                Write-Host "产物文件: $($apk.FullName)" -ForegroundColor Yellow
                Write-Host "文件大小: $sizeMb MB" -ForegroundColor Yellow
                Write-Host "SHA-256:  $hash" -ForegroundColor Yellow
                Copy-Item -LiteralPath $apk.FullName -Destination (Join-Path $mobileOut $apk.Name) -Force
                Write-Host "已收纳到: $(Join-Path $mobileOut $apk.Name)" -ForegroundColor Green
            }
        }
    } else {
        Write-Host "Tauri Android 构建退出码: $LASTEXITCODE" -ForegroundColor Red
    }
} finally {
    Pop-Location
}
