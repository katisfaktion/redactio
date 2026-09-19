# Reliable Batch Processing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Process a selected collection into durable, correctly classified outputs with bounded cancellation, isolated settings, and visible audit results.

**Architecture:** Rust reserves IDs and journals multi-file commits before writing results from the P2 sidecar. An RAII operation guard freezes the pair and prevents concurrent writers; Vue subscribes to pair/run-bound progress and reads final summaries.

**Tech Stack:** P1/P2 stack, Tokio child IO/timeouts, serde, SHA-256, OS file locks, existing atomic-storage helper.

**Spec:** [Approved release](../specs/2026-09-19-redactio-initial-release-design.md), sections 3.2/3.5, 5/6/8/9/11. Requires P1/P2 and [C1–C3](2026-09-19-00-release-overview.md#shared-contracts-to-implement).

## Global Constraints

- “Process one selected pair at a time; there is no queue or run-all-pairs action.”
- “Initialization has a 180-second deadline; each document request has a 120-second deadline.”
- “A document is current only when source hash, processing revision, successful mapping record, and actual generated-output checksum agree.”
- “Reserve a new ID durably before writing its output.”
- “An ID reservation is not a successful processing record.”
- “Stdout carries protocol messages only.”
- All [overview constraints](2026-09-19-00-release-overview.md#global-constraints) apply, particularly IPC limits and no raw content/path/error logging.

## Review Focus

1. A crash publishes Markdown but not its approval metadata: every intermediate state tested in P3.3.
2. The sidecar hangs in regex/model code: kill, reap, and invalidate all pending replies in P3.2.
3. Source bytes change after discovery: verify the analyzed snapshot and rehash before commit in P3.3/P3.4.
4. Cancelling during engine startup leaves the UI busy forever: guard release and completed summary test in P3.4.
5. Audit append is torn or concurrent: recover only an incomplete trailing record under a shared log lock in P3.4.

---

## Planned files

| Files | Responsibility |
| --- | --- |
| `apps/desktop/src-tauri/src/domain/mapping.rs` | Monotonic IDs, generation records, journal/recovery |
| `apps/desktop/src-tauri/src/{protocol.rs,sidecar.rs,resources.rs}` | Pydantic mirrors, process lifecycle, local binaries/models |
| `apps/desktop/src-tauri/src/domain/{sync.rs,audit.rs}` | Frozen run, cancellation, safe append log |
| `apps/desktop/src-tauri/tests/{mapping.rs,sidecar.rs,recovery.rs,sync.rs,audit.rs}` | Failure-injection and real protocol tests |
| `apps/desktop/src-tauri/tests/fake_sidecar.py` | Deterministic process failure injection, never packaged |
| `apps/desktop/src/{components/RunPanel.vue,components/DetectionSettings.vue,composables/useRun.ts,composables/useRun.test.ts}` | Visible progress and per-pair configuration |
| Existing settings.rs/scan.rs/storage.rs/commands.rs/ipc.ts/contracts.ts | Integrate domains through existing boundaries |

## P3.1: Stable identity and exact incremental classification

**Files:** Create mapping.rs/tests/mapping.rs; extend scan.rs and contracts.ts.

**Interfaces:** `Mapping::load(source: &Path, pair_id: Uuid, target: &Path)` validates the P1 header; `reserve(relative_path: &str) -> Result<String, AppError>` persists before returning. `classify(observed_source: Option<&str>, observed_output: Option<&str>, current_revision: &str, committed: Option<&Generation>, pending: bool) -> DocumentState` is pure. Existing pending entries are always recovery work, never current.

```rust
// Mapping state; derive strict serde models in mapping.rs.
pub struct Generation {
    pub source_hash: String,
    pub revision: String,
    pub output_hash: String,
    pub review_hash: String,
    pub first_processed_at: String,
    pub last_processed_at: String,
}
```

Each mapping entry has `doc_id`, `relative_path`, `reserved_at`,
and `committed: Option<Generation>`. P3.3 adds
`pending: Option<PendingCommit>` with a serde default of None; until then pass
false to classify's pending parameter. DocumentState variants are New, Current,
Stale, MissingOutput, Conflict, MissingSource, and RecoveryPending; serialize
them as C3's lowercase kebab-case strings.
The mapping header has monotonic `next_document_number`, not max(existing)+1.

- [ ] Write the checksum/revision regression:

```rust
use redactio_lib::domain::mapping::{classify, DocumentState, Generation};

#[test]
fn source_equality_does_not_hide_output_tampering_or_config_changes() {
    let generation = Generation {
        source_hash: "a".repeat(64), revision: "old".into(),
        output_hash: "b".repeat(64), review_hash: "c".repeat(64),
        first_processed_at: "2026-09-19T12:00:00Z".into(),
        last_processed_at: "2026-09-19T12:00:00Z".into(),
    };
    assert_eq!(classify(Some(&generation.source_hash), Some("tampered"), "old",
                       Some(&generation), false), DocumentState::Conflict);
    assert_eq!(classify(Some(&generation.source_hash), Some(&generation.output_hash),
                       "new", Some(&generation), false), DocumentState::Stale);
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test mapping`; expect failure before new classification.
- [ ] Implement precedence: malformed ownership/pending-recovery first, missing source, unowned/changed existing output conflict, missing output, no committed generation/new, source/revision mismatch/stale, otherwise current. Review JSON checksum/identity must also match before a current document is shown as approved. Never repair tampering automatically.
- [ ] Allocate an ID once per relative path, advance the persisted counter, and leave reservations after failures. Validate doc IDs against `^doc-[0-9]{4,}$` and u64 range; refuse exhausted counters. Deleted source entries stay reserved. Both pairs may independently use doc-0001.
- [ ] Add disk round-trip checks for failed reservation save, retry reuses the reservation, counter >9999, deleted source retention, foreign pair/target binding, missing/corrupt mapping with existing outputs, and fresh empty-target recovery. Recovery from a lost mapping does not silently adopt any old output; starting fresh requires an explicit user action preserving the old target and resetting private metadata into a retained backup under source.
- [ ] Run mapping/scan tests and format check; expect no false skip and no ID reuse.
- [ ] Commit: `rtk git add apps/desktop`; `rtk git commit -m 'feat: track stable document generations and stale outputs'`.

## P3.2: Bounded, restartable sidecar client

**Files:** Create sidecar.rs/protocol.rs/resources.rs, tests/sidecar.rs/fake_sidecar.py; extend lib.rs and safe errors. Use the prototype's child transport structure, replacing raw stderr forwarding and unbounded line reads.

**Interfaces:** `Sidecar::new(executable: PathBuf, args: Vec<OsString>, model_root: PathBuf)`, `request<T: Serialize, R: DeserializeOwned>(&self, kind: &str, payload: &T, timeout: Duration) -> Result<R, AppError>`, `shutdown(&self)`. `resources::resolve() -> ResourcePaths { sidecar_executable, sidecar_args, model_root }` selects explicit development or packaged resources. P2's strict messages are mirrored in protocol.rs; response ID/type/pair/revision are validated, not just deserialized.

- [ ] Add a fake child script that emits only JSON: modes `echo`, `exit-once`, `hang`, `oversized`, `wrong-id`, `wrong-pair`, `stderr-canary`. Pass its mode and optional marker file as args, never source data. Add a real process timeout check:

```rust
use redactio_lib::sidecar::Sidecar;
use std::{ffi::OsString, path::PathBuf, time::{Duration, Instant}};

#[tokio::test]
async fn hung_child_is_reaped_before_reuse() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake_sidecar.py");
    let python = std::env::var_os("REDACTIO_TEST_PYTHON")
        .expect("Set REDACTIO_TEST_PYTHON to the test interpreter's absolute path");
    let child = Sidecar::new(PathBuf::from(python),
        vec![script.into_os_string(), OsString::from("hang")], PathBuf::from("."));
    let started = Instant::now();
    let result: Result<serde_json::Value, _> = child.request(
        "ping", &serde_json::json!({}), Duration::from_millis(100)).await;
    assert_eq!(result.unwrap_err().code, "engine_timeout");
    assert!(started.elapsed() < Duration::from_secs(5));
    child.shutdown().await;
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test sidecar`; expect missing client failure. Document test Python executable selection explicitly for Windows/POSIX.
- [ ] Implement one serialized in-flight request, capped 64 MiB reads, strict envelope checks, random correlation IDs, bounded stderr draining without persistence, process-exit detection, and `kill_on_drop`. Restart/reconfigure once after unexpected exit, then retry the original read/analyze request with a fresh ID. Malformed protocol/second exit fails the operation; timeout kills/reaps without retrying automatically. Ignore unknown/late IDs without letting them reset the deadline; fail malformed replies for the current ID. Initialize/configure has 180 seconds, document/preview/render 120 seconds; tests inject short timeouts.
- [ ] Remember the complete last successfully configured pair/revision/config in private host memory; after restart send configure before retrying process/render. Do not retry just the document against an unconfigured engine. Reject stale/unknown replies and clear pending state on every exit.
- [ ] Add tests for two exits, full reconfiguration on retry, incorrect pair/revision/ID, over-limit frame, arbitrary library stderr, graceful EOF then forced shutdown, and cancellation during startup. Run the P2 contract emitter and deserialize its real replies in Rust; serialize Rust requests back through the real Python validator.
- [ ] Run sidecar tests and the actual offline model protocol round-trip. Expected: no orphan process, no false result routing, no raw secret logs. After this task, execute P5.1's early package smoke.
- [ ] Commit: `rtk git add apps/desktop/src-tauri`; `rtk git commit -m 'feat: manage bounded offline processing child'`.

## P3.3: Recoverable output and private review commits

**Files:** Extend mapping.rs/storage.rs; add mapping.rs's test-only recovery_tests module and tests/recovery.rs for public disk-recovery behavior. Define the private ReviewRecord here because batch generation already needs to persist automatic detections for P4.

**Interfaces:** `ReviewRecord` contains schema_version, key, source_hash, revision, detections, decisions, status, notes, acknowledged_warnings, warnings, redacted_at, reviewed_at, engine, and output_hash; no original text. `CommitCandidate { key, source_hash, revision, markdown: Vec<u8>, review: ReviewRecord }`. `commit_generation(pair: &SyncPair, mapping: &mut Mapping, candidate: CommitCandidate, expected_output_hash: Option<&str>) -> Result<Generation, AppError>`; `recover_pending(pair, mapping) -> Result<(), AppError>`. `PendingCommit` contains intended Generation, prior output/review hashes, private candidate review metadata, and UUID temporary basenames; never arbitrary paths.

- [ ] Add test-only `CommitStage::{Reserved,Journaled,OutputStaged,ReviewStaged,OutputReplaced,ReviewReplaced}` interruption injection inside mapping.rs's `#[cfg(test)] mod recovery_tests`; integration tests cannot access library items gated by cfg(test). Define `exercise_commit_crash(stage) -> Result<(), AppError>` there: create source/target/config temp folders, create a pair, reserve doc-0001, generate one minimal candidate with empty detection/decision lists and pending status, fail at stage, reload the on-disk mapping, recover/retry, and assert exactly one ID and matching output/review hashes. Its full input is synthetic; it must use the actual IO code, not a duplicate state machine. Public recovery tests in tests/recovery.rs use only production APIs and on-disk records.

```rust
#[test]
fn every_commit_boundary_is_recoverable_without_id_reuse() {
    for stage in [CommitStage::Reserved, CommitStage::Journaled,
                  CommitStage::OutputStaged, CommitStage::ReviewStaged,
                  CommitStage::OutputReplaced, CommitStage::ReviewReplaced] {
        exercise_commit_crash(stage).unwrap();
    }
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --lib recovery_tests`; expect missing commit protocol failure. Also run `--test recovery` after adding the public disk-recovery cases.
- [ ] Implement this exact durable order, using P1's atomic helper:

```text
1. Persist reservation and counter if new; never erase it on error.
2. Hash candidate Markdown and final serialized review metadata.
3. Persist PendingCommit with expected prior hashes and temporary basenames.
4. Stage and flush candidate Markdown in target and review JSON under source.
5. Recheck source hash, current config revision, ownership, prior hashes, and roots.
6. Atomically replace output, then private review JSON.
7. Persist committed Generation and clear pending; only now report success.
```

ReviewRecord.output_hash hashes final Markdown, including review status/time;
Generation.review_hash hashes the serialized ReviewRecord. There is no recursive
hash field inside Markdown or ReviewRecord. Preserve first_processed_at on later
generations. A review-only save uses this same protocol, not an ad-hoc write.

- [ ] Implement restart reconciliation under the pair lock: if both installed hashes match candidate, finalize; if one matches and the remaining recorded temporary file matches, finish replacing only after ownership checks; if candidate bytes are missing, retain ID and mark for retry. Unexpected installed bytes are conflicts, never overwrite. Effective approval additionally requires current source/revision; completing an interrupted old generation cannot make it exportable after those changed.
- [ ] Add real failure cases: destination locked, disk write failure, source changes after analysis, config revision mismatch, alien file takes reserved name, old output edited during a review, and crash during approved review save. Ensure cleanup touches only recorded temporary basenames inside validated roots. Missing prior output is not permission to overwrite a new file at that name.
- [ ] Run recovery/mapping tests on Windows and Linux. Expected: every failure preserves old data or recoverable pending state; no pending record is classified current or exportable.
- [ ] Commit: `rtk git add apps/desktop/src-tauri`; `rtk git commit -m 'feat: journal document and review commits for recovery'`.

## P3.4: Frozen runs, progress, cancellation, and audit

**Files:** Create sync.rs/audit.rs, tests/sync.rs/audit.rs, RunPanel.vue/useRun.ts/useRun.test.ts; extend AppState, commands.rs, ipc.ts, contracts.ts.

**Interfaces:** C3 start/cancel commands. `RunProgress {sync_pair_id,run_id,stage,discovered,processed,skipped,failed,unprocessed,warned}`; stage is initializing/scanning/processing/finished. `RunSummary` adds outcome, per-file safe errors, and audit_warning. `append_audit(config_dir, entry) -> Result<(), AppError>`. `OperationGuard` owns the app mutex and source/target file locks; locks live in private source metadata and app config, never exported.

`RunCounts` holds the six u64 count fields above. `completed()` returns
processed + skipped + failed; `is_consistent()` checks completed + unprocessed
equals discovered and warned <= processed. Use checked arithmetic at boundaries.

- [ ] Write the progress conservation test and cancellation integration case:

```rust
use redactio_lib::domain::sync::RunCounts;

#[test]
fn skipped_and_warned_do_not_inflate_the_denominator() {
    let counts = RunCounts { discovered: 4, processed: 1, skipped: 1,
        failed: 1, unprocessed: 1, warned: 1 };
    assert!(counts.is_consistent());
    assert_eq!(counts.completed(), 3);
}
```

In tests/sync.rs also run two generated documents using fake_sidecar modes:
cancel after the first committed result; assert second stays unprocessed, the
guard releases, first generation remains valid, and a new run can start.

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test sync`; expect no orchestrator implementation yet.
- [ ] Implement the orchestrator with a frozen pair/root/config snapshot, revalidation and pair lock, pending recovery, scan, classification, and per-file pipeline. Confirm force_doc_ids for reviewed/manual corrections before discarding them; retry selection cannot silently imply force. Resolve relative input paths through known scan entries, never arbitrary traversal strings. Rehash before commit. Invalid individual documents continue; systemic sidecar/protocol/storage failure ends the run safely.
- [ ] Start summaries before engine initialization so early failure/cancel is audited. On cancel stop scheduling files; bound a hung current request through the sidecar timeout/kill. Emit an authoritative final summary and expose it via `get_run_summary(pair_id, run_id) -> RunSummary | null` so a missed event cannot strand the UI. All commands/updates check pair and run identity.
- [ ] Implement audit append under an interprocess file lock. Serialize only whitelisted fields; trim an incomplete trailing line to the last validated complete boundary before appending a recovery record. A malformed complete earlier line is an explicit audit error, not data to silently erase. Flush and sync each record. Audit failures set visible audit_warning without deleting outputs. Settings displays the host-resolved audit location and an open-folder action; the host opens only that known directory through the native platform, never a path or shell command supplied by document data.
- [ ] Add actual truncated/concurrent-log tests, startup failure audit, no eligible documents, rejected duplicate run, filesystem error rollback, case-sensitive canary leakage scan, and UI listener cleanup. Vue subscribes before start_sync, filters pair/run IDs, and disposes listeners on unmount; disable selection/settings during work. Show accessible status text alongside progress.
- [ ] Run sync/audit tests, `rtk proxy pnpm test`, typecheck, build; then two-pair synthetic desktop runs with success, cancellation, and a failed document. Counts must satisfy the spec denominator and contain no other pair's results.
- [ ] Commit: `rtk git add apps/desktop`; `rtk git commit -m 'feat: run cancellable audited document batches'`.

## P3.5: Detection settings and controlled reprocessing

**Files:** Create DetectionSettings.vue and DetectionSettings.test.ts; extend settings.rs, commands.rs, protocol.rs, ipc.ts, contracts.ts, and tests/pairs.rs.

**Interfaces:** C3 `save_processing_config`, plus `preview_rules(pair_id, config, text) -> Detection[]` that validates/configures a temporary snapshot under the operation guard without persisting settings. Restore the saved snapshot after preview or restart the sidecar if preview times out. `list_models() -> ModelInfo[]` reads the bundled/dev resource manifest and verifies actual packages during configure.

After successful engine validation, the command calls
`Settings::apply_validated_config(id: Uuid, config: ProcessingConfig, fingerprint: ProcessingFingerprint) -> Result<bool, AppError>`
and atomically saves settings; the bool reports whether the revision changed.
ProcessingFingerprint is the C1 struct with four String fields and derives
Clone/PartialEq/Eq. This method performs no sidecar IO.

- [ ] Write a settings test creating two C1 pairs, then changing only A's enabled entities: B's config/revision remain byte-equal and A's revision rotates once. Saving the identical config twice does not rotate again. Invalid regex or unavailable model leaves settings and both revisions unchanged.

```rust
use redactio_lib::domain::settings::{ProcessingConfig, ProcessingFingerprint, Settings};

#[test]
fn validated_config_is_local_to_one_pair_and_idempotent() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a-in", "a-out", "b-in", "b-out"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    let mut settings = Settings::default();
    let a = settings.add("A", &root.path().join("a-in"), &root.path().join("a-out")).unwrap();
    settings.add("B", &root.path().join("b-in"), &root.path().join("b-out")).unwrap();
    let unchanged = settings.sync_pairs[1].clone();
    let previous_revision = settings.sync_pairs[0].processing_revision;
    let mut config = ProcessingConfig::default();
    config.enabled_entities.clear();
    let fingerprint = ProcessingFingerprint {
        engine_version: "test-1".into(), extraction_version: "1".into(),
        model_name: "de_core_news_lg".into(), model_version: "test-1".into(),
    };
    assert!(settings.apply_validated_config(a, config.clone(), fingerprint.clone()).unwrap());
    assert_ne!(settings.sync_pairs[0].processing_revision, previous_revision);
    assert_eq!(settings.sync_pairs[1], unchanged);
    assert!(!settings.apply_validated_config(a, config, fingerprint).unwrap());
}
```

Derive Clone/PartialEq/Eq on Settings' pair/config types. Exercise validation
failure through the command plus fake sidecar, asserting the saved settings
bytes do not change; the pure method is never called for a failed configure.
- [ ] Run pairs tests and the new Vue test before implementation; expect configuration behavior missing.
- [ ] Implement a value comparison of normalized ProcessingConfig before assigning a fresh revision. Configure/validate through the bounded engine before saving; failure never writes unvalidated settings. On an engine/model/extraction fingerprint change, rotate each affected pair once on next load. Preserve unrelated settings under a settings-file lock.

```rust
if proposed != pair.config || resolved_fingerprint != saved_fingerprint {
    pair.config = proposed;
    pair.processing_revision = uuid::Uuid::new_v4();
}
```

`resolved_fingerprint` and `saved_fingerprint` are private structured tuples of
engine/extraction/model versions; custom rules are compared as local config,
never emitted as hashes. Persist the resolved fingerprint with each pair.

- [ ] Build labeled Onyx controls for model, enabled entities, custom regex/words, include_positions, preview, and save. Explain unavailable models in German. A preview can display input/detections privately but must not log them. Show per-document stale/conflict states and an explicit reprocess dialog listing affected reviewed/manual-corrected documents.
- [ ] Add real two-pair rule-switch tests through the desktop client, a catastrophic-regex preview timeout, pair switching during preview, fingerprint upgrade, and no-op config save. Assert cancelled/failed previews do not alter committed settings.
- [ ] Run relevant Rust/Python tests, Vue interaction tests, typecheck, and build. Expected: original-same/config-changed documents become stale, only the edited pair is affected, and reviewed work is preserved until explicit reprocessing.
- [ ] Commit: `rtk git add apps/desktop`; `rtk git commit -m 'feat: configure detection independently for each pair'`.

## Completion evidence

- [ ] A05–A09/A13/A17/A18 processing paths have focused passing checks.
- [ ] A synthetic two-pair desktop workflow creates valid Markdown and private review records without any review approval being implied.
- [ ] Record the early P5.1 package smoke result before expanding the review UI.
