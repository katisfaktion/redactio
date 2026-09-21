# Redactio — model catalog and compatible NER imports

Status: **written for review** · 21 September 2026

## Purpose and agreed scope

Distribute Redactio without bundling every NER model. Users choose and download
models from a small catalog and can additionally import compatible models by
pasting a Hugging Face repository URL. Installed models run locally and remain
usable when the portable Windows app is copied to a USB stick.

The user approved catalog downloads plus compatible NER imports. The successful
USB demonstration establishes portability as an existing workflow to preserve.
This design assumes models remain in the app's `models` directory; it does not
introduce a separate user-profile cache or a model-storage preference.

Once approved, this specification supersedes the initial-release requirements that every
distribution bundle a default model and that runtime downloads never occur.
Only explicitly requested model metadata checks and downloads may use the
network. Document processing, previews, and review remain local. Opening the app,
opening a collection, or discovering a missing model never starts a download.

## User experience

### Models page and first launch

Add an app-wide **Modelle** page, accessible without a folder pair. Use the
existing Onyx navigation, cards, status tags, progress, and form controls, with
the current appearance and spacing conventions. This page is independent of the
selected collection; the pair selector does not imply that installations belong
to that pair.

A model-free app opens successfully and offers **Modell auswählen**. Users can
inspect settings and existing pairs. Creating a new pair requires an installed,
ready model and offers a direct route to the Models page. When multiple models
are installed, pair creation lets the user choose one explicitly; when only one
is installed, it is shown as the selected choice. No nullable processing config
or silently substituted model is introduced.

The catalog initially contains the two models already supported:

| Model | Pinned catalog revision |
| --- | --- |
| `OpenMed/OpenMed-PII-German-BiomedBERT-Large-340M-v1` | `ce797d58600cc20bba9a2500dafc0b7f5c3270c1` |
| `HuggingLil/pii-sensitive-ner-german` | `6af88facbb75da7be737da55d2c411c7ce79e5a1` |

Each entry shows its name, publisher, source link, revision, required download
size, native entity labels, installation state, and available action. Display
license metadata with a source link when available; absent metadata is shown as
unspecified, never inferred. The shipped catalog works offline and changes with
app releases. Catalog availability means tested integration, not a guarantee of
recognition quality. Do not rank models using invented accuracy scores.

### Import from Hugging Face

The Models page also offers **Hugging-Face-Modell hinzufügen**:

1. Paste `https://huggingface.co/<owner>/<repository>` and press **Prüfen**.
   Inspecting metadata is an explicit network action. Accept an optional trailing
   slash; reject credentials, query strings, fragments, file URLs, other hosts,
   and paths into repository files or branches with a specific explanation.
2. Resolve the repository's default branch to an immutable commit and inspect
   only that revision. Show the source, revision, size, native labels, license
   metadata, and any compatibility rejection before downloading weights.
3. Press **Herunterladen** to install the exact inspected revision. If it is
   already installed, show that entry instead of downloading another copy.
4. Show byte progress, then **Wird geprüft**, then **Bereit** only after integrity
   checks and a local runtime check have succeeded.

Public repositories are supported initially. Private or gated repositories get
a clear unsupported-access message; do not ask for or store account tokens.
Do not scrape arbitrary pages or let users provide download URLs for individual
files. Repository text is untrusted display data, not instructions or executable
HTML. A loadable model can still perform poorly; its existing local preview is
the place to try recognition after installation.

### Progress, selection, and removal

Downloads offer cancel and retry. Cancellation leaves all ready models usable.
Retry may reuse complete, verified staged files, but restarts an incomplete file;
byte-range resume is outside this version. Failure explains the actionable cause
in German, such as insufficient space, inaccessible repository, incompatible
model, interrupted connection, or corrupt download.

Pair settings list only ready installed models, with a link to model management.
Model labels continue to come from the selected model. Refreshing installation
status must preserve unsaved model selection, label choices, and custom rules.
Installing a model does not select it for existing pairs or change their
processing revisions. Saving a different pair model uses the existing explicit
config-change and reprocessing behavior.

A missing model leaves an existing pair and its settings intact. Actions needing
that model explain which exact revision is missing and offer its installation.
Do not silently select another model or claim review replay works without the
model required by the current review implementation.

Removal shows the model and occupied size. Block removal while any saved pair
selects that model and list the affected pairs. Also block removal while the
model is in use. Retain the small source/revision receipt after removal so the
exact version can be reinstalled. Previously generated outputs and review records
are never deleted by model removal; a later replay may require reinstalling its
recorded revision. Removing an installed model never removes document files.

## Compatibility contract

An import must satisfy all of these conditions:

- It is a Transformers token-classification model supported by the packaged
  runtime: initially `bert` / `BertForTokenClassification` or `deberta-v2` /
  `DebertaV2ForTokenClassification`, with a 512-token position limit.
