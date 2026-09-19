#requires -Version 7.0
param([string]$DesktopExecutable, [string]$DesktopNotices)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or -not [Environment]::Is64BitOperatingSystem) { throw 'Windows x64 build host required' }
$root = Split-Path $PSScriptRoot -Parent
$inputs = Get-Content -Raw -LiteralPath (Join-Path $root 'packaging/build-inputs.json') | ConvertFrom-Json
$work = Join-Path $root 'dist/windows'
$package = Join-Path $work 'redactio'
if ($root.StartsWith('\\')) { throw 'Build from a local Windows checkout; UNC paths are unsupported' }
if (Test-Path -LiteralPath $package) { throw 'Package output already exists; use a fresh build directory' }
New-Item -ItemType Directory -Force -Path (Join-Path $work 'inputs') | Out-Null
function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Build command failed: $Program" }
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
    if ((& uv --version) -notlike "uv $($inputs.tools.uv) *") { throw 'Pinned uv version required' }
    $env:UV_PROJECT_ENVIRONMENT = Join-Path $work 'venv'
    $env:UV_PYTHON_INSTALL_DIR = Join-Path $work 'python'
    $env:PYINSTALLER_CONFIG_DIR = Join-Path $work 'pyinstaller-cache'
    Invoke-Checked uv @('--directory', 'apps/sidecar', 'sync', '--locked', '--no-dev', '--group', 'build', '--python', $inputs.tools.python)
    $python = Join-Path $env:UV_PROJECT_ENVIRONMENT 'Scripts/python.exe'
    if ((& $python --version) -cne "Python $($inputs.tools.python)") { throw 'Pinned Python version required' }
    foreach ($notice in $inputs.notices) {
        $file = Join-Path $root ('packaging/' + $notice.path)
        if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $notice.sha256) {
            throw 'License input checksum mismatch'
        }
    }
    if (-not $DesktopExecutable) {
        if ((& node --version) -cne "v$($inputs.tools.node)" -or (& pnpm --version) -cne $inputs.tools.pnpm) {
            throw 'Pinned Node/pnpm versions required'
        }
        if ((& rustc --version) -notlike "rustc $($inputs.tools.rust) *") { throw 'Pinned MSVC Rust toolchain required' }
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
    $wheel = Get-VerifiedArtifact $inputs.model
    $cab = Get-VerifiedArtifact $inputs.webview2
    Invoke-Checked $python @('-m', 'PyInstaller', '--noconfirm', '--clean', '--distpath', (Join-Path $work 'frozen'),
        '--workpath', (Join-Path $work 'freeze-work'), 'packaging/sidecar.spec')
    New-Item -ItemType Directory -Path $package | Out-Null
    Copy-Item -LiteralPath $DesktopExecutable -Destination (Join-Path $package 'redactio.exe')
    Copy-Item -LiteralPath (Join-Path $work 'frozen/redactio-sidecar') -Destination (Join-Path $package 'sidecar') -Recurse
    Invoke-Checked $python @('packaging/stage.py', '--wheel', $wheel, '--destination', (Join-Path $package 'models'))
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
        @{ path = [IO.Path]::GetRelativePath($package, $_.FullName).Replace('\', '/')
           sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $locks = @('pnpm-lock.yaml', 'apps/sidecar/uv.lock', 'apps/desktop/src-tauri/Cargo.lock') | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $root $_)).Hash.ToLowerInvariant() }
    }
    @{ schema_version = 1; inputs = $inputs; lockfiles = @($locks); files = $files } | ConvertTo-Json -Depth 20 |
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
