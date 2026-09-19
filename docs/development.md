# Development

Run commands from the repository root unless stated otherwise. Application and
model setup may download build inputs; processing and prepared offline tests must
not download anything. Keep real documents, private settings and evaluation data
outside the checkout. Generate synthetic fixtures for tests.

## Toolchains

Windows 11 x64 is the release target. The pinned package-build tools are recorded
in [build-inputs.json](../packaging/build-inputs.json): Node 26.7.0, pnpm 10.11.0,
Python 3.13.13, Rust 1.95.0, uv 0.12.9 and PyInstaller 6.22.3. Native desktop builds
need the MSVC Rust target and Microsoft C++ Build Tools with the **Desktop
development with C++** workload. Development windows need WebView2; the portable
release ships its own fixed runtime and does not require a user installation.
Use a local Windows checkout for packaging, not a WSL UNC path.

Linux/WSL is a development environment, not a release target. The Tauri/WebKitGTK
4.1 prerequisites used here are `build-essential`, `curl`, `wget`, `file`,
`libssl-dev`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev` and `libxdo-dev`. A graphical session
such as WSLg is needed for interaction; Xvfb startup checks do not establish
Windows usability. Keep any extra toolchain/download caches local to the build
workspace; a cross-build must set both `CARGO_HOME` and `RUSTUP_HOME` to its
isolated tool directories.

## Dependencies and frontend

```sh
pnpm install --frozen-lockfile
uv --directory apps/sidecar sync --locked
pnpm test
pnpm typecheck
pnpm build
```

The pnpm commands have passed during component verification. `pnpm build` runs
`vue-tsc` and Vite; it produces frontend assets, not a distributable desktop app.
Once dependencies are cached, `pnpm install --offline --frozen-lockfile` is the
verified offline install variant. `pnpm dev` starts only Vite; host-backed file
operations need the Tauri window described below.

## Offline German models

Install model wheels explicitly during setup. The default is `de_core_news_lg`;
`de_core_news_sm` is optional for ordinary development but required by the full
model-switch tests. Both are 3.8.0. The URL fragments below pin the model hashes
used in verification. The portable package stages only the default large model
from its separately verified build input.

POSIX shell:

```sh
mkdir -p apps/sidecar/models/vendor
uv pip install --target apps/sidecar/models/vendor --no-deps \
  'https://github.com/explosion/spacy-models/releases/download/de_core_news_lg-3.8.0/de_core_news_lg-3.8.0-py3-none-any.whl#sha256=36fda650e476b54d5e87803635e36dadd1e8e034c4b5962088586d684f4c9fed'
uv pip install --target apps/sidecar/models/vendor --no-deps \
  'https://github.com/explosion/spacy-models/releases/download/de_core_news_sm-3.8.0/de_core_news_sm-3.8.0-py3-none-any.whl#sha256=fec69fec52b1780f2d269d5af7582a5e28028738bd3190532459aeb473bfa3e7'
