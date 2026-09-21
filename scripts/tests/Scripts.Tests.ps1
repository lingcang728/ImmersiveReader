# Pester 3.4-compatible tests for the PowerShell script surface.
# Covers the findings the script layer previously had no automated proof for:
# junction/reparse safety, runtime manifest validation, the migration rollback
# journal, and the runtime-bundle pack/verify round-trip.
# Run: Invoke-Pester scripts\tests   (verify.ps1 wires this in automatically)

# $PSScriptRoot = <repo>\scripts\tests → repo root is two levels up.
$scriptsDir = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $scriptsDir
. (Join-Path $scriptsDir 'common.ps1')

function Invoke-UnderTest {
    # Positive-path runner: forces Stop so a child script's non-terminating
    # error record can't slip past the assertions that follow.
    param([Parameter(Mandatory)][scriptblock]$Body)
    $ErrorActionPreference = 'Stop'
    & $Body
}

function Test-Throws {
    # Pester 3.4's `Should Throw` does not observe exceptions when invoked
    # under pwsh — assert on an explicit try/catch result instead.
    param([Parameter(Mandatory)][scriptblock]$Body)
    try {
        & $Body | Out-Null
        return $false
    } catch {
        return $true
    }
}

function New-SyntheticRuntime {
    # A minimal schema-v2 runtime tree: caller-supplied files + a manifest.json
    # whose entries record each file's relative path, size and SHA-256.
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][hashtable]$Files
    )
    New-Item -ItemType Directory -Path $Root -Force | Out-Null
    $entries = @()
    foreach ($relative in $Files.Keys) {
        $path = Join-Path $Root ($relative -replace '/', '\')
        New-Item -ItemType Directory -Path (Split-Path -Parent $path) -Force | Out-Null
        [IO.File]::WriteAllBytes($path, $Files[$relative])
        $entries += [ordered]@{
            path   = $relative
            bytes  = (Get-Item -LiteralPath $path).Length
            sha256 = (Get-FileSha256Hex -Path $path).ToLowerInvariant()
        }
    }
    [ordered]@{
        schemaVersion = 2
        entryCount    = $entries.Count
        entries       = $entries
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $Root 'manifest.json') -Encoding UTF8
}

Describe 'common.ps1 reparse-point safety' {
    It 'flags a directory junction as a reparse point' {
        $target = Join-Path $TestDrive 'real-target'
        New-Item -ItemType Directory -Path $target | Out-Null
        $junction = Join-Path $TestDrive 'junction'
        cmd /c mklink /J "$junction" "$target" | Out-Null
        Test-ReparsePoint -Path $junction | Should Be $true
        Test-ReparsePoint -Path $target | Should Be $false
    }

    It 'Remove-DirectoryTree removes the junction link without touching the target' {
        $target = Join-Path $TestDrive 'linked-target'
        New-Item -ItemType Directory -Path $target | Out-Null
        $sentinel = Join-Path $target 'keep.txt'
        Set-Content -LiteralPath $sentinel -Value 'must survive'
        $junction = Join-Path $TestDrive 'link'
        cmd /c mklink /J "$junction" "$target" | Out-Null
        Remove-DirectoryTree -Path $junction
        Test-Path -LiteralPath $junction | Should Be $false
        Test-Path -LiteralPath $sentinel -PathType Leaf | Should Be $true
    }

    It 'Remove-DirectoryTree still deletes a real directory recursively' {
        $dir = Join-Path $TestDrive 'real-dir'
        New-Item -ItemType Directory -Path (Join-Path $dir 'nested') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $dir 'nested\file.txt') -Value 'x'
        Remove-DirectoryTree -Path $dir
        Test-Path -LiteralPath $dir | Should Be $false
    }

    It 'Assert-NotReparsePoint throws on a junction' {
        $target = Join-Path $TestDrive 'assert-target'
        New-Item -ItemType Directory -Path $target | Out-Null
        $junction = Join-Path $TestDrive 'assert-link'
        cmd /c mklink /J "$junction" "$target" | Out-Null
        Test-Throws { Assert-NotReparsePoint -Path $junction } | Should Be $true
    }
}

