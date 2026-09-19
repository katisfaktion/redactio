#requires -Version 7.0
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Load only the source check: this test must never build or download a package.
$tokens = $null
$errors = $null
$script = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'package-windows.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'package_script_parse_failed' }
$function = $script.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-SourceIdentity'
}, $true)
if (-not $function) { throw 'Missing source identity validation' }
. ([scriptblock]::Create($function.Extent.Text))
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('redactio-manifest-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
function Invoke-TestGit([string[]]$Arguments) {
    & git -c core.autocrlf=false -C $temporary @Arguments | Out-Null
    if ($LASTEXITCODE) { throw 'test_git_failed' }
}
function Expect-Failure([string]$Code, [string]$Path = $temporary) {
    try { Get-SourceIdentity $Path | Out-Null }
    catch { if ($_.Exception.Message.Contains($Code)) { return }; throw }
    throw "Expected failure: $Code"
}
try {
    Expect-Failure 'clean_git_checkout_required'
    Invoke-TestGit @('init', '--quiet')
    [IO.File]::WriteAllText((Join-Path $temporary '.gitignore'), "dist/`n")
    [IO.File]::WriteAllText((Join-Path $temporary 'source.txt'), 'committed source')
    Invoke-TestGit @('add', '.')
    Invoke-TestGit @('-c', 'user.name=Manifest Test', '-c', 'user.email=manifest@example.invalid',
          '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'Synthetic source')
    $head = & git -C $temporary rev-parse HEAD
    $identity = Get-SourceIdentity $temporary
    if ($identity.commit -cne $head -or $identity.verification -cne 'clean_git_checkout') {
        throw 'incorrect_source_identity'
    }
    New-Item -ItemType Directory -Path (Join-Path $temporary 'dist') | Out-Null
    [IO.File]::WriteAllText((Join-Path $temporary 'dist/build.txt'), 'ignored build output')
    Get-SourceIdentity $temporary | Out-Null
    Expect-Failure 'clean_git_checkout_required' (Join-Path $temporary 'dist')
    [IO.File]::WriteAllText((Join-Path $temporary 'source.txt'), 'changed source')
    Expect-Failure 'dirty_source_checkout'
    Invoke-TestGit @('add', 'source.txt')
    Expect-Failure 'dirty_source_checkout'
    Invoke-TestGit @('reset', '--hard', '--quiet', 'HEAD')
    [IO.File]::WriteAllText((Join-Path $temporary 'untracked.txt'), 'untracked source')
    Expect-Failure 'dirty_source_checkout'
    # Execute the real serialization pipeline against inert bytes, never a fake build.
    $writer = $script.Find({ param($node)
        $node -is [Management.Automation.Language.PipelineAst] -and
        $node.Extent.Text.StartsWith('@{ schema_version = 1; inputs = $inputs;')
    }, $true)
    if (-not $writer) { throw 'Missing build manifest writer' }
    $package = $temporary
    [IO.File]::WriteAllText((Join-Path $package 'redactio.exe'), 'abc')
    $inputs = @{}; $locks = @(); $files = @(); $source = $identity
    $tools = @{ python = 'Python 3.13.13' }; $distributions = @()
    $references = @(@{ path = 'synthetic-procedure'; kind = 'procedure_only' })
    foreach ($desktopSupplied in @($true, $false)) {
        & ([scriptblock]::Create($writer.Extent.Text))
        $manifest = Get-Content -Raw -LiteralPath (Join-Path $package 'build-manifest.json') | ConvertFrom-Json
        $mode, $provenance = if ($desktopSupplied) { 'supplied', 'caller_supplied_unverified' } else { 'built_here', 'local_build' }
        if ($manifest.desktop.mode -cne $mode -or $manifest.desktop.provenance -cne $provenance -or
            $manifest.desktop.sha256 -cne 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad' -or
            $manifest.source.commit -cne $head -or $manifest.acceptance -cne 'unverified' -or
            $manifest.test_evaluation_references[0].kind -cne 'procedure_only' -or
            $manifest.observed_tools.python -cne 'Python 3.13.13') { throw 'incorrect_manifest_provenance' }
    }
    Write-Output 'build_manifest_tests_ok checks=9'
} finally { Remove-Item -LiteralPath $temporary -Recurse -Force }
