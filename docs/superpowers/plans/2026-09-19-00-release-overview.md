# Redactio Initial Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved offline Windows application through five testable increments.

**Architecture:** Vue and sit-onyx call a Rust host that owns private state and file writes. One bounded Python subprocess extracts, detects, and renders; the host commits results and approves exports. Build on the useful prototype algorithms without importing its history or process archive.

**Tech Stack:** Tauri 2, Vue 3, TypeScript, Vite, sit-onyx, Zod, Rust, Python, Presidio, spaCy, python-docx, Pydantic, pnpm, Cargo, uv.

**Spec:** [Approved initial release](../specs/2026-09-19-redactio-initial-release-design.md).

## Global Constraints

- “Process one selected pair at a time; there is no queue or run-all-pairs action.”
- “No runtime network communication, telemetry, update checks, external content, remote fonts, or model downloads; dependency setup at build time is separate.”
- “Windows 11 x64 is the initial supported release target.”
- “Source documents are never edited.”
- “Offsets are zero-based Unicode code-point positions with an exclusive end.”
- “Initial limits are 64 MiB compressed input, 256 MiB cumulative uncompressed ZIP content, a 1,000:1 expansion ratio, 1,000,000 extracted Unicode code points, and 64 MiB per IPC message.”
- “Initialization has a 180-second deadline; each document request has a 120-second deadline.”
- “Use versioned, validated formats with timezone-aware RFC-3339 timestamps.”
- “The full document identity is `(sync_pair_id, doc_id)`; two pairs may both contain `doc-0001` without sharing data.”
- “Frontend verification includes Vue/TypeScript type checking with `vue-tsc`, a Vite production build, and focused interaction checks for pair selection, configuration validation, progress/cancellation, and unsaved review changes.”
- The UI is German; code, identifiers, and technical documentation are English. Treat the entire approved spec as binding, including requirements not quoted here.
- Every shell command in this workspace is prefixed with `rtk`; use `rtk proxy` for commands without an RTK adapter. Never use production document data in repository tests or CI.

## Review Focus

1. A destination is replaced by a Windows junction after setup: revalidate before committing; test in P1.2 and P5.2.
2. One pair's custom rules remain loaded when another is selected: configure the entire engine snapshot, not a delta; test in P2.4 and P3.2.
3. Power loss between output, review record, and mapping updates: reconciliation never restores an approval to different bytes; test in P3.3 and P4.1.
4. Emoji before a manual selection: DOM UTF-16 positions must become code-point offsets, including nested highlight nodes; test in P4.2.
5. The portable build accidentally finds the developer's model or system WebView: verify a clean, disconnected standard-user machine using only packaged resources; test in P5.1 and P5.2.

---

## Execution order and deliverables

| Order | Plan | Working result | Depends on |
| --- | --- | --- | --- |
| 1 | [P1 Desktop and sync pairs](2026-09-19-01-desktop-and-pairs.md) | Launch a German Vue/Onyx app; add/select/remove isolated pairs and inspect sources | None |
| 2 | [P2 Local processing engine](2026-09-19-02-processing-engine.md) | Independently testable JSONL engine extracts and pseudonymizes synthetic DOCX | None; uses contracts below |
| 3 | [P3 Reliable batch processing](2026-09-19-03-batch-and-recovery.md) | Process selected pairs with cancellation, recovery, settings, and audit | P1 + P2 |
| 4 | [P4 Review and export](2026-09-19-04-review-and-export.md) | Correct and approve documents, then export verified approved files | P3 |
| 5 | [P5 Windows delivery](2026-09-19-05-windows-delivery.md) | Offline portable ZIP, Windows evidence, German quick start | P1–P4 |

Run P5.1's package smoke as soon as P3.2 works, before finishing the review UI.
Repeat P5.1 on the final branch in P5.3. This catches runtime/model packaging
problems early without delaying the standalone engine work.

These are dependency-ordered subsystem plans, not permission to implement in
parallel. P1/P2 can be developed independently only if their shared contracts stay
aligned. Execution begins after plan review and an execution-method choice.

