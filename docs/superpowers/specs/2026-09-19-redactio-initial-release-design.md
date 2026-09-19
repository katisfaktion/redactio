# Redactio — initial release specification

Status: **approved for implementation planning** · 19 September 2026

This document defines the intended complete first release. Requirements describe
the release contract, not claims about what the prototype already delivers.
The original product and Tauri/Rust/Python architecture are retained, with a
Vue 3 and sit-onyx frontend. This frontend choice and multiple saved sync pairs
are explicitly requested initial-release scope. The review/correction/export
workflow and extraction improvements are approved release scope. This document
replaces the old development-process documents and requires none of them to be
understood or implemented.

## 1. Purpose and success

Redactio helps a single Windows user prepare sensitive German documents for
subsequent work outside the app. The first user is a doctor preparing roughly
400 case reports for a book project. The app should remain useful for other
document collections without medical-specific fields or workflows.

A second project goal is practical Vue.js experience for the developer's intended
job direction. Use idiomatic, readable Vue with TypeScript while implementing the
real document workflow; the learning goal does not add unrelated product features.

The user configures one or more named source/working-output folder pairs, selects
a pair, runs local pseudonymization, checks the result, and obtains Markdown files
with typed placeholders such as `<PERSON_1>`. The app never sends documents to another
service. Detection assists the user; the product does not claim guaranteed
anonymity or that approval establishes legal permission to share a document.

Release success means:

- A non-developer can launch the portable Windows distribution and process a
  collection without installing development tools or downloading a model.
- Multiple saved pairs remain independent: switching collections cannot mix
  documents, detection settings, review decisions, or outputs.
- A run of 400 representative documents completes in under 30 minutes on the
  reference PC defined in section 12; the app remains responsive and cancellable.
- Documents retain readable narrative content, including text in tables.
- Repeated occurrences of the same recognized value within one document use
  the same placeholder; different documents have independent numbering.
- Unchanged successful documents are skipped. Changed inputs or processing
  settings trigger reprocessing, with explicit protection for reviewed work.
- Errors, incomplete extraction, and stale reviews are visible. A failed file
  does not silently become a successful output.
- The approved expanded release supports inspection, correction, approval, and
  export of approved documents entirely inside the app.

## 2. Release boundary

| Capability | Release requirement | Basis |
| --- | --- | --- |
| Offline Windows desktop, German UI | Single user, no account or backend service | Inherited |
| Folder configuration | Multiple persistent named source/working-output pairs, with one selected at a time | Explicitly requested expansion |
| Batch processing | Recursive DOCX discovery, progress, cancel, retry, result summary | Inherited |
| Detection | Local Presidio and German spaCy model, typed placeholders | Inherited |
| Recognizer settings | Built-in toggles, custom patterns and word lists, local model selection | Inherited but unfinished |
| Output | Stable `doc-NNNN.md` names and versioned YAML frontmatter | Inherited |
| Traceability | Source-local mapping, source hashes, metadata-only audit log | Inherited |
| Repeat-value consistency | Same entity type and normalized value use one placeholder per document | Inherited requirement missing from current substitution code |
| Correct reprocessing | Processing revision, missing-output recovery, overwrite protection | Resolution of prototype gaps |
| Usable distribution | Bundled interpreter, dependencies, model, offline runtime prerequisites | Inherited release requirement, not delivered by prototype |
| Extraction completeness | Paragraphs and tables in order; explicit unsupported-content warnings | Approved expansion |
| In-app review and correction | Original/output comparison, add or dismiss redactions, review states | Approved expansion |
| Approved export | Export only current, explicitly approved results | Approved expansion |

Three viable release sizes were considered:

1. **Complete the original scope:** batch processing, configurable detection,
   Markdown output, audit, and a usable Windows package. Smallest scope, but
   review still happens outside the app.
2. **Complete the user workflow — recommended:** original scope plus extraction
   improvements, review, correction, and approved export. This directly supports
   the book-preparation use case and provides a way to correct missed detections.
3. **Broaden ingestion and automation:** add PDF/TXT, cross-pair batch execution,
   rename detection, and watching. Useful for larger workflows, but increases
   extraction and synchronization cases before improving the review step.

