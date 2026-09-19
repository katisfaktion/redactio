#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('redactio-check-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
function Expect-Failure([string]$Code) {
    $start = [Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
    foreach ($argument in @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
                           (Join-Path $PSScriptRoot 'check-package.ps1'), '-PackageRoot', $temporary)) {
        $start.ArgumentList.Add($argument)
    }
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
               'webview2/msedgewebview2.exe', 'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')
    foreach ($relative in $files) {
        $path = Join-Path $temporary $relative
        New-Item -ItemType Directory -Force (Split-Path $path -Parent) | Out-Null
        [IO.File]::WriteAllText($path, 'negative-test-input')
    }
    $entries = @($files | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $temporary $_)).Hash.ToLowerInvariant() }
    })
    @{ schema_version = 1; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'tampered')
    Expect-Failure 'package_checksum_mismatch'
    [IO.File]::WriteAllText((Join-Path $temporary 'redactio.exe'), 'negative-test-input')
    [IO.File]::WriteAllText((Join-Path $temporary 'extra.dll'), 'unlisted')
    Expect-Failure 'unlisted_package_resource'
    $entries[0].path = '../outside'
    @{ schema_version = 1; files = $entries } | ConvertTo-Json -Depth 5 |
        Set-Content -LiteralPath (Join-Path $temporary 'build-manifest.json')
    Expect-Failure 'invalid_build_manifest'
    Write-Output 'package_check_negative_tests_ok checks=4'
} finally { Remove-Item -LiteralPath $temporary -Recurse -Force }
