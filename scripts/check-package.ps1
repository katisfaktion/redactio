#requires -Version 7.0
param([Parameter(Mandatory)][string]$PackageRoot,
      [string]$CorpusRoot = (Join-Path $PSScriptRoot 'smoke'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Test-ModelStore([string]$Root, $BuildManifest) {
    $store = Join-Path $Root 'models'
    $registryPath = Join-Path $store 'manifest.json'
    if (-not (Test-Path -LiteralPath $registryPath -PathType Leaf) -or
        (Get-Item -LiteralPath $registryPath).Length -gt 1MB -or
        $BuildManifest.inputs.PSObject.Properties.Name -cnotcontains 'models' -or
        $BuildManifest.inputs.PSObject.Properties.Name -ccontains 'model') {
        throw 'invalid_model_manifest'
    }
    try { $registry = Get-Content -Raw -LiteralPath $registryPath | ConvertFrom-Json -Depth 30 }
    catch { throw 'invalid_model_manifest' }
    if ($registry.schema_version -ne 2 -or
        $registry.PSObject.Properties.Name -cnotcontains 'models' -or
        $registry.PSObject.Properties.Name -cnotcontains 'legacy_unavailable') {
        throw 'invalid_model_manifest'
    }
    $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $directories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($record in @($registry.models)) {
        $descriptor = $record.descriptor
        if (-not $descriptor -or -not $names.Add($descriptor.name) -or
            $descriptor.version -cnotmatch '^[0-9a-f]{40}$' -or
            $record.state -cnotin @('ready', 'available', 'removing')) {
            throw 'invalid_model_manifest'
        }
        if ($record.state -eq 'available') {
            if ($null -ne $record.path) { throw 'invalid_model_manifest' }
            continue
        }
        if ($record.path -cnotmatch '^[a-z0-9][a-z0-9-]{0,127}$' -or
            $record.path -cmatch '^(con|prn|aux|nul|com[1-9]|lpt[1-9])$' -or
            -not $directories.Add($record.path)) { throw 'invalid_model_manifest' }
        $directory = Join-Path $store $record.path
        if (-not (Test-Path -LiteralPath $directory -PathType Container)) { throw 'preloaded_model_missing' }
        $receiptPath = Join-Path $directory 'redactio-model.json'
        try { $receipt = Get-Content -Raw -LiteralPath $receiptPath | ConvertFrom-Json }
        catch { throw 'invalid_model_receipt' }
        if (($receipt.PSObject.Properties.Name | Sort-Object) -join ',' -cne 'name,repository,version' -or
            $receipt.name -cne $descriptor.name -or $receipt.version -cne $descriptor.version -or
            $receipt.repository -cne $descriptor.repository) { throw 'invalid_model_receipt' }
        $files = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        [void]$files.Add('redactio-model.json')
        foreach ($artifact in @($descriptor.files)) {
            $base = [IO.Path]::GetFileNameWithoutExtension($artifact.filename)
            if ($artifact.filename -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$' -or
                $artifact.filename.EndsWith('.') -or
                $base -match '^(con|prn|aux|nul|com[1-9]|lpt[1-9])$' -or
                -not $files.Add($artifact.filename) -or
                $artifact.size -lt 0 -or $artifact.sha256 -cnotmatch '^[0-9a-f]{64}$') {
                throw 'invalid_model_manifest'
            }
            $file = Join-Path $directory $artifact.filename
            if (-not (Test-Path -LiteralPath $file -PathType Leaf) -or
                (Get-Item -LiteralPath $file).Length -ne $artifact.size) { throw 'preloaded_model_missing' }
            if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                $artifact.sha256) { throw 'model_checksum_mismatch' }
        }
        foreach ($file in Get-ChildItem -LiteralPath $directory -File -Force) {
            if (-not $files.Contains($file.Name)) { throw 'invalid_model_receipt' }
        }
    }
    foreach ($preload in @($BuildManifest.inputs.models)) {
        $found = @($registry.models | Where-Object {
            $_.descriptor.name -ceq $preload.descriptor.name -and
            $_.descriptor.version -ceq $preload.descriptor.version -and
            $_.descriptor.repository -ceq $preload.descriptor.repository
        })
        if ($found.Count -ne 1) { throw 'preloaded_model_missing' }
        $actual = $found[0].descriptor
        foreach ($property in @('name', 'version', 'repository', 'title', 'license', 'model_type',
                                'architecture', 'window_tokens', 'stride_tokens', 'special_tokens')) {
            if ($actual.$property -cne $preload.descriptor.$property) { throw 'invalid_model_manifest' }
        }
        if (($actual.entity_types -join ',') -cne ($preload.descriptor.entity_types -join ',') -or
            @($actual.files).Count -ne @($preload.descriptor.files).Count) {
            throw 'invalid_model_manifest'
        }
        foreach ($expected in @($preload.descriptor.files)) {
            $artifact = @($actual.files | Where-Object filename -CEQ $expected.filename)
            if ($artifact.Count -ne 1 -or $artifact[0].size -ne $expected.size -or
                $artifact[0].sha256 -cne $expected.sha256 -or
                $artifact[0].upstream_hash.algorithm -cne $expected.upstream_hash.algorithm -or
                $artifact[0].upstream_hash.value -cne $expected.upstream_hash.value) {
                throw 'invalid_model_manifest'
            }
        }
    }
    foreach ($item in Get-ChildItem -LiteralPath $store -Force) {
        if ($item.Name -cne 'manifest.json' -and -not $directories.Contains($item.Name)) {
            throw 'invalid_model_receipt'
        }
    }
    return $registry
}
foreach ($relative in @('redactio.exe', 'sidecar/redactio-sidecar.exe',
                       'models/manifest.json',
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
    if ($relative -ne 'build-manifest.json' -and
        -not $relative.StartsWith('models/', [StringComparison]::OrdinalIgnoreCase) -and
        -not $seen.Contains($relative)) {
        throw 'unlisted_package_resource'
    }
}
if (@(Get-ChildItem -LiteralPath $PackageRoot -Recurse -Force |
      Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) {
    throw 'redirected_package_resource'
}
$modelManifest = Test-ModelStore $PackageRoot $manifest
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
function New-SidecarProcess([bool]$ManageModels) {
    $child = [Diagnostics.Process]::new()
    $child.StartInfo.FileName = Join-Path $PackageRoot 'sidecar/redactio-sidecar.exe'
    if ($ManageModels) { $child.StartInfo.ArgumentList.Add('--manage-models') }
    $child.StartInfo.ArgumentList.Add('--model-dir')
    $child.StartInfo.ArgumentList.Add((Join-Path $PackageRoot 'models'))
    $child.StartInfo.WorkingDirectory = $temporary
    $child.StartInfo.UseShellExecute = $false
    $child.StartInfo.CreateNoWindow = $true
    $child.StartInfo.RedirectStandardInput = $true
    $child.StartInfo.RedirectStandardOutput = $true
    $child.StartInfo.RedirectStandardError = $true
    $child.StartInfo.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($variable in @('PYTHONPATH', 'PYTHONHOME', 'REDACTIO_MODEL_DIR', 'REDACTIO_SIDECAR_EXECUTABLE')) {
        [void]$child.StartInfo.Environment.Remove($variable)
    }
    $child.StartInfo.Environment['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
    $child.StartInfo.Environment['PYTHONUTF8'] = '1'
    $child.StartInfo.Environment['HF_HUB_OFFLINE'] = '1'
    $child.StartInfo.Environment['TRANSFORMERS_OFFLINE'] = '1'
    return $child
}
$catalog = @{}
foreach ($key in @('biomedbert', 'hugginglil')) {
    $manager = New-SidecarProcess $true
    try {
        if (-not $manager.Start()) { throw 'sidecar_start_failed' }
        $stderrDrain = $manager.StandardError.BaseStream.CopyToAsync([IO.Stream]::Null)
        $id = [guid]::NewGuid().ToString()
        $manager.StandardInput.WriteLine((@{ id = $id; type = 'check_model';
            payload = @{ kind = 'catalog'; key = $key } } | ConvertTo-Json -Compress))
        $manager.StandardInput.Close()
        $reply = [RedactioSmokeFrame]::Read($manager.StandardOutput.BaseStream, 30).GetAwaiter().GetResult() |
            ConvertFrom-Json -Depth 30
        if ($reply.id -cne $id -or $reply.type -cne 'result' -or
            -not $manager.WaitForExit(5000) -or $manager.ExitCode -ne 0) {
            throw 'sidecar_catalog_check_failed'
        }
        $catalog[$key] = $reply.payload
    } finally {
        if (-not $manager.HasExited) { $manager.Kill($true); $manager.WaitForExit() }
        $manager.Dispose()
    }
}
foreach ($preload in @($manifest.inputs.models)) {
    if (-not $catalog.ContainsKey($preload.key) -or
        $catalog[$preload.key].name -cne $preload.descriptor.name -or
        $catalog[$preload.key].version -cne $preload.descriptor.version -or
        $catalog[$preload.key].repository -cne $preload.descriptor.repository) {
        throw 'invalid_model_catalog'
    }
}
$process = New-SidecarProcess $false
$started = $false
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
    $readyModels = @($modelManifest.models | Where-Object state -CEQ 'ready')
    $processed = 0
    $redactionCount = 0
    if (-not $readyModels) {
        Send-Request 'configure' @{ sync_pair_id = [guid]::NewGuid().ToString()
            processing_revision = [guid]::NewGuid().ToString()
            config = @{ model = $catalog.biomedbert.name; model_entities = @(); enabled_entities = @()
                custom_rules = @(); include_positions = $true } } 30 'model_not_found'
    }
    foreach ($model in $readyModels) {
        $pair = [guid]::NewGuid().ToString()
        $revision = [guid]::NewGuid().ToString()
        $config = @{ model = $model.descriptor.name; model_entities = @(); enabled_entities = @()
            include_positions = $true; custom_rules = @(@{ id = [guid]::NewGuid().ToString()
                entity_type = 'CUSTOM'; enabled = $true; kind = 'words'
                words = @('anna.beispiel@example.invalid') }) }
        $configured = Send-Request 'configure' @{
            sync_pair_id = $pair; processing_revision = $revision; config = $config
        } 180
        if ($configured.engine.model_name -cne $model.descriptor.name -or
            $configured.engine.model_version -cne $model.descriptor.version) { throw 'sidecar_model_identity' }
        $replayed = $false
        foreach ($entry in $expectations.files) {
            $source = Join-Path $CorpusRoot $entry.path
            $sourceHash = $entry.sha256
            $docId = 'doc-' + ($processed + 1).ToString('D4')
            $expectedError = if ($entry.profile -eq 'corrupt') { 'invalid_docx' } else { '' }
            $request = @{ sync_pair_id = $pair; processing_revision = $revision; doc_id = $docId
                source_path = $source; source_hash_sha256 = $sourceHash
                redacted_at = '2026-09-19T12:00:00Z' }
            $result = Send-Request 'process_document' $request 120 $expectedError
            if ((Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() -cne $sourceHash) {
                throw 'source_modified'
            }
            $processed++
            if ($expectedError) { continue }
            if ($result.sync_pair_id -cne $pair -or $result.processing_revision -cne $revision -or
                $result.source_hash_sha256 -cne $sourceHash -or $result.doc_id -cne $docId -or
                $result.body_was_empty -ne ($entry.profile -eq 'empty') -or
                ($result.warnings -join ',') -cne ($entry.warnings -join ',')) {
                throw 'sidecar_process_identity'
            }
            if ($entry.profile -ne 'empty' -and
                ('CUSTOM' -cnotin $result.detections.entity_type -or
                 $result.body.Contains('anna.beispiel@example.invalid') -or
                 $result.markdown.Contains('anna.beispiel@example.invalid'))) {
                throw 'smoke_custom_rule_failed'
            }
            foreach ($redaction in $result.redactions) {
                if (-not $result.body.Contains($redaction.placeholder)) { throw 'smoke_missing_placeholder' }
            }
            if (-not $replayed) {
                $review = Send-Request 'render_review' ($request + @{
                    detections = @($result.detections)
                    decisions = @{ dismissed_ids = @(); manual = @() }; review_status = 'pending'
                    reviewed_at = $null; acknowledged_warnings = @()
                }) 120
                if ($review.body -cne $result.body -or $review.markdown -cne $result.markdown) {
                    throw 'smoke_review_replay_mismatch'
                }
                $replayed = $true
            }
            $redactionCount += $result.redactions.Count
        }
    }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(5000) -or $process.ExitCode -ne 0) { throw 'sidecar_shutdown_failed' }
    Write-Output "package_smoke_ok models=$($readyModels.Count) documents=$processed redactions=$redactionCount"
} finally {
    if ($started -and -not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $process.Dispose()
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