The approved release includes option 2 plus the explicitly requested support for
multiple saved sync pairs. Permission to consider a wider release is not treated
as a request to add every possible feature.

Not included in this release: PDF/TXT ingestion, OCR, automatic folder watching,
parallel processing of pairs, cross-pair batch execution, automatic rename
matching, cross-document identity resolution, disease/drug entity detection, local generative-model
inference, rich Word-format preservation, application authentication, multi-user
collaboration, signed installers, auto-update, or supported macOS/Linux builds.
Cloud processing, uploads, telemetry, and runtime network downloads are excluded.
More capable implementation tools do not change the app's offline contract.

## 3. User experience

The German-language UI has three main views: documents/run status, document
review, and settings. This retains the prototype's simple folder-based desktop
workflow. Screens show product language rather than implementation milestones.
An always-visible pair selector identifies the active collection. Document
lists, run summaries, review views, and pair-specific settings show its name.

### 3.1 Configure and inspect a collection

1. Add a named pair and select source and working-output folders using native
   folder dialogs, or select an existing saved pair.
2. Validate the canonical paths, permissions, ownership of existing output, and
   separation from every other saved pair.
3. Offer to create a missing working-output folder after an explicit user action.
4. Explain that the working output can contain unreviewed material. Show file
   counts, mapping status, and any discovery errors before processing.
5. Persist the pair and its processing settings locally; remember the selected
   pair across app restarts. An empty pair list shows the setup action.

Each pair has an immutable random UUID `sync_pair_id` and a user-editable display name.
Names must be non-empty and unique among saved pairs after trimming and
case-insensitive comparison. Names may contain identifying information, so remain
private UI/settings data. Renaming a pair does not change its identity, document
IDs, processing revision, or reviews.

Users can add, rename, select, and remove saved pairs. Removing a pair removes
only its configuration after confirmation; source files, mappings, review data,
working outputs, and exported files remain untouched. If the selected pair is
removed, select the next saved pair or show setup when none remain. Do not allow
removal during an operation or silently discard unsaved review edits.

Source and target paths are fixed after the pair is saved; relocation is outside
this release. To use different folders, configure another pair. Re-adding a
previously managed source adopts its stored pair ID and requires the target
recorded in its mapping. Reject conflicting metadata or an ID already registered
to another source. Initialize detection settings from defaults if the removed
configuration is unavailable; a new processing revision makes existing results
stale, preserving the explicit reprocessing protection for reviewed work.

Identical or nested folders are rejected within a pair and across all pairs:
source/source, target/target, and either source/target combination. This includes
overlaps through Windows junctions, symlinks, case differences, or alternate path
spellings. One source cannot feed multiple saved targets. Revalidate all pair
roots before a write operation because filesystem aliases can change after setup;
unavailable or unresolvable roots block writes until separation can be verified.
The release supports local filesystem folders; network shares and cloud-synchronized folders
are outside the tested operating environment. Source documents are never edited.
The source must be writable for app metadata; lack of permission is reported
before a run. A non-empty target prompts a warning and ownership checks, never
blanket permission to replace arbitrary files.

### 3.2 Run and recover

The selected pair's document list shows local filenames, stable IDs where assigned,
processing state, and review state. The user can process all its eligible files
or retry selected failed files. An explicit reprocess action handles otherwise
unchanged files.

Discovery recursively finds case-insensitive `.docx` extensions in a stable path
order. Ignore hidden/dot-prefixed directories and files, Word lock files beginning
`~$`, and app-owned metadata. Do not traverse symlinks or junctions. Read or hash
errors are recorded per file and do not discard readable peers. An unreadable root
is a run-level error.

During a run, show initialization/scanning/processing stages and counts. Snapshot
the pair ID, canonical folders, and that pair's configuration for the run.
Process one selected pair at a time; there is no queue or run-all-pairs action.
Disable pair selection and configuration changes during processing, review saves,
and export. Allow only one such operation per app instance, and only one writer
across app instances sharing the same collection; a second writer receives a
clear busy error. Cancelling or failing one pair does not modify another pair.

Collection commands and events identify their pair explicitly. Reject unknown pair IDs;
never fall back to the currently selected pair. Results and events cannot be
applied to a different collection after selection changes. Switching pairs with
unsaved review edits requires saving or explicitly discarding those edits first.

