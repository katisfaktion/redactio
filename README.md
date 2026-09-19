# Redactio

An offline Windows desktop app for turning German DOCX documents into
pseudonymized Markdown, with multiple saved sync pairs, stable document IDs,
and a local audit trail.

The initial release is defined in the
[release specification](docs/superpowers/specs/2026-09-19-redactio-initial-release-design.md).
It uses Vue 3, TypeScript, Vite, and sit-onyx for the frontend, retains the
prototype's Tauri / Rust / Python architecture, and includes an integrated
review-and-correction workflow before export. Building practical Vue skills is
an explicit project goal.

The release specification is approved. The five
[implementation plans](docs/superpowers/plans/2026-09-19-00-release-overview.md)
cover the desktop, processing engine, reliable batches, review/export, and Windows
delivery. They are ready for review; application implementation has not started.
The earlier prototype remains a reference; its Git history and development
process documents are not part of this project.

## Development boundaries

- Process documents entirely on the user's device.
- Keep original documents, identifying filenames, mappings, and review data
  out of exports, logs, and source control.
- Generate synthetic test documents at test time; never commit real records.
- Reuse useful prototype code only after checking it against the release spec.

## License

[MIT](LICENSE)
