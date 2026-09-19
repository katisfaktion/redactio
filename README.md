# Redactio

Redactio turns German DOCX documents into pseudonymized Markdown on the user's
device. Named source/working-output pairs keep collections separate; local
detection, review and correction support an explicit approved-export workflow.
Detection can miss identifying information. Redactio does not guarantee anonymity.

**Release status:** implementation and Windows acceptance are still in progress.
An early portable package has passed startup and synthetic engine checks; this
does not establish final review/export, offline clean-machine, accessibility,
reference-PC performance or private-corpus acceptance. The initial release is
not yet complete.

## Use on Windows

The release target is Windows 11 x64. The portable ZIP includes the Python
engine, German `de_core_news_lg` model and fixed WebView2 runtime; users do not
need administrator rights or development tools. Extract the complete archive
before starting `redactio.exe`.

The [German quick start](docs/quick-start.de.md) covers private folders, pair
configuration, processing, review, export, backups and recovery. Working outputs
may contain unreviewed information; only explicitly approved exports are intended
for subsequent use. Exports are snapshots and are not revoked by later edits.

## Development and package checks

The app uses Tauri 2/Rust, Vue 3/TypeScript/Vite/sit-onyx and a Python
Presidio/spaCy sidecar. See [development setup](docs/development.md) for pinned
tools, offline model preparation, PowerShell/POSIX commands and explicit sidecar
configuration.

For manual evaluation of names and addresses, the optional
[BiomedBERT setup](docs/development.md#optional-biomedbert-detector-for-local-evaluation)
adds the pinned German OpenMed 340M model to the per-pair selector. Its runtime and
weights are installed explicitly during setup and run locally afterward.

From the repository root, after dependency setup:

```sh
pnpm test
pnpm typecheck
pnpm build
```

These commands have passed during component verification. `pnpm build` builds
the frontend; it does not produce the portable Windows distribution.
[Windows packaging](docs/development.md#windows-package) describes the actual
package script, tested early build route and remaining verification limits.

The [release specification](docs/superpowers/specs/2026-09-19-redactio-initial-release-design.md)
and [implementation plans](docs/superpowers/plans/2026-09-19-00-release-overview.md)
define the full release, including the 400-document/30-minute and 200-ms UI
targets. Synthetic engine speed is not evidence of representative-document
quality or full desktop throughput.

## Development boundaries

- Process documents entirely on the user's device.
- Keep originals, identifying filenames, private mappings, rule text and review
  notes out of exports, logs and source control.
- Generate synthetic test documents at test time; never commit real records.
- Evaluate representative private documents locally; never upload originals or
  place them in CI. Independently inspect at least 10% plus every warning/failure.

## License

[MIT](LICENSE)
