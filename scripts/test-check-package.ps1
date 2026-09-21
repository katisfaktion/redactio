#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('redactio-check-test-' + [guid]::NewGuid())
$corpus = $temporary + '-corpus'
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
        if ($child.ExitCode -eq 0 -or -not $errors.Result.Contains($Code)) {
            throw "Expected check failure: $Code; stderr=$($errors.Result)"
        }
    } finally { $child.Dispose() }
}
function Expect-ModelFailure([string]$Code, [scriptblock]$Action) {
    try { & $Action }
    catch { if ($_.Exception.Message.Contains($Code)) { return }; throw }
    throw "Expected model-store failure: $Code"
}
function New-TestDescriptor([string]$Name, [string]$Revision, [string]$Content) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Content)
    $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
    return [pscustomobject]@{
        name = $Name; version = $Revision; repository = 'Example/Model'; title = $Name
        license = 'Apache-2.0'; model_type = 'bert'; architecture = 'BertForTokenClassification'
        entity_types = @('PERSON'); window_tokens = 32; stride_tokens = 8; special_tokens = 2
        files = @([pscustomobject]@{ filename = 'model.safetensors'; size = $bytes.Length
            upstream_hash = [pscustomobject]@{ algorithm = 'sha256'; value = $digest }; sha256 = $digest })
    }
}
function New-ReadyModel([string]$Store, $Descriptor, [string]$Directory, [string]$Content) {
    $path = Join-Path $Store $Directory
    New-Item -ItemType Directory -Path $path | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $path 'model.safetensors'),
        [Text.Encoding]::UTF8.GetBytes($Content))
    @{ name = $Descriptor.name; version = $Descriptor.version; repository = $Descriptor.repository } |
        ConvertTo-Json | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $path 'redactio-model.json')
    return [pscustomobject]@{ descriptor = $Descriptor; path = $Directory; state = 'ready' }
}
function Set-TestRegistry([string]$Store, $Models) {
    @{ schema_version = 2; models = @($Models); legacy_unavailable = @() } |
        ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $Store 'manifest.json')
}
try {
    $tokens = $null
    $errors = $null
    $script = [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot 'check-package.ps1'), [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw 'check_package_script_parse_failed' }
    $modelCheck = $script.Find({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Test-ModelStore'
    }, $true)
    if (-not $modelCheck) { throw 'Missing model-store validation' }
    . ([scriptblock]::Create($modelCheck.Extent.Text))

    $modelPackage = Join-Path $temporary 'model-package'
    $modelStore = Join-Path $modelPackage 'models'
    New-Item -ItemType Directory -Path $modelStore -Force | Out-Null
    $emptyBuild = [pscustomobject]@{ inputs = [pscustomobject]@{ models = @() } }
    Set-TestRegistry $modelStore @()
    Test-ModelStore $modelPackage $emptyBuild | Out-Null
    [IO.File]::WriteAllText((Join-Path $modelStore '.redactio-models-lock'), '')
    Test-ModelStore $modelPackage $emptyBuild | Out-Null
    Remove-Item -LiteralPath (Join-Path $modelStore '.redactio-models-lock')
    New-Item -ItemType Directory -Path (Join-Path $modelStore '.redactio-models-lock') | Out-Null
    Expect-ModelFailure 'invalid_model_receipt' { Test-ModelStore $modelPackage $emptyBuild }
    Remove-Item -LiteralPath (Join-Path $modelStore '.redactio-models-lock')
    $lockTarget = Join-Path $modelPackage 'lock-target'
    [IO.File]::WriteAllText($lockTarget, '')
    New-Item -ItemType HardLink -Path (Join-Path $modelStore '.redactio-models-lock') -Target $lockTarget | Out-Null
    Expect-ModelFailure 'invalid_model_receipt' { Test-ModelStore $modelPackage $emptyBuild }
    Remove-Item -LiteralPath (Join-Path $modelStore '.redactio-models-lock'), $lockTarget
    [IO.File]::WriteAllText((Join-Path $modelStore '.redactio-models-lock'), '')

    $revision1 = '1' * 40
    $revision2 = '2' * 40
    $descriptor1 = New-TestDescriptor "hf:Example/Model@$revision1" $revision1 'first model'
    $descriptor2 = New-TestDescriptor "hf:Example/Model@$revision2" $revision2 'second model'
    $preloaded = [pscustomobject]@{ inputs = [pscustomobject]@{ models = @(
        [pscustomobject]@{ key = 'biomedbert'; descriptor = $descriptor1 },
        [pscustomobject]@{ key = 'hugginglil'; descriptor = $descriptor2 }
    ) } }
    Expect-ModelFailure 'preloaded_model_missing' { Test-ModelStore $modelPackage $preloaded }
    $record1 = New-ReadyModel $modelStore $descriptor1 'model-one' 'first model'
    $record2 = New-ReadyModel $modelStore $descriptor2 'model-two' 'second model'
    Set-TestRegistry $modelStore @($record1, $record2)
    Test-ModelStore $modelPackage $preloaded | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $modelStore 'model-two/extra') | Out-Null
    [IO.File]::WriteAllText((Join-Path $modelStore 'model-two/extra/payload.dll'), 'unexpected')
    Expect-ModelFailure 'invalid_model_receipt' { Test-ModelStore $modelPackage $preloaded }
    Remove-Item -LiteralPath (Join-Path $modelStore 'model-two/extra') -Recurse -Force
    [IO.File]::WriteAllText((Join-Path $modelStore 'model-two/model.safetensors'), 'second modeX',
        [Text.UTF8Encoding]::new($false))
    Expect-ModelFailure 'model_checksum_mismatch' { Test-ModelStore $modelPackage $preloaded }
    [IO.File]::WriteAllBytes((Join-Path $modelStore 'model-two/model.safetensors'),
        [Text.Encoding]::UTF8.GetBytes('second model'))
    $laterInstall = [pscustomobject]@{ inputs = [pscustomobject]@{ models = @() } }
    Test-ModelStore $modelPackage $laterInstall | Out-Null
    $record1.path = $null
    $record1.state = 'available'
    Remove-Item -LiteralPath (Join-Path $modelStore 'model-one') -Recurse -Force
    Set-TestRegistry $modelStore @($record1, $record2)
    Test-ModelStore $modelPackage $preloaded | Out-Null
    Remove-Item -LiteralPath $modelPackage -Recurse -Force

    Expect-Failure 'Package resource missing: redactio.exe'
    # Inert negative-test files; these never serve as executable package evidence.
    $files = @('redactio.exe', 'sidecar/redactio-sidecar.exe', 'models/manifest.json',
               'webview2/msedgewebview2.exe', 'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')
    foreach ($relative in $files) {
        $path = Join-Path $temporary $relative
        New-Item -ItemType Directory -Force (Split-Path $path -Parent) | Out-Null
        [IO.File]::WriteAllText($path, 'negative-test-input')
    }
    Set-TestRegistry (Join-Path $temporary 'models') @()
    foreach ($relative in @('webview2/msedgewebview2.exe', 'models/manifest.json')) {
        $resource = Join-Path $temporary $relative
        Remove-Item -LiteralPath $resource
        Expect-Failure "Package resource missing: $relative"
        if ($relative -eq 'models/manifest.json') { Set-TestRegistry (Join-Path $temporary 'models') @() }
        else { [IO.File]::WriteAllText($resource, 'negative-test-input') }
    }
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
    $entries = @($files | Where-Object { -not $_.StartsWith('models/') } | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $temporary $_)).Hash.ToLowerInvariant() }
    })
    $runtimeInputs = @{ models = @(); webview2 = @{ version = 'fixture' }
        suffix_snapshot = @{ sha256 = '0' * 64 } }
    @{ schema_version = 1; inputs = $runtimeInputs; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'tampered')
    Expect-Failure 'package_checksum_mismatch' $corpus
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'negative-test-input')
    [IO.File]::WriteAllText((Join-Path $temporary 'extra.dll'), 'unlisted')
    Expect-Failure 'unlisted_package_resource' $corpus
    Remove-Item -LiteralPath (Join-Path $temporary 'extra.dll')
    $laterRecord = New-ReadyModel (Join-Path $temporary 'models') $descriptor1 'model-later' 'first model'
    Set-TestRegistry (Join-Path $temporary 'models') @($laterRecord)
    Expect-Failure 'webview_version_mismatch' $corpus
    $entries[0].path = '../outside'
    @{ schema_version = 1; inputs = $runtimeInputs; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    Expect-Failure 'invalid_build_manifest' $corpus
    Write-Output 'package_check_negative_tests_ok checks=21'
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force
    if (Test-Path -LiteralPath $corpus) { Remove-Item -LiteralPath $corpus -Recurse -Force }
}
