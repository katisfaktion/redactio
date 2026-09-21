# Model Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users install catalog models and compatible Hugging Face NER models, with model-dependent context windows and continued offline USB use.

**Architecture:** Keep network operations in a separate mode of the existing Python executable, supervised by the Rust host. A versioned app-local model registry replaces repository allowlists while retaining legacy identities. Vue presents one global Models page and selects ready models per pair.

**Tech Stack:** Existing Vue 3, TypeScript, sit-onyx, Zod, Tauri/Rust, Python standard library, Pydantic, Transformers, and safetensors. Use the existing pytest, Vitest, Rust, and PowerShell checks.

**Spec:** [Approved model-management design](../specs/2026-09-21-model-management-design.md)

## Global Constraints

- "Only explicitly requested model metadata checks and downloads may use the network."
- "Document processing, previews, and review remain local."
- "Use `trust_remote_code=False`, `use_safetensors=True`, and local-only loading for validation and inference."
- "Public repositories are supported initially."
- "Weights are a single `model.safetensors` file."
- Supported runtime families remain `bert` / `BertForTokenClassification` and `deberta-v2` / `DebertaV2ForTokenClassification`.
- "Its context limit is validated from the model configuration; it need not equal 512 tokens."
- "For the existing pinned models, preserve the current 512-token window and 128-token overlap."
- "Entity names fit the existing `[A-Z][A-Z0-9_]{0,63}` contract."
- "Installing a model does not select it for existing pairs or change their processing revisions."
- "Use `<app>/models` for installed files, staging, catalog import receipts, and the model registry."
- "Legacy stores remain readable without writes."
- "Use synthetic documents and model fixtures only. Do not inspect private documents, control the user's running app, or launch its native GUI for verification."
- Continue the existing `feat/redactio-initial-release` branch and PR #1; preserve the docs-only main history. All shell commands start with `rtk`; do not install global tools.
- Keep interpreter/runtime and dependency versions pinned. Model import never installs packages. No private tokens, automatic updates, sharded weights, parallel downloads, or alternative storage settings.

## Review Focus

1. A tokenizer advertises an effectively infinite limit or conflicts with the model: use the validated smaller usable window, never silently truncate. Task 1.
2. The repository branch changes between inspection and installation: every artifact still comes from the inspected commit. Task 3.
3. Windows closes a download during publication or another app has a model loaded: partial installations stay unavailable and ready models stay intact. Tasks 3–4.
4. A model-status response arrives after the user changes a settings draft or starts another download: preserve the draft and ignore obsolete progress. Tasks 5–6.
5. An existing USB installation is relocated or made read-only: discover its legacy models without writes, preserve old reviews, and explain unavailable mutations. Tasks 2 and 7–8.

## Execution order and ownership

The previously selected execution method is **subagent-driven, parallel where possible**. Preserve it after plan approval; do not ask the user to select it again.

Run Tasks 1 and 2 sequentially to establish the runtime and shared contracts. Then Tasks 3, 4, and 5 can run in parallel: Python installer, Rust controller, and Vue management UI have separate owned files and share the contracts below. Task 4 tests against a fake worker until Task 3 is integrated. Task 6 follows Tasks 4–5. Task 7 follows Task 3 and may run alongside Task 6. Task 8 integrates and reviews the result. Each task gets its focused test cycle and a commit; do not create a commit solely for scaffolding.

File responsibilities:

| Files | Responsibility |
| --- | --- |
| `apps/sidecar/src/redactio_sidecar/model_store.py` | Context calculation, catalog/registry DTOs, validation, migration, safe atomic registry writes |
| `apps/sidecar/src/redactio_sidecar/model_catalog.json` | Single source of pinned catalog metadata, moved from existing packaging inputs |
| Existing `biomedbert.py`, `engine.py`, `frontmatter.py` | Generic validated local model loading, native detections, legacy identity preservation |
| `apps/sidecar/src/redactio_sidecar/model_manager.py` | Explicit preflight/download/validation/removal worker and safe transfer helpers |
| `apps/desktop/src-tauri/src/model_store.rs` | Registry/catalog projection and model file leases |
| `apps/desktop/src-tauri/src/model_manager.rs` | Worker process, job state, cancellation, progress and management commands |
| Existing Rust `resources.rs`, `commands.rs`, `sidecar.rs` | Model-free resources, pair admission, inference lifetime integration |
| `apps/desktop/src/lib/modelContracts.ts`, `modelIpc.ts` | Strict management-only IPC types and wrappers |
| `apps/desktop/src/composables/useModels.ts`, `components/ModelManager.vue` | Global model state and Onyx management page |
| Existing pair/detection components and composables, `App.vue` | Navigation, explicit pair model selection, preserved drafts |
| Existing packaging scripts, checks and docs | Model-free ZIP, optional preloading, native verification and user instructions |

Do not refactor unrelated run, review, extraction, or redaction code.

## Shared contracts

Task 2 implements these types in Python, Rust, and Zod, with strict unknown-field rejection and a shared JSON fixture at `tests/fixtures/model-management.json`. Keep existing `ModelInfo {name, version, compatible, entity_types}` unchanged for inference/review IPC.

