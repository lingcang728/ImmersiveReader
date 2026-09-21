param(
    [switch]$DryRun,
    [Parameter(Mandatory)][string]$SourceRoot,
    [string]$LibraryRoot = (Join-Path $env:USERPROFILE 'Documents\沉浸阅读\Library'),
    # Matches storage.rs: the sidecar DB lives at Data\Zhihu\zhihu-packer.db and
    # the browser profile at Data\Private\ZhihuProfile — not a top-level
    # "zhihu" dir (that was the pre-v3 layout).
    [string]$DataRoot = (Join-Path $env:LOCALAPPDATA 'ImmersiveReader\Data')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'common.ps1')

function Get-Hash {
    param([Parameter(Mandatory)][string]$Path)
    # Get-FileHash is absent on some Windows PowerShell 5.1 images — fall back
    # to raw .NET so this script runs under either shell.
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

function Test-IncludedArchiveFile {
    param([Parameter(Mandatory)][System.IO.FileInfo]$File)
    return $File.Name -notin @('reader.html', 'universal-reader.html', 'manifest.json', '.reading.json')
}

function Get-SourceFiles {
    param(
        [Parameter(Mandatory)][string]$Root,
        [string[]]$ExcludeDirectories = @()
    )
    if (-not (Test-Path -LiteralPath $Root)) {
        return @()
    }
    # Match migrate-v3-production-data.ps1: a reparse directory in the source
    # would be followed and copy the link target — refuse outright instead.
    $reparse = @(Get-ChildItem -LiteralPath $Root -Directory -Force -Recurse | Where-Object {
        ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
    })
    if ($reparse.Count -gt 0) {
        throw "迁移源包含重解析目录，已停止：$($reparse[0].FullName)"
    }
    return @(Get-ChildItem -LiteralPath $Root -Recurse -File | Where-Object {
        $file = $_
        if (-not (Test-IncludedArchiveFile -File $file)) { return $false }
        if ($ExcludeDirectories.Count -gt 0) {
            $relative = $file.FullName.Substring($Root.TrimEnd('\').Length).TrimStart('\')
            # 与 migrate-v3 的 Test-ExcludedRelativePath 一致：任一路径段命中即排除。
            foreach ($part in ($relative -split '\\')) {
                if ($part -in $ExcludeDirectories) { return $false }
            }
        }
        return $true
    })
}

function Copy-TreeSafely {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][bool]$Preview,
        [string[]]$ExcludeDirectories = @()
    )

    $files = Get-SourceFiles -Root $Source -ExcludeDirectories $ExcludeDirectories
    $conflicts = [System.Collections.Generic.List[string]]::new()
    $pending = [System.Collections.Generic.List[object]]::new()
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($Source.TrimEnd('\\').Length).TrimStart('\\')
        $target = Join-Path $Destination $relative
        if (Test-Path -LiteralPath $target) {
            if ((Get-Hash -Path $file.FullName) -ne (Get-Hash -Path $target)) {
                $conflicts.Add($relative)
            }
        } else {
            $pending.Add([pscustomobject]@{ Source = $file.FullName; Target = $target; Relative = $relative })
        }
    }
    if ($conflicts.Count -gt 0) {
        throw "目标存在不同内容，迁移已停止：$($conflicts -join ', ')"
    }
    if (-not $Preview) {
        foreach ($item in $pending) {
            $parent = Split-Path -Parent $item.Target
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
            Copy-Item -LiteralPath $item.Source -Destination $item.Target
        }
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($Source.TrimEnd('\\').Length).TrimStart('\\')
            $target = Join-Path $Destination $relative
            if (-not (Test-Path -LiteralPath $target)) {
                throw "迁移后缺少文件：$relative"
            }
            if ((Get-Hash -Path $file.FullName) -ne (Get-Hash -Path $target)) {
                throw "迁移后哈希不一致：$relative"
            }
        }
    }
    return [pscustomobject]@{
        Source = $Source
        Destination = $Destination
        Total = $files.Count
        Pending = $pending.Count
        Existing = $files.Count - $pending.Count
        Conflicts = $conflicts.Count
    }
}

function Invoke-SqliteScalar {
    param(
        [Parameter(Mandatory)][string]$Sqlite,
        [Parameter(Mandatory)][string]$Database,
        [Parameter(Mandatory)][string]$Sql
    )
    $result = & $Sqlite -batch -noheader $Database $Sql
    if ($LASTEXITCODE -ne 0) { throw "SQLite 命令失败，退出码 $LASTEXITCODE" }
    return ([string]($result -join "`n")).Trim()
}