Cancel finishes the current bounded operation or terminates a stalled sidecar
after its deadline, then stops scheduling work. Results already committed remain
usable. A final summary distinguishes completed, completed-with-errors,
cancelled, and failed runs. Counts use the same discovered-file denominator:
processed + skipped + failed + unprocessed = discovered; warned is a subset of
processed and is not added again. Display failed paths only in the local UI.

### 3.3 Inspect and correct

The review view displays extracted original text and the generated text with
highlighted detections, entity types, and detector confidence. Confidence is
detector output, not a guarantee of privacy. Show extraction warnings prominently.

Users can select original-text spans to add redactions, dismiss false positives,
and change a detection's entity type. Corrections regenerate the output, offsets,
and counts together. Free-form replacement text and general document editing are
outside this release. A manual redaction is labeled manual, with no invented
confidence score. Provide undo for edits made during the current review session
and warn before discarding unsaved changes.

Save detections and decisions under the selected pair's protected source metadata
directory, keyed to pair ID, document ID, source hash, and processing revision.
Store offsets and decisions rather than copies of original text. Reopen a saved
review by reading the current source and verifying the binding. Changed
input/configuration invalidates the previous review; never
reapply old offsets to new text. Original text may cross the private IPC boundary
for this view, but must not be written to logs or exported metadata.

Review states:

- `pending`: processed, awaiting a human decision.
- `approved`: explicitly accepted for the current source, settings, and output.
- `rejected`: reviewed and withheld from export.
- `needs-rework`: incomplete extraction or a user-identified unresolved issue.

Edits return a document to `pending`; known extraction issues keep it
`needs-rework`. A user can approve a non-empty document with extraction warnings
only after explicitly acknowledging those warnings. Empty extraction cannot be
approved. Any regeneration invalidates approval. Local review notes remain in
protected metadata because notes can themselves contain identifying information.

### 3.4 Export approved documents

An explicit export action copies only selected approved Markdown files from the
selected pair into a user-selected export directory. Export one pair per action;
combining pairs into a flat directory is outside this release because document
filenames can repeat across pairs. Validate the destination independently; it
must not overlap any saved pair's source or working output, app settings, or their
protected metadata.
The smallest initial behavior requires an empty destination, avoiding merges or
silent replacement of older exports.

Immediately before exporting, verify pair ownership, current source hash,
processing revision, output checksum, and saved approval binding. Block stale or externally edited
results. Copy only allowlisted Markdown files named by document ID; never copy
mappings, original files, review notes, custom recognition terms, or temporary
files. Show a per-file export result and clearly identify an incomplete export if
a write fails. A later source edit cannot revoke files the user already exported;
export is a verified snapshot at the time of the action.

### 3.5 Configure detection

Each pair has its own detection and output settings, initialized from product
defaults when added. There is no shared mutable profile or cross-pair inheritance.
Settings expose the available built-in detectors, custom regex rules and word
lists, bundled German model choice, and inclusion of detailed redaction positions
in Markdown frontmatter (default on). Always retain the internal positions needed
for review regardless of the export option.

Validate configuration before saving: non-empty opaque rule identity, a type
chosen from the public entity set or the generic CUSTOM type, valid regex,
non-empty word-list entries, no duplicate rule IDs, and a locally available model.
Run regex previews on synthetic user-entered text in the bounded sidecar, so a
pathological expression cannot hang the UI indefinitely. Show a clear timeout
error. Custom terms, names, patterns, and preview text are sensitive local data;
do not include them in logs or output metadata.

Bundle `de_core_news_lg` as the standard model. Offer smaller `sm`/`md` choices
only when their compatible offline model packages are installed. Missing or
incompatible models produce a setup error, never an automatic download. Model,
recognizer, rule, or output-option changes issue a new processing revision for
that pair only; affected documents become stale even when their source bytes are
unchanged. An app/engine/extractor upgrade invalidates every pair whose processing
behavior changed. Selecting or renaming a pair does not invalidate anything.

## 4. Extraction and pseudonymization contract

### 4.1 DOCX extraction

Extract visible main-document paragraphs and table-cell text in document order,
using deterministic separators. Preserve paragraph breaks and readable table row
boundaries; pixel-perfect layout, fonts, and pagination are not goals. The exact
normalized text is the coordinate space for analysis and manual review.