Describe 'verify-runtime.ps1 manifest validation' {
    $verifier = Join-Path $scriptsDir 'verify-runtime.ps1'

    It 'passes a consistent synthetic runtime' {
        $rt = Join-Path $TestDrive 'ok-runtime'
        New-SyntheticRuntime -Root $rt -Files @{ 'bin/tool.exe' = [byte[]](1, 2, 3) }
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $false
    }

    It 'rejects a file whose bytes changed after manifest generation' {
        $rt = Join-Path $TestDrive 'tampered-runtime'
        New-SyntheticRuntime -Root $rt -Files @{ 'bin/tool.exe' = [byte[]](1, 2, 3) }
        [IO.File]::WriteAllBytes((Join-Path $rt 'bin\tool.exe'), [byte[]](9, 9, 9, 9))
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $true
    }

    It 'rejects a manifest entry whose file is missing' {
        $rt = Join-Path $TestDrive 'missing-runtime'
        New-SyntheticRuntime -Root $rt -Files @{
            'bin/tool.exe'  = [byte[]](1, 2, 3)
            'bin/other.dll' = [byte[]](4, 5)
        }
        Remove-Item -LiteralPath (Join-Path $rt 'bin\other.dll') -Force
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $true
    }

    It 'rejects a manifest with an unsafe (escaping) path' {
        $rt = Join-Path $TestDrive 'escape-runtime'
        New-SyntheticRuntime -Root $rt -Files @{ 'a.txt' = [byte[]](1) }
        $manifestPath = Join-Path $rt 'manifest.json'
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $manifest.entries[0].path = '../outside.txt'
        $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $true
    }

    It 'rejects a stale schemaVersion-1 manifest' {
        $rt = Join-Path $TestDrive 'stale-runtime'
        New-SyntheticRuntime -Root $rt -Files @{ 'a.txt' = [byte[]](1) }
        $manifestPath = Join-Path $rt 'manifest.json'
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $manifest.schemaVersion = 1
        $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $true
    }

    It 'rejects an entryCount that disagrees with the entries array' {
        $rt = Join-Path $TestDrive 'count-runtime'
        New-SyntheticRuntime -Root $rt -Files @{ 'a.txt' = [byte[]](1) }
        $manifestPath = Join-Path $rt 'manifest.json'
        $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        $manifest.entryCount = 99
        $manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
        Test-Throws { & $verifier -RuntimeRoot $rt } | Should Be $true
    }
}

Describe 'migrate-v3-production-data.ps1 -Rollback journal execution' {
    $migrator = Join-Path $scriptsDir 'migrate-v3-production-data.ps1'

    function Write-RollbackJournal {
        param([string]$LocalAppRoot, [string]$RunId, [object[]]$Actions)
        $dir = Join-Path $LocalAppRoot "Data\Migrations\$RunId\rollback"
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        @{ schemaVersion = 1; actions = $Actions } |
            ConvertTo-Json -Depth 6 |
            Set-Content -LiteralPath (Join-Path $dir 'actions.json') -Encoding UTF8
    }

    It 'undoes deleteFile / deleteDirectory / restoreFile in one pass' {
        $local = Join-Path $TestDrive 'local-app'
        $junkFile = Join-Path $TestDrive 'migrated-junk.txt'
        Set-Content -LiteralPath $junkFile -Value 'apply wrote this'
        $junkDir = Join-Path $TestDrive 'migrated-dir'
        New-Item -ItemType Directory -Path $junkDir -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $junkDir 'x.txt') -Value 'x'
        $backup = Join-Path $TestDrive 'backup.bin'
        $restored = Join-Path $TestDrive 'restored.bin'
        Set-Content -LiteralPath $backup -Value 'original-bytes'
        Write-RollbackJournal -LocalAppRoot $local -RunId 'pester-run' -Actions @(
            @{ type = 'restoreFile'; source = $backup; target = $restored },
            @{ type = 'deleteFile'; target = $junkFile },
            @{ type = 'deleteDirectory'; target = $junkDir }
        )
        Invoke-UnderTest { & $migrator -Rollback -RunId 'pester-run' -LocalAppRoot $local -Force | Out-Null }
        Test-Path -LiteralPath $junkFile | Should Be $false
        Test-Path -LiteralPath $junkDir | Should Be $false
        (Get-Content -LiteralPath $restored -Raw).Trim() | Should Be 'original-bytes'
        $applied = Join-Path $local 'Data\Migrations\pester-run\rollback\applied.json'
        Test-Path -LiteralPath $applied -PathType Leaf | Should Be $true
        $receipt = Get-Content -LiteralPath $applied -Raw | ConvertFrom-Json
        [int]$receipt.applied | Should Be 3
    }

    It 'fails loudly on an unknown action type' {
        $local = Join-Path $TestDrive 'local-app2'
        Write-RollbackJournal -LocalAppRoot $local -RunId 'pester-bad' -Actions @(
            @{ type = 'explode'; target = 'nowhere' }
        )
        Test-Throws { & $migrator -Rollback -RunId 'pester-bad' -LocalAppRoot $local -Force } | Should Be $true
    }

    It 'refuses -Apply and -Rollback together' {
        Test-Throws { & $migrator -Apply -Rollback -LocalAppRoot $TestDrive -Force } | Should Be $true
    }
}

