param(
    [switch]$Apply,
    # Consume a run's rollback/actions.json and undo it in reverse order —
    # the journal is an executable record, not only documentation.
    [switch]$Rollback,
    [switch]$Force,
    [string]$LibraryRoot = (Join-Path $env:USERPROFILE 'Documents\沉浸阅读\Library'),
    [string]$LegacyReaderRoot = (Join-Path $env:APPDATA 'mmbook'),
    [string]$ReaderRoot = (Join-Path $env:APPDATA 'immersive-reader'),
    [string]$LegacyPodcastRoot = (Join-Path $env:LOCALAPPDATA 'ImmersiveReader\podcast'),
    [string]$LegacyZhihuRoot = (Join-Path $env:LOCALAPPDATA 'ImmersiveReader\zhihu'),
    [string]$LocalAppRoot = (Join-Path $env:LOCALAPPDATA 'ImmersiveReader'),
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'common.ps1')

$CredentialTargets = @(
    'com.lingcang.immersivereading/deepseek-api-key',
    # QA channel target — must match secrets.rs QA_TARGET exactly; the app
    # never reads a ".dev" target.
    'com.lingcang.immersivereading.qa/deepseek-api-key'
)
$ExcludedProfileDirectories = @('Cache', 'Code Cache', 'GPUCache', 'GrShaderCache', 'ShaderCache')

function Get-Sha256 {
    param([Parameter(Mandatory)][string]$Path)
    # Get-FileHash is the fast path, but some Windows PowerShell 5.1 images
    # lack it — fall back to raw .NET so the migration runs under either shell.
    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $algorithm = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        } finally {
            $algorithm.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function Get-RelativePath {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Path
    )
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $pathFull = [System.IO.Path]::GetFullPath($Path)
    if (-not $pathFull.StartsWith($rootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "路径不在受管根目录内：$pathFull"
    }
    return $pathFull.Substring($rootFull.Length)
}

function Get-ByteArraySha256 {
    param([Parameter(Mandatory)][byte[]]$Bytes)
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($algorithm.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $algorithm.Dispose() }
}

function Write-JsonAtomic {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)]$Value
    )
    $parent = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temporary = "$Path.tmp-$PID"
    $Value | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $temporary -Encoding utf8
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Test-ExcludedRelativePath {
    param(
        [Parameter(Mandatory)][string]$RelativePath,
        [string[]]$ExcludedDirectories = @()
    )
    $parts = $RelativePath -split '[\\/]'
    foreach ($part in $parts) {
        if ($ExcludedDirectories -contains $part) { return $true }
    }
    return $false
}

function Get-TreeFiles {
    param(
        [Parameter(Mandatory)][string]$Root,
        [string[]]$ExcludedDirectories = @(),
        [string[]]$ExcludedFileNames = @(),
        [string]$Filter = '*'
    )
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return @() }
    $reparse = @(Get-ChildItem -LiteralPath $Root -Directory -Force -Recurse | Where-Object {
        ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
    })
    if ($reparse.Count -gt 0) {
        throw "迁移源包含重解析目录，已停止：$($reparse[0].FullName)"
    }
    return @(Get-ChildItem -LiteralPath $Root -File -Force -Recurse -Filter $Filter | Where-Object {
        $relative = Get-RelativePath -Root $Root -Path $_.FullName
        $_.Name -notin $ExcludedFileNames -and
            -not (Test-ExcludedRelativePath -RelativePath $relative -ExcludedDirectories $ExcludedDirectories)
    })
}

function Get-TreeSummary {
    param(
        [Parameter(Mandatory)][string]$Root,
        [string[]]$ExcludedDirectories = @(),
        [string[]]$ExcludedFileNames = @(),
        [string]$Filter = '*'
    )
    $files = @(Get-TreeFiles -Root $Root -ExcludedDirectories $ExcludedDirectories -ExcludedFileNames $ExcludedFileNames -Filter $Filter)
    $lines = @(foreach ($file in $files | Sort-Object FullName) {
        $relative = (Get-RelativePath -Root $Root -Path $file.FullName).Replace('\', '/')
        "$relative|$($file.Length)|$(Get-Sha256 -Path $file.FullName)"
    })
    $fingerprint = if ($lines.Count -eq 0) {
        Get-ByteArraySha256 -Bytes ([byte[]]@())
    } else {
        Get-ByteArraySha256 -Bytes ([Text.Encoding]::UTF8.GetBytes(($lines -join "`n")))
    }
    $measuredBytes = ($files | Measure-Object Length -Sum).Sum
    if ($null -eq $measuredBytes) { $measuredBytes = 0 }
    [pscustomobject]@{
        files = $files.Count
        bytes = [long]$measuredBytes
        sha256 = [string]$fingerprint
    }
}

function Copy-TreeVerified {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Target,
        [string[]]$ExcludedDirectories = @()
    )
    $sourceSummary = Get-TreeSummary -Root $Source -ExcludedDirectories $ExcludedDirectories
    if (Test-Path -LiteralPath $Target) {
        # Re-entrant apply: a target left by an earlier run is fine only when its
        # content already matches the source; anything else still stops the run.
        $existingSummary = Get-TreeSummary -Root $Target -ExcludedDirectories $ExcludedDirectories
        if ($sourceSummary.files -eq $existingSummary.files -and
            $sourceSummary.bytes -eq $existingSummary.bytes -and
            $sourceSummary.sha256 -eq $existingSummary.sha256) {
            return $existingSummary
        }
        throw "目标目录已存在且内容不一致，不能无损迁移：$Target"
    }
    New-Item -ItemType Directory -Path $Target -Force | Out-Null
    foreach ($file in Get-TreeFiles -Root $Source -ExcludedDirectories $ExcludedDirectories) {
        $relative = Get-RelativePath -Root $Source -Path $file.FullName
        $destination = Join-Path $Target $relative
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath $file.FullName -Destination $destination
    }
    $targetSummary = Get-TreeSummary -Root $Target
    if ($sourceSummary.files -ne $targetSummary.files -or
        $sourceSummary.bytes -ne $targetSummary.bytes -or
        $sourceSummary.sha256 -ne $targetSummary.sha256) {
        throw "目录复制校验失败：$Source -> $Target"
    }
    return $targetSummary
}

