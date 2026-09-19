# Local Processing Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a standalone, bounded, offline JSONL engine for DOCX extraction, configurable detection, consistent placeholders, and safe Markdown metadata.

**Architecture:** Pure extraction/span/render functions sit behind one Pydantic-validated dispatcher. Model state is held in an Engine instance and replaced on pair/configuration changes; the sidecar never writes user documents or app state.

**Tech Stack:** Python, uv, Pydantic, python-docx, Presidio analyzer, spaCy, PyYAML, stdlib zipfile/hashlib/unicodedata, existing prototype pytest/Ruff/mypy conventions.

**Spec:** [Approved release](../specs/2026-09-19-redactio-initial-release-design.md), sections 3.5, 4, 6–9, 11. Implements [shared contract C2](2026-09-19-00-release-overview.md#c2-sidecar-requests-and-results-p21-rust-mirrors-in-p32); can run independently of the desktop.

## Global Constraints

- “Offsets are zero-based Unicode code-point positions with an exclusive end.”
- “Initial limits are 64 MiB compressed input, 256 MiB cumulative uncompressed ZIP content, a 1,000:1 expansion ratio, 1,000,000 extracted Unicode code points, and 64 MiB per IPC message.”
- “Initialization has a 180-second deadline; each document request has a 120-second deadline.”
- “Recognized values are transient memory-only data.”
- “Stdout carries protocol messages only.”
- “Source documents are never edited.”
- All [overview constraints](2026-09-19-00-release-overview.md#global-constraints) apply. Shell checks run at repository root with `uv --directory apps/sidecar`; the host enforces wall-clock deadlines by terminating a stuck process.

## Review Focus

1. ZIP headers understate expansion or contain XML entity declarations: count actual inflated bytes and reject dangerous package XML in P2.2.
2. Merged Word cells duplicate narrative text: logical cell identity and reading order regression in P2.2.
3. A partially overlapping longer detection exposes a suffix: union coverage regression in P2.3.
4. German regex categories are configured but no effective German recognizer exists: real-engine coverage and pair-switch check in P2.4.
5. Invalid Pydantic payloads leak filenames in exception strings: captured stdout/stderr canary regression in P2.1/P2.5.

---

## Planned files

All Python modules below live under `apps/sidecar/src/redactio_sidecar/`.

| Files | Responsibility |
| --- | --- |
| `apps/sidecar/{pyproject.toml,uv.lock}` | Installable sidecar and pinned development dependencies |
| `__init__.py`, `__main__.py`, `schemas.py`, `ipc.py` | Version, entrypoint, strict messages, bounded JSONL |
| `extract.py` | Single-snapshot bounded DOCX read and content warnings |
| `redaction.py` | Validated span edits, overlap union, placeholders/offsets |
| `engine.py` | Local models and complete per-pair configuration |
| `frontmatter.py` | Pydantic frontmatter and deterministic Markdown |
| `apps/sidecar/tests/test_{ipc,extract,redaction,engine,frontmatter}.py` | Generated inputs and focused outcome checks |
| `apps/sidecar/tests/test_contract.py` | Synthetic serialization cases consumed by Rust in P3 |
| `docs/development.md` | Offline model preparation and engine check commands |

## P2.1: Strict schemas and bounded JSONL transport

**Files:** Create pyproject.toml, uv.lock, package entry files, schemas.py/ipc.py, test_ipc.py/test_contract.py. Start with Pydantic and pytest/Ruff/mypy; add domain dependencies in their owning task, without copying unused presidio-anonymizer.

**Interfaces:** C2 models use `ConfigDict(extra="forbid")`. `run_loop(stdin: BinaryIO, stdout: BinaryIO, dispatch: Callable[[Request], dict]) -> None` validates the returned dictionary as Response before serialization; `EngineError(code: str, retryable: bool = False)` is the only serialized failure. `dispatch` receives a validated Request. Initially `main()` wires a ping handler and returns `unsupported_request` for unimplemented commands; P2.4/P2.5 replace it with the Engine dispatcher. Define `RegexRule`/`WordRule` as the two discriminated CustomRule variants; ProcessingConfig and Decisions use the C1 defaults and empty decision lists respectively.

- [ ] Write the canary/error-survival test before the loop:

```python
import io
import json
from redactio_sidecar.ipc import run_loop

def test_invalid_payload_does_not_echo_private_input(capsys):
    incoming = io.BytesIO(
        b'{"id":"a","type":"ping","payload":{"secret":"CANARY_PATIENT"}}\n'
        b'{"id":"b","type":"ping","payload":{}}\n'
    )
    outgoing = io.BytesIO()
    def ping(request):
        return {"id": request.id, "type": "ping_result", "payload": {"protocol_version": 1}}
    run_loop(incoming, outgoing, ping)
    replies = [json.loads(line) for line in outgoing.getvalue().splitlines()]
    assert replies[0]["type"] == "error"
    assert replies[1]["id"] == "b"
    assert b"CANARY_PATIENT" not in outgoing.getvalue()
    assert "CANARY_PATIENT" not in capsys.readouterr().err
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest tests/test_ipc.py -q`; expect missing module failure after basic package setup.
- [ ] Define C2 models in schemas.py with UUID/revision/hash validation, permitted request tags, finite confidence in [0,1], `start < end`, integer nonnegative offsets, valid entity types, and timezone-aware timestamps. Empty payloads reject extra fields. Unknown/malformed requests return `invalid_request` without exception text. Request IDs have 1–128 characters; unrecoverable malformed JSON returns an empty reply ID.
- [ ] Implement bounded byte reading and output size checks:

```python
MAX_MESSAGE_BYTES = 64 * 1024 * 1024

def read_frame(stream):
    line = stream.readline(MAX_MESSAGE_BYTES + 1)
    if len(line) > MAX_MESSAGE_BYTES:
        raise EngineError("message_too_large")
    return line
```

Decode UTF-8 strictly; stop the child after an over-limit frame instead of
consuming an unbounded stream to find the next newline. Return only fixed error
codes for parser, validation, and unexpected errors. Do not call logger.exception
or stringify Pydantic ValidationError. Terminate normally on EOF. Serialize
validated replies before checking byte length, and flush each line.

- [ ] Add a compact parameterized check for invalid UTF-8/JSON/type/version, over-limit input/output, extra fields, NaN confidence, and EOF. test_contract.py exports synthetic valid C2 JSON objects via a `--emit` CLI branch for P3's real serialization check; emit no source paths or identifiers beyond generated temporary/synthetic values.
- [ ] Run the IPC/contract tests and `rtk proxy uv --directory apps/sidecar run ruff check .`; expect no secret canaries in stderr/stdout errors and a working `ping` through `python -m redactio_sidecar`.
- [ ] Commit: `rtk git add apps/sidecar`; `rtk git commit -m 'feat: define bounded private processing protocol'`.

## P2.2: Bounded extraction with explicit coverage warnings

**Files:** Create extract.py/test_extract.py; add python-docx to pyproject.toml/uv.lock.

**Interfaces:** `Extraction(text: str, source_hash_sha256: str, warnings: list[str])` frozen dataclass; `extract_document(path: Path) -> Extraction`. Warning codes: `empty_document`, `headers_footers`, `notes`, `text_boxes`, `tracked_changes`, `comments`, `embedded_objects`, `images`. Errors: `unreadable_document`, `invalid_docx`, `document_too_large`, `unsafe_xml`, `unsupported_document`.

- [ ] Generate a DOCX in the test, never a committed fixture:

```python
from docx import Document
from redactio_sidecar.extract import extract_document

def test_paragraphs_and_merged_table_cells_keep_order(tmp_path):
    document = Document()
    document.add_paragraph("Vorher")
    table = document.add_table(rows=1, cols=2)
    table.cell(0, 0).merge(table.cell(0, 1)).text = "Gemeinsam"
    document.add_paragraph("Nachher")
    path = tmp_path / "case.docx"
    document.save(path)
    result = extract_document(path)
    assert result.text == "Vorher\n\nGemeinsam\n\nNachher"
    assert result.warnings == []
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest tests/test_extract.py -q`; expect failure before extraction exists.
- [ ] Read at most the compressed limit + 1 into one immutable bytes snapshot, hash that snapshot, then inspect its ZIP. Reject duplicate member names, encrypted members, traversal names, missing DOCX parts, entity/DOCTYPE declarations, invalid package XML, and actual inflated-byte/ratio violations. Count every chunk while streaming members; do not trust ZipInfo.file_size alone. Use safe XML parsing with entity resolution and networking disabled; no external relationship is dereferenced.
- [ ] Open the validated snapshot with python-docx and traverse `iter_inner_content()` recursively for paragraphs/tables. Plain paragraph text includes visible hyperlink labels; table cells join with ` | ` and rows with newlines, blocks with two newlines. Visit merged physical cell XML only once per table. Recursively include nested tables. Enforce code-point length while collecting blocks, before joining unbounded lists.
- [ ] Inspect unsupported Word parts/elements and add distinct warning codes once each. Hidden/deleted revision text is not treated as visible main text. Empty text yields `empty_document`. No original filename/property/image/relationship target is serialized into output. Verify extraction from the same bytes later used for the request hash check.
- [ ] Extend test_extract.py with in-memory ZIP creation for deceptive sizes/expansion, corrupt/encrypted packages, nested tables, headers/footers, footnotes, comments, text boxes, revisions, images, hyperlinks, and empty docs. For each assert bounded error or warning; source bytes remain unchanged. Include a hyperlink pointing at an unreachable URL and assert no network calls are attempted.
- [ ] Run extraction tests, Ruff and mypy. Expected: stable text coordinate space, all unsupported content flagged, bad packages bounded.
- [ ] Commit: `rtk git add apps/sidecar`; `rtk git commit -m 'feat: extract DOCX text with bounded coverage checks'`.

## P2.3: Correct overlaps, decisions, and repeated placeholders

**Files:** Create redaction.py/test_redaction.py; use schemas.py's Detection/Decisions/OutputEntry.

**Interfaces:** `apply_redactions(text: str, detections: list[Detection], decisions: Decisions) -> tuple[str, list[OutputEntry]]`. Validate dismissed IDs and manual spans against original text. Manual type changes are represented as dismissing an automatic detection plus adding a manual span, keeping the saved format small.

- [ ] Write the two core outcome checks:

```python
from redactio_sidecar.redaction import apply_redactions
from redactio_sidecar.schemas import Detection, Decisions

def test_repeated_values_share_a_placeholder_and_overlaps_cover_the_union():
    def d(id, start, end):
        return Detection(id=id, start=start, end=end, entity_type="PERSON",
                         confidence=0.9, recognizer="SpacyRecognizer", origin="automatic")
    body, entries = apply_redactions("Anna Anna", [d("a", 0, 4), d("b", 5, 9)], Decisions())
    assert body == "<PERSON_1> <PERSON_1>"
    assert len(entries) == 2
    body, entries = apply_redactions("abcdef", [d("a", 0, 4), d("b", 2, 6)], Decisions())
    assert body == "<PERSON_1>"
    assert body[entries[0].start_offset:entries[0].end_offset] == entries[0].placeholder
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest tests/test_redaction.py -q`; expect failure.
- [ ] Implement a sorted interval sweep: remove only explicitly dismissed IDs, append manual spans, sort by start/end, union strictly overlapping spans transitively, and choose type by manual precedence, confidence, then type-name tie-break. For equally ranked contributors use smallest start/end/ID as stable tie-break. Do not merge merely adjacent intervals. A merged entry's confidence is the chosen automatic contributor's confidence, or null if manual wins; recognizer is `merged` and origin is `merged`.
- [ ] Emit replacements left-to-right using `(entity_type, unicodedata.normalize("NFC", matched_value).strip())` as the in-memory numbering key. Preserve case distinctions; counters restart per document. Keep original prefix/suffix bytes as text and compute code-point output offsets as pieces are appended.

```python
key = (entity_type, unicodedata.normalize("NFC", value).strip())
if key not in placeholders:
    counters[entity_type] = counters.get(entity_type, 0) + 1
    placeholders[key] = f"<{entity_type}_{counters[entity_type]}>"
placeholder = placeholders[key]
```

- [ ] Add cases for transitive overlaps, a dismissed detection under another active interval, manual type precedence, unknown dismissed ID, bad bounds, composed/decomposed Unicode, emoji prefixes, and literal `<PERSON_1>` in source text. Real highlights come from offsets, never from matching placeholder strings.
- [ ] Run redaction tests plus Ruff/mypy. Expected: union coverage, deterministic numbering, correct Python code-point indexing, no persistence of recognized values.
- [ ] Commit: `rtk git add apps/sidecar`; `rtk git commit -m 'feat: apply consistent redactions and manual decisions'`.

## P2.4: Pair-isolated local detection and model configuration

**Files:** Create engine.py/test_engine.py; add presidio-analyzer/spaCy dependencies and documented local model setup. No unused AnonymizerEngine is needed for the custom interval renderer.

**Interfaces:** `Engine(model_root: Path)`, `configure(pair_id: str, revision: str, config: ProcessingConfig) -> EngineInfo`, `analyze(pair_id: str, revision: str, text: str) -> list[Detection]`, `preview_rules(pair_id: str, revision: str, text: str) -> list[Detection]` uses the same analyze boundary. `available_models() -> list[ModelInfo]`, where ModelInfo is `{name, version, compatible: bool}`. The local `models/manifest.json` maps names/versions to directories relative to model_root; validate those directories rather than trusting compatibility declared in a file. Document creating this same layout for development; P5 packages it unchanged. P3 uses configure to validate saved settings.

- [ ] Write a test using a configured local test model and two complete snapshots. Model preparation is a build/dev operation, never part of test execution:

```python
import os
from pathlib import Path
from uuid import uuid4
from redactio_sidecar.engine import Engine
from redactio_sidecar.schemas import ProcessingConfig, WordRule

def test_switching_pair_removes_previous_custom_terms():
    engine = Engine(Path(os.environ["REDACTIO_MODEL_DIR"]))
    pair_a, pair_b, rev_a, rev_b = [str(uuid4()) for _ in range(4)]
    rule = WordRule(id=str(uuid4()), entity_type="CUSTOM", enabled=True,
                    kind="words", words=["KUNSTWORTXYZ"])
    engine.configure(pair_a, rev_a, ProcessingConfig(enabled_entities=[], custom_rules=[rule]))
    assert engine.analyze(pair_a, rev_a, "KUNSTWORTXYZ")
    engine.configure(pair_b, rev_b, ProcessingConfig(enabled_entities=[], custom_rules=[]))
    assert engine.analyze(pair_b, rev_b, "KUNSTWORTXYZ") == []
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest tests/test_engine.py -q`; before implementation expect missing Engine behavior, not a silently skipped model test. Document the exact local model acquisition command/version/hash in development.md and lock the installed package set.
- [ ] Resolve allowlisted model names only beneath model_root. Check existence/compatibility before creating Presidio's NLP engine so its loader cannot auto-download. Reuse model weights only when name/version matches; rebuild the recognizer registry from the full selected snapshot. Use actual German/language-neutral recognizers for all eight enabled categories, with synthetic probes for each; fix missing German registration explicitly.
- [ ] Compile regex rules with Python `re` during configure, reject invalid/empty patterns and empty word lists, deduplicate list terms, assign fixed custom detector score 1.0 as a rule-match score (not accuracy). Escape words before pattern creation and apply appropriate boundaries. Restrict custom entity types and IDs to C1. Configure is transactional: errors leave the prior active config intact, replies identify the attempted pair. After failed configure the host must not process under that attempted pair.
- [ ] Add real analyzer probes for PERSON/LOCATION plus contact/date categories, missing/incompatible model, invalid rules, and switching model name. Assert model initialization calls no downloader and that logs contain no custom terms. Bound all regex/model work through the host timeout; test a catastrophic regex in P3.2's killed subprocess instead of hanging this test process.
- [ ] Run engine tests with the offline model installed, Ruff/mypy, and previous Python tests. Expected: effective default categories and no cross-pair rule retention.
- [ ] Commit: `rtk git add apps/sidecar docs/development.md`; `rtk git commit -m 'feat: configure isolated offline detection engines'`.

## P2.5: Metadata rendering and complete document handlers

**Files:** Create frontmatter.py/test_frontmatter.py; extend Engine dispatcher, schemas.py, ipc.py and test_contract.py.

**Interfaces:** `render_document(meta: DocumentMeta, engine: EngineInfo, body: str, entries: list[OutputEntry], warnings: list[str], status: ReviewStatus, reviewed_at: datetime | None, include_positions: bool) -> str`. `Engine.process_document(request: ProcessRequest) -> ProcessResult`; `Engine.render_review(request: ReviewRequest) -> ProcessResult`. Request hash must equal Extraction.source_hash_sha256 before decisions are applied.

- [ ] Write frontmatter serialization test with explicit safe data:

```python
from datetime import datetime, timezone
from uuid import uuid4
import yaml
from redactio_sidecar.frontmatter import render_document
from redactio_sidecar.schemas import DocumentMeta, EngineInfo

def test_metadata_is_structured_and_positions_can_be_omitted():
    meta = DocumentMeta(sync_pair_id=str(uuid4()), doc_id="doc-0001",
                        source_hash_sha256="0" * 64, processing_revision=str(uuid4()),
                        redacted_at=datetime.now(timezone.utc))
    engine = EngineInfo(engine_version="test", model_name="test", model_version="1",
                        recognizers=[], extraction_version="1")
    markdown = render_document(meta, engine, "Hello", [], [], "pending", None, False)
    frontmatter = yaml.safe_load(markdown.split("---", 2)[1])
    assert frontmatter["doc_id"] == "doc-0001"
    assert "redactions" not in frontmatter
    assert "review_notes" not in frontmatter
    assert markdown.endswith("Hello\n")
```

- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest tests/test_frontmatter.py -q`; expect missing renderer failure.
- [ ] Define every frontmatter field from spec §6.1, preserving the field order, using safe_dump/allow_unicode and RFC-3339 timestamps. Count occurrences; null confidence aggregates when no automatic confidence is available. Keep raw body text in the Markdown artifact and return that exact body separately for text-only rendering in P4. Include only public recognizer names/opaque rule UUIDs. New processing results with warnings are needs-rework; render_review accepts approval only for non-empty text when acknowledged_warnings covers the current warnings, with the host enforcing the same rule.
- [ ] Wire process_document to extract → verify hash → analyze → apply_redactions → render_document. Wire render_review to re-extract/verify hash → validate stored detections/decisions → apply → render. Do not analyze again during review. Verify active pair/revision before either path; return complete C2 results without persisting original text.
- [ ] Extend contract tests with real JSONL configure/process/render replies, repeat render using fixed timestamps for byte equality, changed source rejection, include_positions=false with retained private entries, YAML delimiter-like source text, empty documents, and errors containing canary filenames. Ensure original_text is in private ProcessResult only, never frontmatter/audit.
- [ ] Run `rtk proxy uv --directory apps/sidecar run pytest -q`, `ruff check .`, `ruff format --check .`, and `mypy src` via the same uv prefix. Expected: a complete synthetic DOCX round trip works with network unavailable and no data leaks in errors.
- [ ] Commit: `rtk git add apps/sidecar`; `rtk git commit -m 'feat: render private document processing results'`.

## Completion evidence

- [ ] A03/A04 and engine portions of A05/A09/A12/A17 pass against synthetic data with a real installed German model.
- [ ] Commands are documented and lockfile matches the tested model/library versions.
- [ ] No host filesystem writes, downloads, or prototype process rules entered the sidecar.