```ts
type Artifact = {
  filename: string;
  size: number;
  upstream_hash: { algorithm: "git-sha1" | "sha256"; value: string };
  sha256: string | null; // null only before an imported file is downloaded
};
type ModelDescriptor = {
  name: string; version: string; repository: string; title: string;
  license: string | null;
  model_type: "bert" | "deberta-v2";
  architecture: "BertForTokenClassification" | "DebertaV2ForTokenClassification";
  entity_types: string[];
  window_tokens: number; stride_tokens: number; special_tokens: number | null;
  files: Artifact[];
};
type CatalogEntry = {
  key: "biomedbert" | "hugginglil";
  directory: string;
  descriptor: ModelDescriptor;
};
type ModelRecord = {
  descriptor: ModelDescriptor;
  path: string | null;
  state: "ready" | "available" | "removing";
};
type LegacyEntry = { name: string; version: string; path: string };
type ModelRegistry = {
  schema_version: 2; models: ModelRecord[]; legacy_unavailable: LegacyEntry[];
};
type ManagedModel = {
  name: string; version: string; repository: string; title: string;
  license: string | null; entity_types: string[];
  window_tokens: number | null; download_bytes: number; installed_bytes: number;
  state: "available" | "ready" | "invalid" | "removing";
  catalog_key: "biomedbert" | "hugginglil" | null;
  used_by_pairs: { id: string; name: string }[];
  error: SafeError | null;
};
type ModelSource =
  | { kind: "catalog"; key: "biomedbert" | "hugginglil" }
  | { kind: "url"; url: string }
  | { kind: "receipt"; name: string };
type CheckedModel = { plan_id: string; model: ManagedModel };
type ModelJob = {
  job_id: string; model_name: string;
  stage: "downloading" | "validating" | "ready" | "cancelled" | "failed" | "removing" | "removed";
  downloaded_bytes: number; total_bytes: number; error: SafeError | null;
};
```

`SafeError` is the existing `{code: string, retryable: boolean}`. UUID strings identify plans/jobs/pairs. Byte counts are nonnegative safe integers; revisions are lowercase 40-digit commit hashes. Entity arrays are nonempty, sorted and unique. Window/stride values must satisfy Task 1. Relative paths are one generated directory component, never publisher-supplied paths. Registry `ready` requires a path, known special-token overhead and fully verified hashes. Unknown future versions fail without rewriting files. The catalog file is `{schema_version: 1, models: CatalogEntry[]}`.

A checked descriptor's window is provisional until local tokenizer validation supplies special-token overhead. The installer recomputes it before publishing. No provisional descriptor becomes a ready registry record.

Host commands:

```text
list_managed_models() -> ManagedModel[]
check_model(source: ModelSource) -> CheckedModel
start_model_install(plan_id: UUID) -> UUID
cancel_model_job(job_id: UUID) -> ()
get_model_job(job_id: UUID) -> ModelJob | null
remove_model(name: string) -> UUID
event "model-progress" -> ModelJob
```

The host retains one checked install plan, including its descriptor and artifact list, in memory. A new successful check replaces it; the frontend can submit only its ID. Catalog checks use shipped metadata without network access. URL checks perform explicit metadata requests. Receipt checks preserve the recorded commit. An install captures the checked plan before starting and never resolves `main` again.

The management worker uses a separate process invocation, `--manage-models --model-dir <root>`, a single JSON request on stdin, and bounded JSON lines on stdout. Requests are `{id, type, payload}`, with types `check_model`, `install_model`, `remove_model`; payloads carry respectively `ModelSource`, `ModelDescriptor`, or a model name. Replies use `{id, type, payload}`, where type is `result`, `progress`, or `error`. A check returns a descriptor; install returns the verified descriptor; remove returns the retained available record. Progress carries `ModelJob`; errors carry `SafeError`. Request ID becomes job ID for mutations. Diagnostic stderr never becomes a user-facing error.

## Task 1: Adapt processing windows and expose generic local validation

**Files:** Create `apps/sidecar/src/redactio_sidecar/model_store.py`, `apps/sidecar/tests/test_model_store.py`; modify `apps/sidecar/src/redactio_sidecar/biomedbert.py`, `apps/sidecar/tests/test_biomedbert.py`.

**Interfaces:** Produce `Window(tokens: int, stride: int)`, `processing_window(model_limit: object, tokenizer_limit: object, special_tokens: int) -> Window`, and `validate_local_model(path: Path, model_type: str, architecture: str) -> Window`. The latter performs safe loading and the synthetic tokenizer/inference check, raising `EngineError("model_incompatible")` on incompatibility.

- [ ] Add this parameterized regression to `test_model_store.py`:

```python
import pytest
from redactio_sidecar.model_store import Window, processing_window

@pytest.mark.parametrize("model,tokenizer,special,expected", [
    (512, 512, 2, Window(512, 128)),
    (256, 256, 2, Window(256, 64)),
    (1024, 1024, 2, Window(1024, 256)),
    (2048, 1024, 2, Window(1024, 256)),
    (1024, int(1e30), 2, Window(1024, 256)),
    (1024, None, 2, Window(1024, 256)),
    (8, 8, 6, Window(8, 1)),
])
def test_model_dependent_window(model, tokenizer, special, expected):
    assert processing_window(model, tokenizer, special) == expected

@pytest.mark.parametrize("model,tokenizer,special", [
    (None, 512, 2), (0, 512, 2), (True, 512, 2),
    (512, -1, 2), (512, "512", 2), (2, 2, 2), (512, 512, -1),
])
def test_invalid_capacity_is_explicit(model, tokenizer, special):
    with pytest.raises(ValueError):
        processing_window(model, tokenizer, special)
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run --locked --offline pytest tests/test_model_store.py -q`; confirm it fails because the new function is absent.
- [ ] Implement the pure calculation, using Transformers' installed sentinel convention without importing Torch into metadata discovery:

