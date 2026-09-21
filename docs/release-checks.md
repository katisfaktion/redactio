# Windows release acceptance

Status: **incomplete**. Stage 1 checks are synthetic development/native-engine evidence.
P3.4–P4 integration and the final rebuilt ZIP must pass the real-window matrix below
before release. CI on Windows Server is not Windows 11 acceptance. A passing
package check is not review/export, clean-machine, accessibility, or quality acceptance.

## Reproduce the synthetic checks

Use a fresh dedicated output directory; the generator refuses every nonempty directory,
including prior generated corpora. It never deletes or replaces user documents. ZIP
member dates and document metadata are fixed, so equal inputs generate equal bytes.

```sh
uv --directory apps/sidecar sync --locked
apps/sidecar/.venv/bin/python scripts/generate-corpus.py --output local-data/synthetic --count 400 --edge-output local-data/synthetic-edges
apps/sidecar/.venv/bin/python scripts/test_packaging.py
```

On Windows use `.venv/Scripts/python.exe` for build-time generation. Ship the resulting
synthetic directories to the test environment; the checker needs PowerShell 7, but the
application itself must need no separately installed runtime or development tool.

```powershell
./scripts/test-check-package.ps1
./scripts/test-build-manifest.ps1
./scripts/check-package.ps1 -PackageRoot 'C:\Users\Public\Redactio Prüfung' -CorpusRoot 'C:\TestInputs\synthetic'
./scripts/check-package.ps1 -PackageRoot 'C:\Users\Public\Redactio Prüfung' -CorpusRoot 'C:\TestInputs\synthetic-edges'
```

`expectations.json` schema 2 contains count, relative synthetic filenames, profile,
SHA-256, and expected safe warning/error/state codes. It contains no extracted text,
recognizer values, original filenames, or private-document measurements. The six edge
profiles are header warnings, corrupt ZIP, empty body, repeated names, Unicode, and
output tampering. Package smoke uses explicit synthetic custom rules; it proves local
model loading, processing and replay without treating a particular NER result as a
quality claim.
The baseline lg 3.8.0 model missed the name at code-point offset 310 in this edge
(occurrences at 63 and 326 were detected). This observed NER limitation remains open
for human quality acceptance; independent baseline PERSON canaries are unchanged.
For `tamper`, process the source in the real host first, append a
synthetic marker to that generated Markdown, then rescan: expected state is `conflict`,
with the externally edited output preserved. The engine-only check does not perform or
claim that host action. No generated document or corpus is committed or cached in CI.

The 400 documents intentionally repeat known synthetic patterns with different case
numbers. This is a deterministic load/smoke corpus, not representative clinical-quality
or 30-minute throughput acceptance. P5.3 requires a user-local representative corpus,
independent inspection of at least 10% plus every warning/failure, and separate metrics.

## Acceptance matrix

Every row requires recorded outcomes and evidence against the final artifact. Status
`partial` means automated evidence exists but the final real-window flow remains pending;
`unverified` means the required end-to-end evidence has not been obtained.