```

PowerShell equivalent (each install is one line):

```powershell
New-Item -ItemType Directory -Force 'apps/sidecar/models/vendor' | Out-Null
uv pip install --target apps/sidecar/models/vendor --no-deps 'https://github.com/explosion/spacy-models/releases/download/de_core_news_lg-3.8.0/de_core_news_lg-3.8.0-py3-none-any.whl#sha256=36fda650e476b54d5e87803635e36dadd1e8e034c4b5962088586d684f4c9fed'
uv pip install --target apps/sidecar/models/vendor --no-deps 'https://github.com/explosion/spacy-models/releases/download/de_core_news_sm-3.8.0/de_core_news_sm-3.8.0-py3-none-any.whl#sha256=fec69fec52b1780f2d269d5af7582a5e28028738bd3190532459aeb473bfa3e7'
```

Save the following UTF-8 JSON as `apps/sidecar/models/manifest.json`. If only the
large model is installed, omit the second entry; do not advertise missing models.

```json
{
  "models": [
    {
      "name": "de_core_news_lg",
      "version": "3.8.0",
      "path": "vendor/de_core_news_lg/de_core_news_lg-3.8.0"
    },
    {
      "name": "de_core_news_sm",
      "version": "3.8.0",
      "path": "vendor/de_core_news_sm/de_core_news_sm-3.8.0"
    }
  ]
}
```

The engine checks each model's `meta.json`, `config.cfg`, identity, language,
version and spaCy compatibility. Models are ignored by Git. Missing/incompatible
models are errors; neither the engine nor its tests fetch replacements.

## Desktop development window

Debug resource resolution requires an absolute interpreter and model directory.
The locked uv environment installs the sidecar package; the host clears
`PYTHONPATH`/`PYTHONHOME` before spawning it. Do not rely on those variables to
make an uninstalled sidecar importable. The host adds `--model-dir` itself.

POSIX shell:

```sh
export REDACTIO_SIDECAR_EXECUTABLE="$PWD/apps/sidecar/.venv/bin/python"
export REDACTIO_MODEL_DIR="$PWD/apps/sidecar/models"
export REDACTIO_SIDECAR_ARGS_JSON='["-m","redactio_sidecar"]'
pnpm tauri:dev
```

PowerShell:

```powershell
$env:REDACTIO_SIDECAR_EXECUTABLE = (Resolve-Path 'apps/sidecar/.venv/Scripts/python.exe').Path
$env:REDACTIO_MODEL_DIR = (Resolve-Path 'apps/sidecar/models').Path
$env:REDACTIO_SIDECAR_ARGS_JSON = '["-m","redactio_sidecar"]'
pnpm tauri:dev
```

The Tauri dev command has been exercised under Linux/WSLg/Xvfb. The PowerShell
recipe matches the checked resource resolver; it is not a recorded native
Windows dev-server acceptance run. Release executables ignore development
resource overrides and resolve the sidecar, models and WebView2 beside the app.

## Engine and host checks

Set the offline model root and explicit test interpreter in the current shell.
The model directory must contain both test models for the full engine suite.

POSIX shell:

```sh
export REDACTIO_MODEL_DIR="$PWD/apps/sidecar/models"
export REDACTIO_TEST_PYTHON="$PWD/apps/sidecar/.venv/bin/python"
```

For cross-built Windows test executables, set `REDACTIO_TEST_MANIFEST_DIR` to the
native Windows path of `apps/desktop/src-tauri` in a checkout or fixture mirror.
The mirror must contain `tests/fake_sidecar.py` and the sibling
`apps/sidecar/tests/test_contract.py`; use a locked native Python test environment
with the current sidecar and pytest installed. Run from an unrelated local Windows
working directory. With no override, tests use Cargo's build-time manifest path.
This override affects test fixtures only; it is not an application resource override.

PowerShell:

```powershell
$env:REDACTIO_MODEL_DIR = (Resolve-Path 'apps/sidecar/models').Path
$env:REDACTIO_TEST_PYTHON = (Resolve-Path 'apps/sidecar/.venv/Scripts/python.exe').Path
```

Prepared-environment commands (the same arguments work in either shell):

```sh
uv --directory apps/sidecar run --locked --offline pytest -q
uv --directory apps/sidecar run --locked --offline ruff check .
uv --directory apps/sidecar run --locked --offline ruff format --check .
uv --directory apps/sidecar run --locked --offline mypy src
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check
cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
```

Recorded component verification ran pytest, Ruff and mypy in prepared environments
and passed the Linux host/static checks. The uv wrappers above keep subsequent
runs locked and offline; they require completed dependency setup. Rust transport
tests require `REDACTIO_TEST_PYTHON`;
absence is a setup failure. The real-model host roundtrip is explicitly opt-in.
Set its model/interpreter variables before invoking it:

```sh
export REDACTIO_REAL_SIDECAR_PYTHON="$REDACTIO_TEST_PYTHON"
export REDACTIO_REAL_MODEL_DIR="$REDACTIO_MODEL_DIR"
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --test sidecar reviewed_cli_roundtrips_through_the_bundled_model -- --ignored
```

```powershell
$env:REDACTIO_REAL_SIDECAR_PYTHON = $env:REDACTIO_TEST_PYTHON
$env:REDACTIO_REAL_MODEL_DIR = $env:REDACTIO_MODEL_DIR
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --test sidecar reviewed_cli_roundtrips_through_the_bundled_model -- --ignored
```

Packaging/benchmark regression checks use the same prepared interpreter:

```sh
apps/sidecar/.venv/bin/python scripts/test_packaging.py
apps/sidecar/.venv/bin/python scripts/test_benchmark.py
```

```powershell
& 'apps/sidecar/.venv/Scripts/python.exe' scripts/test_packaging.py
& 'apps/sidecar/.venv/Scripts/python.exe' scripts/test_benchmark.py
```

Benchmark tests have run on Linux and native Windows. Platform/privilege skips
remain skips, not acceptance evidence. Native Windows functional and final
package checks are separate from a Linux test pass.

## Windows package

Use PowerShell 7, Git, the pinned build tools and a fresh, clean local Git checkout
with native-readable Git metadata. A copied WSL linked worktree is not sufficient;
use an independent Windows clone. The script rejects tracked/staged changes and
untracked source files, while ordinary Git-ignored build output is allowed.
It refuses an existing `dist/windows/redactio`; preserve prior artifacts
instead of deleting a directory that may contain other data.

The script's native compiler route is:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows.ps1
```