```python
from typing import NamedTuple

class Window(NamedTuple):
    tokens: int
    stride: int

def processing_window(model_limit: object, tokenizer_limit: object,
                      special_tokens: int) -> Window:
    if type(model_limit) is not int or not 0 < model_limit < 10**20:
        raise ValueError("invalid model context limit")
    if type(special_tokens) is not int or special_tokens < 0:
        raise ValueError("invalid special-token count")
    limit = model_limit
    if tokenizer_limit is not None:
        if type(tokenizer_limit) is not int or tokenizer_limit <= 0:
            raise ValueError("invalid tokenizer context limit")
        if tokenizer_limit < 10**20:
            limit = min(limit, tokenizer_limit)
    content = limit - special_tokens
    if content < 2:
        raise ValueError("unusable content window")
    return Window(limit, min(max(1, limit // 4), content - 1))
```

- [ ] Remove `model_max_length=512` and the equality-to-512 check from `_load_pipeline`; after loading the fast tokenizer and model safely, calculate and assign the validated window:

```python
window = processing_window(
    model.config.max_position_embeddings,
    tokenizer.model_max_length,
    tokenizer.num_special_tokens_to_add(pair=False),
)
tokenizer.model_max_length = window.tokens
# Keep CPU, aggregation_strategy="simple", and use window.stride in pipeline().
```

  Preserve architecture checks. Check loading information for missing/mismatched classifier weights rather than accepting a randomly initialized head. Leave both existing recognizer wrappers and their identities intact. Update synthetic config fixtures to contain valid architecture/capacity facts instead of relaxing checks for fixtures.
- [ ] Extend mocked-pipeline tests to assert the exact tokenizer window and stride supplied for 256/512/1024 contexts. Add this local runtime regression; it downloads nothing and requires no particular random-model prediction:

```python
def test_small_context_keeps_long_unicode_offsets(tmp_path):
    from transformers import BertConfig, BertForTokenClassification, BertTokenizerFast
    from redactio_sidecar.biomedbert import _load_pipeline, validate_local_model
    from redactio_sidecar.model_store import Window

    vocab = tmp_path / "vocab.txt"
    vocab.write_text("[PAD]\n[UNK]\n[CLS]\n[SEP]\n[MASK]\nhans\nende\n", encoding="utf-8")
    tokenizer = BertTokenizerFast(vocab_file=str(vocab), model_max_length=32)
    tokenizer.save_pretrained(tmp_path)
    config = BertConfig(vocab_size=len(tokenizer), hidden_size=16,
                        num_hidden_layers=1, num_attention_heads=2,
                        intermediate_size=32, max_position_embeddings=32,
                        id2label={0: "O", 1: "B-PERSON", 2: "I-PERSON"},
                        label2id={"O": 0, "B-PERSON": 1, "I-PERSON": 2})
    BertForTokenClassification(config).save_pretrained(tmp_path, safe_serialization=True)
    text = ("Hans München 😀\r\n" * 30) + "Ende"
    encoded = tokenizer(text, truncation=True, max_length=32, stride=8,
                        return_overflowing_tokens=True, return_offsets_mapping=True)
    assert len(encoded["input_ids"]) > 1
    assert max(end for chunk in encoded["offset_mapping"] for _, end in chunk) == len(text)
    assert validate_local_model(tmp_path, "bert", "BertForTokenClassification") == Window(32, 8)
    for detection in _load_pipeline(tmp_path)(text):
        assert 0 <= detection["start"] < detection["end"] <= len(text)
```

  Run the same production offset validation with a longer context fixture, and inspect overflow offset coverage rather than relying on the last predicted entity. Expose the check through `validate_local_model`; it must never use real document text.
- [ ] Run `rtk proxy uv --directory apps/sidecar run --locked --offline pytest tests/test_model_store.py tests/test_biomedbert.py -q`, Ruff, and mypy. Commit as `feat: adapt NER windows to validated model context limits`.

## Task 2: Establish the catalog, registry and compatible-model discovery

**Files:** Modify `apps/sidecar/src/redactio_sidecar/model_store.py`, `biomedbert.py`, `engine.py`, `frontmatter.py` and their existing Python tests; create `apps/sidecar/src/redactio_sidecar/model_catalog.json`, `tests/fixtures/model-management.json`, `apps/desktop/src-tauri/src/model_store.rs`, `apps/desktop/src-tauri/tests/model_store.rs`, `apps/desktop/src/lib/modelContracts.ts`, `apps/desktop/src/lib/modelContracts.test.ts`; modify Rust `resources.rs`, `lib.rs`, `tests/resources.rs`, `tests/common/mod.rs`; move the two `packaging/*-inputs.json` model records into the catalog and adjust their consumers in `scripts/prepare-biomedbert.py`, `scripts/package-windows.ps1`, `scripts/test_prepare_biomedbert.py`, `scripts/test_packaging.py`, and `packaging/build-inputs.json`. Add catalog package data to `packaging/sidecar.spec`.

**Interfaces:** Python produces `read_registry(root: Path) -> ModelRegistry`, `write_registry(root: Path, registry: ModelRegistry) -> None`, `catalog_models() -> list[CatalogEntry]`, and `selection_name(repository: str, revision: str) -> str`. Rust produces equivalent DTOs, `model_store::list_models(root: &Path) -> Result<Vec<ModelInfo>, AppError>`, and `model_store::list_managed(root: &Path) -> Result<Vec<ManagedModel>, AppError>`. Task 4 fills `used_by_pairs`; storage projection starts with an empty array. Zod exports schemas and inferred types for every management contract above.

- [ ] Write discovery tests for empty/missing stores, both legacy aliases, a new publisher/model with 1024 context, two publishers with the same basename, two revisions, removed receipts, invalid paths, invalid labels and unsupported future schema. Preserve unknown legacy entries in `legacy_unavailable` without advertising them as runnable or deleting their files. Add these concrete identity/empty-store checks:

