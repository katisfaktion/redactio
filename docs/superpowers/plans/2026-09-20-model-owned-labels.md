# Model-owned detection labels

The user approved preserving native model labels and exposing each model's own
label set. This updates the existing detection/settings/review flow. No new model
downloads or dependencies are needed.

## Shared contract and constraints

- `ModelInfo.entity_types: string[]` contains the installed model's native entity
  labels, sorted and unique. Read BiomedBERT `config.json` `id2label` (strip BIO
  prefixes, omit O). Missing or malformed label
  metadata makes the model incompatible, with an empty list. Discovery is offline
  and never loads model weights.
- Entity identifiers across Python, Rust, and TypeScript become validated strings
  matching `[A-Z][A-Z0-9_]{0,63}`. This is transport validation, not a global list
  of model capabilities. Validate configured native labels against that model's
  metadata; reject duplicates and unsupported choices.
- Add optional `ProcessingConfig.model_entities: string[] | null`. Missing/null
  retains the exact existing legacy recognizer mapping and behavior, so old
  settings and reviews remain usable. A list, including an empty list, opts into
  native labels. `enabled_entities` then controls only the six supplementary
  recognizers: EMAIL_ADDRESS, PHONE_NUMBER, IBAN_CODE, IP_ADDRESS, URL, DATE_TIME.
  CUSTOM remains available for manual/custom rules, not an automatic category.
- Native BiomedBERT accepts every declared model label, filters only explicit
  user choices, and preserves label IDs and scores. Merge adjacent/overlapping
  fragments of the same label only; never collapse different labels into PERSON
  or LOCATION. Keep the existing offline CPU pipeline and window handling.
- Follow-up user decision: retire core-news models from discovery, processing,
  setup and packaging. BiomedBERT becomes the default and its runtime is required.
  Existing core-news pairs require an explicit switch/reprocessing; do not silently
  rewrite their stored model or results. Presidio may still use spaCy as a library.
  New pairs get all native labels from installed metadata at the command boundary.
- Custom rules do not re-enable disabled automatic types. Review validates
  automatic detections against the configured recognizers/types, while manual
  labels may use this model's full label set, supplementary/legacy labels, CUSTOM,
  and configured custom-rule labels. Preserve checksum, offset, export, and
  unsaved-review safeguards.
- Preserve the legacy engine fingerprint; distinguish native label semantics in
  the engine fingerprint so saving native selections makes results stale through
  the existing revision mechanism. Never rewrite saved detections or approvals.
- Settings gets a model-label checkbox section and a separate supplementary
  recognizer section, with select-all/clear controls for the model section.
  Selecting a different model defaults to its complete native label set. For a
  legacy pair, stage all native labels in the settings draft while respecting
  disabled legacy name/address groups; keep supplementary choices. Nothing takes
  effect until explicit Save. Explain that saving requires reprocessing existing
  documents. A draft selection persists when switching models within the form.
- Review/custom-rule choices come from current model metadata, supplementary and
  legacy labels, and labels already present in the review/rules. Display useful
  German names together with native codes; unknown valid labels fall back to the
  code. Do not create a global capability enum or a 54-item hardcoded model list.
- Continue on the existing feature branch and PR. Never open private documents,
  control the user's app, or launch a native GUI. Use synthetic evidence only.
- All shell commands use `rtk`; no global installs. Focused regression tests,
  frontend/engine/host checks, real pinned-model synthetic checks, and a scoped
  independent review are required. Build and update the existing Windows manual
  launcher after committing verified source.

## Task 1: Python detection engine

Own apps/sidecar only. Implement metadata discovery, validated string labels,
optional native selections, BiomedBERT native output, retired core-news handling,
legacy compatibility, supplementary recognizers, and native review/export.
Test dynamic metadata, native FIRSTNAME/LASTNAME/ZIPCODE plus previously dropped
AGE/ORGANIZATION, disabled labels, custom rules, model switching, malformed label
IDs, and legacy replay. Run engine tests, Ruff and mypy, plus pinned offline model
checks where prepared. Report actual model omissions separately from adapter loss.

## Task 2: Rust host

Own apps/desktop/src-tauri only. Implement validated dynamic labels, optional
config field and metadata-only model label discovery. Preserve stored settings
serialization when the optional field is absent, strict IPC validation and review
checks. Test a label outside the current models to prove no global allowlist,
malformed metadata/labels, legacy settings, native selections affecting revisions,
and native detection/review wire data. Run Rust checks. No GUI launch.

## Task 3: Vue settings and review

Own apps/desktop/src only. Update contracts for the shared fields, drive model
checkboxes from metadata, preserve drafts by selected model, show supplementary
recognizers separately, and carry native labels through previews/manual review.
Add shared display-label helpers with safe code fallback, not capability lists.
Keep Onyx components and responsive layouts. Test metadata with a previously
unknown valid label, 54-label model controls, per-model switching, save/preview,
legacy configuration draft, and native review projection. Run frontend tests/build.

## Task 4: Integration and handoff

Check Python/Rust metadata parity against the prepared model. Run a
synthetic native-label process/review/export flow and compare raw model spans to
adapter output. Review the combined diff, fix findings, update development docs
and PR, capture synthetic UI evidence, then update the native Windows manual
build from the final commit. Preserve the existing launcher environment and
previous binaries; do not launch it.

## Verification

- 141 frontend tests and the production build pass. Synthetic browser checks
  cover all 54 label controls, select-all/clear, native review labels, both themes,
  and narrow layouts without horizontal overflow.
- 158 engine tests pass with the pinned offline model enabled; Ruff, formatting,
  mypy, and locked dependency checks pass. Direct model comparison retains all
  17 predictions in the synthetic document. A review recorded by the previous
  committed engine replays with identical Markdown and engine identity.
- The regular Rust suite and all six real-sidecar integration tests pass.
  Independent review found and fixed native-label ordering on Save, migration
  from retired models, and preservation of disabled legacy name/address groups.
- Native Windows CLI processing preserves FIRSTNAME, LASTNAME, and ZIPCODE;
  review replay and source checksums match. No private documents or GUI were used.
- A newly frozen BiomedBERT portable package and representative private-document
  recall remain separate release acceptance checks.
