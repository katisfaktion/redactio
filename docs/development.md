# Development

Redactio is a pnpm workspace. Install dependencies from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
pnpm tauri:dev
```

The application does not load fonts, scripts, updates, or other resources from the network at runtime. Dependency installation is the only build-time network step.

## Windows 11

Windows 11 x64 is the supported release target. Install Node.js 26, pnpm 10.11, Rust with the stable MSVC toolchain, Microsoft C++ Build Tools with the **Desktop development with C++** workload, and the WebView2 runtime. Run the commands above in PowerShell from a native Windows checkout.

## Linux and WSL development

Linux/WSL is for development checks and is not a release target. Install the Tauri system packages for a WebKitGTK 4.1 build: `build-essential`, `curl`, `wget`, `file`, `libssl-dev`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libxdo-dev`. A graphical session such as WSLg is required to interact with `pnpm tauri:dev`; CI-style startup checks can use Xvfb.

## Sidecar and offline German model

Create the sidecar environment from the locked package set:

```sh
uv --directory apps/sidecar sync --frozen
```

Model installation is an explicit development/build step. Runtime and tests never download a
model. Install the official German 3.8.0 wheels into the ignored local model layout. The URL
fragments pin Explosion's published wheel SHA-256 values; `de_core_news_lg` remains the standard
model and `de_core_news_sm` provides the model-switch development check:

```sh
mkdir -p apps/sidecar/models/vendor
uv pip install --target apps/sidecar/models/vendor --no-deps \
  'https://github.com/explosion/spacy-models/releases/download/de_core_news_lg-3.8.0/de_core_news_lg-3.8.0-py3-none-any.whl#sha256=36fda650e476b54d5e87803635e36dadd1e8e034c4b5962088586d684f4c9fed'
uv pip install --target apps/sidecar/models/vendor --no-deps \
  'https://github.com/explosion/spacy-models/releases/download/de_core_news_sm-3.8.0/de_core_news_sm-3.8.0-py3-none-any.whl#sha256=fec69fec52b1780f2d269d5af7582a5e28028738bd3190532459aeb473bfa3e7'
```

Create `apps/sidecar/models/manifest.json` with the packaged layout consumed unchanged by the
sidecar:

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

The engine verifies the directory's `meta.json`, `config.cfg`, model identity, version, language,
and spaCy compatibility before loading it. Run the real offline checks with:

```sh
export REDACTIO_MODEL_DIR="$PWD/apps/sidecar/models"
uv --directory apps/sidecar run pytest tests/test_engine.py -q
uv --directory apps/sidecar run pytest -q
uv --directory apps/sidecar run ruff check .
uv --directory apps/sidecar run mypy src
```