- Weights are a single `model.safetensors` file. Sharded weights and other weight
  formats are rejected in this first version.
- It supplies a usable fast tokenizer, including `tokenizer.json` and the
  configuration/assets required by the packaged tokenizer implementation.
  Tokenization and inference need no repository Python code or extra packages.
- The classifier has a consistent, complete label mapping and matching output
  dimensions. Supported labels are `O` plus BIO labels or unprefixed entity
  labels. Entity names fit the existing `[A-Z][A-Z0-9_]{0,63}` contract. Unsupported
  tagging schemes and anonymous `LABEL_0`-style mappings are rejected with a
  reason; no guessed semantic mapping is introduced.
- A local load and synthetic offset check pass with the same tokenizer,
  aggregation, and overlapping-window behavior used for document processing.

Use `trust_remote_code=False`, `use_safetensors=True`, and local-only loading for
validation and inference. No automatic dependency installation, pickle fallback,
custom remote architecture, or tokenizer fallback is permitted. Compatibility
comes from the config, files, and actual runtime check, not model-card tags.

Preserve native labels exactly after removing the supported BIO prefix. Unknown
display labels fall back to their native code. Keep supplementary recognizers
separate, as they are today. Installing a model must not collapse FIRSTNAME,
LASTNAME, CITY, or ZIPCODE into broader UI categories or discard their results.

The synthetic check covers Unicode, emoji, CRLF, and text spanning overlapping
windows. Validate tokenizer offsets and the bounds and source-text correspondence
of returned detections. It must not require the model to recognize a particular
name or city: an empty detection result is not by itself incompatibility. These
checks establish integration correctness, not detection accuracy.

## Runtime and storage design

### Reuse the host and bundled worker

The Vue frontend uses validated Tauri commands and events; it never downloads
model files directly. The Rust host owns operation admission, the model-store
lock, progress forwarding, cancellation, and UI-safe error codes.

Add a distinct model-management mode to the existing bundled Python executable.
It receives only the app model root and a validated model-management request,
never document content, source/output folders, preview text, or collection rules.
Reuse the standard-library transfer, hashing, staging, and atomic-write pattern
from `scripts/prepare-biomedbert.py` and the installed tokenizer/model runtime.
Keep downloads out of the document-processing IPC handler. The setup script
should call the shared installation logic instead of maintaining a second
downloader. No new service, package manager, or generic plugin framework is needed.

Serialize model mutations across app instances sharing a model root. Within an
app, install/validation/removal and document engine operations do not overlap;
reuse the existing busy-state convention with a precise explanation. Listing
metadata remains available. Release idle inference workers before validation or
removal so Windows file locks and two loaded large models do not obstruct the
operation. Model-use tracking must also prevent another app instance from
removing files used by a live inference worker. Navigation retains the existing
unsaved-review guard.

The catalog is derived from the existing pinned packaging inputs, supplemented
with verified display metadata and sizes. Maintain one catalog source rather
than separate Python and Rust lists of allowed repositories. Runtime architecture
support remains an explicit capability check. A previously unknown repository
that meets the contract must work without adding it to source code.

### App-local model store

Use `<app>/models` for installed files, staging, catalog import receipts, and the
model registry. Release resource resolution requires the bundled runtime, not a
specific model or pre-existing models directory. A missing directory represents
an empty installation. Installed models can be used from read-only storage;
installation/removal requires writable storage and explains how to move the app
to a writable folder when needed. Do not silently fall back to a profile cache.

Use a versioned registry at `models/manifest.json` containing source, immutable
revision, stable selection identity, architecture/tokenizer facts, native labels,
relative installation path, artifact sizes/hashes, and installation receipt.
An uninstalled retained receipt is distinguishable from a ready installation.
It contains no collection data. Keep paths relative and generated from validated
identities so relocating the whole app does not invalidate model discovery.

Read the existing unversioned manifest and existing `redactio-model.json` files
without requiring a migration download. Upgrade the registry atomically on the
first model-store mutation, preserving existing directories. Legacy stores remain
readable without writes. Treat unsupported future registry versions as explicit
errors rather than overwriting them.

New selection identities use the full repository and commit, avoiding collisions
between publishers and allowing revisions to coexist. For the two existing
pinned models, preserve their legacy selection names, exact model versions,
recognizer identities, and review fingerprints; importing those exact revisions
deduplicates against the catalog entry. Later revisions get new identities.
Never rewrite old pair configs or saved review metadata simply to modernize an ID.
New imports use one app-defined public recognizer name; the model's identity is
already recorded separately. Extend the output metadata allowlist for that name
without accepting recognizer names supplied by a remote repository.

### Transfer, integrity, and publication

