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
./scripts/check-package.ps1 -PackageRoot 'C:\Users\Public\Redactio Prüfung' -CorpusRoot 'C:\TestInputs\synthetic'
./scripts/check-package.ps1 -PackageRoot 'C:\Users\Public\Redactio Prüfung' -CorpusRoot 'C:\TestInputs\synthetic-edges'
```

`expectations.json` schema 2 contains count, relative synthetic filenames, profile,
SHA-256, and expected safe warning/error/state codes. It contains no extracted text,
recognizer values, original filenames, or private-document measurements. The six edge
profiles are header warnings, corrupt ZIP, empty body, repeated names, Unicode, and
output tampering. The repeated-placeholder check uses an explicit literal PERSON rule
for the known synthetic name; it measures placeholder reuse, not baseline NER recall.
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
| A05 Configuration | Toggle entities, add invalid/valid regex and words, switch prefetched models and simulate engine/extraction revisions. Only affected pair results become stale; invalid/unavailable configuration does not save; rename/select leaves revisions stable. | unverified |
| A06 Incremental | Run twice, edit a source, remove output, tamper with output, and force a reviewed document. Unchanged work skips; changed/missing work regenerates; external edits/reviews require explicit resolution and no silent replacement. | unverified |
| A07 Recovery | Inject every journal boundary, native locked-file failure and restart. Incomplete work stays pending, original bytes survive failed replacement, retry completes coherently and IDs are never reused. | partial |
| A08 Cancellation | Cancel initialization and a document, double-click start, switch pair while busy and start another instance. No partial success or cross-pair changes appear; only one writer runs and cancellation visibly settles. | unverified |
| A09 Sidecar | Run real serialized requests plus malformed input, EOF/crash, timeout, late reply and shutdown tests. Errors remain bounded/safe, restart is limited, model identity matches and retries never duplicate commits. | partial |
| A10 Review | Keyboard-add/dismiss/change entity types, save/reopen and change source/config. Body, offsets and counts agree; bound decisions persist; changed inputs invalidate decisions; approval binds exact final bytes. | unverified |
| A11 Export | Export current approvals to an empty directory outside all pairs, then try mixed ownership/stale/tampered output. Only selected current approved Markdown is copied; invalid selections fail and originals, notes, mappings and identifying rules are absent. | unverified |
| A12 Privacy | Use canaries in paths/rules/invalid data/errors and malicious Markdown links/HTML. Logs/frontmatter contain no canaries; preview cannot execute scripts or request remote assets. Observe app and all children during cold startup and the full flow: zero external attempts. | unverified |
| A13 Audit | Complete/fail/cancel attempts, concurrent append/recovery and unwritable audit directory. Records retain correct opaque pair IDs, prior entries remain valid, no pair names leak, and audit failure is visibly announced. | unverified |
| A14 Windows artifact | Extract exact ZIP to `C:\Users\Public\Redactio Prüfung` in a disposable Windows 11 x64 standard-user environment without dev tools/network. Launch/configure/scan/process/review/export/restart/exit succeed; app-local fixed WebView identity is observed; all Onyx assets/German text render without remote attempts. | unverified |
| A15 Usability | Complete the main flow with Tab/Shift+Tab/Enter/Space/Escape and a screen reader. All controls have names, visible ordered focus and focus restoration; progress/errors announce, warning status is not color-only, contrast passes and large views/ordinary controls respond within 200 ms. | unverified |
| A16 Pair lifecycle | Add/rename/select/restart/remove/re-add. Duplicate names fail, selected pair persists, removal preserves all data and re-adding restores managed identity and target binding. | partial |
| A17 Pair isolation | Use two `doc-0001` records with distinct rules/models. Processing/review/retry/export stay in their owning pair; sidecar switches remove previous custom rules and load the selected local model. | unverified |
| A18 Pair routing | Send unknown/mismatched pair ownership, late replies, and switch with unsaved edits. Host rejects mismatches, late events never populate another pair, and switching requires save/discard. | unverified |

## Real-window evidence protocol

Record build/source revision, ZIP SHA-256, OS edition/build/x64, CPU/core count, RAM,
SSD/no GPU requirement, account/token type, process PATH and tool-absence probes. The
reference acceptance machine is Windows 11 x64, four CPU cores, 16 GB RAM, and SSD.
Capture a full window using DPI-aware physical coordinates; inspect top/right/bottom
controls at actual scaling. Record accessible names, focus sequence/restoration,
keyboard outcomes, screen-reader announcements, contrast measurements and control/view
latencies. Browser/Vitest checks complement this record; they cannot replace it.

Use a disposable VM/Sandbox/profile with synthetic documents only. Verify guest startup
and standard-user token rather than assuming them: Sandbox's default account is admin.
Disable networking only in that disposable environment. Observe app and all child
network attempts separately (for example guest-local process-attributed ETW/WFP trace,
started before first app launch); zero established connections or a disabled adapter
alone does not prove zero attempted connections. Keep traces content-free and local.
Record the loaded WebView executable path and version, plus app settings, WebView cache
and OS cache locations separately; do not claim Windows performs zero unrelated writes.

For missing-resource cases use a disposable copy of the ZIP, remove `webview2/` or
`models/de_core_news_lg/`, and confirm setup fails without falling back to installed
runtimes/models. Preserve expected errors, screenshots and command exit codes. Never
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

Detailed command outputs and local synthetic evidence locations are maintained in the
P5.2 task report. Remote CI has not run: no remote or publishing action was authorized.
CI definitions use frozen/locked dependencies and build-time hash-pinned model downloads,
then offline uv/Cargo checks. Only dependency caches are enabled. The manually triggered
Windows workflow uploads a retained CI artifact; it never publishes a public release.

Primary configuration references: [Windows Sandbox configuration](https://learn.microsoft.com/en-us/windows/security/application-security/application-isolation/windows-sandbox/windows-sandbox-configure-using-wsb-file),
[setup-node](https://github.com/actions/setup-node), [setup-uv](https://github.com/astral-sh/setup-uv).