```python
from redactio_sidecar.model_store import read_registry, selection_name

def test_empty_store_does_not_write(tmp_path):
    root = tmp_path / "models"
    assert read_registry(root).models == []
    assert not root.exists()

def test_repository_and_revision_are_part_of_new_identity():
    a = selection_name("one/model", "a" * 40)
    assert a != selection_name("two/model", "a" * 40)
    assert a != selection_name("one/model", "b" * 40)
    assert a == "hf:one/model@" + "a" * 40
```

  Port the shared fixture to Python/Rust/Zod tests and require identical names, labels, state and window metadata. Add `common::fixture_model_store(root: &Path) -> String` to the existing Rust test helper: write a valid ready fixture record in the `fixture-model` subdirectory, its config/tokenizer metadata, synthetic artifact bytes with matching recorded sizes/hashes, and immutable identity receipt; return its model selection name. It must not load weights. Test malformed variant fields in each boundary parser, not every serialization getter.
- [ ] Run the new focused tests and confirm failures before adding discovery. Keep the fixture's safetensors validation in Task 1/3; metadata-only discovery must never import model weights.
- [ ] Implement the schema above with strict Pydantic/serde/Zod validation. Migrate legacy records in memory only; atomic disk migration happens with a successful mutation. Resolve the two legacy aliases from their exact catalog repository/revision. Use `hf:<repository>@<commit>` for new selections and a generated hash-based directory name. Retain one immutable `redactio-model.json` identity receipt in every installed directory; Task 4 uses it for read-only model-use leases.
- [ ] Consolidate the existing catalog inputs, preserving every current file hash/revision. Supplement each entry with its key, title, license metadata, native labels, window facts, and verified file sizes from the pinned source. Project each descriptor's artifact list to the existing packaging `files` hash map until Task 7 changes packaging shape; do not store a second source copy. Python reads package data via `importlib.resources`; Rust embeds the same JSON with `include_str!`; the setup script imports the shared catalog. Update all old input-path references in the same commit so existing packaging checks continue to work.
- [ ] Replace `MODEL_SPECS` and Rust `SUPPORTED` repository gating with validated registry descriptors plus supported architecture checks. Keep legacy constants/wrappers as compatibility aliases to catalog-derived descriptors. Generic models use `TransformersNerRecognizer`; add exactly that public name to `frontmatter.py`, never arbitrary remote names. Engine configuration must require explicit native labels for all nonlegacy imports, not just HuggingLil.
- [ ] Let `resolve_packaged` return `<app>/models` when it does not yet exist, while retaining ancestor/link checks and required sidecar/WebView2 validation. Distinguish a missing store from a malformed existing manifest. An unrelated incomplete model must not prevent a different ready model from being discovered.
- [ ] Run Python store/engine/frontmatter tests, `rtk cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --test model_store --test resources`, and `rtk pnpm --filter @redactio/desktop test src/lib/modelContracts.test.ts`. Run the existing preparation/packaging script checks. Commit as `feat: discover catalog and imported models through a portable registry`.

## Task 3: Implement explicit preflight and installation in the bundled worker

**Files:** Create `apps/sidecar/src/redactio_sidecar/model_manager.py`, `apps/sidecar/tests/test_model_manager.py`; modify `apps/sidecar/src/redactio_sidecar/__main__.py`, `packaging/sidecar-entry.py`, `scripts/prepare-biomedbert.py`, `scripts/test_prepare_biomedbert.py`. Own these files while Tasks 4–5 run.

**Interfaces:** Consume Task 1 validation and Task 2 descriptors/registry. Produce `parse_repository_url(url: str) -> str`, `preflight(source: ModelSource, root: Path) -> ModelDescriptor`, `install_model(root: Path, descriptor: ModelDescriptor, job_id: str, emit: Callable[[ModelJob], None]) -> ModelDescriptor`, `remove_model(root: Path, name: str) -> ModelRecord`, `mutation_lock(root: Path) -> ContextManager[None]`, `exclusive_model_lock(root: Path, name: str) -> ContextManager[None]`, and `main(argv: list[str] | None = None) -> None`. Install/removal acquire their own native guards; removal additionally holds the exclusive per-model receipt lock. The standalone preparation command calls these same operations and generates a UUID for progress. Mutation locks use persistent `.redactio-models-lock` without unlinking it. Use POSIX `flock` and Windows native byte-range locking that conflicts with Rust's shared receipt lock; Task 4 proves contention rather than assuming it. Locks belong to the mutating process so a host crash cannot release them before a surviving worker exits.

- [ ] Start with URL and object-identity tests:

```python
import hashlib
import pytest
from redactio_sidecar.model_manager import parse_repository_url, object_digest

@pytest.mark.parametrize("url", [
    "http://huggingface.co/a/b", "https://huggingface.co.evil.invalid/a/b",
    "https://user@huggingface.co/a/b", "https://huggingface.co/a/b/tree/main",
    "https://huggingface.co/a/b?token=secret", "file:///tmp/model",
    "https://huggingface.co/a/%2e%2e", "https://127.0.0.1/a/b",
])
def test_reject_non_repository_urls(url):
    with pytest.raises(ValueError):
        parse_repository_url(url)

def test_repository_url_and_git_object_identity():
    assert parse_repository_url("https://huggingface.co/one/model/") == "one/model"
    data = b"test"
    assert object_digest(data, "git-sha1") == hashlib.sha1(b"blob 4\0test").hexdigest()
    assert object_digest(data, "sha256") == hashlib.sha256(data).hexdigest()
```

  Define `object_digest(data: bytes, algorithm: str) -> str` for bounded metadata tests; stream the equivalent algorithm for large files, never load weights into a byte string to hash them.