| ID | Procedure and explicit expected outcome | Current final-artifact status |
| --- | --- | --- |
| A01 Setup | Create two pairs in spaced German paths; test same/nested/case/junction roots and a nonempty target. Valid pairs persist; every overlap/redirect is rejected and unrelated target bytes remain intact. | partial |
| A02 Discovery | Scan nested `.docx`/`.DOCX`, hidden/lock/metadata files, junctions, and an ACL-unreadable peer. Stable ordered DOCX records appear; exclusions are absent and unreadable peers receive safe errors. | partial |
| A03 Extraction | Run paragraphs/tables, separate warning/corrupt/empty cases and bounded size/expansion tests. Reading order is preserved; unsupported surfaces warn, corrupt input errors, empty bodies cannot approve, and limits stop work safely. | partial |
| A04 Redaction | Inspect canary, repeated-name, overlap and Unicode outputs. All active synthetic identifiers are removed; repeated values reuse placeholders; zero-based code-point offsets delimit exactly the output placeholders. | partial |
| A05 Configuration | Toggle entities, add invalid/valid regex and words, switch prefetched models and simulate engine/extraction revisions. Only affected pair results become stale; invalid/unavailable configuration does not save; rename/select leaves revisions stable. | partial |
| A06 Incremental | Run twice, edit a source, remove output, tamper with output, and force a reviewed document. Unchanged work skips; changed/missing work regenerates; external edits/reviews require explicit resolution and no silent replacement. | partial |
| A07 Recovery | Inject every journal boundary, native locked-file failure and restart. Incomplete work stays pending, original bytes survive failed replacement, retry completes coherently and IDs are never reused. | partial |
| A08 Cancellation | Cancel initialization and a document, double-click start, switch pair while busy and start another instance. No partial success or cross-pair changes appear; only one writer runs and cancellation visibly settles. | partial |
| A09 Sidecar | Run real serialized requests plus malformed input, EOF/crash, timeout, late reply and shutdown tests. Errors remain bounded/safe, restart is limited, model identity matches and retries never duplicate commits. | partial |
| A10 Review | Keyboard-add/dismiss/change entity types, save/reopen and change source/config. Body, offsets and counts agree; bound decisions persist; changed inputs invalidate decisions; approval binds exact final bytes. | partial |
| A11 Export | Export current approvals to an empty directory outside all pairs, then try mixed ownership/stale/tampered output. Only selected current approved Markdown is copied; invalid selections fail and originals, notes, mappings and identifying rules are absent. | partial |
| A12 Privacy | Use canaries in paths/rules/invalid data/errors and malicious Markdown links/HTML. Logs/frontmatter contain no canaries; preview cannot execute scripts or request remote assets. Observe app and all children during cold startup and the full flow: zero external attempts. | unverified |
| A13 Audit | Complete/fail/cancel attempts, concurrent append/recovery and unwritable audit directory. Records retain correct opaque pair IDs, prior entries remain valid, no pair names leak, and audit failure is visibly announced. | partial |
| A14 Windows artifact | Extract exact ZIP to `C:\Users\Public\Redactio Prüfung` in a disposable Windows 11 x64 standard-user environment without dev tools/network. Launch/configure/scan/process/review/export/restart/exit succeed; app-local fixed WebView identity is observed; all Onyx assets/German text render without remote attempts. | unverified |
| A15 Usability | Complete the main flow with Tab/Shift+Tab/Enter/Space/Escape and a screen reader. All controls have names, visible ordered focus and focus restoration; progress/errors announce, warning status is not color-only, contrast passes and large views/ordinary controls respond within 200 ms. | unverified |
| A16 Pair lifecycle | Add/rename/select/restart/remove/re-add. Duplicate names fail, selected pair persists, removal preserves all data and re-adding restores managed identity and target binding. | partial |
| A17 Pair isolation | Use two `doc-0001` records with distinct rules/models. Processing/review/retry/export stay in their owning pair; sidecar switches remove previous custom rules and load the selected local model. | partial |
| A18 Pair routing | Send unknown/mismatched pair ownership, late replies, and switch with unsaved edits. Host rejects mismatches, late events never populate another pair, and switching requires save/discard. | partial |

## Model-management handoff (2026-09-21)

Status: **automated native acceptance passed**. The standard package starts with
zero model weights; the demo contains exactly the two catalog models. Catalog and
compatible-URL installation, explicit pair selection, dynamic windows, legacy
review compatibility, relocation, and offline frozen inference are verified. Native
GUI interaction and a physically disconnected USB run remain user acceptance steps.

The final clean packaging source is
`79c7a3e6e6cb1fc01ebdbebb03c7bb03c08635ad`. The reviewed runtime and desktop source
is `7636b77f03bbab3ff90c91d6ddc902563eea4f29`; the later commit only corrects the
PowerShell tokenizer-file filter in `scripts/package-windows.ps1`. Desktop, sidecar,
locks, build inputs, and freezer specification are unchanged.

The later CI-only commit `a382ddb1af40f1ab60383ced4a7aa9b482f6d500`
expresses the existing Windows lock guard as `sys.platform == "win32"` so mypy can
narrow the native imports, and formats two preparation scripts. Pinned Windows and
Linux branch probes, both-platform mypy over 12 source files, ten focused Linux
lock/lease tests, and five native lock/lease cases confirmed equivalent supported
runtime behavior. The delivered ZIPs remain the verified `79c7a3e` artifacts and
retain the hashes below; they were not rebuilt from the CI-only commit. A fresh
hosted CI run follows the source and documentation push and is not claimed here.

| Artifact delivered to Windows Downloads | Bytes | SHA-256 | Preloaded models |
| --- | ---: | --- | ---: |
| `C:\Users\katisfaktion\Downloads\redactio-79c7a3e-windows-x64.zip` | 564,245,887 | `5758d333dc0198588cff6eae6b5f45e0e59f29977c21f9b2c72943159d623c13` | 0 |
| `C:\Users\katisfaktion\Downloads\redactio-demo-79c7a3e-windows-x64.zip` | 2,713,638,843 | `af75e0731e831d653b0341d50688cd46b026eed35f5def5ec1ddcb971599dbd9` | 2 |