Describe 'runtime bundle pack/verify round-trip' {
    # End-to-end: synthetic runtime -> pack-runtime-bundle -> runtime-parts.json
    # -> verify-runtime-bundle. Uses a synthetic version so the generated
    # manifest lands in docs\release\9.9.9-pester\ and is removed afterwards.
    $packer = Join-Path $scriptsDir 'pack-runtime-bundle.ps1'
    $bundleVerifier = Join-Path $scriptsDir 'verify-runtime-bundle.ps1'
    $testVersion = '9.9.9-pester'

    It 'round-trips parts, bundle hash, manifest and source parity' {
        $rt = Join-Path $TestDrive 'runtime'
        $out = Join-Path $TestDrive 'bundle-out'
        # The bundle verifier hash-compares five app-code pairs against the
        # checkout — copy the real build outputs so parity holds.
        $pairs = @(
            @{ Source = 'tools\zhihu-packer\dist\reader-template.html'; Runtime = 'zhihu/app/dist/reader-template.html' },
            @{ Source = 'tools\zhihu-packer\dist\server.js'; Runtime = 'zhihu/app/dist/server.js' },
            @{ Source = 'packages\contracts\dist\index.js'; Runtime = 'packages/contracts/dist/index.js' },
            @{ Source = 'tools\podcast-transcriber\scripts\polish_interview_markdown.py'; Runtime = 'podcast/app/scripts/polish_interview_markdown.py' },
            @{ Source = 'tools\podcast-transcriber\scripts\podcast_transcriber\language.py'; Runtime = 'podcast/app/scripts/podcast_transcriber/language.py' }
        )
        $files = @{}
        foreach ($pair in $pairs) {
            $files[$pair.Runtime] = [IO.File]::ReadAllBytes((Join-Path $repoRoot $pair.Source))
        }
        New-SyntheticRuntime -Root $rt -Files $files
        $releaseMeta = Join-Path $repoRoot "docs\release\$testVersion"
        try {
            Invoke-UnderTest { & $packer -RuntimeRoot $rt -OutputDirectory $out -Version $testVersion | Out-Null }
            Test-Path -LiteralPath (Join-Path $out 'runtime-bundle.zip.001') -PathType Leaf | Should Be $true
            Test-Path -LiteralPath (Join-Path $releaseMeta 'runtime-parts.json') -PathType Leaf | Should Be $true
            Test-Throws { & $bundleVerifier -PartsDirectory $out -Version $testVersion } | Should Be $false
        } finally {
            Remove-DirectoryTree -Path $out
            if (Test-Path -LiteralPath $releaseMeta) { Remove-DirectoryTree -Path $releaseMeta }
        }
    }
}