function Copy-SqliteDatabaseSafely {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][bool]$Preview,
        [Parameter(Mandatory)][string]$Sqlite
    )
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "源文件不存在：$Source"
    }
    # A plain file copy of a live WAL database can tear: committed rows may still sit
    # in the -wal sidecar. VACUUM INTO reads through a transaction and emits one
    # consistent main-file snapshot (same recipe as migration/sqlite.rs).
    # It runs against a throwaway COPY of the source (db + -wal/-shm): opening
    # the live source database — even for VACUUM INTO — can create or replay
    # -wal/-shm sidecars, mutating the very migration source a dry run must
    # leave untouched (same guard as Test-SqliteIntegrityViaCopy in the v3 script).
    $snapshot = Join-Path ([IO.Path]::GetTempPath()) ("immersive-zhihu-db-{0}.db" -f [guid]::NewGuid().ToString('N'))
    $workDir = "$snapshot.src"
    New-Item -ItemType Directory -Path $workDir -Force | Out-Null
    try {
        $sourceCopy = Join-Path $workDir 'source.db'
        Copy-Item -LiteralPath $Source -Destination $sourceCopy
        foreach ($suffix in @('-wal', '-shm')) {
            $sidecar = "$Source$suffix"
            if (Test-Path -LiteralPath $sidecar -PathType Leaf) {
                Copy-Item -LiteralPath $sidecar -Destination "$sourceCopy$suffix"
            }
        }
        $escapedSnapshot = $snapshot.Replace("'", "''")
        $null = Invoke-SqliteScalar -Sqlite $Sqlite -Database $sourceCopy -Sql "VACUUM INTO '$escapedSnapshot';"
        if ((Invoke-SqliteScalar -Sqlite $Sqlite -Database $snapshot -Sql 'PRAGMA integrity_check;') -ne 'ok') {
            throw "数据库快照完整性校验失败：$Source"
        }
        if (Test-Path -LiteralPath $Destination) {
            if ((Get-Hash -Path $snapshot) -eq (Get-Hash -Path $Destination)) {
                return 'existing'
            }
            throw "目标数据库已存在且内容不同：$Destination"
        }
        if (-not $Preview) {
            New-Item -ItemType Directory -Path (Split-Path -Parent $Destination) -Force | Out-Null
            Copy-Item -LiteralPath $snapshot -Destination $Destination
            if ((Get-Hash -Path $snapshot) -ne (Get-Hash -Path $Destination)) {
                throw "复制后哈希不一致：$Destination"
            }
        }
        return 'pending'
    } finally {
        if (Test-Path -LiteralPath $snapshot) { Remove-Item -LiteralPath $snapshot -Force }
        if (Test-Path -LiteralPath $workDir) { Remove-Item -LiteralPath $workDir -Recurse -Force }
    }
}

if (-not (Test-Path -LiteralPath $SourceRoot)) {
    throw "知乎源项目不存在：$SourceRoot"
}

$sourceOutput = Join-Path $SourceRoot 'output'
$targetOutput = Join-Path $LibraryRoot '知乎'
$sourceDb = Join-Path $SourceRoot 'zhihu-packer.db'
$targetDb = Join-Path $DataRoot 'Zhihu\zhihu-packer.db'
$sourceProfile = Join-Path $SourceRoot '.browser-profile'
$targetProfile = Join-Path $DataRoot 'Private\ZhihuProfile'
$reports = [System.Collections.Generic.List[object]]::new()

foreach ($book in Get-ChildItem -LiteralPath $sourceOutput -Directory) {
    $reports.Add((Copy-TreeSafely -Source $book.FullName -Destination (Join-Path $targetOutput $book.Name) -Preview $DryRun.IsPresent))
}

$sqlite = (Get-Command sqlite3.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source)
if (-not $sqlite) { throw '未找到已安装的 sqlite3.exe；按仓库规则不会自动安装第二份。' }
$dbState = Copy-SqliteDatabaseSafely -Source $sourceDb -Destination $targetDb -Preview $DryRun.IsPresent -Sqlite $sqlite
if (Test-Path -LiteralPath $sourceProfile) {
    # 与 migrate-v3-production-data.ps1 同一排除清单：浏览器 profile 里的
    # Cache/Code Cache/GPUCache 等是再生缓存，整树搬运只会把旧垃圾带进新 profile。
    $excludedProfileDirectories = @('Cache', 'Code Cache', 'GPUCache', 'GrShaderCache', 'ShaderCache')
    $reports.Add((Copy-TreeSafely -Source $sourceProfile -Destination $targetProfile -Preview $DryRun.IsPresent -ExcludeDirectories $excludedProfileDirectories))
}

$root = Get-RepoRoot
$npm = Require-Command -Name 'npm.cmd'
$env:IMMERSIVE_ZHIHU_OUTPUT = if ($DryRun) { $sourceOutput } else { $targetOutput }
$env:IMMERSIVE_ZHIHU_DB = if ($DryRun) { $sourceDb } else { $targetDb }
$arguments = @('--prefix', (Join-Path $root 'tools\zhihu-packer'), 'run', 'build-manifests')
if ($DryRun) {
    $arguments += @('--', '--dry-run')
}
& $npm @arguments
if ($LASTEXITCODE -ne 0) {
    throw "manifest 生成检查失败，退出码 $LASTEXITCODE"
}

if (-not $DryRun) {
    & $npm --prefix (Join-Path $root 'tools\zhihu-packer') run build-reader:from-manifests
    if ($LASTEXITCODE -ne 0) {
        throw "Reader 构建失败，退出码 $LASTEXITCODE"
    }
}

$report = [ordered]@{
    timestamp = (Get-Date).ToString('o')
    dryRun = $DryRun.IsPresent
    sourceRoot = $SourceRoot
    libraryRoot = $LibraryRoot
    dataRoot = $DataRoot
    database = $dbState
    trees = @($reports)
}
$report | ConvertTo-Json -Depth 6

if (-not $DryRun) {
    $reportDir = Join-Path $root 'artifacts\migration'
    New-Item -ItemType Directory -Path $reportDir -Force | Out-Null
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $reportDir 'latest.json') -Encoding UTF8
}