Detect content outside this supported surface, including headers/footers,
footnotes/endnotes, text boxes, tracked changes, comments, embedded objects, and
images. Warn that these are not included and set `needs-rework`; do not silently
claim a complete conversion. Do not fetch external relationships or hyperlink
targets. Read visible hyperlink labels as text where part of supported content.
Metadata, hidden package parts, filenames, and image bytes are not copied out.

Corrupt, encrypted, unreadable, over-limit, or unsupported documents receive a
file-level error. Initial limits are 64 MiB compressed input, 256 MiB cumulative
uncompressed ZIP content, a 1,000:1 expansion ratio, 1,000,000 extracted Unicode
code points, and 64 MiB per IPC message. Enforce archive limits during extraction,
not only against declared ZIP sizes. Initialization has a 180-second deadline;
each document request has a 120-second deadline. Exercise these limits against
the representative corpus before shipping; any adjustment is documented and
tested, and cannot silently remove a bound. Empty extraction produces an
explicit empty `needs-rework` result and warning, never an apparently valid case.

### 4.2 Detection and replacement

Use local Presidio with the selected German spaCy model. The default entity set
is PERSON, LOCATION, EMAIL_ADDRESS, PHONE_NUMBER, IBAN_CODE, IP_ADDRESS, URL, and
DATE_TIME. Verify each enabled category has an effective German or language-neutral
recognizer in the packaged configuration. Do not confuse entity types with the
actual recognizer implementations.

The approved optional detector extension is OpenMed's German BiomedBERT 340M PII
model, selected per pair after explicit setup. Pin its revision and verify downloaded
files during setup; inference uses only local CPU weights. Map name components to
PERSON and address components to LOCATION, preserving the existing categories and
structured recognizers. Process long text in overlapping windows with original
Unicode offsets. Keep spaCy as the default and evaluate detection quality locally;
model availability and successful processing do not establish name/address recall.

Apply enabled custom rules and manual decisions to the extracted text. Validate
all spans before replacement. Resolve overlapping detections deterministically
without leaving any part of a still-active detected span exposed: merge their
covered interval, prefer a manual entity type, then the highest-confidence type,
then a stable type-name tie-break. A dismissed detection does not dismiss another
overlapping active detection. Adjacent spans need not be merged.

Assign typed placeholders by first occurrence, with counters per entity type.
Within one document, equal values for the same type reuse a placeholder after
Unicode NFC normalization and trimming surrounding whitespace. Do not infer that
different names, abbreviations, case variants, or pronouns identify the same
person. No linking is persisted across documents. Recognized values are transient
memory-only data.

Example: two occurrences of the same detected `Anna Beispiel` become
`<PERSON_1>` both times; a distinct detected name becomes `<PERSON_2>`.
Redaction counts count occurrences, not distinct placeholders. Escape literal
source strings that could be confused with generated placeholder syntax in the
rendered review UI; never identify real redactions by regex matching alone.

Offsets are zero-based Unicode code-point positions with an exclusive end.
Output offsets refer to the exact generated body, not YAML or original text.
The JavaScript UI must account for UTF-16 indexing. Replacements and any Markdown
escaping happen before final offsets and the output checksum are computed.

## 5. Incremental processing, identity, and persistence

Document IDs are monotonically allocated independently within each pair:
`doc-0001`, `doc-0002`, and so on, growing beyond four digits as needed. They are
stable for the same relative source path within that pair and never reused there.
The full document identity is `(sync_pair_id, doc_id)`; two pairs may both contain
`doc-0001` without sharing data. Mappings, review records, pending writes, and
ownership checks carry this pair identity. A moved/renamed file is a new document
in this release. Deleted source entries remain recorded and become unavailable
for approval/export; no
automatic source or output deletion occurs.

A document is current only when source hash, processing revision, successful
mapping record, and actual generated-output checksum agree. Missing output is
eligible for regeneration. Externally modified output is a visible conflict, not
an automatic overwrite. Reprocessing reviewed or manually corrected work shows
the affected documents and requires an explicit discard/reprocess action.