Resolve the repository once during preflight and pin every metadata and artifact
request to the same commit. The install request refers to that checked result;
the worker validates it again rather than trusting arbitrary frontend file lists.
Show the actual required artifact total, check available disk space, and handle
write failures without modifying installed models.

Download only allowlisted config/tokenizer files and safetensors weights into an
app-local staging directory. Never download Python code or archives for
execution/extraction. Bound metadata size, redirects, and timeouts; accept HTTPS
only and validate redirects against the supported Hugging Face delivery hosts.
Reject local/private-network destinations, unsafe filenames, path traversal,
symlinks, and Windows reparse points. Signed download URLs and authentication
details must not appear in logs or public errors.

Verify catalog files against the bundled SHA256 values. For imported files,
verify upstream object identifiers using the appropriate Git-blob or LFS hash
scheme and record local SHA256 values for subsequent checks. Refuse a source
whose required artifact identity cannot be verified. A recorded hash proves byte
identity, not publisher trust or recognition quality.

Load and validate from staging without network access. Publish files by an atomic
same-filesystem rename, then atomically update the registry to mark them ready.
A crash between those steps leaves an unlisted directory, never a selectable
partial installation; retry revalidates or cleans up that owned directory.
Registry discovery ignores staging and unlisted files. Incomplete-file cleanup
is restricted to manager-owned paths. Persist no ready state on cancellation,
hash mismatch, validation failure, or worker crash.

Removal first makes the installation unavailable in the registry, then deletes
only its owned files. Interrupted or failed deletion retains enough state for
retry and reports any remaining disk usage. The remembered source receipt allows
explicit reinstallation; no background repair download occurs.

## Existing behavior and packaging

Model management must not change detection thresholds, aggregation, offsets,
legacy label migration, or redaction behavior for the two current pinned models.
Do not bump their engine fingerprint merely for a discovery/storage refactor;
prove unchanged review replay with existing regression cases. If a real inference
change is needed, it requires an explicit compatibility decision, not a hidden
rewrite of saved fingerprints.

The standard Windows package includes the desktop app, fixed WebView2, frozen
Python runtime/dependencies, and catalog metadata, with zero model weights.
Keep optional preloading for an offline demonstration using the same model store
and installer. Once installed, copying the whole app folder carries its models;
existing collection-settings storage is outside this change.

The packaged runtime must include both supported architecture/tokenizer paths,
the dynamically required spaCy support packages discovered during the USB build,
and their notices. Update packaging checks so a model-free package is valid while
each explicitly preloaded model is still verified. Small distribution means
removing optional weights; the ML runtime itself remains substantial.

## Acceptance and verification

Use synthetic documents and model fixtures only. Do not inspect private documents,
control the user's running app, or launch its native GUI for verification.

- Model-free startup reaches model management without a setup error. Existing
  pairs with missing models remain visible, unchanged, and actionable.
- Catalog metadata works offline. Install both catalog entries independently;
  selecting either preserves all of its native label options.
- Import an unfamiliar compatible repository without a code change. Exercise
  rejection of unsupported architecture, missing tokenizer, invalid labels,
  unsafe weights, private/gated access, and unsafe URLs/paths.
- With a controlled HTTP transport fixture, verify pinned-revision requests,
  redirected downloads, hashes, cancellation, retry, low-space/write failures,
  worker termination, and interruption during publication/removal. No failing
  path leaves a ready partial model or damages another installation.
- Test cross-instance model-store locking and operation conflicts, including
  releasing loaded Windows files before removal.
- Refresh model status while pair settings contain unsaved changes; the draft
  stays intact. Installing an unrelated model leaves pair revisions and saved
  review fingerprints unchanged. Legacy manifests and review replay still work.
- Validate Unicode and long-text offsets through the installed runtime, then
  process and correct synthetic documents with both current real models.
- Build a Windows package with no weights and one with preloaded models. Check
  frozen imports and notices. After one explicit installation, run native engine
  checks with networking unavailable and cold external caches. Relocate the
  folder and repeat discovery/processing to verify USB portability.
- Hand the GUI checks to the user: first launch, catalog/import progress,
  cancellation, pair selection, removal feedback, and offline use from USB.

## Deferred scope

No automatic model updates, remote catalog refresh, in-app repository search,
gated/private authentication, arbitrary architectures, sharded weights, custom
repository code, automatic package installation, resumable partial files,
multiple simultaneous downloads, or recognition-quality benchmark suite.
Add support for a new runtime family when a concrete desired model needs it.

## Source references

- [Transformers model loading and custom models](https://huggingface.co/docs/transformers/en/models)
- [Token-classification pipelines](https://huggingface.co/docs/transformers/en/main_classes/pipelines)
- [Hugging Face revision-pinned downloads](https://huggingface.co/docs/huggingface_hub/en/guides/download)
- [Hugging Face guidance on pickle security](https://huggingface.co/docs/hub/en/security-pickle)
