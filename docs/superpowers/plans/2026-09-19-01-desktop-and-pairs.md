# Desktop and Sync Pairs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Launch a German Vue/Onyx desktop app that manages multiple isolated folder pairs and inspects their sources.

**Architecture:** Thin Vue components call typed Rust commands. Rust owns persisted pair identity, validated roots, and scanning; no document processing is exposed until P3 is complete.

**Tech Stack:** Tauri 2, Vue 3, TypeScript, Vite, sit-onyx, Zod, serde, uuid, time, sha2, walkdir; native Rust tests and focused Vue interaction tests.

**Spec:** [Approved release](../specs/2026-09-19-redactio-initial-release-design.md), sections 1–3, 5–7, 9, 11. Read [shared contracts C1–C3](2026-09-19-00-release-overview.md#shared-contracts-to-implement) first.

## Global Constraints

- “Source documents are never edited.”
- “Windows 11 x64 is the initial supported release target.”
- “Process one selected pair at a time; there is no queue or run-all-pairs action.”
- “Use versioned, validated formats with timezone-aware RFC-3339 timestamps.”
- “No runtime network communication, telemetry, update checks, external content, remote fonts, or model downloads; dependency setup at build time is separate.”
- “The full document identity is `(sync_pair_id, doc_id)`; two pairs may both contain `doc-0001` without sharing data.”
- All [overview constraints](2026-09-19-00-release-overview.md#global-constraints) apply. Commands below run at repository root unless a working directory is explicitly given. Existing files are documentation only.

## Review Focus

1. Junction/case/short-name aliases bypass folder separation: Windows path tests in P1.2 and P5.2.
2. Removing then re-adding a source resets identity: metadata round-trip in P1.3.
3. Duplicate names differing only by Unicode case: normalized-name test in P1.3.
4. A scanner stops at one inaccessible child: per-file error test in P1.4.
5. Late events populate the newly selected pair: explicit pair routing test in P1.3, extended in P3.2.

---

## Planned files

| Files | Responsibility |
| --- | --- |
| Root `package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml` | Commands and workspace |
| `apps/desktop/{package.json,index.html,vite.config.ts,tsconfig.json}` | Vue app/build/typecheck |
| `apps/desktop/src/{App.vue,main.ts,styles.css}` | Onyx bootstrap and empty/setup state |
| `apps/desktop/src-tauri/{Cargo.toml,Cargo.lock,build.rs,tauri.conf.json,capabilities/default.json}` | Minimal desktop host |
| `apps/desktop/src-tauri/src/{lib.rs,main.rs,error.rs,commands.rs,domain/mod.rs}` | Host boot and typed command boundary |
| `apps/desktop/src-tauri/src/domain/{paths.rs,storage.rs,settings.rs,scan.rs}` | Root validation, durability, state, discovery |
| `apps/desktop/src/{components/PairManager.vue,components/DocumentList.vue,composables/usePairs.ts,lib/ipc.ts,lib/contracts.ts}` | Pair/list UI and IPC validation |
| `apps/desktop/src/App.test.ts`, `apps/desktop/src/composables/usePairs.test.ts` | Observable UI behavior |
| `apps/desktop/src-tauri/tests/{paths.rs,pairs.rs,scan.rs}` | Generated filesystem checks |
| `docs/development.md`, `.gitignore` | Setup and generated-output exclusions |

## P1.1: Runnable Vue/Onyx desktop shell

**Files:** Create root workspace files, desktop build files, App.vue/main.ts/styles.css, Tauri host files, App.test.ts, docs/development.md; extend .gitignore for generated Tauri schema/build files.

**Interfaces:** Produces `App.vue` with `initialSettings: Settings` prop and an empty-pair state. C1's `SettingsSchema` is defined in `lib/contracts.ts`; `ipc.ts` will receive actual commands in P1.3. App initially receives an empty Settings object from main.ts and does not present working processing buttons.

- [ ] Scaffold with `rtk proxy pnpm create vite@latest apps/desktop --template vue-ts`. Rename the package to `@redactio/desktop`, add Tauri 2/API/dialog, sit-onyx, and Zod with exact resolved versions; add vue-tsc, Vitest, Vue Test Utils, and jsdom for the specified interaction checks. Commit the resolved pnpm lockfile. Use pnpm only; remove generated duplicate npm locks.
- [ ] Define workspace and scripts, then write this first observable test before the empty-state UI:

```json
{"name":"redactio","private":true,"scripts":{"dev":"pnpm --filter @redactio/desktop dev","tauri:dev":"pnpm --filter @redactio/desktop tauri dev","typecheck":"pnpm --filter @redactio/desktop typecheck","test":"pnpm --filter @redactio/desktop test","build":"pnpm --filter @redactio/desktop build"}}
```

Desktop scripts: `dev: vite`, `tauri: tauri`, `typecheck: vue-tsc --noEmit`,
`test: vitest run`, `build: vue-tsc --noEmit && vite build`. Vite tests use jsdom.

```ts
import { mount } from "@vue/test-utils";
import { expect, test } from "vitest";
import App from "./App.vue";

test("an empty registry offers setup without a pretend sync action", () => {
  const wrapper = mount(App, { props: {
    initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
  } });
  expect(wrapper.text()).toContain("Ordnerpaar hinzufügen");
  expect(wrapper.find('[data-testid="start-sync"]').exists()).toBe(false);
});
```

- [ ] Run `rtk proxy pnpm --filter @redactio/desktop test src/App.test.ts`; expect a failing assertion against the generated starter.
- [ ] Implement the German setup state with OnyxAppLayout/OnyxPageLayout, a labeled OnyxButton, and Vue props. Bootstrap Onyx with local CSS, local fonts, and German locale according to its pinned package documentation:

```ts
import { createApp, ref } from "vue";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import "@fontsource-variable/source-sans-3";
import "@fontsource-variable/source-code-pro";
import "sit-onyx/style.css";
import "sit-onyx/global.css";
import "./styles.css";
import App from "./App.vue";

const app = createApp(App, {
  initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
});
app.use(createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } }));
app.mount("#app");
```

Install both imported font packages with exact versions. This uses Onyx's
[locale configuration](https://onyx.schwarz/development/i18n.html) and
[local font packages](https://onyx.schwarz/development/typography.html), without
adding a translation framework for one language. Use scoped CSS for layout.

- [ ] Initialize `apps/desktop/src-tauri` with app name Redactio, identifier `com.redactio.app`, Vite port 1420, frontendDist `../dist`, Rust library `redactio_lib`. Limit native dialog capabilities to the main window. Deny remote content in production CSP and omit shell/network/updater plugins.
- [ ] Run the focused test, `rtk proxy pnpm typecheck`, `rtk proxy pnpm build`, and `rtk proxy pnpm tauri:dev`. Expect passing checks and a keyboard-operable setup window. Document Linux/WSL prerequisites separately from Windows build prerequisites.
- [ ] Commit: `rtk git add package.json pnpm-workspace.yaml pnpm-lock.yaml apps/desktop docs/development.md .gitignore`; `rtk git commit -m 'feat: add Vue and Onyx desktop shell'`.

## P1.2: Validated roots and atomic private storage

**Files:** Create domain/paths.rs, domain/storage.rs, error.rs, tests/paths.rs; update domain/mod.rs and Cargo.toml. Use serde/serde_json, uuid, time, sha2, walkdir, tempfile already familiar from the prototype; add a focused Windows API binding only for handle/path and atomic replacement behavior unavailable in stdlib.

**Interfaces:** `validate_roots(source: &Path, target: &Path, other_roots: &[PathBuf]) -> Result<(PathBuf, PathBuf), AppError>`; `validate_write(root: &Path, path: &Path) -> Result<PathBuf, AppError>`; `write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError>`. AppError carries `code: String, retryable: bool`; public errors never format input paths. Write helpers require a validated path and revalidate directory identity immediately before replacement.

- [ ] Write a real filesystem regression before the helpers:

```rust
use redactio_lib::domain::paths::validate_roots;
use std::fs;

#[test]
fn another_pairs_source_cannot_become_a_target() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir(&a).unwrap(); fs::create_dir(&b).unwrap();
    let error = validate_roots(&a, &b, &[b.canonicalize().unwrap()]).unwrap_err();
    assert_eq!(error.code, "folder_overlap");
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test paths`; expect missing helper failure, then implement helpers.
- [ ] Normalize canonical roots with path-component comparisons, never textual starts-with. On Windows compare handle-resolved paths without case-sensitive alias bypass; reject reparse-point descendants and non-directory roots. Validate all six across-pair combinations and app-config overlap. New targets require a native user action, a checked existing parent, exclusive creation, and validation of the created directory.
- [ ] Implement storage using exclusive random same-directory temporary files, write_all, sync_all, and OS replacement semantics. The intended IO order is:

```text
validate parent/object identity -> create_new temporary -> write_all -> sync_all
-> revalidate parent/object identity -> atomic replace -> sync parent where supported
```

For Windows use ReplaceFileW for an existing regular destination and a checked
rename/move for an absent one. Treat sharing violations as safe retryable errors;
never delete the old destination first. Remove only the temporary file created
by this operation on failure. Do not reuse the prototype's fixed `.tmp` filename.

- [ ] Add table-driven cases to tests/paths.rs: equal/nested roots both ways, siblings with shared name prefixes, case/short-path aliases, path traversal, a temporary-file symlink, and a junction swapped after validation. Add `write_atomic_keeps_old_bytes_on_replace_failure` by holding the Windows destination without delete-sharing. Linux checks cannot claim these Windows cases passed.
- [ ] Run the path test on Linux and Windows plus `rtk cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check`. Expected: original bytes preserved on injected failures and every unsafe route rejected.
- [ ] Commit: `rtk git add apps/desktop/src-tauri`; `rtk git commit -m 'feat: validate pair roots and write private state atomically'`.

## P1.3: Persistent pair registry and pair selector

**Files:** Create settings.rs, tests/pairs.rs, PairManager.vue, usePairs.ts/usePairs.test.ts, ipc.ts; extend contracts.ts, commands.rs, App.vue, main.ts.

**Interfaces:** Implement C1's Settings/SyncPair/ProcessingConfig structs and strict Zod projections. `Settings::default()`, `Settings::add(name, source, target) -> Result<Uuid, AppError>`, `rename(id, name)`, `select(id)`, `remove(id)` mutate validated state; `load_settings(path)` and `save_settings(path, settings)` own atomic persistence. C3's commands call these under the settings lock. A source mapping's initial header is `{schema_version:1,sync_pair_id,target_folder,next_document_number:1,entries:[]}`; P3.1 owns future entries. P1.3 writes this header before acknowledging a new pair.

- [ ] Write a focused round-trip and removal test using real temporary folders:

```rust
use redactio_lib::domain::settings::{load_settings, save_settings, Settings};
use std::fs;

#[test]
fn remove_keeps_collection_data_and_readd_keeps_identity() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap(); fs::create_dir(&target).unwrap();
    let mut settings = Settings::default();
    let id = settings.add("Buch", &source, &target).unwrap();
    settings.select(id).unwrap();
    let path = root.path().join("settings.json");
    save_settings(&path, &settings).unwrap();
    let mut loaded = load_settings(&path).unwrap();
    assert_eq!(loaded.selected_sync_pair_id, Some(id));
    loaded.remove(id).unwrap();
    assert!(source.join("_document-mapping.json").exists());
    assert_eq!(loaded.add("Buch erneut", &source, &target).unwrap(), id);
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test pairs`; expect failure before the registry exists.
- [ ] Implement the registry in one settings file. Validate UUID uniqueness, non-empty names after trim and Unicode `to_lowercase()` comparison, root separation, target binding, and selected ID referential integrity. Re-add adopts the private mapping ID; settings missing after a failed add can retry adopting that same header. Refuse unknown-schema/corrupt files; never overwrite them with defaults. Removal preserves files and selects the next list entry, or the previous last entry if the last was removed, or null when empty. Re-add uses defaults and a fresh processing revision, making retained results stale. A non-empty target requires a warning and ownership checks; acknowledgement never authorizes replacing unknown files.
- [ ] Connect native folder selection to PairManager and strict `invoke` wrappers. Load real settings before rendering App. Require explicit removal confirmation. Keep paths and pair names out of log/error strings. For asynchronous updates use this binding rule in usePairs.ts:

```ts
export function isSelectedPair(selectedId: string | null, eventPairId: string): boolean {
  return selectedId !== null && selectedId === eventPairId;
}
```

- [ ] Add a Vue test that selects pair B, receives a pair A update, and asserts B's document view is unchanged. In Rust add duplicate `BÜCHER`/`bücher`, empty names, invalid selected ID, missing root, wrong re-add target, duplicate mapping UUID, re-add revision invalidation, non-empty target ownership, and rename-does-not-change-revision cases. Block registry writes during active operations once P3 introduces the operation guard.
- [ ] Run pairs tests, `rtk proxy pnpm test`, and `rtk proxy pnpm typecheck`; expect save/reload identity and UI selection checks to pass.
- [ ] Commit: `rtk git add apps/desktop`; `rtk git commit -m 'feat: manage persistent independent sync pairs'`.

## P1.4: Source discovery and document list

**Files:** Create scan.rs, tests/scan.rs, DocumentList.vue; extend commands.rs, ipc.ts, contracts.ts, App.vue.

**Interfaces:** `scan_source(source: &Path) -> Result<ScanReport, AppError>` returns C3 fields; initial states are new or missing-source. P3.1 adds mapping/output-based classification without changing the scan projection. Add `scan_pair(pair_id)` and display only the selected pair's results.

- [ ] Write the discovery test:

```rust
use redactio_lib::domain::scan::scan_source;
use std::fs;

#[test]
fn scan_filters_metadata_lockfiles_and_hidden_children() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    for name in ["a.DOCX", "nested/b.docx", "~$locked.docx", ".hidden.docx", "note.txt"] {
        fs::write(root.path().join(name), b"synthetic").unwrap();
    }
    let report = scan_source(root.path()).unwrap();
    let names: Vec<_> = report.files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(names, vec!["a.DOCX", "nested/b.docx"]);
}
```

- [ ] Run `rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --test scan`; expect missing scanner failure.
- [ ] Implement stable recursive discovery with WalkDir configured not to follow links, Windows hidden/reparse attribute checks, source-relative normalized paths, streamed SHA-256, byte size, and RFC-3339 mtime. Use per-entry Result handling so an inaccessible child becomes ScanFailure while unreadable root fails the call. File-type classification uses extension only; P2 validates actual DOCX structure.
- [ ] Expose the result through the IPC wrapper and an Onyx table with name/status/mtime plus per-file errors. Use an explicit scan action and post-operation refresh, not a watcher. Read/hash work runs off the UI thread. Show no eligible documents distinctly from scan failure.
- [ ] Add source rename/delete behavior, dot-directory/metadata exclusion, Windows hidden files, an inaccessible child, and symlink/junction traversal cases. Use an unprivileged Windows test process for permission checks; skip with an explicit unsupported-platform reason elsewhere.
- [ ] Run scan tests, `rtk proxy pnpm test`, typecheck, and build. In the actual window, configure two synthetic pairs and verify selection/scanning never merges their file lists.
- [ ] Commit: `rtk git add apps/desktop`; `rtk git commit -m 'feat: inspect selected sync pair documents'`.

## Completion evidence

- [ ] P1's Rust and UI tests pass, the desktop app launches, and README/development.md contain tested setup commands.
- [ ] A01/A02/A16 routing-related A18 are covered; Windows-specific path results are explicitly recorded rather than inferred from WSL.
- [ ] No PII corpus, prototype process files, React dependencies, or pretend processing behavior was introduced.