Reserve a new ID durably before writing its output. An ID reservation is not a
successful processing record. Validate the source hash again before committing
the result; if the source changed during processing, discard that result and
mark the file for retry. Serialize processing, review saves, and export snapshots
for a collection so none can race a write from another app action.

Use same-directory temporary files and atomic replacement for settings, mapping,
review data, and Markdown, with durability handling verified on Windows. The
output and mapping are separate files, not a single atomic transaction: record
pending writes with the expected source/revision/output checksum before replacing
output, then mark successful only after output is durable. On restart reconcile
pending records against disk, or retry them. Never reuse a reserved filename or
mistake an incomplete commit for a current output.

App-owned files may be replaced only after ownership is established. Unknown
`doc-NNNN.md` files cause a conflict. Missing/corrupt mapping data must not cause
silent initialization over existing generated output; prompt for restoration or
a fresh empty output destination. Unsupported schema versions are rejected with
an actionable message. Old prototype data migration is outside the initial
release; originals can be processed into a fresh collection.

## 6. Data contracts

Use versioned, validated formats with timezone-aware RFC-3339 timestamps.

| Record | Location and contents |
| --- | --- |
| Settings | App configuration directory: schema version, ordered `sync_pairs` list, nullable `selected_sync_pair_id`; each pair contains immutable ID, private name, canonical source/target folders, creation time, model identity, recognizer configuration, processing revision, and output options |
| Mapping | Each source root `_document-mapping.json`: schema version, pair ID, canonical target binding, ID allocation state, relative source paths, reservations/pending writes, source hashes, successful revision and output checksum, first/last success timestamps |
| Review | Each source `_redactio/reviews/`: schema version, pair ID, document ID, source/revision binding, automatic span metadata and manual decisions, status, private notes, warnings acknowledged, approval timestamp and output checksum |
| Working output | Each target root `doc-NNNN.md`: UTF-8 Markdown with frontmatter and generated body |
| Audit | Shared app configuration directory `audit-log.jsonl`: one versioned record per run/export attempt, identified by opaque pair ID |

The source metadata directory is excluded from discovery. Settings contain paths
and possibly custom identifying terms, so belong to the same private trust zone
as originals. File permissions inherit the current Windows user environment;
application-level encryption and protection against malware running as that user
are not promised.

### 6.1 Markdown frontmatter

| Field | Contract |
| --- | --- |
| `schema_version` | Initial release output schema version |
| `sync_pair_id` | Opaque pair identity; never the pair's display name or folder path |
| `doc_id` | Stable document ID matching the output filename |
| `source_hash_sha256` | Lowercase SHA-256 of the processed source bytes |
| `processing_revision` | Opaque revision identifying the local processing configuration; not a hash of secret word lists |
| `redacted_at` | Generation timestamp with timezone |
| `redaction_engine` | Actual packaged engine/version identifier |
| `nlp_model` | Actual model name and version |
| `recognizers_used` | Public built-in identifiers and opaque custom-rule IDs; no custom names/patterns/terms |
| `redactions` | Occurrence entries: entity type, placeholder, output start/end, confidence or null for manual, safe recognizer ID, automatic/manual/merged origin; omitted if the position option is disabled |
| `redaction_summary` | Per-type occurrence count and min/mean of available automatic confidence values; null aggregates if none exist |
| `review_status` | `pending`, `approved`, `rejected`, or `needs-rework` |
| `reviewed_at` | Last explicit review timestamp, or null |
| `warnings` | Safe structured extraction/processing warning codes |

No original filename, path, detected value, raw exception text, or private review
note is permitted. Export may retain source hashes as in the original design;
they are provenance identifiers, not an anonymity guarantee. The checksum used
to bind approval covers the complete final Markdown bytes, including the updated
review status and timestamp, and lives in private
metadata, avoiding a self-referential checksum inside the file.

### 6.2 Audit

Append JSON Lines with schema version, opaque pair ID, random run ID, action,
start/end time, outcome, counts, engine/model versions, processing revision, and
safe error codes. Never log pair display names, paths, filenames, content, detected
values, custom rule text, review notes, or validation input. Failure before model initialization may leave model
metadata null. Include cancelled and failed attempts, not only successful runs.