## File and ownership map

Paths in the plans are relative to the repository root. The root currently
contains documentation only; every application path below is a planned creation.

| Location | Responsibility |
| --- | --- |
| `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml` | Desktop workspace and commands |
| `apps/desktop/src/{App.vue,main.ts,styles.css}` | Vue root, Onyx/local assets, three-view navigation |
| `apps/desktop/src/components/` | `PairManager.vue`, `DocumentList.vue`, `RunPanel.vue`, `DetectionSettings.vue`, `ReviewView.vue`, `ExportDialog.vue` |
| `apps/desktop/src/lib/{contracts.ts,ipc.ts}` | Zod host projections and typed Tauri calls |
| `apps/desktop/src/composables/{usePairs.ts,useRun.ts,useReview.ts}` | Focused state/event lifetimes, only introduced with their consumer |
| `apps/desktop/src-tauri/src/{lib.rs,main.rs,commands.rs,error.rs}` | Tauri boot, thin command boundary, safe error codes |
| `apps/desktop/src-tauri/src/{sidecar.rs,protocol.rs,resources.rs}` | Child lifecycle, typed wire mirrors, installed resource paths |
| `apps/desktop/src-tauri/src/domain/{paths.rs,storage.rs,settings.rs,scan.rs}` | Canonical roots, atomic IO/locks, pair registry, discovery |
| `apps/desktop/src-tauri/src/domain/{mapping.rs,sync.rs,audit.rs}` | Identity/journal, orchestration, bounded-content audit |
| `apps/desktop/src-tauri/src/domain/{review.rs,export.rs}` | Review binding and current-approved snapshot export |
| `apps/sidecar/src/redactio_sidecar/{schemas.py,extract.py,redaction.py,engine.py,frontmatter.py,ipc.py,__main__.py}` | Validated contract, extraction, span logic, local NLP, rendering, JSONL |
| `apps/sidecar/tests/`, `apps/desktop/src-tauri/tests/`, `apps/desktop/src/**/*.test.ts` | Outcome-focused tests with generated inputs |
| `scripts/{package-windows.ps1,check-package.ps1,generate-corpus.py,benchmark.py}` | Package and release evidence; never installed as a background service |
| `.github/workflows/check.yml`, `.github/workflows/windows-release.yml` | Synthetic checks and manually requested package build |
| `docs/{development.md,quick-start.de.md,release-checks.md}` | Runnable setup, end-user instructions, evidence protocol |

Use prototype source only as a reference at `/home/katisfaktion/projects/redactio`.
Reuse extraction/frontmatter/scan ideas after removing old assumptions. Do not
copy its React UI, broad stderr forwarding, source-hash-only skip logic, unsafe
overlap dropping, or per-occurrence placeholder numbering.

## Shared contracts to implement

These signatures are planning decisions that make the five plans executable
together. P1 owns Rust state and its Zod projection; P2 owns Pydantic wire/output
types. No extra schema generator or separate framework is needed.

### C1: Pair state (P1.2–P1.3)

Persisted root records start at `schema_version: 1`; all types deny unknown
fields and validate IDs.
Rust `Uuid` serializes as a lowercase hyphenated string; paths remain host-only
except local folder labels and native UI displays. The frontend does not send
arbitrary paths to document operations.

```ts
type EntityType = "PERSON" | "LOCATION" | "EMAIL_ADDRESS" | "PHONE_NUMBER"
  | "IBAN_CODE" | "IP_ADDRESS" | "URL" | "DATE_TIME" | "CUSTOM";
type CustomRule = {
  id: string; entity_type: EntityType; enabled: boolean;
} & ({ kind: "regex"; pattern: string } | { kind: "words"; words: string[] });
type ProcessingConfig = {
  model: string; enabled_entities: EntityType[]; custom_rules: CustomRule[];
  include_positions: boolean;
};
type SyncPair = {
  id: string; name: string; source_folder: string; target_folder: string;
  created_at: string; processing_revision: string; config: ProcessingConfig;
};
type Settings = {
  schema_version: 1; sync_pairs: SyncPair[]; selected_sync_pair_id: string | null;
};
type DocumentKey = { sync_pair_id: string; doc_id: string };
type SafeError = { code: string; retryable: boolean };
```