function Remove-SecretProperties {
    param($Value)
    if ($null -eq $Value -or $Value -is [string] -or $Value.GetType().IsPrimitive) { return }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [pscustomobject]) {
        foreach ($item in $Value) { Remove-SecretProperties -Value $item }
        return
    }
    $properties = @($Value.PSObject.Properties)
    foreach ($property in $properties) {
        if ($property.Name -match '^(?i:api_?key|deepseek_?api_?key)$') {
            $Value.PSObject.Properties.Remove($property.Name)
        } else {
            Remove-SecretProperties -Value $property.Value
        }
    }
}

function Read-JsonObject {
    param([Parameter(Mandatory)][string]$Path)
    Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json
}

function Get-DeepSeekValue {
    param($Config)
    # A plaintext key can sit in either LLM section of the podcast config
    # (translation.api_key or markdown.llm_polish.api_key). Reading only the
    # first under-reported legacyCredentialFieldPresent in the receipt when the
    # secret actually lived in the polish section. Property-by-property
    # traversal keeps this StrictMode-safe when sections are absent.
    if ($null -eq $Config) { return $null }
    foreach ($sectionPath in @('translation', 'markdown.llm_polish')) {
        $node = $Config
        foreach ($part in ($sectionPath -split '\.')) {
            $property = if ($null -ne $node) { $node.PSObject.Properties[$part] } else { $null }
            $node = if ($null -ne $property) { $property.Value } else { $null }
        }
        $keyProperty = if ($null -ne $node) { $node.PSObject.Properties['api_key'] } else { $null }
        $value = if ($null -ne $keyProperty) { [string]$keyProperty.Value } else { $null }
        if (-not [string]::IsNullOrWhiteSpace($value)) { return $value }
    }
    return $null
}