Serialize writes to the shared audit log across app instances. Flush completed
entries. Detect and recover an incomplete trailing record before the next append,
preserving preceding complete records and reporting recovery.
An audit-write failure does not remove completed documents, but must be visible
in the run/export summary. No rotation or audit analytics UI is needed initially;
settings exposes the log location and a native open-folder action.

## 7. Architecture

- **Desktop UI:** Tauri 2 with Vue 3, strict TypeScript, and Vite. Use Vue
  single-file components with Composition API and `<script setup lang="ts">`.
  Keep Zod validation at incoming UI boundaries. Start with local reactive state;
  extract composables for shared UI logic and Tauri calls when needed.
- **Design system:** `sit-onyx` components, styles, and design tokens. Use its
  layouts and controls directly, with scoped CSS for application-specific needs.
  Bundle required fonts, icons, and German component translations locally. Avoid
  a second component library or utility CSS framework without a concrete need.
- **Rust host:** native dialogs, pair management and selection, path validation,
  settings/mapping/review state, batch orchestration, file commits, audit, sidecar
  lifetime, and exports.
- **Python sidecar:** python-docx extraction, Presidio/spaCy analysis, applying
  corrections, placeholder substitution, and Markdown/frontmatter generation.
  Pydantic validates sidecar payloads and output metadata.
- **Communication:** UTF-8 JSON Lines over child stdin/stdout with correlated
  request IDs and typed success/error envelopes. No local HTTP server or port.
- **Tooling:** pnpm, Cargo, and uv with committed lockfiles. Pin compatible release
  dependencies/model artifacts during implementation; do not preserve prototype
  versions merely because they appeared in its scaffold.

Vue is an intentional choice for the developer's learning goal. Build the
frontend as a client-side application with static bundled assets; no server-side
rendering or Nuxt is required. Use normal Vue props/events, computed state, and
lifecycle cleanup for asynchronous Tauri listeners. Prefer small, understandable
components over custom framework abstractions. A global state library is added
only if shared state becomes difficult to manage with Vue's built-in facilities.