- [ ] Run `rtk proxy uv --directory apps/sidecar run --locked --offline pytest tests/test_model_manager.py -q`; confirm missing-function failures.
- [ ] Implement repository parsing with `urllib.parse`, strict component patterns, and no embedded credentials/ports/query/fragment. Obtain an immutable revision, then fetch the config and file-tree metadata at that revision using Hugging Face's official API. Use the existing installed `huggingface_hub` metadata types where useful, but no automatic cache download or credential discovery. Verify current official API/hash fields during implementation. Reject mismatched architecture, anonymous/malformed labels, remote-code requirements, gated access, unsafe weights, or missing tokenizer artifacts with specific safe error codes.
- [ ] Implement one checked stdlib HTTP opener: HTTPS, certificate verification, at most five redirects, 30-second connection/read timeouts, bounded JSON/config responses, and approved Hugging Face/CDN destinations. Validate every redirect and destination before connecting; public-address checks must apply to the addresses actually used, not just a preliminary DNS lookup. Use the packaged trusted CA data on Windows. Reject malformed sizes, ambiguous object hashes, and files outside the fixed config/tokenizer/weights allowlist. Never forward credentials or log signed URLs.
- [ ] With a fake transport at the opener boundary, return commit A for inspection then change the simulated default branch to B. Assert installation still requests only A. Add hash mismatch, truncated stream, larger-than-declared stream, redirect-to-private-host, malformed JSON, slow/stalled response, unavailable repo and source-ID mismatch cases. The test transport supplies bytes and headers only; it must not disable production URL/path checks.
- [ ] Stage under a generated `.model-<identity>` directory. Check `shutil.disk_usage`, verify each complete file using its upstream hash and catalog SHA256 when present, record SHA256, then call `validate_local_model`. Emit monotonic byte progress with the same job ID. A retry rechecks complete staged files and restarts an incomplete file. Before loading, validate actual config, tokenizer, head dimensions, native labels and window against the checked descriptor. Cancellation kills the separate worker; its staging stays unavailable and recoverable.

```python
# Publication order after all files and the runtime have been verified:
staging.rename(destination)
record = ModelRecord(descriptor=verified_descriptor, path=destination.name, state="ready")
registry.models = [item for item in registry.models if item.descriptor.name != record.descriptor.name]
registry.models.append(record)
write_registry(root, registry)
```

  Before that sequence, require an absent or positively identified manager-owned destination. On retry after a rename/registry-write interruption, verify the orphan destination instead of overwriting it or treating it as ready. Release loaded validation objects before renaming on Windows.
- [ ] Test install/verify/publication failures by injecting transport, validation and atomic-write failures. Assert the previous manifest bytes and ready model files stay unchanged; test cancel after a complete file and during validation, and retry after rename-before-registry. Removal persists `state="removing"` first, deletes only owned paths, then retains an available receipt with `path=null`; deletion failure remains retryable and reports remaining usage. Check symlinks and Windows reparse points in both staging and removal.
- [ ] Route the new CLI flag in both development and frozen entry points before importing the document engine. Keep existing document IPC unchanged and unable to dispatch management commands. The setup script becomes a thin catalog/source adapter to these functions, preserving its current `--model` choices. Its standalone mutation lock must contend with the Rust host's lock; verify that in Task 4.
- [ ] Run manager tests, existing IPC tests and `rtk proxy apps/sidecar/.venv/bin/python scripts/test_prepare_biomedbert.py`; run Ruff and mypy. Commit as `feat: install compatible NER models through an isolated worker`.

## Task 4: Supervise model jobs and protect model use in the host

**Files:** Create `apps/desktop/src-tauri/src/model_manager.rs`, `apps/desktop/src-tauri/tests/model_manager.rs`, `apps/desktop/src-tauri/tests/fake_model_worker.py`; modify `model_store.rs`, `commands.rs`, `lib.rs`, `sidecar.rs`, and `tests/resources.rs`, `tests/sidecar.rs`.

**Interfaces:** Implement the six management commands and `model-progress` contract above. Add `ModelManager::new(resources: ResourcePaths) -> ModelManager`, job storage and child ownership to `AppState`. Add `ModelUseGuard::acquire(root: &Path, name: &str) -> Result<ModelUseGuard, AppError>` for inference lifetime. The worker owns the exclusive removal lock. Keep `resources::list_models` as the compatibility-facing entry point delegating to Task 2.

- [ ] Add a fake worker accepting the specified request envelope. For install it emits downloading and validating messages followed by a result; selectable test modes block, exit early, send malformed/oversized JSON, or send a different ID. Tests must prove every failure releases the operation guard and never reports ready without a verified result and registry refresh. Use a real subprocess for cancellation and lock-lifetime tests.
- [ ] Add a read-only lease check using a synthetic legacy directory and its existing `redactio-model.json`. Holding an inference lease in one process must block removal in another; releasing the process permits removal. A second writer must fail while the root mutation lock is held, including a standalone Python preparation process. Verify no new file is required merely to read a legacy installation.