Both archives and their `.sha256` sidecars passed post-copy verification. The prior
`84b110b` ZIP and extracted demo remained unchanged. The desktop executable is
12,647,424 bytes with SHA-256
`05aa1ba2a095851b9ae681bac0547e72b9969187a9b2ae8ff5b99a4830a09276`;
the exact final frozen worker SHA-256 is
`eec025fce7efa1c0d0b44c8513822deec86277c683f5f5b986b760189224353f`.

Completed automated evidence:

- The frozen worker accepted both catalog models, inspected and deduplicated the
  live public HuggingLil URL, rejected an incompatible public BERT with
  `model_incompatible`, and returned `model_not_found` for an empty store. A
  BiomedBERT transfer cancelled after 1,057,680 bytes and then completed through
  download, validation, and ready publication on retry.
- The standard ZIP passed before archiving and after extraction beneath a path with
  spaces and Unicode with an empty schema-v2 registry. The demo passed both checks
  with exactly two ready catalog models and reported `models=2 documents=2
  redactions=2`. Archive integrity, immutable file hashes, static-CRT checks, both
  tokenizer families, and mutable model receipts passed.
- After relocation with an initially empty external cache and `HF_HUB_OFFLINE=1`
  plus `TRANSFORMERS_OFFLINE=1`, the exact final worker produced three detections
  with each pinned model and left the synthetic DOCX unchanged. These flags do not
  prove physical network disconnection. Separate source-runtime evidence denied
  sockets with a cold cache.
- The non-catalog `SyntheticChecks/german-tiny-ner` fixture used a real BERT model
  with a 32-token window, 8-token stride, and two special tokens. Exact final frozen
  inference reached source offset 414 beyond its first window. The separate
  source-runtime fixture produced 83 detections over 414 code points, also ending
  at offset 414.
- Both pinned models' process and corrected-review replies exactly matched the
  archived `9c0ce09` synthetic baseline before and after installing/removing an
  unrelated model. Original records stayed unchanged and legacy storage migrated
  once to schema 2.
- Python passed 388 regular tests plus both explicit real-model canaries, Ruff
  check/format, and mypy over 12 source files. The final fixes passed 237 affected
  manager/store tests; the CDN follow-up passed 135 manager tests and a 27-test
  focused rerun. The exact CDN allowlist uses no wildcard.
- Rust passed 215 regular tests plus all six explicit real-model checks, Rustfmt,
  and all-target Clippy. Native Windows additionally passed 13 host/manager/lock
  checks and five worker-owned lease cases.
- Frontend passed the final 208-test suite across 23 files, typecheck, build, and
  16 actual-App browser checks including delayed legacy metadata and preserved
  drafts. The existing 530.9 kB Vite chunk advisory is nonblocking.
- Packaging passed five executed Python cases with one Windows-only skip, then 21
  native package cases and 20 native build-manifest cases. Final source review
  approved all model-management findings and the exact-CDN follow-up.

Durable content-free evidence is under `dist/model-management-evidence/`; the native
handoff is `dist/model-management-windows/handoff.json` and the detailed report is
`dist/model-management-evidence/task-8-native-report.md`. These ignored artifacts
must accompany the release record; repository documentation alone is not the proof.

### Manual Windows model and USB procedure

This is user-run native GUI acceptance. Browser tests, command-line checks, and the
automated native checks above do not mark these steps passed.

1. On Windows 11 x64 as a standard user, verify the selected final ZIP SHA-256 from
   the artifact table above, then extract the **entire** ZIP to a new writable local
   folder. Do not start from the
   ZIP or a network drive. The standard ZIP contains zero model weights.
2. Start `redactio.exe`, open **Modelle**, and install one catalog model. For a
   compatible public Hugging Face model, enter its repository URL, choose **Prüfen**,
   inspect the exact revision, labels, size, and license, then choose
   **Herunterladen**. This first download needs connectivity; no document processing
   should generate network traffic.
3. Create or open an isolated pair containing synthetic DOCX files. Select the exact
   ready model for that pair, review its entity types, save, process the documents,
   inspect the replacements, and approve a result. A missing model must remain
   visibly missing rather than fall back to another ready model.
4. Exit cleanly. Copy the **entire extracted application folder**, including
   `models`, `sidecar`, and `webview2`, to the USB drive. Do not copy only the EXE
   or only a model directory.
5. On the acceptance Windows machine, copy the whole folder from USB to a new local
   writable directory, disconnect networking and remove external model caches from
   the test environment, then launch it. Confirm the installed model is ready and
   repeat synthetic processing, review, approval, restart, and clean exit without
   a network attempt.