`ProcessingConfig::default()` uses `de_core_news_lg`, the eight spec entity types
excluding CUSTOM, no custom rules, and `include_positions: true`. Rule IDs are
UUIDs. Regex rules use Python `re` syntax without implicit flags; word-list
matching is literal with word boundaries where applicable, case-sensitive unless
the regex explicitly selects otherwise. Engine/model package identity is kept in
the processing fingerprint privately; public `processing_revision` is a UUID
rotated by Rust on behavioral change, not a secret-derived hash.
The persisted Rust pair extends this UI projection with
`processing_fingerprint: Option<ProcessingFingerprint>`, initially null until the
first successful configure. The fingerprint contains engine_version,
extraction_version, model_name, and model_version. Keep it out of UI writes;
P3.5 compares and persists it when configuration becomes available.

### C2: Sidecar requests and results (P2.1; Rust mirrors in P3.2)

One UTF-8 JSON object per line: `{id, type, payload}`. Types are `ping`,
`configure`, `process_document`, `preview_rules`, `render_review`, and `error`.
Replies echo `id` and use `<request_type>_result`; errors contain only SafeError.
Every collection request/result contains `sync_pair_id` and
`processing_revision`. Host callbacks bind both plus a request/run ID.

```ts
type Detection = {
  id: string; start: number; end: number; entity_type: EntityType;
  confidence: number | null; recognizer: string; origin: "automatic" | "manual";
};
type Decisions = { dismissed_ids: string[]; manual: Detection[] };
type OutputEntry = {
  start_offset: number; end_offset: number; entity_type: EntityType;
  placeholder: string; confidence: number | null; recognizer: string;
  origin: "automatic" | "manual" | "merged";
};
type EngineInfo = {
  engine_version: string; model_name: string; model_version: string;
  recognizers: string[]; extraction_version: string;
};
type ReviewStatus = "pending" | "approved" | "rejected" | "needs-rework";
type DocumentMeta = DocumentKey & {
  source_hash_sha256: string; processing_revision: string; redacted_at: string;
};
type ProcessRequest = DocumentMeta & { source_path: string };
type ReviewRequest = ProcessRequest & {
  detections: Detection[]; decisions: Decisions; review_status: ReviewStatus;
  reviewed_at: string | null; acknowledged_warnings: string[];
};
type ProcessResult = DocumentMeta & {
  markdown: string; body: string; original_text: string; detections: Detection[];
  redactions: OutputEntry[]; warnings: string[]; body_was_empty: boolean;
  review_status: ReviewStatus; engine: EngineInfo;
};
```

`configure` payload: pair ID, revision, ProcessingConfig; response adds EngineInfo.
`preview_rules` payload: pair ID, revision, `text: string`; response contains
`detections: Detection[]`. `ping` returns `protocol_version: 1`. `process_document`
extracts and analyzes; `render_review` re-extracts the same hash-bound source and
applies stored automatic spans/manual decisions without rerunning analysis.
Host supplies generation/review timestamps; retrying render does not invent a
new timestamp. Sidecar rejects mismatched active pair/revision and changed bytes.

Original text is returned only through private IPC and kept in memory; normal
batch UI events receive counts/IDs, never text. The host persists detections and
decisions but strips original text. Detection IDs are deterministic within one
source/revision result; no value hash is exposed. Invalid span IDs/offsets fail.

### C3: Host operation boundaries (P3.1–P4.3)

Rust `AppState` holds the settings path, operation mutex, cancellation flag, and
Sidecar. Domain methods return `Result<T, AppError>`; AppError serializes SafeError
only. An RAII operation guard releases locks on all exit paths.