```rust
mod common;
use redactio_lib::model_store::ModelUseGuard;

#[test]
fn model_use_lease_blocks_removal_until_released() {
    let root = tempfile::tempdir().unwrap();
    let name = common::fixture_model_store(root.path());
    let receipt = std::fs::File::open(root.path().join("fixture-model/redactio-model.json")).unwrap();
    let lease = ModelUseGuard::acquire(root.path(), &name).unwrap();
    assert!(receipt.try_lock().is_err());
    drop(lease);
    receipt.try_lock().unwrap();
    assert!(ModelUseGuard::acquire(root.path(), &name).is_err());
    receipt.unlock().unwrap();
}
```
- [ ] Run `rtk cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --test model_manager --test model_store`; confirm the new behavior is absent.
- [ ] Implement the controller using existing Tokio process/IO support and `RunController::try_operation()`. Bound messages and drain stderr without forwarding it. Use `AppState::shutdown_sidecar()` before model validation/removal. Acquire app operation and config lock when inspecting pair references; the worker then acquires its mutation lock and exclusive receipt lock for removal, in that fixed order. Release through owned guards on all exits. Lock the immutable per-model identity receipt for inference so read-only stores work, and validate the receipt/path again after acquiring the lease. Read leases use `File::try_lock_shared`; the Python worker's exclusive lock must conflict with them on both platforms.

```rust
let operation = state.runs.try_operation()?;
state.shutdown_sidecar().await;
// Move `operation` and the config guard into the spawned job future.
// Keep them until child exit AND registry/result reconciliation are complete.
```

  Model-use leases must survive as long as the cached inference process can access files, including idle time; shutdown releases them. Pair addition/configuration must share admission protection with removal so a model cannot become selected during deletion. Include all saved pairs in `used_by_pairs`, not just the active one.
- [ ] Keep one active management job and one checked plan. `start_model_install` captures the plan by ID; a different ID fails without spawning anything. Job progress must match the active ID/model and remain within declared byte totals. Store terminal state before emitting it; `get_model_job` recovers a terminal event missed by the UI. Cancel kills/reaps only that worker, then reconciles the registry before releasing locks. If atomic publication already succeeded, report ready rather than claiming a cancelled nonexistent installation.
- [ ] Add safe error codes for absent models, incompatible model metadata, inaccessible repository, model-store read-only, insufficient space, model in use, hash mismatch and interrupted installation. Expose no raw paths, private settings, document content or signed URLs. `list_managed_models` works without starting a document engine; missing model directories return catalog entries. Malformed existing stores remain visible as an explicit error.
- [ ] Run the focused tests including the Python/Rust lock contention check, then existing resource, sidecar, pair and review-command regressions. Commit as `feat: supervise model downloads and protect active installations`.

## Task 5: Build the global Models page with reliable progress

**Files:** Create `apps/desktop/src/lib/modelIpc.ts`, `apps/desktop/src/composables/useModels.ts`, `apps/desktop/src/composables/useModels.test.ts`, `apps/desktop/src/components/ModelManager.vue`, `apps/desktop/src/components/ModelManager.test.ts`. Task 6 owns `App.vue` integration, avoiding parallel edits.

**Interfaces:** `modelApi` exposes `list(): Promise<ManagedModel[]>`, `check(source: ModelSource): Promise<CheckedModel>`, `install(planId: string): Promise<string>`, `cancel(jobId: string): Promise<void>`, `job(jobId: string): Promise<ModelJob | null>`, `remove(name: string): Promise<string>`, and `listen(receive: (value: unknown) => void): Promise<() => void>`. These map to the exact commands above; export `ModelApi = typeof modelApi`. `useModels(api: ModelApi = modelApi)` returns `models`, `readyModels`, `checked`, `job`, `busy`, `error`, `refresh()`, `check(source)`, `install(planId)`, `cancel()`, and `remove(name)`. `readyModels` projects unchanged `ModelInfo` objects. `ModelManager` receives `{models, checked, job, busy, error}` and emits `check(ModelSource)`, `install(planId)`, `cancel()`, `remove(name)`.

- [ ] Add IPC validation and state tests: offline catalog visible with no pair; URL inspection does not start installation; ready requires a terminal job plus refreshed registry; stale progress from job A cannot overwrite job B; unmount cleans event listeners; missing completion is recovered from `get_model_job`. Construct complete `ManagedModel`/`ModelJob` data from Task 2's shared fixture rather than permissive partial mocks.

```ts
import { effectScope } from "vue";
import { expect, test } from "vitest";
import type { ModelApi } from "../lib/modelIpc";
import { useModels } from "./useModels";

test("ignores progress belonging to a previous job", async () => {
  const current = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const previous = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
  let receive: (value: unknown) => void = () => { throw new Error("not listening"); };
  const api: ModelApi = {
    list: async () => [],
    check: async () => { throw new Error("unexpected inspection"); },
    install: async () => current, remove: async () => current,
    cancel: async () => {}, job: async () => null,
    listen: async callback => { receive = callback; return () => {}; },
  };
  const scope = effectScope();
  const models = scope.run(() => useModels(api))!;
  try {
    await models.install("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    const event = { job_id: previous, model_name: "fixture-model", stage: "downloading",
      downloaded_bytes: 1, total_bytes: 2, error: null };
    receive(event);
    expect(models.job.value?.job_id).not.toBe(previous);
    receive({ ...event, job_id: current });
    expect(models.job.value?.job_id).toBe(current);
  } finally { scope.stop(); }
});
```
- [ ] Run `rtk pnpm --filter @redactio/desktop test src/composables/useModels.test.ts src/components/ModelManager.test.ts` and confirm failures before implementing.
- [ ] Implement wrappers using the exact command names, Zod parsing, and existing `safeError`. Subscribe before starting jobs, retain the returned job ID, and reconcile completion through `get_model_job` using the same listener/summary pattern as `useRun`. Preserve current data on failed refresh; show the failure. Initialize once independently of pair selection.

```ts
function acceptProgress(value: unknown) {
  const parsed = ModelJobSchema.safeParse(value);
  if (!parsed.success || parsed.data.job_id !== activeJobId.value) return;
  job.value = parsed.data;
}
```

  `activeJobId` is a local `ref<string | null>` initialized to null and set from install/remove results. Buffer an event received before the start call resolves, or immediately query that returned job ID; do not lose a very fast completion.
