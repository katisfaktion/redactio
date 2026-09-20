#requires -Version 7.0
param([Parameter(Mandatory)][string]$PackageRoot,
      [string]$CorpusRoot = (Join-Path $PSScriptRoot 'smoke'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
foreach ($relative in @('redactio.exe', 'sidecar/redactio-sidecar.exe',
                       'models/manifest.json', 'models/biomedbert-de/config.json',
                       'models/biomedbert-de/model.safetensors', 'models/biomedbert-de/redactio-model.json',
                       'webview2/msedgewebview2.exe',
                       'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $PackageRoot $relative) -PathType Leaf)) {
        throw "Package resource missing: $relative"
    }
}
# Validate the supplied synthetic corpus before launching any executable.
$corpusManifest = Join-Path $CorpusRoot 'expectations.json'
if (-not (Test-Path -LiteralPath $corpusManifest -PathType Leaf) -or
    (Get-Item -LiteralPath $corpusManifest).Length -gt 4MB) { throw 'invalid_corpus' }
if ((Get-Item -LiteralPath $CorpusRoot).Attributes -band [IO.FileAttributes]::ReparsePoint -or
    @(Get-ChildItem -LiteralPath $CorpusRoot -Recurse -Force |
        Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) {
    throw 'redirected_corpus_resource'
}
$CorpusRoot = (Resolve-Path -LiteralPath $CorpusRoot).Path
try {
    $expectations = Get-Content -Raw -LiteralPath $corpusManifest | ConvertFrom-Json
    if ($expectations.schema_version -ne 2 -or $expectations.documents -lt 1 -or
        $expectations.documents -gt 10000 -or $expectations.files.Count -ne $expectations.documents) {
        throw 'invalid_corpus'
    }
    $corpusPaths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $expectations.files) {
        if ($entry.path -cnotmatch '^[a-z0-9-]+\.docx$' -or -not $corpusPaths.Add($entry.path) -or
            $entry.sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            $entry.profile -cnotin @('canary', 'warnings', 'corrupt', 'empty', 'repeated', 'unicode', 'tamper')) {
            throw 'invalid_corpus'
        }
        $expectedWarnings = switch ($entry.profile) {
            'warnings' { 'headers_footers' }; 'empty' { 'empty_document' }; default { }
        }
        if (($entry.warnings -join ',') -cne ($expectedWarnings -join ',')) { throw 'invalid_corpus' }
    }
} catch { throw 'invalid_corpus' }
foreach ($entry in $expectations.files) {
    $source = Join-Path $CorpusRoot $entry.path
    if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or
        (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.sha256) {
        throw 'corpus_checksum_mismatch'
    }
}
foreach ($source in Get-ChildItem -LiteralPath $CorpusRoot -Filter '*.docx' -Recurse -File) {
    if (-not $corpusPaths.Contains([IO.Path]::GetRelativePath($CorpusRoot, $source.FullName))) {
        throw 'unlisted_corpus_document'
    }
}
$PackageRoot = (Resolve-Path -LiteralPath $PackageRoot).Path
$manifest = Get-Content -Raw -LiteralPath (Join-Path $PackageRoot 'build-manifest.json') | ConvertFrom-Json
if ($manifest.schema_version -ne 1) { throw 'invalid_build_manifest' }
$seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $manifest.files) {
    if ($entry.path -match '(^/|\\|:|(^|/)\.\.?(/|$))' -or
        -not $seen.Add($entry.path) -or $entry.sha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw 'invalid_build_manifest'
    }
    $file = Join-Path $PackageRoot $entry.path
    if (-not (Test-Path -LiteralPath $file -PathType Leaf) -or
        (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.sha256) {
        throw 'package_checksum_mismatch'
    }
}
$allFiles = @(Get-ChildItem -LiteralPath $PackageRoot -File -Recurse -Force)
foreach ($file in $allFiles) {
    $relative = [IO.Path]::GetRelativePath($PackageRoot, $file.FullName).Replace('\', '/')
    if ($relative -ne 'build-manifest.json' -and -not $seen.Contains($relative)) {
        throw 'unlisted_package_resource'
    }
}
if (@(Get-ChildItem -LiteralPath $PackageRoot -Recurse -Force |
      Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) {
    throw 'redirected_package_resource'
}
$modelManifest = Get-Content -Raw -LiteralPath (Join-Path $PackageRoot 'models/manifest.json') | ConvertFrom-Json
if ($modelManifest.models.Count -ne 1 -or $modelManifest.models[0].name -cne $manifest.inputs.model.name -or
    $modelManifest.models[0].version -cne $manifest.inputs.model.revision -or
    $modelManifest.models[0].path -cne 'biomedbert-de') { throw 'invalid_model_manifest' }
foreach ($file in $manifest.inputs.model.files.PSObject.Properties) {
    if ((Get-FileHash -LiteralPath (Join-Path $PackageRoot ('models/biomedbert-de/' + $file.Name))).Hash.ToLowerInvariant() -cne $file.Value) {
        throw 'model_checksum_mismatch'
    }
}
$modelConfig = Get-Content -Raw -LiteralPath (Join-Path $PackageRoot 'models/biomedbert-de/config.json') | ConvertFrom-Json
$modelEntities = @($modelConfig.id2label.PSObject.Properties.Value | Where-Object { $_ -cne 'O' } |
    ForEach-Object { $_ -creplace '^[BI]-', '' } | Sort-Object -Unique)
$runtime = Get-Item -LiteralPath (Join-Path $PackageRoot 'webview2/msedgewebview2.exe')
if ($runtime.VersionInfo.ProductVersion -cne $manifest.inputs.webview2.version) {
    throw 'webview_version_mismatch'
}
$snapshot = Join-Path $PackageRoot 'sidecar/_internal/tldextract/.tld_set_snapshot'
if (-not (Test-Path -LiteralPath $snapshot -PathType Leaf) -or
    (Get-FileHash -LiteralPath $snapshot).Hash.ToLowerInvariant() -cne $manifest.inputs.suffix_snapshot.sha256) {
    throw 'offline_suffix_snapshot_missing'
}
# Read bytes under a deadline and a size limit; ReadLineAsync alone is unbounded.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
public static class RedactioSmokeFrame {
    public static async Task<string> Read(Stream stream, int seconds) {
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(seconds));
        using var output = new MemoryStream();
        byte[] buffer = new byte[1];
        while (output.Length <= 1024 * 1024) {
            int count = await stream.ReadAsync(buffer, 0, 1, deadline.Token);
            if (count == 0) throw new IOException("sidecar_eof");
            if (buffer[0] == 10) return new UTF8Encoding(false, true).GetString(output.ToArray());
            output.WriteByte(buffer[0]);
        }
        throw new IOException("sidecar_message_too_large");
    }
}
'@
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('redactio-smoke-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
$process = [Diagnostics.Process]::new()
$started = $false
$process.StartInfo.FileName = Join-Path $PackageRoot 'sidecar/redactio-sidecar.exe'
$process.StartInfo.ArgumentList.Add('--model-dir')
$process.StartInfo.ArgumentList.Add((Join-Path $PackageRoot 'models'))
$process.StartInfo.WorkingDirectory = $temporary
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
$process.StartInfo.RedirectStandardInput = $true
$process.StartInfo.RedirectStandardOutput = $true
$process.StartInfo.RedirectStandardError = $true
$process.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
foreach ($variable in @('PYTHONPATH', 'PYTHONHOME', 'REDACTIO_MODEL_DIR', 'REDACTIO_SIDECAR_EXECUTABLE')) {
    [void]$process.StartInfo.Environment.Remove($variable)
}
$process.StartInfo.Environment['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
$process.StartInfo.Environment['PYTHONUTF8'] = '1'
$process.StartInfo.Environment['HF_HUB_OFFLINE'] = '1'
$process.StartInfo.Environment['TRANSFORMERS_OFFLINE'] = '1'
function Send-Request([string]$Type, [hashtable]$Payload, [int]$Seconds, [string]$ExpectedError = '') {
    $id = [guid]::NewGuid().ToString()
    $message = @{ id = $id; type = $Type; payload = $Payload } | ConvertTo-Json -Depth 20 -Compress
    $process.StandardInput.WriteLine($message)
    $process.StandardInput.Flush()
    try {
        $line = [RedactioSmokeFrame]::Read($process.StandardOutput.BaseStream, $Seconds).GetAwaiter().GetResult()
        $reply = $line | ConvertFrom-Json -Depth 30
    } catch { throw 'sidecar_invalid_or_timed_out_reply' }
    if ($reply.id -cne $id) { throw 'sidecar_protocol_error' }
    if ($ExpectedError) {
        if ($reply.type -cne 'error' -or $reply.payload.code -cne $ExpectedError) { throw 'sidecar_expected_error' }
        return $null
    }
    if ($reply.type -cne ($Type + '_result')) { throw 'sidecar_protocol_error' }
    return $reply.payload
}
try {
    if (-not $process.Start()) { throw 'sidecar_start_failed' }
    $started = $true
    $stderrDrain = $process.StandardError.BaseStream.CopyToAsync([IO.Stream]::Null)
    $ping = Send-Request 'ping' @{} 180
    if ($ping.protocol_version -ne 1) { throw 'sidecar_protocol_version' }
    $pair = [guid]::NewGuid().ToString()
    $revision = [guid]::NewGuid().ToString()
    $entities = @('EMAIL_ADDRESS', 'PHONE_NUMBER', 'IBAN_CODE', 'IP_ADDRESS', 'URL', 'DATE_TIME')
    $config = @{ model = $modelManifest.models[0].name; model_entities = $modelEntities;
        enabled_entities = $entities; include_positions = $true }
    $baseRule = @{ id = [guid]::NewGuid().ToString(); entity_type = 'CUSTOM';
        enabled = $true; kind = 'words'; words = @('anna.beispiel@example.invalid') }
    function Configure-Engine {
        $configured = Send-Request 'configure' @{
            sync_pair_id = $pair; processing_revision = $revision; config = $config
        } 180
        if ($configured.sync_pair_id -cne $pair -or $configured.processing_revision -cne $revision -or
            $configured.engine.model_name -cne $modelManifest.models[0].name -or
            $configured.engine.model_version -cne $manifest.inputs.model.revision) { throw 'sidecar_model_identity' }
    }
    $config.custom_rules = @($baseRule)
    Configure-Engine
    $previousProfile = ''
    $processed = 0
    $redactionCount = 0
    foreach ($entry in $expectations.files) {
        if ($entry.profile -eq 'repeated' -or $previousProfile -eq 'repeated') {
            $revision = [guid]::NewGuid().ToString()
            $config.custom_rules = @($baseRule)
            if ($entry.profile -eq 'repeated') {
                # Deterministic placeholder check; baseline NER recall is recorded separately.
                $config.custom_rules += @{ id = [guid]::NewGuid().ToString(); entity_type = 'PERSON';
                    enabled = $true; kind = 'words'; words = @('Max Mustermann') }
            }
            Configure-Engine
        }
        $previousProfile = $entry.profile
        $source = Join-Path $CorpusRoot $entry.path
        $sourceHash = $entry.sha256
        $docId = 'doc-' + ($processed + 1).ToString('D4')
        $expectedError = if ($entry.profile -eq 'corrupt') { 'invalid_docx' } else { '' }
        $result = Send-Request 'process_document' @{
            sync_pair_id = $pair; processing_revision = $revision; doc_id = $docId
            source_path = $source; source_hash_sha256 = $sourceHash
            redacted_at = '2026-09-19T12:00:00Z'
        } 120 $expectedError
        if ((Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() -cne $sourceHash) { throw 'source_modified' }
        $processed++
        if ($expectedError) { continue }
        if ($result.sync_pair_id -cne $pair -or $result.processing_revision -cne $revision -or
            $result.source_hash_sha256 -cne $sourceHash -or $result.doc_id -cne $docId -or
            $result.body_was_empty -ne ($entry.profile -eq 'empty') -or
            ($result.warnings -join ',') -cne ($entry.warnings -join ',')) { throw 'sidecar_process_identity' }
        if ($entry.profile -ne 'empty') {
            foreach ($entity in ($entities + 'CUSTOM')) {
                if ($entity -cnotin $result.detections.entity_type) { throw 'smoke_missing_detection' }
            }
            foreach ($canary in @('anna.beispiel@example.invalid', 'Max Mustermann', 'Berlin',
                                 'max@example.com', '+49 30 12345678', 'DE89370400440532013000',
                                 '192.168.1.1', 'https://example.de/path', '19.09.2026')) {
                if ($result.body.Contains($canary) -or $result.markdown.Contains($canary)) {
                    throw "smoke_unredacted_canary profile=$($entry.profile)"
                }
            }
        }
        foreach ($redaction in $result.redactions) {
            if (-not $result.body.Contains($redaction.placeholder)) { throw 'smoke_missing_placeholder' }
        }
        if ($entry.profile -eq 'repeated') {
            # The canary and both repeated names must reuse one PERSON placeholder.
            $names = @($result.redactions | Where-Object entity_type -CEQ 'PERSON')
            if ($names.Count -ne 3 -or @($names.placeholder | Select-Object -Unique).Count -ne 1) {
                throw 'repeated_placeholder_mismatch'
            }
        }
        $redactionCount += $result.redactions.Count
    }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(5000) -or $process.ExitCode -ne 0) { throw 'sidecar_shutdown_failed' }
    Write-Output "package_smoke_ok documents=$processed redactions=$redactionCount"
} finally {
    if ($started -and -not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $process.Dispose()
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