Integration references: [Tauri frontend configuration](https://v2.tauri.app/start/frontend/),
[Onyx setup and browser support](https://onyx.schwarz/development/), and
[Vue's TypeScript Composition API](https://vuejs.org/guide/typescript/composition-api).

Keep domain operations testable apart from Tauri. Rust owns persistent app-state
schemas; Python owns analysis/IPC payload and output schemas. UI projections are
validated only where consumed. Shared contract tests exercise real serialized
messages; comments alone are insufficient to prevent schema drift. Do not add a
database, generalized plugin system, or unused abstraction for hypothetical scale.

## 8. Sidecar lifecycle and failures

Start one sidecar lazily per app instance; multiple saved pairs do not spawn
multiple sidecars. Reuse the loaded model when compatible, but apply and verify
the selected pair's full recognizer/rule configuration before each operation.
Clear previous pair-specific state; reload the model when its identity changes.
Custom terms and analysis state must not leak between pairs. Distinguish
initialization and document-processing deadlines, with a longer initialization
deadline. Production starts the bundled executable/runtime by a resolved local
path, never `uv`, a shell command string, or a developer checkout path.

Only the Rust host writes managed outputs. On unexpected sidecar exit, restart
and retry the read/analyze request once with a fresh correlation ID. A second
exit, protocol corruption, or unavailable model fails the run; bad individual
documents fail individually. On timeout, terminate the stuck process before a
later request is allowed to use it. Ignore late/unknown replies safely. On app
exit, close stdin, wait briefly, and terminate the remaining child if necessary.

Stdout carries protocol messages only. Application logs and dependency stderr
must be sanitized before persistence; do not forward raw exception tracebacks
or library output into production logs. Display safe error codes/messages and
resolve local filenames in the host UI. Validation errors must not echo payloads.

## 9. Privacy, filesystem, and rendering boundaries

- No runtime network communication, telemetry, update checks, external content,
  remote fonts, or model downloads; dependency setup at build time is separate.
- The host grants only the commands required by the UI. Production content
  security policy denies remote loads and arbitrary script execution.
- Validate IPC, settings, mapping, review spans, and serialized output at their
  trust boundaries; reject unknown versions, invalid offsets/IDs, and pair
  ownership mismatches. Scope filesystem access to the operation's pair, not the
  union of all saved folders.
- Canonicalize and validate reads/writes, including temporary files and existing
  destination objects. Handle junctions/symlinks and Windows path aliases; reject
  unsafe redirections rather than relying on a string prefix check.
- App data writes are limited to its configuration directory, source metadata,
  working target, and the explicitly selected export destination. Document OS and
  WebView runtime caches separately; do not claim the operating system writes
  nothing outside these folders. Never deliberately cache original document text.
- Render source/output as text or safely rendered Markdown. Disable raw HTML,
  executable links, and remote images. Document content is untrusted data.
- The working-output folder remains private until review; only the explicit
  export contains approved results. User approval cannot certify undetected
  contextual identifiers; the UI must not label output as guaranteed anonymous.

## 10. Windows delivery

Ship a versioned Windows x64 portable ZIP containing the desktop app, a packaged
Python runtime/sidecar, pinned dependencies, the default German model, required
license notices, a short German quick start, and checksums. The package must be
usable without administrator rights and without separately installed Node,
Python, Rust, pnpm, or uv. Use a bundled app-local WebView2 runtime where needed
to satisfy the offline/no-install requirement; test the same shipped layout.
Pin compatible Vue/sit-onyx versions and select a WebView2 runtime meeting the
pinned Onyx release's browser requirements. Verify the shipped runtime rather
than assuming compatibility from development-browser testing. All UI styles,
fonts, icons, and translations must load from bundled assets without a CDN.

Windows 11 x64 is the initial supported release target. WSL/Linux remains a
development environment, not evidence of Windows release readiness. A build must
run from paths containing spaces and non-ASCII characters. Portable distribution
means no application installer; settings/audit may live in the user's documented
app configuration directory. Models are read-only package resources.

Build automation creates the complete archive on Windows and runs synthetic
smoke tests; no private corpus enters CI. Include a dependency/model manifest so
the tested artifact can be reproduced. A missing runtime/model results in an
actionable offline setup error. Signing and automatic updates are deferred.

## 11. Acceptance checks

| ID | Runnable or observable acceptance condition |
| --- | --- |
| A01 Setup | Persist multiple valid pairs and selected pair; reject identical/nested/aliased paths within and between pairs and redirected write targets; handle non-empty targets without replacing unrelated files |
| A02 Discovery | Nested DOCX and uppercase extensions are found deterministically; hidden files, Word lock files, app metadata, and links are excluded; unreadable peers are reported |
| A03 Extraction | Synthetic paragraphs/tables retain reading order; unsupported surfaces trigger warnings; corrupt, empty, large, and expansion-heavy packages have bounded outcomes |
| A04 Redaction | Known synthetic names/contact details are replaced; repeated values reuse placeholders; overlapping spans leave no active detected suffix exposed; Unicode offsets index the exact generated body |
| A05 Configuration | Toggle/regex/word-list/model changes invalidate only the edited pair's results; engine/extractor upgrades invalidate affected pairs; selection/rename does not; invalid rules and unavailable models cannot silently save as working configuration |
| A06 Incremental processing | Second unchanged run skips; source or configuration changes reprocess; missing output regenerates; external edits and reviewed work require explicit conflict resolution |
| A07 Recovery | Inject failure between ID reservation, pending-write persistence, output replacement, and final mapping commit; restart keeps IDs unique and never skips incomplete work |
| A08 Cancellation | Cancel during initialization and processing leaves no partial success record or changes in other pairs; double-clicks, pair switches during work, and second app instances do not create concurrent writers |
| A09 Sidecar | Exercise real serialized requests, invalid payloads, EOF, crashes, timeouts, late replies, and shutdown; bounded restart does not duplicate commits |
| A10 Review | Add/dismiss/change-type decisions regenerate coherent offsets/counts; reopen preserves bound decisions; changing source/config invalidates them; approvals bind final output bytes |
| A11 Export | Only the selected pair's current approved files reach an empty directory outside every saved pair; mixed-pair selections and stale/tampered outputs are blocked; no mappings, notes, identifying rule text, or originals are copied |
| A12 Privacy | Canary identifiers in paths, invalid inputs, custom rules, and forced errors do not occur in logs/frontmatter; Markdown cannot execute or load remote content |
| A13 Audit | Successful/failed/cancelled attempts carry the correct opaque pair ID without its name; concurrent append attempts and interrupted recovery preserve valid prior entries; unwritable log produces a visible warning |
| A14 Windows artifact | Fresh standard-user Windows environment, no dev tools, runtime network disabled: launch, scan, process, review, export, restart, and exit work from the shipped archive; Onyx controls, styles, fonts, icons, and German text render correctly in the packaged WebView2 runtime with no remote asset requests |
| A15 Usability | Main workflow is keyboard-operable, controls are labeled, progress/errors are announced, focus is visible, and state is understandable without color alone |
| A16 Pair lifecycle | Add, rename, select, restart, and remove pairs; duplicate names are rejected; selection persists; removal leaves every data file intact; re-adding a managed source restores its identity and enforces its target binding |
| A17 Pair isolation | Two pairs both use `doc-0001` with different rules; processing, review, retry, and export never mix records; switching the shared sidecar removes previous custom rules and reloads a different model |
| A18 Pair routing | Unknown pair IDs and mismatched mapping/review/output ownership are rejected; late results cannot populate another pair's UI; unsaved edits require save/discard before switching |

Use generated synthetic fixtures for automated tests, including input strings
designed to exercise errors and leakage. Include focused real-engine tests in
addition to mocked detections; a fake analyzer cannot establish that the packaged
German recognizers work. The private representative corpus is evaluated locally
by the user and never committed, uploaded, or logged.

Frontend verification includes Vue/TypeScript type checking with `vue-tsc`, a
Vite production build, and focused interaction checks for pair selection,
configuration validation, progress/cancellation, and unsaved review changes.

## 12. Release evidence and performance

Use a Windows 11 x64 reference PC with four CPU cores, 16 GB RAM, and an SSD,
with no GPU requirement. Record exact hardware, model/dependency versions,
document counts, page/character distribution, cold-start time, total processing
time, and peak memory. The 400-document/30-minute target excludes human review
time. Ordinary controls should respond within 200 ms while work runs off the UI
thread; large document views must not freeze the window.

Before release, all acceptance checks pass on the packaged artifact or the
relevant component. Independently inspect at least 10% of the representative
corpus plus every warning/failure case locally. Report missed identifiers and
false positives by category; resolve known blocking cases and retest. A clean
sample does not establish zero leaks in uninspected documents.

The release record includes the artifact checksum, test results, documented
operating limits, local corpus evaluation summary without content, performance
measurements, and remaining limitations. Any untested Windows behavior or unmet
acceptance criterion means the initial release is not yet complete.

## 13. Prototype evidence and fresh-project policy

Inspection source: the working tree at `~/projects/redactio`, reviewed on
19 September 2026, including uncommitted files. Its README and UI describe an
earlier state than its code. No build/test success is inferred from inspection.

| Observed area | Evidence and release implication |
| --- | --- |
| Folder settings, scan, mapping | Rust modules `settings.rs`, `scan.rs`, `mapping.rs`, `paths.rs`; existing basis for folder workflow and document IDs |
| Batch and progress | `sync.rs`, `commands.rs`, frontend run/list components; code exists, but skip logic currently considers source hash without configuration/output validity |
| Redaction | Uncommitted `redaction.py` and modified `handlers.py`; fixed model/entity set and per-occurrence numbering, requiring consistency/overlap changes |
| Extraction | `extract.py` reads main paragraphs only; tables and other content coverage need explicit treatment |
| Output metadata | `frontmatter.py` defines typed entries, summaries, and review states; no integrated review flow yet |
| Audit and restart | Uncommitted `audit_log.rs` and modified `sidecar.rs`; intended behavior exists in code, but failure visibility/log sanitization still need verification |
| Delivery | Tauri bundling is disabled and sidecar startup targets the development checkout; no complete distributable is established |
| Tests | Rust/Python tests exist for selected domains; no claim is made here that the uncommitted working tree passes them |

The new repository begins with this specification and a new root commit. Do not
import the prototype's Git ancestry, process archive, iteration prompts/logs,
historical review gates, stale status copy, or agent-specific process rules.
Retain the MIT attribution when reusing code. Useful implementation may be ported
after this release contract is approved, with comments and documentation written
for the final behavior. No migration of old development-process documents is
needed, and this document is the sole initial-release requirements reference.