- [ ] Build the page with existing installed Onyx components; verify their actual exports/props locally before choosing a progress component. Show source/revision, size, labels, license or **Nicht angegeben**, status, and one state-appropriate action per model. Import uses a labeled URL field, **Prüfen**, then a result with **Herunterladen**. Show **Wird geprüft** during local validation and make cancel reachable while busy. Use existing typography/spacing tokens and accessible status text, keyboard focus and narrow-screen layout.
- [ ] Add component tests that an invalid import shows its specific rejection, a ready entry cannot be downloaded twice, removal shows affected pairs, and malicious repository text is rendered as text. No third-party card HTML, generic spinners without stage text, or model-quality promises. Give read-only storage an actionable German message.
- [ ] Run focused Vitest checks and `rtk pnpm typecheck`. Commit as `feat: add catalog and compatible-model import controls`.

## Task 6: Connect model management to pairs without losing settings drafts

**Files:** Modify `apps/desktop/src/App.vue`, `App.test.ts`, `App.review.test.ts`, `components/PairManager.vue`, `PairManager.test.ts`, `components/DetectionSettings.vue`, `DetectionSettings.test.ts`, `composables/usePairs.ts`, `usePairs.test.ts`, `composables/useDetection.ts`, `useDetection.test.ts`, `lib/ipc.ts`, and Rust `commands.rs` plus `tests/pairs.rs`/command tests.

**Interfaces:** Extend pair creation to `addPair(name, sourceFolder, targetFolder, createTarget, modelName)` in Vue/IPC and the Rust `add_pair` command; require a ready named model. Pass the global `readyModels` to detection/review components. Remove model fetching from `useDetection`; it retains pair-specific refresh/save/preview and stale-request protection.

- [ ] Write the regression in `DetectionSettings.test.ts` using its existing mount fixture: change the selected model, toggle a native label and edit a custom rule, then replace `models` with equivalent refreshed metadata plus another newly installed model. Assert the save event contains the draft verbatim. Add a separate test that changing the actual selected pair resets to that pair's config.

```ts
test("model discovery refresh preserves unsaved settings", async () => {
  const wrapper = setup();
  await wrapper.setProps({ models: [...models, hugginglil] });
  await wrapper.get('[role="option"][aria-label="HuggingLil – Deutsch, PII (6af88fa)"]').trigger("click");
  await wrapper.get('[data-testid="model-entity-GIVENNAME"]').setValue(false);
  await wrapper.get('[data-testid="add-regex"]').trigger("click");
  await wrapper.get('[data-testid="rule-pattern"]').setValue("Synthetic name");
  await wrapper.get('[data-testid="preview-text"]').setValue("Synthetic name");
  await wrapper.get('[data-testid="preview"]').trigger("click");
  const beforeRefresh = structuredClone(wrapper.emitted("preview")!.at(-1)![1]);
  const added = { name: "hf:fixture/model@" + "a".repeat(40), version: "a".repeat(40),
    compatible: true, entity_types: ["PERSON"] };
  await wrapper.setProps({ models: [...models, { ...hugginglil }, added] });
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")!.at(-1)![1]).toEqual(beforeRefresh);
});
```

  This code uses the existing `setup`, `models`, and `hugginglil` fixtures in `DetectionSettings.test.ts`. Add assertions for label choices cached while switching between models. Metadata refresh is not a form reset trigger.
- [ ] Test first launch with zero models, one model, and two models. The Models tab is accessible with no pair, exactly one usable model is selected visibly for creation, multiple models require an explicit choice, and no-ready-model creation links to Models. A missing model in an existing pair is shown as missing rather than replaced by the first option. Preview/save/run/review actions needing it stay explanatory; users can still inspect the pair.
- [ ] Run the focused App, PairManager, DetectionSettings and composable tests, then implement the wiring. Add `models` to the view union and exact nav active-state conditions; hide the pair selector on the app-wide Models page. Retain the unsaved-review navigation/close guard and include management job activity in operation admission without disabling its own Cancel button.
- [ ] Reset detection drafts only when pair identity or saved config changes. Reconcile available options separately; do not deep-watch transient progress. Keep the pair's settings component mounted with `v-show` when navigating to Models so that visiting the download page also preserves its draft; cover that round trip in `App.test.ts`. Stop the old pair watcher from clearing global models or trying to refresh inference when its required model is absent. A model becoming ready can refresh that pair's availability without saving a changed config or clearing a draft.
- [ ] At the host boundary, validate `modelName` under the existing operation guard, then call `native_default_config` for that exact model. Remove the hardcoded HuggingLil-only native-label rule and require native selections for all nonlegacy imports. Test a concurrent removal attempt and an unavailable supplied name; neither may create a pair using a fallback model.
- [ ] Run frontend tests/typecheck/build and focused Rust pair/detection/review-command tests. Commit as `feat: connect installed models to pair setup and preserve detection drafts`.

## Task 7: Ship a model-free Windows package with optional preloading

**Files:** Modify `scripts/package-windows.ps1`, `scripts/check-package.ps1`, `scripts/test-check-package.ps1`, `scripts/test-build-manifest.ps1`, `scripts/test_packaging.py`, `packaging/sidecar.spec`, `packaging/build-inputs.json`, `packaging/notices.py`, `.github/workflows/windows-release.yml`, `.github/workflows/check.yml`, `docs/development.md`, `docs/quick-start.de.md`, `docs/release-checks.md`; add `packaging/licenses/tokenizers-0.22.2-LICENSE` with its verified source/hash.