| Method | Contract |
| --- | --- |
| `list_pairs() -> Settings` | Validate stored settings before exposing them |
| `add_pair(name, source_folder, target_folder, create_target) -> Settings` | Validate canonical roots; persist identity binding |
| `rename_pair(pair_id, name) -> Settings`, `select_pair(pair_id) -> Settings`, `remove_pair(pair_id) -> Settings` | Reject unknown IDs/busy operations; remove configuration only |
| `scan_pair(pair_id) -> ScanReport` | `files: ScannedFile[]`, `errors: ScanFailure[]`; paths private local UI only |
| `save_processing_config(pair_id, config) -> Settings` | Sidecar validates first; persist new revision only on actual config change |
| `preview_rules(pair_id, config, text) -> Detection[]` | Temporary bounded configuration; restore the saved snapshot afterwards |
| `list_models() -> ModelInfo[]` | Read local manifest; ModelInfo contains name, version, compatible |
| `start_sync(pair_id, relative_paths: string[] \| null, force_doc_ids: string[]) -> run_id` | Own immutable pair snapshot; return promptly; force IDs are explicitly confirmed by UI |
| `cancel_sync(pair_id, run_id) -> ()` | Only matching active operation is cancelled |
| `get_run_summary(pair_id, run_id) -> RunSummary \| null` | Recover the authoritative result if an event was missed |
| `open_review(key) -> ReviewViewData` | Read current source/result bindings; return private original text and public output |
| `save_review(key, expected_output_hash, decisions, status, notes, acknowledged_warnings) -> ReviewViewData` | Optimistic checksum check, atomic journal, approval policy |
| `export_approved(pair_id, doc_ids, destination) -> ExportSummary` | Bind a single pair; validate every approval and recheck bytes |

`ScannedFile` contains relative_path, size_bytes, mtime, source_hash_sha256 and
state (`new`, `current`, `stale`, `missing-output`, `conflict`, `missing-source`,
`recovery-pending`).
`ScanFailure` contains relative_path and safe code. `ReviewViewData` contains key,
source hash, revision, expected_output_hash, original_text, markdown, body,
detections, redactions, decisions, warnings, acknowledged_warnings, notes, and
status. Output offsets index body, so the UI never has to split YAML from
Markdown. `ExportSummary`
contains pair ID, exported doc IDs, failed doc IDs/safe codes, and audit_warning.
These UI projections use Zod; P1 creates them only when the corresponding method
is implemented, avoiding unused scaffolding.

## Acceptance coverage

| Spec check | Owning tasks |
| --- | --- |
| A01 | P1.2, P1.3, P5.2 |
| A02 | P1.4 |
| A03 | P2.2 |
| A04 | P2.3, P2.4 |
| A05 | P2.4, P3.5 |
| A06 | P3.1, P3.3, P3.4 |
| A07 | P3.3, P4.1 |
| A08 | P3.2, P3.4 |
| A09 | P2.1, P3.2 |
| A10 | P4.1, P4.2 |
| A11 | P4.3 |
| A12 | P2.1, P2.5, P3.2, P4.2, P5.2 |
| A13 | P3.4, P4.3 |
| A14 | P5.1, P5.2 |
| A15 | P1.1, P4.2, P5.2 |
| A16 | P1.3 |
| A17 | P2.4, P3.5, P4.3 |
| A18 | P1.3, P3.2, P4.2 |

## Verification and handoff

- [ ] Review all five plans and the shared contracts together before execution.
- [ ] Select Native or Subagent-driven execution; no method has been chosen yet.
- [ ] At execution start, use the worktree skill to establish isolation and read
  the chosen execution skill. Never restart from the prototype checkout.
- [ ] Execute red/green checks per task, keep commits small, and record actual
  command results. Examples below are planned tests, not reported passing tests.
- [ ] Run the release matrix on the real Windows artifact. Do not equate WSL or
  browser mocks with Windows acceptance evidence.

Recommendation: Subagent-driven execution with each implementer followed by a
reviewer. The file-commit protocol, private IPC, and review approval boundary
justify independent review; dependencies remain sequential. Native execution is
the lower-overhead alternative with one independent review after implementation.
