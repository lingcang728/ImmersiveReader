param(
    [string]$RunId = "zhihu-$(Get-Date -Format 'yyyyMMdd-HHmmss')",
    [string]$PeopleId = "xiao-xue-shi-46-24",
    [ValidateSet('time', 'vote')]
    [string]$SortBy = 'time'
)

$ErrorActionPreference = 'Stop'

if ($RunId -notmatch '^[A-Za-z0-9_-]+$') {
    throw "RunId must contain only letters, digits, '-' or '_'."
}
if ($PeopleId -notmatch '^[A-Za-z0-9_-]{1,80}$') {
    throw "PeopleId must contain only letters, digits, '-' or '_'."
}

$localRoot = Join-Path $env:LOCALAPPDATA (Join-Path 'ImmersiveReader-QA' $RunId)
$documentsRoot = Join-Path $env:USERPROFILE 'Documents'
$libraryRoot = Join-Path $documentsRoot (Join-Path 'Codex\ImmersiveReader-QA' (Join-Path $RunId 'Library'))
$dataRoot = Join-Path $localRoot 'Data'
$cacheRoot = Join-Path $localRoot 'Cache'
$logsRoot = Join-Path $localRoot 'Logs'
$profileRoot = Join-Path $dataRoot 'Private\ZhihuProfile'
$browserCache = Join-Path $cacheRoot 'Zhihu\BrowserCache'
$zhihuLibrary = Join-Path $libraryRoot '知乎'
$dbPath = Join-Path $dataRoot 'Zhihu\zhihu-packer.db'

foreach ($directory in @($localRoot, $dataRoot, $cacheRoot, $logsRoot, $profileRoot, $browserCache, $zhihuLibrary)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

$receipt = [ordered]@{
    schemaVersion = 1
    runId = $RunId
    peopleId = $PeopleId
    itemTypes = 'all'
    sortBy = $SortBy
    topN = 5
    localRoot = $localRoot
    libraryRoot = $libraryRoot
    zhihuLibrary = $zhihuLibrary
    dataRoot = $dataRoot
    cacheRoot = $cacheRoot
    profileRoot = $profileRoot
    browserCache = $browserCache
    database = $dbPath
    externalNetworkRun = $false
}

$receiptPath = Join-Path $localRoot 'qa-receipt.json'
$receipt | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $receiptPath -Encoding UTF8
$receipt | ConvertTo-Json -Depth 4