**Interfaces:** Packaging adds `[string[]]$PreloadModels = @()` with allowed catalog keys. Build manifest records `inputs.models` as an array rather than a mandatory single model. Default ZIP has no weights; optional preloads use the same registry and installation logic as the app. Package validation separates immutable shipped runtime hashes from mutable model-store receipts.

- [ ] Update negative packaging tests first: zero-model package is accepted, a declared preload missing/corrupt files is rejected, two preloads are both checked, and a model installed after extraction does not invalidate immutable runtime-file verification. Keep rejection of unexpected files outside the mutable model store. Run `rtk proxy apps/sidecar/.venv/bin/python scripts/test_packaging.py`; confirm the old mandatory-BiomedBERT assumptions fail.
- [ ] Change the build default and preserve explicit preloading:

```powershell
param([string]$DesktopExecutable, [string]$DesktopNotices,
      [ValidateSet('biomedbert', 'hugginglil')][string[]]$PreloadModels = @())
foreach ($model in $PreloadModels) {
    Invoke-Checked $python @('scripts/prepare-biomedbert.py', '--model', $model,
                            '--model-dir', (Join-Path $package 'models'))
}
```

  With no preloads, validate the frozen worker's catalog/listing and model-free host resources. With preloads, loop over every ready model and run local synthetic processing/replay. Do not require a particular NER result as proof of successful loading; use explicit synthetic custom rules for deterministic redaction assertions.
- [ ] Bundle the canonical catalog, both Transformers implementations, tokenizer support, trusted CA data, `spacy_legacy`, and `spacy_loggers` in the frozen runtime. Incorporate the verified tokenizers license supplement from the earlier USB build into reproducible source inputs; do not leave fixes solely in ignored `dist/usb-demo` scripts. Preserve the portable desktop CRT treatment from that build. Verify dynamic imports in the frozen executable before attempting a full model run.
- [ ] Update build-manifest provenance and checks for zero/multiple preloads. Runtime file checks remain immutable; model entries validate through registry receipts and allow explicit later installations/removals. Ensure docs distinguish a first download requiring connectivity from an optionally preloaded offline demonstration. Document copying the entire folder, model location, read-only behavior, supported import contract and exact-revision reinstall.
- [ ] Keep regular PR CI's real-model preparation explicit and existing single-trigger policy (`push` on main, `pull_request`, manual). Add manager tests to existing jobs; do not create duplicate feature-branch push checks. Windows release remains manual and defaults to the model-free package; optional preload input is a fixed catalog-key choice.
- [ ] Run Python packaging checks and native PowerShell negative/build-manifest checks. Commit as `build: ship model-free Windows packages with optional preloads`.

## Task 8: Integrate, verify old reviews, and prepare the manual handoff

**Files:** Modify only integration fixes found by checks, `docs/release-checks.md`, and the progress/evidence section at the end of this plan. Reuse synthetic fixtures and existing real-sidecar tests; keep generated packages and sensitive-free logs under ignored build directories.

**Interfaces:** All commands, worker events and registry types above must agree. `ModelInfo` and legacy review metadata remain compatible with the pre-change app.

- [ ] Before relying on new-engine replay, generate a synthetic legacy review using the pre-feature engine at commit `9c0ce09` in an isolated temporary checkout and the exact existing local models. Keep its source/output hashes, config, decisions, Markdown and fingerprint as temporary evidence. Replay through the new engine; compare them exactly. Installing/removing an unrelated model must not alter that pair's processing revision or result.
- [ ] Run the normal checks once against the combined result:

```sh
rtk pnpm typecheck
rtk pnpm test
rtk pnpm build
rtk proxy uv --directory apps/sidecar run --locked --offline ruff check .
rtk proxy uv --directory apps/sidecar run --locked --offline ruff format --check .
rtk proxy uv --directory apps/sidecar run --locked --offline mypy src
rtk proxy uv --directory apps/sidecar run --locked --offline pytest -q
rtk cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --check
rtk cargo clippy --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
rtk cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml
```

  Use existing prepared models and the documented real-sidecar environment variables for ignored integration checks. Do not silently skip them and claim real-model validation. Repeat only affected checks after fixes; avoid repeated full matrices without a new reason.
- [ ] Exercise the frozen Windows management worker: catalog install for each current model, URL inspection/import, cancelled download followed by retry, incompatible import, and offline processing with a cold external HF cache. Test a locally generated compatible model with a different context limit and a repository identity absent from the catalog through the controlled transport; verify source-relative spans past its first window. The remote URL test may use an existing catalog repository, but that alone does not prove arbitrary compatible repositories work.
- [ ] Produce the default no-weights ZIP and a preloaded demonstration variant from the final committed source. Check archive contents, hashes, notices, worker loading, and model discovery after extraction/relocation. Report the exact commit and distinguish automated checks from manual GUI acceptance.
- [ ] Use browser-based synthetic UI tests for Onyx layout, focus, progress/cancel, empty state, draft preservation and narrow screens. Do not launch the native GUI. Give the user concise native Windows steps for download/import, processing and USB verification; preserve the existing working demo and launcher unless explicitly replacing an identified artifact.
- [ ] Request a whole-change review focused on registry atomicity, imported-model validation, cancellation races, native file locks, dynamic windows and legacy replay. Fix actionable findings, rerun affected checks, commit and push to existing PR #1, and attach that PR to the task. Do not merge.

## Progress and evidence

This document is an implementation plan. The specification is approved; the plan
is awaiting review. No task above has been implemented or verified by writing
this plan. Record implementation commits and actual test evidence here during
execution, including any unavailable native/manual checks.