This route is implemented but has **not** been verified end-to-end on a native
MSVC build host in the recorded early package run. The route actually exercised
used an independently built locked Windows MSVC desktop executable and complete
desktop notices, then performed native Python freezing, model/runtime staging,
checks, ZIP creation, extraction into a spaced/non-ASCII path and rechecking:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/package-windows.ps1 -DesktopExecutable 'dist/windows/desktop.exe' -DesktopNotices 'dist/windows/desktop-notices.txt'
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/check-package.ps1 -PackageRoot 'dist/windows/redactio' -CorpusRoot 'dist/windows/smoke'
Get-FileHash -Algorithm SHA256 -LiteralPath 'dist/windows/redactio-0.1.0-windows-x64.zip'
```

`ExecutionPolicy Bypass` applies only to the launched PowerShell process, as in
the early verification; it does not change a saved machine/user policy. Paths
above are expressed relative to the checkout instead of the verification machine.
The executable and notices arguments are required existing inputs for that route,
not files produced by the frontend build. The actual desktop cross-build used
the pinned, isolated cargo-xwin/LLVM/MSVC SDK toolchain and this command:

```sh
pnpm --filter @redactio/desktop tauri build --no-bundle --runner cargo-xwin --target x86_64-pc-windows-msvc -- --locked
```

The script writes the versioned ZIP and `.zip.sha256` beside its staged package,
plus checker and synthetic smoke files. `build-manifest.json` records the exact
clean source commit, package files, lock hashes, pinned inputs, observed build tools
and Python distributions. Hashed test/evaluation procedure references identify
what to run; acceptance remains explicitly unverified. A supplied desktop binary
is recorded by hash with unverified caller-supplied provenance; retain its actual
same-commit cross-build command and tool evidence beside the artifact. Final
artifact test/benchmark receipts and the ZIP checksum also remain beside the ZIP,
avoiding a circular hash reference. Third-party notices are bundled; Redactio's MIT
attribution is retained. The final build and full artifact acceptance remain pending;
the early ZIP is not the final release.

## Engine benchmark and private evaluation

The standalone benchmark uses stdlib Python 3.11+ on the measurement machine.
That is a measurement-tool requirement, not a requirement for end users running
the portable app. Point it at an already checked package and an explicitly
selected local corpus; choose an output file that does not exist:

```powershell
& 'apps/sidecar/.venv/Scripts/python.exe' scripts/benchmark.py --package 'dist/windows/redactio' --corpus 'local-data/synthetic' --output 'dist/windows/benchmark.json'
```

The same CLI has processed the early frozen Windows engine's supplied
400-document synthetic corpus with no custom rules. It records aggregate
counts, byte/character distributions, actual engine/model versions, CPU/RAM/OS,
fresh-process initialization, wall time and native peak sidecar memory. It saves
no filenames, source text, paths or rule text. Existing output files are refused;
empty corpora, protocol failures and incomplete runs cannot become successful
throughput. Failed document outcomes are counted and make the CLI exit nonzero.

The corpus path above is illustrative, not a bundled real-data directory. For a
private evaluation, both corpus and metrics belong outside version control.
Never upload originals or run them in CI. Benchmark timing excludes GUI review,
host journal/export work and corpus inventory; a new process does not clear OS
caches. Character counts do not establish page counts.

Release evaluation still requires a Windows 11 x64 reference PC with four CPU
cores, 16 GB RAM and SSD: 400 representative documents in under 30 minutes,
excluding human review, and ordinary UI responses within 200 ms. Independently
inspect at least 10% plus every warning/failure locally; record page/character
distribution, missed identifiers and false positives by category, corrections,
retest results and remaining limits without document contents. A known repeated-name
miss with `de_core_news_lg` 3.8.0 remains a quality limitation; successful synthetic
processing does not establish recall. Unavailable private evaluation remains an
unverified acceptance condition.
