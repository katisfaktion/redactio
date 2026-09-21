#requires -Version 7.0
param([string]$DesktopExecutable, [string]$DesktopNotices,
      [ValidateSet('biomedbert', 'hugginglil')][string[]]$PreloadModels = @())
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or -not [Environment]::Is64BitOperatingSystem) { throw 'Windows x64 build host required' }
$root = Split-Path $PSScriptRoot -Parent
$inputs = Get-Content -Raw -LiteralPath (Join-Path $root 'packaging/build-inputs.json') | ConvertFrom-Json
$catalog = Get-Content -Raw -LiteralPath (Join-Path $root $inputs.model_catalog) | ConvertFrom-Json
if ($catalog.schema_version -ne 1 -or
    @($PreloadModels | Select-Object -Unique).Count -ne $PreloadModels.Count) {
    throw 'invalid_model_catalog'
}
$selected = foreach ($model in $PreloadModels) {
    $entry = @($catalog.models | Where-Object { $_.key -ceq $model })
    if ($entry.Count -ne 1) { throw 'invalid_model_catalog' }
    $entry[0]
}
$inputs.models = @($selected)
$work = Join-Path $root 'dist/windows'
$package = Join-Path $work 'redactio'
$desktopSupplied = [bool]$DesktopExecutable
if ($root.StartsWith('\\')) { throw 'Build from a local Windows checkout; UNC paths are unsupported' }
if (Test-Path -LiteralPath $package) { throw 'Package output already exists; use a fresh build directory' }
function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Build command failed: $Program" }
}
function Get-SourceIdentity([string]$Path) {
    $checkout = & git -C $Path rev-parse --show-toplevel 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $checkout -or
        [IO.Path]::GetFullPath($checkout) -ne [IO.Path]::GetFullPath($Path)) {
        throw 'clean_git_checkout_required'
    }
    $commit = & git -C $Path rev-parse --verify HEAD
    if ($LASTEXITCODE -ne 0 -or $commit -cnotmatch '^[0-9a-f]{40}$') { throw 'invalid_source_commit' }
    $status = & git -C $Path status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'dirty_source_checkout' }
    return @{ commit = $commit; verification = 'clean_git_checkout' }
}
function Get-VerifiedArtifact($Artifact) {
    $file = Join-Path $work ('inputs/' + ([uri]$Artifact.url).Segments[-1])
    if (-not (Test-Path -LiteralPath $file)) { Invoke-WebRequest -Uri $Artifact.url -OutFile $file }
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Artifact.sha256) {
        throw 'Build input checksum mismatch'
    }
    return $file
}
Push-Location $root
try {
    $source = Get-SourceIdentity $root
    New-Item -ItemType Directory -Force -Path (Join-Path $work 'inputs') | Out-Null
    $tools = @{ powershell = $PSVersionTable.PSVersion.ToString(); git = (& git --version); uv = (& uv --version) }
    if ($tools.uv -notlike "uv $($inputs.tools.uv) *") { throw 'Pinned uv version required' }
    $env:UV_PROJECT_ENVIRONMENT = Join-Path $work 'venv'
    $env:UV_PYTHON_INSTALL_DIR = Join-Path $work 'python'
    $env:PYINSTALLER_CONFIG_DIR = Join-Path $work 'pyinstaller-cache'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
    Invoke-Checked uv @('--directory', 'apps/sidecar', 'sync', '--locked', '--no-dev', '--group', 'build', '--python', $inputs.tools.python)
    $python = Join-Path $env:UV_PROJECT_ENVIRONMENT 'Scripts/python.exe'
    $tools.python = & $python --version
    if ($tools.python -cne "Python $($inputs.tools.python)") { throw 'Pinned Python version required' }
    $tools.pyinstaller = & $python -m PyInstaller --version
    if ($LASTEXITCODE -ne 0 -or $tools.pyinstaller -cne $inputs.tools.pyinstaller) { throw 'Pinned PyInstaller version required' }
    $distributions = & $python -c 'import importlib.metadata as m, json; print(json.dumps(sorted(({"name": d.metadata["Name"], "version": d.version} for d in m.distributions()), key=lambda d: d["name"].lower())))'
    if ($LASTEXITCODE -ne 0) { throw 'Python dependency metadata failed' }
    $distributions = @($distributions | ConvertFrom-Json)
    foreach ($notice in $inputs.notices) {
        $file = Join-Path $root ('packaging/' + $notice.path)
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $notice.sha256) {
            throw 'License input checksum mismatch'
        }
    }
    if (-not $DesktopExecutable) {
        $tools.node = & node --version
        $tools.pnpm = & pnpm --version
        $tools.rustc = & rustc --version
        $tools.cargo = & cargo --version
        if ($tools.node -cne "v$($inputs.tools.node)" -or $tools.pnpm -cne $inputs.tools.pnpm) {
            throw 'Pinned Node/pnpm versions required'
        }
        if ($tools.rustc -notlike "rustc $($inputs.tools.rust) *") { throw 'Pinned MSVC Rust toolchain required' }
        Invoke-Checked pnpm @('install', '--frozen-lockfile')
        Invoke-Checked pnpm @('--filter', '@redactio/desktop', 'tauri', 'build', '--no-bundle', '--target', $inputs.target, '--', '--locked')
        $DesktopExecutable = Join-Path $root "apps/desktop/src-tauri/target/$($inputs.target)/release/redactio.exe"
        $metadata = & cargo metadata --manifest-path apps/desktop/src-tauri/Cargo.toml --locked --format-version 1 --filter-platform $inputs.target
        if ($LASTEXITCODE) { throw 'Cargo metadata failed' }
        $metadata | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $work 'cargo-metadata.json')
        $DesktopNotices = Join-Path $work 'desktop-notices.txt'
        Invoke-Checked $python @('packaging/notices.py', '--output', $DesktopNotices, '--cargo-metadata', (Join-Path $work 'cargo-metadata.json'))
    }
    if (-not $DesktopNotices -or -not (Test-Path -LiteralPath $DesktopExecutable -PathType Leaf) -or
        -not (Test-Path -LiteralPath $DesktopNotices -PathType Leaf)) { throw 'Desktop executable and complete desktop notices required' }
    Invoke-Checked $python @('-c', 'import pefile,sys; p=pefile.PE(sys.argv[1]); names=[e.dll.decode().lower() for e in p.DIRECTORY_ENTRY_IMPORT]; p.close(); assert not any(n.startswith(("vcruntime", "msvcp")) for n in names), names', $DesktopExecutable)
    $tools.desktop_crt = 'static'
    $cab = Get-VerifiedArtifact $inputs.webview2
    Invoke-Checked $python @('-m', 'PyInstaller', '--noconfirm', '--clean', '--distpath', (Join-Path $work 'frozen'),
        '--workpath', (Join-Path $work 'freeze-work'), 'packaging/sidecar.spec')
    $frozenExecutable = Join-Path $work 'frozen/redactio-sidecar/redactio-sidecar.exe'
    $viewer = Join-Path $env:UV_PROJECT_ENVIRONMENT 'Scripts/pyi-archive_viewer.exe'
    $archiveContents = & $viewer -r -b $frozenExecutable
    if ($LASTEXITCODE) { throw 'Frozen sidecar import inspection failed' }
    foreach ($module in @('transformers.models.bert.modeling_bert',
                           'transformers.models.deberta_v2.modeling_deberta_v2',
                           'spacy_legacy', 'spacy_loggers')) {
        if (-not ($archiveContents -match [regex]::Escape($module))) {
            throw "Frozen sidecar import missing: $module"
        }
    }
    New-Item -ItemType Directory -Path $package | Out-Null
    Copy-Item -LiteralPath $DesktopExecutable -Destination (Join-Path $package 'redactio.exe')
    Copy-Item -LiteralPath (Join-Path $work 'frozen/redactio-sidecar') -Destination (Join-Path $package 'sidecar') -Recurse
    foreach ($required in @('sidecar/_internal/certifi/cacert.pem',
                             'sidecar/_internal/redactio_sidecar/model_catalog.json')) {
        if (-not (Test-Path -LiteralPath (Join-Path $package $required) -PathType Leaf)) {
            throw "Frozen sidecar resource missing: $required"
        }
    }
    if (-not @(Get-ChildItem -LiteralPath (Join-Path $package 'sidecar/_internal/tokenizers')
                 -Filter '*.pyd' -File).Count) { throw 'Frozen tokenizer support missing' }
    $modelRoot = Join-Path $package 'models'
    Invoke-Checked $python @('-c', 'from pathlib import Path; from redactio_sidecar.model_store import ModelRegistry, write_registry; write_registry(Path(__import__("sys").argv[1]), ModelRegistry(schema_version=2, models=[], legacy_unavailable=[]))', $modelRoot)
    foreach ($model in $PreloadModels) {
        Invoke-Checked $python @('scripts/prepare-biomedbert.py', '--model', $model,
                                '--model-dir', $modelRoot)
    }
    $runtimeStage = Join-Path $work 'runtime'
    New-Item -ItemType Directory -Path $runtimeStage | Out-Null
    Invoke-Checked "$env:SystemRoot\System32\expand.exe" @($cab, '-F:*', $runtimeStage)
    $browser = @(Get-ChildItem -LiteralPath $runtimeStage -Filter msedgewebview2.exe -Recurse)
    if ($browser.Count -ne 1 -or $browser[0].VersionInfo.ProductVersion -cne $inputs.webview2.version) {
        throw 'Fixed runtime identity mismatch'
    }
    Copy-Item -LiteralPath $browser[0].Directory.FullName -Destination (Join-Path $package 'webview2') -Recurse
    Copy-Item -LiteralPath (Join-Path $root 'docs/quick-start.de.md') -Destination $package
    Invoke-Checked $python @('packaging/notices.py', '--output', (Join-Path $work 'python-notices.txt'))
    $notices = (Get-Content -Raw -LiteralPath $DesktopNotices) + (Get-Content -Raw -LiteralPath (Join-Path $work 'python-notices.txt'))
    foreach ($entry in $inputs.models) {
        $license = if ($entry.descriptor.license) { $entry.descriptor.license } else { 'No license declared in the model card' }
        $notices += "`n$($entry.descriptor.title) ($($entry.descriptor.version)), $license.`nhttps://huggingface.co/$($entry.descriptor.repository)`n"
    }
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $package 'models') -File -Recurse |
             Where-Object Name -Like 'LICENSE*') { $notices += "`n" + (Get-Content -Raw -LiteralPath $file.FullName) }
    Copy-Item -LiteralPath (Join-Path $root 'packaging/licenses/webview2-fixed-LICENSE.html') -Destination (Join-Path $package 'webview2/LICENSE.html')
    $notices += "`nMicrosoft Edge WebView2 Runtime (Fixed Version) $($inputs.webview2.version).`nSee webview2/LICENSE.html. The runtime embeds its third-party credits; open webview2/show_third_party_software_licenses.bat to display them locally.`n"
    foreach ($required in @('webview2/LICENSE.html', 'webview2/show_third_party_software_licenses.bat')) {
        if (-not (Test-Path -LiteralPath (Join-Path $package $required))) { throw 'Runtime license notices missing' }
    }
    $notices | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $package 'THIRD-PARTY-NOTICES.txt')
    Invoke-Checked $python @('scripts/generate-corpus.py', '--output', (Join-Path $work 'smoke'), '--count', '1')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'check-package.ps1') -Destination $work
    $files = @(Get-ChildItem -LiteralPath $package -File -Recurse -Force | Sort-Object FullName | ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($package, $_.FullName).Replace('\', '/')
        if (-not $relative.StartsWith('models/', [StringComparison]::OrdinalIgnoreCase)) {
            @{ path = $relative
               sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
        }
    })
    $locks = @('pnpm-lock.yaml', 'apps/sidecar/uv.lock', 'apps/desktop/src-tauri/Cargo.lock') | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $root $_)).Hash.ToLowerInvariant() }
    }
    if ((Get-SourceIdentity $root).commit -cne $source.commit) { throw 'source_commit_changed_during_build' }
    $references = @('docs/release-checks.md', 'scripts/check-package.ps1', 'scripts/benchmark.py') | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $root $_)).Hash.ToLowerInvariant(); kind = 'procedure_only' }
    }
    @{ schema_version = 1; inputs = $inputs; lockfiles = @($locks); files = $files
       source = $source; built_at = [DateTimeOffset]::UtcNow.ToString('o'); observed_tools = $tools
       python_distributions = $distributions
       desktop = @{ mode = $(if ($desktopSupplied) { 'supplied' } else { 'built_here' })
                    provenance = $(if ($desktopSupplied) { 'caller_supplied_unverified' } else { 'local_build' })
                    c_runtime = 'statically_linked'
                    sha256 = (Get-FileHash -LiteralPath (Join-Path $package 'redactio.exe')).Hash.ToLowerInvariant() }
       test_evaluation_references = @($references); acceptance = 'unverified'
    } | ConvertTo-Json -Depth 20 |
        Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $package 'build-manifest.json')
    & (Join-Path $work 'check-package.ps1') -PackageRoot $package -CorpusRoot (Join-Path $work 'smoke')
    $archive = Join-Path $work 'redactio-0.1.0-windows-x64.zip'
    Compress-Archive -LiteralPath $package -DestinationPath $archive
    $relocated = Join-Path ([IO.Path]::GetTempPath()) ('Redactio Prüfung ' + [guid]::NewGuid())
    try {
        Expand-Archive -LiteralPath $archive -DestinationPath $relocated
        & (Join-Path $work 'check-package.ps1') -PackageRoot (Join-Path $relocated 'redactio') -CorpusRoot (Join-Path $work 'smoke')
    } finally {
        if (Test-Path -LiteralPath $relocated) { Remove-Item -LiteralPath $relocated -Recurse -Force }
    }
    "$((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($archive))" |
        Set-Content -Encoding ascii -LiteralPath ($archive + '.sha256')
    Write-Output 'windows_package_created'
} finally { Pop-Location }
