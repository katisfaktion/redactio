# Offline Windows Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and verify a portable Windows ZIP containing all runtimes, models, UI assets, notices, and user instructions needed without network access or developer tools.

**Architecture:** Package the Python engine as an onedir sidecar and ship a fixed WebView2 runtime alongside the Tauri executable. The Rust host resolves resources from its executable directory before creating the window; release checks exercise that exact layout.

**Tech Stack:** Existing app stack, PyInstaller onedir, PowerShell, Windows x64 build runner, locally bundled WebView2/model assets, GitHub Actions.

**Spec:** [Approved release](../specs/2026-09-19-redactio-initial-release-design.md), sections 9–12. Final completion requires P1–P4; run P5.1's early smoke immediately after P3.2, then rerun it for the final artifact.

## Global Constraints

- “Windows 11 x64 is the initial supported release target.”
- “The package must be usable without administrator rights and without separately installed Node, Python, Rust, pnpm, or uv.”
- “All UI styles, fonts, icons, and translations must load from bundled assets without a CDN.”
- “Use a Windows 11 x64 reference PC with four CPU cores, 16 GB RAM, and an SSD, with no GPU requirement.”
- “The 400-document/30-minute target excludes human review time.”
- “Ordinary controls should respond within 200 ms while work runs off the UI thread; large document views must not freeze the window.”
- “Independently inspect at least 10% of the representative corpus plus every warning/failure case locally.”
- All [overview constraints](2026-09-19-00-release-overview.md#global-constraints) apply. Build-time downloads are permitted; runtime downloads are not. Tests use disposable environments and synthetic documents, never disable networking on the developer's host.

## Review Focus

1. A developer-installed model hides missing packaged files: clear interpreter/model paths and test outside the checkout in P5.1.
2. Tauri installer configuration is mistaken for portable ZIP setup: explicitly resolve the app-local fixed WebView2 runtime and test the ZIP in P5.1/P5.2.
3. Non-ASCII or spaced installation paths break child startup: package relocation test in P5.2.
4. A library/CSS/font makes an unexpected network request: offline first launch plus traffic observation in P5.2.
5. Synthetic benchmarks overstate medical-document quality: separate synthetic automation and private user-operated corpus evaluation in P5.3.

---

## Planned files

| Files | Responsibility |
| --- | --- |
| `scripts/package-windows.ps1` | Locked build, fixed resources, archive and checksums |
| `scripts/check-package.ps1` | Package structure, hashes, real sidecar ping/configure/process |
| `scripts/generate-corpus.py` | Synthetic DOCX-only local fixture generation |
| `scripts/benchmark.py` | Timed packaged-engine runs and content-free metrics |
| `packaging/{build-inputs.json,sidecar.spec}` | Exact artifact URLs/versions/hashes and PyInstaller build recipe |
| `.github/workflows/{check.yml,windows-release.yml}` | Linux/Windows checks and manually triggered package artifact |
| `docs/{quick-start.de.md,release-checks.md}` | User workflow and acceptance evidence protocol |
| Existing resources.rs/main.rs/tauri.conf.json/pyproject.toml/uv.lock | Production resource paths and locked packaging dependency |
| Existing README.md/docs/development.md/.gitignore | Tested run/build commands and artifact exclusions |

Build results go under gitignored `dist/windows/`; model wheels, runtime CABs,
corpora, and archives are never committed. `packaging/build-inputs.json` records
their exact versions, upstream locations, SHA-256 values, and notices after
verification in P5.1, with no placeholder entries.

## P5.1: Package the engine and prove the portable resource layout

**Files:** Create packaging inputs/recipe, package/check scripts, generate-corpus.py's single-canary mode, and docs/quick-start.de.md's setup/resource instructions; extend resources.rs/main.rs, model manifest generation, pyproject.toml/uv.lock, and .gitignore. P5.2 extends corpus generation and P5.3 completes the user guide.

**Interfaces:** P3 `resources::resolve()` produces absolute paths beneath the
executable directory in production. Sidecar accepts `--model-dir` and resolves
allowlisted names from `models/manifest.json`; it never searches the user's Python
site-packages. `check-package.ps1 -PackageRoot <directory>` exits nonzero on any
missing resource, checksum mismatch, protocol error, or unredacted synthetic
canary. The command below uses the concrete build-output directory.

- [ ] Write check-package.ps1 first with strict-mode/ErrorActionPreference Stop:

```powershell
param([Parameter(Mandatory)][string]$PackageRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
foreach ($relative in @('redactio.exe', 'sidecar/redactio-sidecar.exe',
                       'models/manifest.json', 'webview2/msedgewebview2.exe',
                       'THIRD-PARTY-NOTICES.txt', 'quick-start.de.md')) {
    if (-not (Test-Path -LiteralPath (Join-Path $PackageRoot $relative))) {
        throw "Package resource missing: $relative"
    }
}
```

- [ ] Run `rtk proxy pwsh -File scripts/check-package.ps1 -PackageRoot dist/windows/redactio`; expect failure for absent resources before the packaging implementation.
- [ ] Resolve exact compatible versions on the Windows build machine; record
  pnpm/Python/Rust/PyInstaller, engine/model package versions, model wheel hash,
  fixed WebView2 CAB version/hash, and dependency licenses in build-inputs.json.
  Keep lockfiles and toolchain requirements consistent. Verify actual model
  compatibility with the real engine before recording the selected versions.
- [ ] Add PyInstaller to the build-only dependency group and create sidecar.spec
  using onedir mode. Collect required spaCy/Presidio data and metadata, preserving
  the JSONL console streams; do not use windowed mode or onefile runtime extraction.
  Package model data once under `models/`, with a manifest mapping the public
  model name/version to its verified local data directory. Test missing model
  files produce a fixed safe setup error with no downloader attempt.

```powershell
rtk proxy pnpm install --frozen-lockfile
rtk proxy uv --directory apps/sidecar sync --locked --group build
rtk proxy uv --directory apps/sidecar run python -m PyInstaller --noconfirm --clean ../../packaging/sidecar.spec
rtk proxy pnpm --filter @redactio/desktop tauri build --no-bundle
```

Configure sidecar.spec's paths relative to its own file, not the caller's working
directory. The package script stages the exact produced onedir tree, model data,
Tauri executable, UI embedded assets, runtime, notices, and quick-start document:

```text
redactio/
  redactio.exe
  sidecar/redactio-sidecar.exe
  sidecar/_internal/
  models/manifest.json
  models/de_core_news_lg/
  webview2/msedgewebview2.exe
  webview2/                 (all fixed-runtime files)
  THIRD-PARTY-NOTICES.txt
  quick-start.de.md
  build-manifest.json
```

- [ ] Set the fixed WebView2 directory before Tauri creates any window. Use
  an explicit resolved browser directory through the supported runtime API or
  `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` set at single-threaded startup, not an
  environment variable the user must set. The portable archive does not inherit
  installer bootstrap behavior. Ensure the compatible WebView2 loader is included
  or statically linked. Validate runtime/resource paths before loading binaries.
- [ ] Extend the check script to launch the packaged sidecar via
  System.Diagnostics.Process with ArgumentList and redirected standard streams;
  send ping, configure with synthetic UUIDs, and process one generated canary DOCX
  with its real SHA-256. Read bounded replies with a deadline, validate model
  version, placeholders, and no unredacted canary. Clear PYTHONPATH/PYTHONHOME and
  launch from an unrelated temporary directory. Emit IDs/counts/codes only.
- [ ] Generate the one-document smoke input at build time using python-docx in
  generate-corpus.py and the same synthetic contact text shown in P5.2. Reject a
  non-empty output directory; never overwrite user documents. The test machine
  receives this synthetic DOCX alongside the check script and needs no Python.
  Write the initial quick start with ZIP extraction, launch, and missing-resource
  guidance so the early package is complete enough to test.
- [ ] Run `rtk proxy pwsh -File scripts/package-windows.ps1` then the check script.
  Move the staged folder outside the repository and rerun the smoke. Launch the
  executable in a standard-user Windows 11 session. Early smoke proves packaging
  and processing only; the final P5.3 rerun adds full review/export evidence.
- [ ] Commit: `rtk git add packaging scripts docs/quick-start.de.md apps/desktop/src-tauri apps/sidecar/pyproject.toml apps/sidecar/uv.lock .gitignore`; `rtk git commit -m 'build: package offline Windows runtimes and processing engine'`.

Sources for packaging decisions: [PyInstaller operating modes](https://pyinstaller.org/en/stable/operating-mode.html),
[Microsoft fixed-runtime distribution](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution),
and [Tauri Windows distribution](https://v2.tauri.app/distribute/windows-installer/).
These document components of the approach; the portable ZIP itself still needs
the explicit launch smoke above.

## P5.2: Windows integration, accessibility, and offline acceptance

**Files:** Create check.yml/windows-release.yml and docs/release-checks.md; extend generate-corpus.py, Windows tests and check-package.ps1. Use existing test runners; no bespoke test orchestration framework.

**Interfaces:** `generate-corpus.py --output local-data/synthetic --count 400` generates deterministic documents from synthetic constants and a content-free expectations manifest. `check-package.ps1` takes PackageRoot and optional CorpusRoot; it verifies packaged engine behavior without requiring Python on the test machine. GUI verification is a recorded real-window test, not a browser mock.

- [ ] Add an integration checklist with one explicit expected outcome for every
  A01–A18 row. Add a native Windows regression for ordinary source/output paths
  containing spaces, German umlauts, and a junction alias. The package check must
  fail if run against an archive missing its bundled runtime or model directory,
  even on a developer machine with those installed elsewhere.
- [ ] Implement the generator without committed fixtures:

```python
from pathlib import Path
from docx import Document

def generate(output: Path, count: int) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for index in range(count):
        document = Document()
        document.add_paragraph(f"Synthetischer Fall {index + 1}.")
        document.add_paragraph("Kontakt: anna.beispiel@example.invalid")
        table = document.add_table(rows=1, cols=2)
        table.cell(0, 0).text = "Kategorie"
        table.cell(0, 1).text = "Synthetischer Inhalt"
        document.save(output / f"case-{index + 1:04}.docx")
```

Add argparse for the stated options and checks that output is an empty dedicated
directory or generator-owned directory; do not overwrite user documents. Include
a separate small edge-case set for warnings, corrupt ZIP, empty body, repeated
names, Unicode, and deliberate output tampering.

- [ ] Configure CI to run pnpm frozen install/typecheck/test/build, locked uv
  checks with the exact pre-fetched offline model, and locked Cargo fmt/clippy/test
  on Linux and Windows. Use `-D warnings` for project code after fixing warnings.
  Cache only dependency/build artifacts, never local-data or test document content.
  Windows release workflow uses workflow_dispatch and uploads a CI artifact;
  publishing a public release is a separate user action.
- [ ] In a disposable Windows 11 test environment, extract to
  `C:\Users\Public\Redactio Prüfung\`, use a standard non-admin account, and
  disconnect the VM/test environment from its network. Remove development-tool
  paths from that process's PATH; do not uninstall host tools or change host
  firewall settings. Exercise configure/process/review/correct/approve/export,
  restart, missing resource, cancellation, and error messages in the real window.
- [ ] Observe runtime network activity during first launch and the full flow;
  assert there are no app/child external requests, including fonts/images/models.
  Verify fixed-runtime identity from its loaded executable path/version in the
  test environment. Record OS/WebView cache locations separately from app data;
  no test may claim Windows performs zero unrelated writes.
- [ ] Check keyboard-only flow, accessible names, focus order/restoration,
  contrast, progress announcements, warning visibility, and long-document
  responsiveness. Use selected Onyx controls in the actual WebView. Browser-unit
  checks complement this evidence; they do not replace it.
- [ ] Run the full matrix and package checks; record command outputs and
  screenshot references without real document data in release-checks.md. If a
  Windows test cannot run, mark the criterion unverified and do not label the
  release complete.
- [ ] Commit: `rtk git add .github scripts docs/release-checks.md apps/desktop`; `rtk git commit -m 'test: verify offline Windows document workflow'`.

## P5.3: Performance, private evaluation protocol, and release artifact

**Files:** Create benchmark.py; complete quick-start.de.md and update release-checks.md, development.md, README.md and packaging scripts. No new product feature is introduced in this task.

**Interfaces:** `benchmark.py --package dist/windows/redactio --corpus local-data/synthetic --output dist/windows/benchmark.json` runs the packaged engine with a single configured model, measures cold start and total wall time, and records count/size distributions plus peak child memory. It records no input filenames/text. Private corpus evaluation uses the same script with an explicitly selected local path and results kept outside version control.

- [ ] Before writing the benchmark, define its result fields and an executable
  sanity check:

```python
def validate_metrics(result: dict) -> None:
    assert result["documents"] > 0
    assert result["cold_start_seconds"] >= 0
    assert result["total_seconds"] >= result["cold_start_seconds"]
    assert result["processed"] + result["failed"] == result["documents"]
    assert result["peak_memory_bytes"] > 0
    assert "filenames" not in result and "text" not in result
```

Implement benchmark.py with argparse, monotonic perf_counter timing, the same
bounded JSONL calls as package smoke, and native OS process-memory sampling.
Reject empty corpora and incomplete runs rather than reporting a misleading
throughput. Include machine CPU/RAM/OS and engine/model versions in the result.

- [ ] Run the synthetic 400-document benchmark and inspect UI interaction timing
  during a real desktop run. Then have the user run the private representative
  corpus locally on the reference-class Windows PC. Record actual page/character
  distribution, throughput, cold start, and peak memory; synthetic speed alone is
  not the spec's performance acceptance. Preserve the 30-minute/200-ms targets.
- [ ] Document the user's independent 10% sample plus every warning/failure case:
  category counts of missed identifiers/false positives, corrections, retest
  outcome, and remaining limits only. Never request uploading originals or put
  this corpus in CI. An unavailable private evaluation remains an explicit
  unverified acceptance condition, not an automatic release approval.
- [ ] Write the German quick start covering extract ZIP/launch, private source
  and working-output locations, add/select/remove pairs, detector configuration,
  run/cancel/retry, extraction warnings, manual review, stale approvals, empty
  export directory, audit location/failure, backup of source metadata, and offline
  model/setup errors. Explain that removal leaves files, notes stay private,
  exports are snapshots, and pseudonymization has no guaranteed-anonymity claim.
- [ ] Rerun P5.1/P5.2 against the release candidate, build the versioned ZIP, create
  SHA-256 checksums with Get-FileHash, and write build-manifest.json containing
  commit ID, exact dependency/model/runtime versions, artifact hashes, and
  test/evaluation references. Include all required third-party license notices
  without changing Redactio's MIT attribution.
- [ ] Verify `rtk proxy pwsh -File scripts/check-package.ps1 -PackageRoot dist/windows/redactio`, all release-matrix entries, archive extraction, checksum
  matches, and clean `rtk git diff --check`. Update README with tested development
  and package commands, not prototype-era assumptions.
- [ ] Commit: `rtk git add scripts packaging docs README.md`; `rtk git commit -m 'docs: record Windows release verification and user workflow'`.
- [ ] Rebuild from this clean committed tree, rerun check-package and the final
  artifact acceptance matrix, and put its exact HEAD in build-manifest.json.
  Keep the final ZIP checksum and its acceptance evidence beside the artifact
  under dist/windows, avoiding a commit/artifact checksum reference cycle.

## Completion evidence

- [ ] The exact portable artifact passes A01–A18 and contains no user corpus.
- [ ] Performance and private sampling evidence satisfy spec §12, or the release
  remains explicitly incomplete with the unverified criteria named.
- [ ] Archive, checksums, notices, quick start, dependency manifest, and evidence
  are locally reviewable. No installer, auto-update service, signing pipeline,
  public release publication, or deployment was added implicitly.