6. Also test a read-only copy: already-ready models must remain usable, while install
   and remove actions fail with the documented storage error and preserve existing
   files. Record screenshots, safe error codes, app/child network observations, OS
   build, account type, WebView version, source revision, ZIP hash, and outcome.

Keep the existing working demo from `84b110b` unchanged. The concise operating
instructions remain in the [Windows quick start](quick-start.de.md#1-entpacken-und-starten),
with model selection and compatibility details under
[Erkennung einstellen](quick-start.de.md#3-erkennung-einstellen). Final Task 8
status and its complete decision record are in the
[implementation plan](superpowers/plans/2026-09-21-model-management.md#progress-and-evidence).

## Real-window evidence protocol

Build the candidate from a clean independent native Git clone; WSL-linked worktree Git
metadata is not source-provenance evidence. Record source/tool manifest and clean status.
Record build/source revision, ZIP SHA-256, OS edition/build/x64, CPU/core count, RAM,
SSD/no GPU requirement, account/token type, process PATH and tool-absence probes. The
reference acceptance machine is Windows 11 x64, four CPU cores, 16 GB RAM, and SSD.
Capture a full window using DPI-aware physical coordinates; inspect top/right/bottom
controls at actual scaling. Record accessible names, focus sequence/restoration,
keyboard outcomes, screen-reader announcements, contrast measurements and control/view
latencies. Browser/Vitest checks complement this record; they cannot replace it.

For the conditional standard-user host route, launch only the reviewed complete candidate
and only after verifying the actual Known Folder `com.redactio.app` is still the empty
task-created directory and no Redactio process or existing settings are present. Record
the candidate SHA-256 and owned synthetic TEMP roots before launch. If user data exists,
stop this route. This route supplies functional evidence only, not clean-machine or
zero-network acceptance. Preserve foreground applications, use only verified app/child
PID-scoped automation, never overwrite the host clipboard, and clean only unchanged
files known to have been created by this test.

A15 must measure the transition into “Per Tastatur auswählen” at the 1,000,000-code-point
limit, not only keys after the view settles. P4.2 Linux evidence found approximately
1,167 ms queued input delay and a 1,240 ms frame gap during this transition; this failure
remains recorded independently of Windows results. On the exact Windows/WebView2 candidate:

1. Record viewport/DPI, source code-point count, source hash, WebView version and cold/warm
   state. Verify native selection, emoji/combining/CRLF offsets and immutable source text.
2. Attach UI Automation observers from a separate process before triggering the transition. Scope elements and
   input handles to the verified process tree. Timestamp with one monotonic clock before
   sending the activation key; queue a selection key during transition and record its
   actual renderer-observable selection/focus effect. Record the ordinary settled-key
   baseline separately. Do not time a later tree search or serialize the million-character
   value inside the timing interval.
3. Measure visible window/repaint response separately during the same transition. A
   successful parent-window `WM_NULL`, `PostMessage` return or `Process.Responding` result
   does not establish WebView renderer response. UIA callback receipt includes accessibility
   delivery overhead; if it is slow, retain that limit and obtain a corroborating renderer
   observation before attributing the delay. Unsupported UIA patterns leave the measurement
   unverified, not passed.
4. Preserve each raw timing and maximum, not only an average. No measured transition input
   or window response above 200 ms may be accepted. Reproduce a breach and send evidence
   to the UI owner for a measured fix. Keep settled timing, transition timing and accessibility
   query overhead distinct. A disposable-control calibration establishes the observer only;
   it is never an application-performance result.

Exercise real native close with dirty review state: stay preserves changes; save failure
keeps the window and changes; save/discard then close succeeds; clean close exits the owned
app and sidecar. Also check pair/document/back navigation focus restoration, warnings and
empty-body approval gates, private notes, hostile Markdown rendered as inert text, and
screen-reader announcements. For final P4.3 rerun the native missing-root sync/export audit
selection, cancellation without source/model and ordinary source exclusion/lock release.
The known file-symlink privilege failure remains a distinct unsupported fixture.

Use a disposable VM/Sandbox/profile with synthetic documents only. Verify guest startup
and standard-user token rather than assuming them: Sandbox's default account is admin.
Disable networking only in that disposable environment. Observe app and all child
network attempts separately (for example guest-local process-attributed ETW/WFP trace,
started before first app launch); zero established connections or a disabled adapter
alone does not prove zero attempted connections. Keep traces content-free and local.
Record the loaded WebView executable path and version, plus app settings, WebView cache
and OS cache locations separately; do not claim Windows performs zero unrelated writes.

For missing-resource cases use a disposable copy of the ZIP and remove `webview2/`.
For each preloaded model, separately remove or corrupt one artifact while leaving its
registry receipt ready; confirm validation fails without falling back to installed
runtimes or another model location. A model-free package and a receipt recording an
explicitly removed model must remain valid. Preserve expected errors, screenshots and
command exit codes. Never
change host firewall/network settings, delete existing settings/pairs, or use private
documents to produce repository or CI evidence.

## Stage 1 evidence (2026-09-19)

- Dependency BASE: `9faa153`; artifact is the reviewed P5.1 early ZIP, not the final P4 build.
- Generator: 400 DOCX, deterministic first-document bytes, six distinct edge profiles,
  hash/expectation checks, empty-directory/count validation and existing-file preservation.
- Linux full engine suite: 133 passed with exact prefetched German 3.8.0 lg/sm models.
- Linux frontend: 13 tests; typecheck and production asset build passed.
- Native frozen package: all 400 baseline canaries passed (3,200 redactions),
  92.60 seconds including package-integrity hashing and model initialization; this is
  synthetic package-check timing, not the reference-machine performance benchmark.
- Native Windows package negative suite: 11 checks passed, including absent bundled
  runtime/model, corpus count/path/duplicate/hash/unlisted input and package integrity.
- Real Windows Sandbox startup was attempted with networking and redirects disabled and
  only task-owned probe/evidence maps. It failed before guest initialization with
  `Windows Sandbox failed to initialize. Exception of type 'System.Exception' was thrown.`
  No guest OS, tool absence, token or network-observation acceptance was established.
- Native file-symlink regression cannot create its fixture under the host's standard-user
  token (`WinError 1314`, required privilege absent). No Developer Mode/security setting
  was changed. This criterion remains unverified; ordinary junction tests do run.

Stage 1b on reviewed integration `e68198e`: actual Windows MSVC release executables
passed 127 default tests plus both real-model opt-ins (129 total), including native
transport, the Python contract emitter, two-pair batch/error/cancellation/rule isolation,
and six concurrent audit writers ×30 appends. The known file-symlink fixture was
explicitly filtered as unverified; its regression remains enabled for capable hosts.
Test fixtures ran from a dedicated native TEMP mirror and local cwd using the test-only
`REDACTIO_TEST_MANIFEST_DIR` override. Linux debug checks passed 133 default tests plus
both real-model opt-ins. No final package/GUI/network acceptance follows from these tests.

Stage 1c on reviewed integration `4fcd67f`: focused Windows MSVC release executables
passed 41 default tests and four explicit real-model checks (45 total). These cover
pair-specific configuration, lg/sm 3.8.0 model switching, preview timeout/abort cleanup,
transport configuration replay, stored-review corrections and approval, tampering,
changed-settings rejection and guarded direct review commands. Strict all-target MSVC
Clippy and rustfmt passed using the isolated pinned toolchain. The two new command
fixtures use the same runtime manifest-directory helper; no production code changed.
CI now explicitly executes the detection and review real-model opt-ins as well.
This bounded checkpoint does not validate the final UI, export flow or rebuilt artifact;
the Sandbox and symlink limitations above remain outstanding.

Reviewed full P4 native regression on `49c6896`: 165 distinct default cases plus all
five real-model opt-ins passed on Windows after focused fixture-only corrections
(170 total). New export coverage includes native junction/alias rejection, destination
ownership/races, exact approved bytes, mixed/stale/tampered records, partial retention,
post-publication uncertainty, and missing-root/cancel audit behavior. The initial failures
and focused reruns are retained separately; this count does not imply an unchanged
all-green initial run. One file-symlink fixture remains explicitly unverified. Strict
MSVC all-target Clippy and rustfmt passed. These checks run actual native test executables
against reviewed production code; exact packaged GUI/A15 acceptance is still pending.

Detailed command outputs and local synthetic evidence locations are maintained in the
P5.2 task report. Remote CI has not run: no remote or publishing action was authorized.
CI definitions use frozen/locked dependencies and build-time hash-pinned model downloads,
then offline uv/Cargo checks. Only dependency caches are enabled. The manually triggered
Windows workflow uploads a retained CI artifact; it never publishes a public release.

Primary configuration references: [Windows Sandbox configuration](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file),
[setup-node](https://github.com/actions/setup-node), [setup-uv](https://github.com/astral-sh/setup-uv).
Hugging Face documents the CDN hosts used behind its download service in
[Downloading behind a proxy or firewall](https://huggingface.co/docs/hub/datasets-downloading#downloading-behind-a-proxy-or-firewall);
the service also publishes current endpoint metadata at
[`/.well-known/meta.json`](https://huggingface.co/.well-known/meta.json).
