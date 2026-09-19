#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('redactio-check-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
function Expect-Failure([string]$Code, [string]$Corpus = '') {
    $start = [Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
    foreach ($argument in @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                           (Join-Path $PSScriptRoot 'check-package.ps1'), '-PackageRoot', $temporary)) {
        $start.ArgumentList.Add($argument)
    }
    if ($Corpus) { $start.ArgumentList.Add('-CorpusRoot'); $start.ArgumentList.Add($Corpus) }
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.UseShellExecute = $false
    $child = [Diagnostics.Process]::Start($start)
    $output = $child.StandardOutput.ReadToEndAsync()
    $errors = $child.StandardError.ReadToEndAsync()
    try {
        if (-not $child.WaitForExit(10000)) { $child.Kill($true); throw 'check_test_timeout' }
        if ($child.ExitCode -eq 0 -or -not $errors.Result.Contains($Code)) { throw "Expected check failure: $Code" }
    } finally { $child.Dispose() }
}
try {
    Expect-Failure 'Package resource missing: redactio.exe'
    # Inert negative-test files; these never serve as executable package evidence.
    $files = @('redactio.exe', 'sidecar/redactio-sidecar.exe', 'models/manifest.json',
               'models/de_core_news_lg/config.cfg', 'models/de_core_news_lg/meta.json',
               'webview2/msedgewebview2.exe', 'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')
    foreach ($relative in $files) {
        $path = Join-Path $temporary $relative
        New-Item -ItemType Directory -Force (Split-Path $path -Parent) | Out-Null
        [IO.File]::WriteAllText($path, 'negative-test-input')
    }
    foreach ($relative in @('webview2/msedgewebview2.exe', 'models/de_core_news_lg/config.cfg')) {
        $resource = Join-Path $temporary $relative
        Remove-Item -LiteralPath $resource
        Expect-Failure "Package resource missing: $relative"
        [IO.File]::WriteAllText($resource, 'negative-test-input')
    }
    $corpus = Join-Path $temporary 'corpus'
    New-Item -ItemType Directory -Path $corpus | Out-Null
    [IO.File]::WriteAllText((Join-Path $corpus 'case-0001.docx'), 'synthetic')
    $hash = (Get-FileHash -LiteralPath (Join-Path $corpus 'case-0001.docx')).Hash.ToLowerInvariant()
    $entry = @{ path = 'case-0001.docx'; sha256 = $hash; profile = 'canary'; warnings = @() }
    function Set-Corpus($Entries, [int]$Count = 1) {
        @{ schema_version = 2; documents = $Count; files = @($Entries) } | ConvertTo-Json -Depth 8 |
            Set-Content -LiteralPath (Join-Path $corpus 'expectations.json')
    }
    Set-Corpus @($entry) 2
    Expect-Failure 'invalid_corpus' $corpus
    $entry.path = '../outside.docx'
    Set-Corpus @($entry)
    Expect-Failure 'invalid_corpus' $corpus
    $entry.path = 'case-0001.docx'
    Set-Corpus @($entry, $entry) 2
    Expect-Failure 'invalid_corpus' $corpus
    $entry.sha256 = '0' * 64
    Set-Corpus @($entry)
    Expect-Failure 'corpus_checksum_mismatch' $corpus
    $entry.sha256 = $hash
    Set-Corpus @($entry)
    [IO.File]::WriteAllText((Join-Path $corpus 'unlisted.docx'), 'synthetic')
    Expect-Failure 'unlisted_corpus_document' $corpus
    Remove-Item -LiteralPath (Join-Path $corpus 'unlisted.docx')
    $entries = @($files | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $temporary $_)).Hash.ToLowerInvariant() }
    })
    @{ schema_version = 1; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'tampered')
    Expect-Failure 'package_checksum_mismatch' $corpus
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'negative-test-input')
    [IO.File]::WriteAllText((Join-Path $temporary 'extra.dll'), 'unlisted')
    Expect-Failure 'unlisted_package_resource' $corpus
    $entries[0].path = '../outside'
    @{ schema_version = 1; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    Expect-Failure 'invalid_build_manifest' $corpus
    Write-Output 'package_check_negative_tests_ok checks=11'
} finally { Remove-Item -LiteralPath $temporary -Recurse -Force }
