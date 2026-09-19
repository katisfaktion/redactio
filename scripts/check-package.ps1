#requires -Version 7.0
param([Parameter(Mandatory)][string]$PackageRoot,
      [string]$CorpusRoot = (Join-Path $PSScriptRoot 'smoke'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
foreach ($relative in @('redactio.exe', 'sidecar/redactio-sidecar.exe',
                       'models/manifest.json', 'webview2/msedgewebview2.exe',
                       'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $PackageRoot $relative) -PathType Leaf)) {
        throw "Package resource missing: $relative"
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
if ($modelManifest.models.Count -ne 1 -or $modelManifest.models[0].name -cne 'de_core_news_lg' -or
    $modelManifest.models[0].version -cne $manifest.inputs.model.version) { throw 'invalid_model_manifest' }
$runtime = Get-Item -LiteralPath (Join-Path $PackageRoot 'webview2/msedgewebview2.exe')
if ($runtime.VersionInfo.ProductVersion -cne $manifest.inputs.webview2.version) {
    throw 'webview_version_mismatch'
}
$snapshot = Join-Path $PackageRoot 'sidecar/_internal/tldextract/.tld_set_snapshot'
if (-not (Test-Path -LiteralPath $snapshot -PathType Leaf) -or
    (Get-FileHash -LiteralPath $snapshot).Hash.ToLowerInvariant() -cne $manifest.inputs.suffix_snapshot.sha256) {
    throw 'offline_suffix_snapshot_missing'
}
$source = (Resolve-Path -LiteralPath (Join-Path $CorpusRoot 'case-0001.docx')).Path
$expectations = Get-Content -Raw -LiteralPath (Join-Path $CorpusRoot 'expectations.json') | ConvertFrom-Json
$sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
if ($expectations.documents -ne 1 -or $sourceHash -cne $expectations.source_hash_sha256) {
    throw 'invalid_smoke_corpus'
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
function Send-Request([string]$Type, [hashtable]$Payload, [int]$Seconds) {
    $id = [guid]::NewGuid().ToString()
    $message = @{ id = $id; type = $Type; payload = $Payload } | ConvertTo-Json -Depth 20 -Compress
    $process.StandardInput.WriteLine($message)
    $process.StandardInput.Flush()
    try {
        $line = [RedactioSmokeFrame]::Read($process.StandardOutput.BaseStream, $Seconds).GetAwaiter().GetResult()
        $reply = $line | ConvertFrom-Json -Depth 30
    } catch { throw 'sidecar_invalid_or_timed_out_reply' }
    if ($reply.id -cne $id -or $reply.type -cne ($Type + '_result')) { throw 'sidecar_protocol_error' }
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
    $entities = @('PERSON', 'LOCATION', 'EMAIL_ADDRESS', 'PHONE_NUMBER', 'IBAN_CODE', 'IP_ADDRESS', 'URL', 'DATE_TIME')
    $configured = Send-Request 'configure' @{
        sync_pair_id = $pair; processing_revision = $revision
        config = @{ model = 'de_core_news_lg'; enabled_entities = $entities; include_positions = $true
            custom_rules = @(@{ id = [guid]::NewGuid().ToString(); entity_type = 'CUSTOM';
                enabled = $true; kind = 'words'; words = @('anna.beispiel@example.invalid') }) }
    } 180
    if ($configured.sync_pair_id -cne $pair -or $configured.processing_revision -cne $revision -or
        $configured.engine.model_name -cne 'de_core_news_lg' -or
        $configured.engine.model_version -cne $manifest.inputs.model.version) { throw 'sidecar_model_identity' }
    $result = Send-Request 'process_document' @{
        sync_pair_id = $pair; processing_revision = $revision; doc_id = 'doc-0001'
        source_path = $source; source_hash_sha256 = $sourceHash
        redacted_at = '2026-09-19T12:00:00Z'
    } 120
    if ($result.sync_pair_id -cne $pair -or $result.processing_revision -cne $revision -or
        $result.source_hash_sha256 -cne $sourceHash -or $result.doc_id -cne 'doc-0001' -or
        $result.body_was_empty -or $result.warnings.Count -ne 0) { throw 'sidecar_process_identity' }
    foreach ($entity in ($entities + 'CUSTOM')) {
        if ($entity -cnotin $result.detections.entity_type) { throw 'smoke_missing_detection' }
    }
    foreach ($canary in @('anna.beispiel@example.invalid', 'Max Mustermann', 'Berlin',
                         'max@example.com', '+49 30 12345678', 'DE89370400440532013000',
                         '192.168.1.1', 'https://example.de/path', '19.09.2026')) {
        if ($result.body.Contains($canary) -or $result.markdown.Contains($canary)) {
            throw 'smoke_unredacted_canary'
        }
    }
    foreach ($entry in $result.redactions) {
        if (-not $result.body.Contains($entry.placeholder)) { throw 'smoke_missing_placeholder' }
    }
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(5000) -or $process.ExitCode -ne 0) { throw 'sidecar_shutdown_failed' }
    Write-Output "package_smoke_ok documents=1 redactions=$($result.redactions.Count)"
} finally {
    if ($started -and -not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
    $process.Dispose()
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