function Initialize-CredentialApi {
    if ('ImmersiveReaderCredential' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class ImmersiveReaderCredential {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct CREDENTIAL {
        public UInt32 Flags;
        public UInt32 Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public UInt32 CredentialBlobSize;
        public IntPtr CredentialBlob;
        public UInt32 Persist;
        public UInt32 AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [DllImport("advapi32.dll", EntryPoint = "CredWriteW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredWrite(ref CREDENTIAL credential, UInt32 flags);
    [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);
    [DllImport("advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredDelete(string target, UInt32 type, UInt32 flags);
    [DllImport("advapi32.dll", EntryPoint = "CredFree")]
    private static extern void CredFree(IntPtr credential);

    public static void Write(string target, string value) {
        byte[] blob = Encoding.UTF8.GetBytes(value);
        IntPtr targetPointer = Marshal.StringToCoTaskMemUni(target);
        IntPtr userPointer = Marshal.StringToCoTaskMemUni("default");
        IntPtr blobPointer = Marshal.AllocCoTaskMem(blob.Length);
        try {
            Marshal.Copy(blob, 0, blobPointer, blob.Length);
            CREDENTIAL credential = new CREDENTIAL {
                Type = 1,
                TargetName = targetPointer,
                CredentialBlobSize = (UInt32)blob.Length,
                CredentialBlob = blobPointer,
                Persist = 2,
                UserName = userPointer
            };
            if (!CredWrite(ref credential, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
        } finally {
            Array.Clear(blob, 0, blob.Length);
            Marshal.FreeCoTaskMem(blobPointer);
            Marshal.FreeCoTaskMem(userPointer);
            Marshal.FreeCoTaskMem(targetPointer);
        }
    }

    public static string Read(string target) {
        IntPtr pointer;
        if (!CredRead(target, 1, 0, out pointer)) {
            int error = Marshal.GetLastWin32Error();
            if (error == 1168) return null;
            throw new Win32Exception(error);
        }
        try {
            CREDENTIAL credential = Marshal.PtrToStructure<CREDENTIAL>(pointer);
            byte[] blob = new byte[credential.CredentialBlobSize];
            if (blob.Length > 0) Marshal.Copy(credential.CredentialBlob, blob, 0, blob.Length);
            try { return Encoding.UTF8.GetString(blob); }
            finally { Array.Clear(blob, 0, blob.Length); }
        } finally { CredFree(pointer); }
    }

    public static void Delete(string target) {
        if (!CredDelete(target, 1, 0)) {
            int error = Marshal.GetLastWin32Error();
            if (error == 1168) return; // ERROR_NOT_FOUND — already gone
            throw new Win32Exception(error);
        }
    }
}
'@
}

function Merge-RecentFiles {
    param([object[]]$Current, [object[]]$Legacy)
    $byPath = [ordered]@{}
    foreach ($entry in @($Current) + @($Legacy)) {
        if ($null -eq $entry -or [string]::IsNullOrWhiteSpace([string]$entry.path)) { continue }
        $key = [System.IO.Path]::GetFullPath([string]$entry.path).ToLowerInvariant()
        if (-not $byPath.Contains($key)) {
            $byPath[$key] = $entry
        } elseif ([string]$entry.openedAt -gt [string]$byPath[$key].openedAt) {
            $byPath[$key] = $entry
        }
    }
    return @($byPath.Values | Sort-Object { [string]$_.openedAt } -Descending)
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

function Test-SqliteIntegrityViaCopy {
    # Runs integrity_check against a throwaway copy (db + -wal/-shm sidecars).
    # Opening the live WAL database directly — even for a read-only PRAGMA —
    # can create or replay -shm/-wal in the source tree, mutating the very
    # migration source a preview run must leave untouched.
    param(
        [Parameter(Mandatory)][string]$Sqlite,
        [Parameter(Mandatory)][string]$Database
    )
    $work = Join-Path ([IO.Path]::GetTempPath()) ("immersive-dbcheck-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work -Force | Out-Null
    try {
        $copy = Join-Path $work 'source.db'
        Copy-Item -LiteralPath $Database -Destination $copy
        foreach ($suffix in @('-wal', '-shm')) {
            $sidecar = "$Database$suffix"
            if (Test-Path -LiteralPath $sidecar -PathType Leaf) {
                Copy-Item -LiteralPath $sidecar -Destination "$copy$suffix"
            }
        }
        return Invoke-SqliteScalar -Sqlite $Sqlite -Database $copy -Sql 'PRAGMA integrity_check;'
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Backup-FileIfPresent {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$BackupRoot,
        [Parameter(Mandatory)][string]$Name
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    $target = Join-Path $BackupRoot $Name
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath $Path -Destination $target
    if ((Get-Sha256 $Path) -ne (Get-Sha256 $target)) { throw "回滚副本校验失败：$Path" }
    return $target
}

function Assert-NoLiveImmersiveProcesses {
    param([switch]$Force)
    # Checkpointing a live zhihu-packer.db or rewriting production settings while the
    # app/sidecars run can tear data; refuse to apply unless -Force is explicit.
    $query = "SELECT ProcessId, Name, CommandLine FROM Win32_Process " +
        "WHERE Name='immersive-reader.exe' OR Name='沉浸阅读.exe' OR Name='node.exe' OR Name='python.exe'"
    $live = @(Get-CimInstance -Query $query -ErrorAction SilentlyContinue | Where-Object {
        $_.Name -ieq 'immersive-reader.exe' -or
        $_.Name -ieq '沉浸阅读.exe' -or
        ($_.Name -ieq 'node.exe' -and [string]$_.CommandLine -match 'zhihu-packer') -or
        ($_.Name -ieq 'python.exe' -and [string]$_.CommandLine -match 'transcribe_podcasts|transcribe_task')
    })
    if ($live.Count -eq 0) { return }
    $list = ($live | ForEach-Object { "$($_.Name) (PID $($_.ProcessId))" }) -join ', '
    if ($Force) {
        Write-Warning "检测到正在运行的相关进程，但被 -Force 放行：$list"
        return
    }
    throw "检测到正在运行的 ImmersiveReader/知乎/播客进程：$list。请先退出应用（含托盘）与相关工具，或确认风险后加 -Force。"
}

if ($Apply -and $Rollback) { throw '-Apply 与 -Rollback 互斥：回滚请单独指定 -Rollback -RunId <id>。' }

if ($Rollback) {
    # Execute a recorded apply journal in reverse order. The journal lives at
    # <LocalAppRoot>\Data\Migrations\<RunId>\rollback\actions.json and uses
    # four action types: deleteFile, restoreFile, deleteDirectory,
    # deleteCredential. Restores are hash-verified; unknown types stop the run.
    Assert-NoLiveImmersiveProcesses -Force:$Force
    Initialize-CredentialApi
    $rollbackDir = Join-Path $LocalAppRoot "Data\Migrations\$RunId\rollback"
    $journalPath = Join-Path $rollbackDir 'actions.json'
    if (-not (Test-Path -LiteralPath $journalPath -PathType Leaf)) {
        throw "回滚日志不存在：$journalPath（确认 -RunId 与 -LocalAppRoot 指向正确的迁移运行）。"
    }
    $journal = Read-JsonObject -Path $journalPath
    $actions = @($journal.actions)
    if ($actions.Count -eq 0) {
        Write-Output '[rollback] journal contains no actions; nothing to undo.'
        return
    }
    $applied = [System.Collections.Generic.List[object]]::new()
    $failed = [System.Collections.Generic.List[object]]::new()
    for ($i = $actions.Count - 1; $i -ge 0; $i--) {
        $action = $actions[$i]
        try {
            switch ([string]$action.type) {
                'deleteFile' {
                    if (Test-Path -LiteralPath ([string]$action.target)) {
                        Remove-Item -LiteralPath ([string]$action.target) -Force
                    }
                }
                'deleteDirectory' {
                    # Reparse-safe delete: a junction removes only the link.
                    Remove-DirectoryTree -Path ([string]$action.target)
                }
                'restoreFile' {
                    $target = [string]$action.target
                    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
                    Copy-Item -LiteralPath ([string]$action.source) -Destination $target -Force
                    if ((Get-Sha256 $action.source) -ne (Get-Sha256 $target)) {
                        throw "回滚恢复校验失败：$target"
                    }
                }
                'deleteCredential' {
                    [ImmersiveReaderCredential]::Delete([string]$action.target)
                }
                default { throw "未知回滚动作类型：$($action.type)" }
            }
            $applied.Add($action)
            Write-Output "[rollback] $($action.type) -> $($action.target)"
        } catch {
            $failed.Add([pscustomobject]@{ action = $action; error = $_.Exception.Message })
            Write-Warning "[rollback] $($action.type) $($action.target) 失败：$($_.Exception.Message)"
        }
    }
    $rollbackReceipt = [ordered]@{
        schemaVersion = 1
        runId = $RunId
        rolledBackAt = (Get-Date).ToUniversalTime().ToString('o')
        applied = $applied.Count
        failed = @($failed)
    }
    Write-JsonAtomic -Path (Join-Path $rollbackDir 'applied.json') -Value $rollbackReceipt
    if ($failed.Count -gt 0) {
        throw "回滚完成但有 $($failed.Count) 个动作失败——详见 rollback\applied.json，需人工收尾。"
    }
    Write-Output "[rollback] applied $($applied.Count) action(s) from $journalPath"
    return
}

foreach ($required in @($LibraryRoot, $LegacyReaderRoot, $ReaderRoot, $LegacyPodcastRoot, $LegacyZhihuRoot)) {
    if (-not (Test-Path -LiteralPath $required -PathType Container)) {
        throw "迁移源不存在：$required"
    }
}

$legacyPodcastConfig = Join-Path $LegacyPodcastRoot 'config.json'
$legacyDatabase = Join-Path $LegacyZhihuRoot 'zhihu-packer.db'
$legacyProfile = Join-Path $LegacyZhihuRoot 'browser-profile'
$zhihuLibrary = Join-Path $LibraryRoot '知乎'
foreach ($requiredFile in @($legacyPodcastConfig, $legacyDatabase)) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) { throw "迁移源文件不存在：$requiredFile" }
}
foreach ($requiredDirectory in @($legacyProfile, $zhihuLibrary)) {
    if (-not (Test-Path -LiteralPath $requiredDirectory -PathType Container)) { throw "迁移源目录不存在：$requiredDirectory" }
}

$sqlite = (Get-Command sqlite3.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source)
if (-not $sqlite) { throw '未找到已安装的 sqlite3.exe；按仓库规则不会自动安装第二份。' }
$npm = Require-Command -Name 'npm.cmd'
$repoRoot = Get-RepoRoot
$migrationRoot = Join-Path $LocalAppRoot "Data\Migrations\$RunId"
$targetPodcast = Join-Path $LocalAppRoot 'Data\Podcast'
$targetDatabase = Join-Path $LocalAppRoot 'Data\Zhihu\zhihu-packer.db'
$targetProfile = Join-Path $LocalAppRoot 'Data\Private\ZhihuProfile'
$targetSettings = Join-Path $ReaderRoot 'settings.json'

# Capture the DeepSeek key into a script variable and scrub it from this
# process's environment immediately: every child spawned below (sqlite3, npm)
# inherits the environment and would otherwise receive the key, and a throw
# before the old end-of-script cleanup left it set.
$providedKey = [string]$env:IMMERSIVE_MIGRATION_DEEPSEEK_KEY
$env:IMMERSIVE_MIGRATION_DEEPSEEK_KEY = $null

$legacyConfigObject = Read-JsonObject -Path $legacyPodcastConfig
$legacyKeyPresent = $null -ne (Get-DeepSeekValue -Config $legacyConfigObject)
$preview = [ordered]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
    apply = $Apply.IsPresent
    sources = [ordered]@{
        reader = $LegacyReaderRoot
        podcast = $LegacyPodcastRoot
        zhihu = $LegacyZhihuRoot
        library = $LibraryRoot
    }
    targets = [ordered]@{
        reader = $ReaderRoot
        podcast = $targetPodcast
        database = $targetDatabase
        privateProfile = $targetProfile
        migration = $migrationRoot
    }
    observations = [ordered]@{
        legacyReadingStateFiles = @(Get-ChildItem -LiteralPath $LegacyReaderRoot -File -Filter '*.json' | Where-Object Name -NotIn @('recent-files.json', 'settings.json')).Count
        libraryReadingStateFiles = @(Get-ChildItem -LiteralPath $LibraryRoot -File -Recurse -Filter '.reading.json').Count
        podcastOutputFiles = @(Get-TreeFiles -Root (Join-Path $LegacyPodcastRoot 'output')).Count
        legacyCredentialFieldPresent = $legacyKeyPresent
        zhihuDatabaseIntegrity = Test-SqliteIntegrityViaCopy -Sqlite $sqlite -Database $legacyDatabase
        zhihuProfile = Get-TreeSummary -Root $legacyProfile -ExcludedDirectories $ExcludedProfileDirectories
        zhihuMarkdown = Get-TreeSummary -Root $zhihuLibrary -Filter '*.md' -ExcludedFileNames @('index.md')
    }
}

if (-not $Apply) {
    $preview | ConvertTo-Json -Depth 10
    return
}
if (Test-Path -LiteralPath $migrationRoot) { throw "迁移 RunId 已存在：$migrationRoot" }
Assert-NoLiveImmersiveProcesses -Force:$Force
if ([string]::IsNullOrWhiteSpace($providedKey)) {
    throw '应用迁移必须通过 IMMERSIVE_MIGRATION_DEEPSEEK_KEY 临时环境变量提供 DeepSeek Key。'
}
if ($providedKey -notmatch '^sk-[A-Za-z0-9_-]{16,}$') { throw 'DeepSeek Key 格式无效。' }

try {

New-Item -ItemType Directory -Path $migrationRoot -Force | Out-Null
$rollbackRoot = Join-Path $migrationRoot 'rollback'
$conflictRoot = Join-Path $migrationRoot 'conflicts'
New-Item -ItemType Directory -Path $rollbackRoot, $conflictRoot -Force | Out-Null
$rollbackActions = [System.Collections.Generic.List[object]]::new()
$script:RollbackJournalPath = Join-Path $rollbackRoot 'actions.json'
function Write-RollbackJournal {
    # The journal is rewritten atomically after every recorded action so a throw
    # mid-apply never leaves rollback payload without its index.
    Write-JsonAtomic -Path $script:RollbackJournalPath -Value ([ordered]@{
        schemaVersion = 1
        generatedAt = (Get-Date).ToUniversalTime().ToString('o')
        note = 'Ordered rollback records consumed by -Rollback -RunId <id> (reverse order). They never restore a plaintext API key.'
        actions = @($rollbackActions)
    })
}
function Add-RollbackAction {
    param([Parameter(Mandatory)]$Action)
    $rollbackActions.Add($Action)
    Write-RollbackJournal
}
Write-RollbackJournal

# Reading state and settings.
$readingCreated = [System.Collections.Generic.List[string]]::new()
$readingExisting = 0
$readingConflicts = [System.Collections.Generic.List[string]]::new()
foreach ($source in Get-ChildItem -LiteralPath $LegacyReaderRoot -File -Filter '*.json' | Where-Object Name -NotIn @('recent-files.json', 'settings.json')) {
    $target = Join-Path $ReaderRoot $source.Name
    if (-not (Test-Path -LiteralPath $target)) {
        Copy-Item -LiteralPath $source.FullName -Destination $target
        $readingCreated.Add($target)
        Add-RollbackAction ([pscustomobject]@{ type = 'deleteFile'; target = $target })
    } elseif ((Get-Sha256 $source.FullName) -eq (Get-Sha256 $target)) {
        $readingExisting += 1
    } else {
        $conflict = Join-Path $conflictRoot "reading-state\$($source.Name).legacy.json"
        New-Item -ItemType Directory -Path (Split-Path -Parent $conflict) -Force | Out-Null
        Copy-Item -LiteralPath $source.FullName -Destination $conflict
        $readingConflicts.Add($source.Name)
    }
}
$recentTarget = Join-Path $ReaderRoot 'recent-files.json'
$recentLegacy = Join-Path $LegacyReaderRoot 'recent-files.json'
$recentBackup = Backup-FileIfPresent -Path $recentTarget -BackupRoot $rollbackRoot -Name 'reader\recent-files.json'
if ($recentBackup) { Add-RollbackAction ([pscustomobject]@{ type = 'restoreFile'; source = $recentBackup; target = $recentTarget }) }
$currentRecent = if (Test-Path $recentTarget) { @(Read-JsonObject $recentTarget) } else { @() }
$legacyRecent = if (Test-Path $recentLegacy) { @(Read-JsonObject $recentLegacy) } else { @() }
Write-JsonAtomic -Path $recentTarget -Value @(Merge-RecentFiles -Current $currentRecent -Legacy $legacyRecent)
$settingsBackup = Backup-FileIfPresent -Path $targetSettings -BackupRoot $rollbackRoot -Name 'reader\settings.json'
if ($settingsBackup) {
    Add-RollbackAction ([pscustomobject]@{ type = 'restoreFile'; source = $settingsBackup; target = $targetSettings })
} else {
    Add-RollbackAction ([pscustomobject]@{ type = 'deleteFile'; target = $targetSettings })
}
# Merge into the existing settings object instead of replacing it — a flat
# rewrite would silently drop any keys a newer app version added since.
$existingSettings = if ($settingsBackup) { Read-JsonObject -Path $targetSettings } else { $null }
$settingsObject = if ($existingSettings -is [pscustomobject]) { $existingSettings } else { [pscustomobject]@{} }
$settingsObject | Add-Member -NotePropertyName schemaVersion -NotePropertyValue 3 -Force
$settingsObject | Add-Member -NotePropertyName libraryRoot -NotePropertyValue ([System.IO.Path]::GetFullPath($LibraryRoot)) -Force
Write-JsonAtomic -Path $targetSettings -Value $settingsObject
$libraryReadingStates = @(Get-ChildItem -LiteralPath $LibraryRoot -File -Recurse -Filter '.reading.json')
foreach ($state in $libraryReadingStates) { $null = Read-JsonObject -Path $state.FullName }

# Credentials are verified in memory before either plaintext config is sanitized.
Initialize-CredentialApi
$legacyKey = Get-DeepSeekValue -Config $legacyConfigObject
$sourceKeyMatchesProvided = if ($null -eq $legacyKey) { $null } else {
    $legacyBytes = [Text.Encoding]::UTF8.GetBytes($legacyKey)
    $providedBytes = [Text.Encoding]::UTF8.GetBytes($providedKey)
    try { (Get-ByteArraySha256 -Bytes $legacyBytes) -ceq (Get-ByteArraySha256 -Bytes $providedBytes) }
    finally { [Array]::Clear($legacyBytes, 0, $legacyBytes.Length); [Array]::Clear($providedBytes, 0, $providedBytes.Length) }
}
$credentialCreated = [System.Collections.Generic.List[string]]::new()
foreach ($target in $CredentialTargets) {
    $existing = [ImmersiveReaderCredential]::Read($target)
    if ($null -ne $existing -and $existing -ne $providedKey) {
        throw "凭据目标已有不同值，未覆盖：$target"
    }
    if ($null -eq $existing) {
        [ImmersiveReaderCredential]::Write($target, $providedKey)
        $credentialCreated.Add($target)
        Add-RollbackAction ([pscustomobject]@{ type = 'deleteCredential'; target = $target })
    }
    if ([ImmersiveReaderCredential]::Read($target) -ne $providedKey) { throw "凭据读回校验失败：$target" }
}
$legacyKey = $null
# The key is only needed for the credential provisioning above — clear the
# in-memory copy here too; the finally block covers every exit path.
$providedKey = $null

# Podcast configuration and legacy outputs.
$sanitizedLegacyBackup = Join-Path $rollbackRoot 'podcast\legacy-config.sanitized.json'
Remove-SecretProperties -Value $legacyConfigObject
Write-JsonAtomic -Path $sanitizedLegacyBackup -Value $legacyConfigObject
Write-JsonAtomic -Path $legacyPodcastConfig -Value $legacyConfigObject
if (Get-DeepSeekValue -Config (Read-JsonObject $legacyPodcastConfig)) { throw '旧 Podcast 配置仍包含明文 Key。' }
$targetConfig = Join-Path $targetPodcast 'config.json'
$podcastTargetCreated = -not (Test-Path -LiteralPath $targetPodcast)
if (-not $podcastTargetCreated) {
    # Re-entrant apply: only tolerate a directory left by an earlier run of this
    # migration (identical sanitized config); foreign content still stops the run.
    if (-not (Test-Path -LiteralPath $targetConfig -PathType Leaf)) {
        throw "Podcast 数据目标已存在且缺少 config.json，无法判定归属：$targetPodcast"
    }
    $existingConfigJson = (Read-JsonObject -Path $targetConfig) | ConvertTo-Json -Depth 30 -Compress
    $expectedConfigJson = $legacyConfigObject | ConvertTo-Json -Depth 30 -Compress
    if ($existingConfigJson -cne $expectedConfigJson) {
        throw "Podcast 数据目标已存在且配置不同：$targetPodcast"
    }
} else {
    New-Item -ItemType Directory -Path $targetPodcast -Force | Out-Null
    Add-RollbackAction ([pscustomobject]@{ type = 'deleteDirectory'; target = $targetPodcast })
}
Write-JsonAtomic -Path $targetConfig -Value $legacyConfigObject
if ((Get-Content -LiteralPath $targetConfig -Raw) -match '(?i)"(?:api_?key|deepseek_?api_?key)"') {
    throw '新 Podcast 配置仍包含 Key 字段。'
}
$legacyOutput = Join-Path $LegacyPodcastRoot 'output'
$targetLegacyOutput = Join-Path $targetPodcast 'LegacyOutput'
$outputSummary = if (Test-Path -LiteralPath $legacyOutput) {
    Copy-TreeVerified -Source $legacyOutput -Target $targetLegacyOutput
} else {
    New-Item -ItemType Directory -Path $targetLegacyOutput -Force | Out-Null
    Get-TreeSummary -Root $targetLegacyOutput
}
$outputIndex = foreach ($file in Get-TreeFiles -Root $targetLegacyOutput) {
    [ordered]@{
        path = (Get-RelativePath -Root $targetLegacyOutput -Path $file.FullName).Replace('\', '/')
        bytes = $file.Length
        sha256 = Get-Sha256 $file.FullName
    }
}
Write-JsonAtomic -Path (Join-Path $targetPodcast 'legacy-output-index.json') -Value ([ordered]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
    files = @($outputIndex)
})

# Zhihu database: preserve WAL set, checkpoint, compact, upgrade, then promote.
foreach ($suffix in @('', '-wal', '-shm')) {
    $source = "$legacyDatabase$suffix"
    if (Test-Path -LiteralPath $source) {
        $null = Backup-FileIfPresent -Path $source -BackupRoot $rollbackRoot -Name "zhihu\source-db\zhihu-packer.db$suffix"
    }
}
$sourceDbCounts = [ordered]@{
    items = [int64](Invoke-SqliteScalar $sqlite $legacyDatabase 'SELECT COUNT(*) FROM items;')
    taskItems = [int64](Invoke-SqliteScalar $sqlite $legacyDatabase 'SELECT COUNT(*) FROM task_items;')
    tasks = [int64](Invoke-SqliteScalar $sqlite $legacyDatabase 'SELECT COUNT(*) FROM tasks;')
}
$checkpoint = Invoke-SqliteScalar $sqlite $legacyDatabase 'PRAGMA wal_checkpoint(TRUNCATE);'
if ((Invoke-SqliteScalar $sqlite $legacyDatabase 'PRAGMA integrity_check;') -ne 'ok') { throw '旧知乎数据库完整性校验失败。' }
if ((Invoke-SqliteScalar $sqlite $legacyDatabase 'PRAGMA foreign_key_check;')) { throw '旧知乎数据库外键校验失败。' }
$markdownBefore = Get-TreeSummary -Root $zhihuLibrary -Filter '*.md' -ExcludedFileNames @('index.md')
$provenanceBefore = @(Get-ChildItem -LiteralPath $zhihuLibrary -File -Recurse -Filter 'provenance.json' | ForEach-Object FullName)
$temporaryDatabase = Join-Path $migrationRoot 'working\zhihu-packer.db'
New-Item -ItemType Directory -Path (Split-Path -Parent $temporaryDatabase) -Force | Out-Null
if (Test-Path -LiteralPath $temporaryDatabase) { Remove-Item -LiteralPath $temporaryDatabase -Force }
$escapedTarget = $temporaryDatabase.Replace("'", "''")
$null = Invoke-SqliteScalar $sqlite $legacyDatabase "VACUUM INTO '$escapedTarget';"
& $npm --prefix (Join-Path $repoRoot 'tools\zhihu-packer') run migrate-legacy -- --database $temporaryDatabase --output $zhihuLibrary
if ($LASTEXITCODE -ne 0) { throw "知乎归档目录迁移失败，退出码 $LASTEXITCODE" }
if ((Invoke-SqliteScalar $sqlite $temporaryDatabase 'PRAGMA integrity_check;') -ne 'ok') { throw '新知乎数据库完整性校验失败。' }
if ((Invoke-SqliteScalar $sqlite $temporaryDatabase 'PRAGMA foreign_key_check;')) { throw '新知乎数据库外键校验失败。' }
$targetDatabaseBackup = Backup-FileIfPresent -Path $targetDatabase -BackupRoot $rollbackRoot -Name 'zhihu\target-db\zhihu-packer.db'
if ($targetDatabaseBackup) {
    Add-RollbackAction ([pscustomobject]@{ type = 'restoreFile'; source = $targetDatabaseBackup; target = $targetDatabase })
} else {
    Add-RollbackAction ([pscustomobject]@{ type = 'deleteFile'; target = $targetDatabase })
}
New-Item -ItemType Directory -Path (Split-Path -Parent $targetDatabase) -Force | Out-Null
Copy-Item -LiteralPath $temporaryDatabase -Destination $targetDatabase -Force
if ((Get-Sha256 $temporaryDatabase) -ne (Get-Sha256 $targetDatabase)) { throw '知乎数据库提升校验失败。' }
$markdownAfter = Get-TreeSummary -Root $zhihuLibrary -Filter '*.md' -ExcludedFileNames @('index.md')
if ($markdownBefore.sha256 -ne $markdownAfter.sha256) { throw '知乎 Markdown 在迁移期间发生变化。' }
$provenanceAfter = @(Get-ChildItem -LiteralPath $zhihuLibrary -File -Recurse -Filter 'provenance.json' | ForEach-Object FullName)
$provenanceCreated = @($provenanceAfter | Where-Object { $provenanceBefore -notcontains $_ })
foreach ($path in $provenanceCreated) { Add-RollbackAction ([pscustomobject]@{ type = 'deleteFile'; target = $path }) }
$targetDbCounts = [ordered]@{
    userVersion = [int](Invoke-SqliteScalar $sqlite $targetDatabase 'PRAGMA user_version;')
    items = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM items;')
    taskItems = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM task_items;')
    tasks = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM tasks;')
    archiveAuthors = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM archive_authors;')
    archiveItems = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM archive_items;')
    archiveRevisions = [int64](Invoke-SqliteScalar $sqlite $targetDatabase 'SELECT COUNT(*) FROM archive_revisions;')
}
if ($targetDbCounts.userVersion -ne 2 -or $targetDbCounts.archiveRevisions -ne $markdownAfter.files) {
    throw '知乎归档数据库与 Markdown 数量不一致。'
}

# Browser profile remains private; disposable Chromium caches are intentionally excluded.
$profileSummary = Copy-TreeVerified -Source $legacyProfile -Target $targetProfile -ExcludedDirectories $ExcludedProfileDirectories
Add-RollbackAction ([pscustomobject]@{ type = 'deleteDirectory'; target = $targetProfile })

# Reconcile every archive path against the catalog.
$archivePathsJson = & $sqlite -batch -json $targetDatabase 'SELECT output_path FROM archive_revisions ORDER BY output_path;'
if ($LASTEXITCODE -ne 0) { throw '读取知乎归档目录失败。' }
$archivePaths = if ([string]::IsNullOrWhiteSpace(($archivePathsJson -join ''))) { @() } else { @(($archivePathsJson -join "`n") | ConvertFrom-Json) }
$catalogPaths = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($row in $archivePaths) { $null = $catalogPaths.Add(([string]$row.output_path).Replace('\', '/')) }
$issues = [System.Collections.Generic.List[object]]::new()
foreach ($file in Get-TreeFiles -Root $zhihuLibrary -Filter '*.md') {
    if ($file.Name -eq 'index.md') { continue }
    $relative = (Get-RelativePath -Root $zhihuLibrary -Path $file.FullName).Replace('\', '/')
    if (-not $catalogPaths.Contains($relative)) {
        $issues.Add([pscustomobject]@{ kind = 'file-only'; path = $relative; suggestion = '保留文件并人工匹配来源' })
    }
}
foreach ($row in $archivePaths) {
    $candidate = Join-Path $zhihuLibrary ([string]$row.output_path)
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        $issues.Add([pscustomobject]@{ kind = 'missing-file'; path = [string]$row.output_path; suggestion = '保留数据库记录并定位或重建文件' })
    }
}
$reconciliation = [ordered]@{
    schemaVersion = 1
    generatedAt = (Get-Date).ToUniversalTime().ToString('o')
    databasePath = $targetDatabase
    outputRoot = $zhihuLibrary
    databaseRows = $targetDbCounts.archiveRevisions
    markdownFiles = $markdownAfter.files
    unresolvedCount = $issues.Count
    issues = @($issues)
}
Write-JsonAtomic -Path (Join-Path $migrationRoot 'reconciliation.json') -Value $reconciliation
$reconciliationMarkdown = @(
    '# V3 数据迁移对账', '',
    "- 生成时间：$($reconciliation.generatedAt)",
    "- 数据库归档记录：$($reconciliation.databaseRows)",
    "- Markdown 文件：$($reconciliation.markdownFiles)",
    "- 未解决项：$($reconciliation.unresolvedCount)", ''
)
if ($issues.Count -eq 0) { $reconciliationMarkdown += '全部知乎归档记录与 Markdown 一一对应。' }
else { foreach ($issue in $issues) { $reconciliationMarkdown += "- [$($issue.kind)] $($issue.path)：$($issue.suggestion)" } }
$reconciliationMarkdown | Set-Content -LiteralPath (Join-Path $migrationRoot 'reconciliation.md') -Encoding utf8

# Legacy trash is inventoried only; unknown original paths are never fabricated.
$trashRoot = Join-Path $LibraryRoot '.trash'
$trashEntries = foreach ($directory in @(Get-ChildItem -LiteralPath $trashRoot -Directory -ErrorAction SilentlyContinue)) {
    $summary = Get-TreeSummary -Root $directory.FullName
    [ordered]@{
        name = $directory.Name
        path = $directory.FullName
        files = $summary.files
        bytes = $summary.bytes
        metadataPresent = Test-Path -LiteralPath (Join-Path $directory.FullName 'trash-entry.json')
        disposition = 'manual-review-preserved'
    }
}
$trashReport = [ordered]@{ schemaVersion = 1; generatedAt = (Get-Date).ToUniversalTime().ToString('o'); entries = @($trashEntries) }
Write-JsonAtomic -Path (Join-Path $migrationRoot 'trash-report.json') -Value $trashReport
@('# Legacy Trash Report', '', "Entries preserved: $(@($trashEntries).Count)", '', 'No original path was invented; every entry remains for manual review.') |
    Set-Content -LiteralPath (Join-Path $migrationRoot 'trash-report.md') -Encoding utf8

$receipt = [ordered]@{
    schemaVersion = 1
    runId = $RunId
    completedAt = (Get-Date).ToUniversalTime().ToString('o')
    status = if ($issues.Count -eq 0) { 'verified' } else { 'verified-with-preserved-conflicts' }
    dataClasses = [ordered]@{
        readingState = [ordered]@{
            source = $LegacyReaderRoot; target = $ReaderRoot
            created = $readingCreated.Count; existing = $readingExisting; conflicts = @($readingConflicts)
            libraryReadingStatesValidated = $libraryReadingStates.Count
            rollback = 'rollback/actions.json'; sensitivity = 'reading history and viewport positions'
        }
        podcast = [ordered]@{
            source = $LegacyPodcastRoot; target = $targetPodcast
            output = $outputSummary; unfinishedTasks = 0; configSecretFields = 0
            rollback = 'rollback/actions.json'; sensitivity = 'local task metadata and generated content'
        }
        credentials = [ordered]@{
            targets = $CredentialTargets; configured = $true; readbackVerified = $true
            legacyFieldWasPresent = $legacyKeyPresent; legacyValueMatchesProvided = $sourceKeyMatchesProvided
            createdTargets = @($credentialCreated); rollback = 'rollback/actions.json'; sensitivity = 'secret value excluded from all reports'
        }
        zhihuDatabase = [ordered]@{
            source = $legacyDatabase; target = $targetDatabase; checkpoint = $checkpoint
            sourceCounts = $sourceDbCounts; targetCounts = $targetDbCounts
            targetSha256 = Get-Sha256 $targetDatabase; integrity = 'ok'; foreignKeys = 'ok'
            rollback = 'rollback/zhihu'; sensitivity = 'private archive metadata; no row content included'
        }
        zhihuProfile = [ordered]@{
            source = $legacyProfile; target = $targetProfile; copied = $profileSummary
            excludedCacheDirectories = $ExcludedProfileDirectories
            rollback = 'rollback/actions.json'; sensitivity = 'private authenticated browser profile'
        }
        libraryArchive = [ordered]@{
            source = $zhihuLibrary; target = $zhihuLibrary; inPlace = $true
            markdown = $markdownAfter; provenanceCreated = $provenanceCreated.Count
            reconciliation = 'reconciliation.json'; unresolved = $issues.Count
            rollback = 'rollback/actions.json'; sensitivity = 'user reading library'
        }
        legacyTrash = [ordered]@{
            source = $trashRoot; target = $trashRoot; inPlace = $true; entries = @($trashEntries).Count
            report = 'trash-report.json'; rollback = 'not modified'; sensitivity = 'deleted reading content preserved'
        }
    }
}
Write-RollbackJournal
Write-JsonAtomic -Path (Join-Path $migrationRoot 'receipt.json') -Value $receipt
$receipt | ConvertTo-Json -Depth 20
} finally {
    # Guaranteed cleanup on every exit path (success, throw, Ctrl+C): the
    # plaintext key copy and the inheritable env var never survive the apply.
    $providedKey = $null
    $env:IMMERSIVE_MIGRATION_DEEPSEEK_KEY = $null
}
