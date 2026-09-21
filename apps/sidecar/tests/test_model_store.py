from pathlib import Path

import pytest

from redactio_sidecar.ipc import EngineError
from redactio_sidecar.model_store import Window, processing_window


@pytest.mark.parametrize(
    ("model", "tokenizer", "special", "expected"),
    [
        (512, 512, 2, Window(512, 128)),
        (256, 256, 2, Window(256, 64)),
        (1024, 1024, 2, Window(1024, 256)),
        (2048, 1024, 2, Window(1024, 256)),
        (1024, int(1e30), 2, Window(1024, 256)),
        (1024, None, 2, Window(1024, 256)),
        (8, 8, 6, Window(8, 1)),
    ],
)
def test_model_dependent_window(model, tokenizer, special, expected):
    assert processing_window(model, tokenizer, special) == expected


@pytest.mark.parametrize(
    ("model", "tokenizer", "special"),
    [
        (None, 512, 2),
        (0, 512, 2),
        (True, 512, 2),
        (512, -1, 2),
        (512, "512", 2),
        (2, 2, 2),
        (512, 512, -1),
    ],
)
def test_invalid_capacity_is_explicit(model, tokenizer, special):
    with pytest.raises(ValueError):
        processing_window(model, tokenizer, special)


class _ValidationTokenizer:
    def __init__(self, invalid_offset: tuple[object, object] | None = None) -> None:
        self.calls: list[tuple[str, dict[str, object]]] = []
        self.invalid_offset = invalid_offset

    def __call__(self, text: str, **kwargs: object) -> dict[str, object]:
        self.calls.append((text, kwargs))
        if kwargs.get("add_special_tokens") is False:
            return {"input_ids": [1, 2]}
        first = [(0, 0), (0, 4), (5, len(text) - 4)]
        if self.invalid_offset is not None:
            first.append(self.invalid_offset)
        return {
            "offset_mapping": [
                first,
                [(0, 0), (len(text) - 4, len(text))],
            ],
            "special_tokens_mask": [[1, 0, 0] + ([0] if self.invalid_offset else []), [1, 0]],
        }


class _NoRawForward:
    def __call__(self, **_: object) -> None:
        raise AssertionError("validation must use the production pipeline")


@pytest.mark.parametrize(
    ("window", "detections"),
    [(Window(32, 8), [{"start": 0, "end": 4}]), (Window(1024, 256), [])],
)
def test_validation_runs_unicode_overflow_through_shared_pipeline(
    monkeypatch, window: Window, detections: list[dict[str, int]]
):
    from redactio_sidecar import model_store

    tokenizer = _ValidationTokenizer()
    model = _NoRawForward()
    captured: dict[str, object] = {}

    monkeypatch.setattr(model_store, "_load_local_model", lambda *_: (tokenizer, model, window))

    def pipeline(actual_model, actual_tokenizer, actual_window):
        captured.update(model=actual_model, tokenizer=actual_tokenizer, window=actual_window)
        return lambda _: detections

    monkeypatch.setattr(model_store, "_token_classification_pipeline", pipeline)

    assert (
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")
        == window
    )
    text, kwargs = tokenizer.calls[-1]
    assert "😀\r\n" in text
    assert len(text) > window.tokens
    assert kwargs == {
        "truncation": True,
        "max_length": window.tokens,
        "stride": window.stride,
        "return_overflowing_tokens": True,
        "return_offsets_mapping": True,
        "return_special_tokens_mask": True,
    }
    assert captured == {"model": model, "tokenizer": tokenizer, "window": window}


@pytest.mark.parametrize(
    "detections",
    [
        lambda text: [{"start": -1, "end": 4}],
        lambda text: [{"start": 0, "end": len(text) + 1}],
        lambda text: [{"start": 1, "end": 4}],
        lambda text: [{"start": 0, "end": 4}, {"start": 0, "end": 4}],
    ],
)
def test_validation_rejects_invalid_truncated_and_reset_offsets(monkeypatch, detections):
    from redactio_sidecar import model_store

    monkeypatch.setattr(
        model_store,
        "_load_local_model",
        lambda *_: (_ValidationTokenizer(), _NoRawForward(), Window(32, 8)),
    )
    monkeypatch.setattr(
        model_store,
        "_token_classification_pipeline",
        lambda *_: lambda text: detections(text),
    )

    with pytest.raises(EngineError, match="^model_incompatible$"):
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")


@pytest.mark.parametrize("invalid_offset", [(-1, 4), (0, 0), ("0", 4)])
def test_validation_rejects_malformed_tokenizer_offsets(monkeypatch, invalid_offset):
    from redactio_sidecar import model_store

    monkeypatch.setattr(
        model_store,
        "_load_local_model",
        lambda *_: (_ValidationTokenizer(invalid_offset), _NoRawForward(), Window(32, 8)),
    )
    monkeypatch.setattr(model_store, "_token_classification_pipeline", lambda *_: lambda _: [])

    with pytest.raises(EngineError, match="^model_incompatible$"):
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")


def test_validation_rejects_reset_overflow_offsets(monkeypatch):
    from redactio_sidecar import model_store

    class ResetTokenizer(_ValidationTokenizer):
        def __call__(self, text: str, **kwargs: object) -> dict[str, object]:
            encoded = super().__call__(text, **kwargs)
            if "offset_mapping" in encoded:
                encoded["offset_mapping"][1] = [(0, 0), (0, 4)]
            return encoded

    monkeypatch.setattr(
        model_store,
        "_load_local_model",
        lambda *_: (ResetTokenizer(), _NoRawForward(), Window(32, 8)),
    )
    monkeypatch.setattr(model_store, "_token_classification_pipeline", lambda *_: lambda _: [])

    with pytest.raises(EngineError, match="^model_incompatible$"):
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")


def test_validation_rejects_dropped_non_whitespace_offsets(monkeypatch):
    from redactio_sidecar import model_store

    class DroppedTokenizer(_ValidationTokenizer):
        def __call__(self, text: str, **kwargs: object) -> dict[str, object]:
            encoded = super().__call__(text, **kwargs)
            if "offset_mapping" in encoded:
                encoded["offset_mapping"][0] = [(0, 0), (0, 4)]
                encoded["special_tokens_mask"][0] = [1, 0]
            return encoded

    monkeypatch.setattr(
        model_store,
        "_load_local_model",
        lambda *_: (DroppedTokenizer(), _NoRawForward(), Window(32, 8)),
    )
    monkeypatch.setattr(model_store, "_token_classification_pipeline", lambda *_: lambda _: [])

    with pytest.raises(EngineError, match="^model_incompatible$"):
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")


def test_empty_store_does_not_write(tmp_path):
    from redactio_sidecar.model_store import read_registry

    root = tmp_path / "models"
    assert read_registry(root).models == []
    assert not root.exists()


def test_repository_and_revision_are_part_of_new_identity():
    from redactio_sidecar.model_store import selection_name

    a = selection_name("one/model", "a" * 40)
    assert a != selection_name("two/model", "a" * 40)
    assert a != selection_name("one/model", "b" * 40)
    assert a == "hf:one/model@" + "a" * 40


def test_shared_management_contract():
    import json

    from pydantic import TypeAdapter

    from redactio_sidecar import model_store as store

    fixture = json.loads(
        (Path(__file__).parents[3] / "tests/fixtures/model-management.json").read_text()
    )
    for key, name in {
        "artifact": "Artifact",
        "descriptor": "ModelDescriptor",
        "catalog": "ModelCatalog",
        "catalog_entry": "CatalogEntry",
        "registry": "ModelRegistry",
        "managed_model": "ManagedModel",
        "checked_model": "CheckedModel",
        "job": "ModelJob",
    }.items():
        assert getattr(store, name).model_validate(fixture[key]).model_dump() == fixture[key]
    adapter = TypeAdapter(store.ModelSource)
    for source in fixture["sources"]:
        assert adapter.validate_python(source).model_dump() == source


def fixture_ready_model(root, repository="one/model", revision="a" * 40):
    import hashlib
    import json

    from redactio_sidecar import model_store as store

    name = store.selection_name(repository, revision)
    directory = store.directory_name(repository, revision)
    path = root / directory
    path.mkdir(parents=True)
    files = {
        "config.json": json.dumps(
            {
                "model_type": "bert",
                "architectures": ["BertForTokenClassification"],
                "max_position_embeddings": 1024,
                "id2label": {"0": "O", "1": "B-PERSON"},
            }
        ).encode(),
        "tokenizer_config.json": b'{"model_max_length":1024}',
        "tokenizer.json": b"{}",
        "model.safetensors": b"synthetic weights",
    }
    artifacts = []
    for filename, data in files.items():
        (path / filename).write_bytes(data)
        digest = hashlib.sha256(data).hexdigest()
        artifacts.append(
            {
                "filename": filename,
                "size": len(data),
                "sha256": digest,
                "upstream_hash": {"algorithm": "sha256", "value": digest},
            }
        )
    (path / "redactio-model.json").write_text(
        json.dumps({"name": name, "version": revision, "repository": repository})
    )
    descriptor = store.ModelDescriptor(
        name=name,
        version=revision,
        repository=repository,
        title="Synthetic model",
        license=None,
        model_type="bert",
        architecture="BertForTokenClassification",
        entity_types=["PERSON"],
        window_tokens=1024,
        stride_tokens=256,
        special_tokens=2,
        files=artifacts,
    )
    return store.ModelRecord(descriptor=descriptor, path=directory, state="ready")


def test_imports_discovered_offline_without_weights_and_partial_models(tmp_path, monkeypatch):
    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine

    records = [
        fixture_ready_model(tmp_path, repository, revision)
        for repository, revision in (
            ("one/model", "a" * 40),
            ("two/model", "a" * 40),
            ("one/model", "b" * 40),
        )
    ]
    store.write_registry(
        tmp_path, store.ModelRegistry(schema_version=2, models=records, legacy_unavailable=[])
    )
    (tmp_path / records[1].path / "model.safetensors").unlink()
    monkeypatch.setattr(store, "_load_local_model", lambda *_: pytest.fail("weights loaded"))
    discovered = Engine(tmp_path).available_models()
    assert [m.name for m in discovered if m.compatible] == [
        records[0].descriptor.name,
        records[2].descriptor.name,
    ]
    assert store.read_registry(tmp_path).models[0].descriptor.window_tokens == 1024
    (tmp_path / records[0].path / "redactio-model.json").unlink()
    assert not Engine(tmp_path).available_models()[0].compatible


def test_legacy_unknown_retained_and_future_registry_untouched(tmp_path):
    import json

    from redactio_sidecar.model_store import read_registry, write_registry

    entry = {"name": "retired", "version": "old", "path": "old/nested"}
    manifest = tmp_path / "manifest.json"
    before = json.dumps({"models": [entry]})
    manifest.write_text(before)
    registry = read_registry(tmp_path)
    assert registry.models == []
    assert registry.legacy_unavailable[0].model_dump() == entry
    assert manifest.read_text() == before
    write_registry(tmp_path, registry)
    assert read_registry(tmp_path).model_dump() == registry.model_dump()
    before = '{"schema_version": 99, "models": []}'
    manifest.write_text(before)
    with pytest.raises(EngineError, match="invalid_model_manifest"):
        read_registry(tmp_path)
    assert manifest.read_text() == before


@pytest.mark.parametrize(
    "field,value",
    [
        ("path", "../escape"),
        ("path", "CON"),
        ("path", "nested/path"),
        ("state", "invalid"),
        ("path", None),
    ],
)
def test_registry_rejects_invalid_record_fields(tmp_path, field, value):
    from pydantic import ValidationError

    from redactio_sidecar.model_store import ModelRecord

    raw = fixture_ready_model(tmp_path).model_dump()
    raw[field] = value
    with pytest.raises(ValidationError):
        ModelRecord.model_validate(raw)


@pytest.mark.parametrize(
    "field,value",
    [
        ("entity_types", ["bad"]),
        ("entity_types", ["PERSON", "DATE"]),
        ("entity_types", []),
        ("window_tokens", True),
        ("stride_tokens", 0),
        ("repository", "bad/../model"),
        ("name", "hf:wrong/model@" + "a" * 40),
        ("architecture", "DebertaV2ForTokenClassification"),
    ],
)
def test_descriptor_rejects_invalid_fields(tmp_path, field, value):
    from pydantic import ValidationError

    from redactio_sidecar.model_store import ModelDescriptor

    raw = fixture_ready_model(tmp_path).descriptor.model_dump()
    raw[field] = value
    with pytest.raises(ValidationError):
        ModelDescriptor.model_validate(raw)


def test_generic_model_requires_native_labels_and_keeps_public_recognizer(tmp_path, monkeypatch):
    from uuid import uuid4

    from redactio_sidecar import biomedbert
    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine
    from redactio_sidecar.frontmatter import PUBLIC_RECOGNIZERS
    from redactio_sidecar.schemas import ProcessingConfig

    record = fixture_ready_model(tmp_path)
    store.write_registry(
        tmp_path, store.ModelRegistry(schema_version=2, models=[record], legacy_unavailable=[])
    )
    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda *_: lambda _: [])
    engine = Engine(tmp_path)
    with pytest.raises(EngineError, match="invalid_configuration"):
        engine.configure(str(uuid4()), str(uuid4()), ProcessingConfig(model=record.descriptor.name))
    info = engine.configure(
        str(uuid4()),
        str(uuid4()),
        ProcessingConfig(
            model=record.descriptor.name, enabled_entities=[], model_entities=["PERSON"]
        ),
    )
    assert info.recognizers == ["TransformersNerRecognizer"]
    assert "TransformersNerRecognizer" in PUBLIC_RECOGNIZERS


def test_catalog_aliases_and_native_labels():
    from redactio_sidecar.model_store import catalog_models, selection_name

    catalog = catalog_models()
    assert [e.descriptor.name for e in catalog] == [
        "OpenMed-PII-German-BiomedBERT-Large-340M-v1",
        "pii-sensitive-ner-german",
    ]
    assert catalog[1].descriptor.license is None
    assert {"ETHN", "REL", "SOR"} <= set(catalog[1].descriptor.entity_types)
    for entry in catalog:
        descriptor = entry.descriptor
        assert selection_name(descriptor.repository, descriptor.version) == descriptor.name
        assert (descriptor.window_tokens, descriptor.stride_tokens) == (512, 128)
        assert all(f.sha256 == f.upstream_hash.value for f in descriptor.files)


def test_discovery_imports_no_heavy_runtime():
    import subprocess
    import sys

    subprocess.run(
        [
            sys.executable,
            "-c",
            "from redactio_sidecar.model_store import catalog_models; "
            "catalog_models(); import sys; "
            "assert not {'transformers', 'torch', 'presidio_analyzer', 'spacy'} "
            "& sys.modules.keys()",
        ],
        check=True,
    )


@pytest.mark.parametrize(
    "key,field,value",
    [
        ("artifact", "filename", "../weights"),
        ("artifact", "size", True),
        ("artifact", "size", 2**53),
        ("descriptor", "special_tokens", 512),
        ("catalog", "schema_version", True),
        ("registry", "schema_version", 3),
        ("managed_model", "state", "removed"),
        ("managed_model", "used_by_pairs", [{"id": "bad", "name": "x"}]),
        ("checked_model", "plan_id", "bad"),
        ("job", "stage", "queued"),
        ("job", "total_bytes", -1),
        ("job", "error", {"code": "x", "retryable": "yes"}),
    ],
)
def test_management_boundaries_reject_malformed_variants(key, field, value):
    import json

    from pydantic import ValidationError

    from redactio_sidecar import model_store as store

    fixture = json.loads(
        (Path(__file__).parents[3] / "tests/fixtures/model-management.json").read_text()
    )
    models = {
        "artifact": store.Artifact,
        "descriptor": store.ModelDescriptor,
        "catalog": store.ModelCatalog,
        "registry": store.ModelRegistry,
        "managed_model": store.ManagedModel,
        "checked_model": store.CheckedModel,
        "job": store.ModelJob,
    }
    raw = fixture[key]
    raw[field] = value
    with pytest.raises(ValidationError):
        models[key].model_validate(raw)


@pytest.mark.parametrize(
    "source",
    [
        {"kind": "catalog", "key": "other"},
        {"kind": "receipt", "name": ""},
        {"kind": "url", "url": "https://huggingface.co/one/model/tree/main"},
        {"kind": "url", "url": "https://huggingface.co:443/one/model"},
        {"kind": "url", "url": "https://huggingface.co/one/model?download=true"},
        {"kind": "url", "url": "https://huggingface.co/one/%6dodel"},
        {"kind": "url", "url": "https://huggingface.co/one/model", "extra": 1},
    ],
)
def test_model_source_rejects_malformed_variants(source):
    from pydantic import TypeAdapter, ValidationError

    from redactio_sidecar.model_store import ModelSource

    with pytest.raises(ValidationError):
        TypeAdapter(ModelSource).validate_python(source)


def test_registry_write_failure_preserves_existing_bytes(tmp_path, monkeypatch):
    from redactio_sidecar import model_store as store

    before = b'{"models": []}'
    (tmp_path / "manifest.json").write_bytes(before)
    registry = store.read_registry(tmp_path)

    def fail_replace(*_):
        raise OSError("synthetic failure")

    monkeypatch.setattr(store.os, "replace", fail_replace)
    with pytest.raises(OSError):
        store.write_registry(tmp_path, registry)
    assert (tmp_path / "manifest.json").read_bytes() == before
    assert sorted(p.name for p in tmp_path.iterdir()) == ["manifest.json"]


def test_registry_never_follows_model_or_manifest_links(tmp_path):
    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine

    root = tmp_path / "root"
    record = fixture_ready_model(root)
    store.write_registry(
        root, store.ModelRegistry(schema_version=2, models=[record], legacy_unavailable=[])
    )
    original = root / record.path
    outside = tmp_path / "outside"
    original.rename(outside)
    original.symlink_to(outside, target_is_directory=True)
    assert not Engine(root).available_models()[0].compatible
    manifest = root / "manifest.json"
    manifest.rename(tmp_path / "outside.json")
    manifest.symlink_to(tmp_path / "outside.json")
    with pytest.raises(EngineError, match="invalid_model_manifest"):
        store.read_registry(root)


@pytest.mark.parametrize("label", ["B-invalid", "B-LABEL_0"])
def test_invalid_labels_in_one_model_do_not_hide_another(tmp_path, label):
    import json

    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine

    records = [
        fixture_ready_model(tmp_path, repository) for repository in ("one/model", "two/model")
    ]
    store.write_registry(
        tmp_path, store.ModelRegistry(schema_version=2, models=records, legacy_unavailable=[])
    )
    config = tmp_path / records[0].path / "config.json"
    raw = json.loads(config.read_text())
    raw["id2label"]["1"] = label
    config.write_text(json.dumps(raw))
    assert [m.compatible for m in Engine(tmp_path).available_models()] == [False, True]


def test_malformed_tokenizer_metadata_is_an_incompatible_model(tmp_path):
    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine

    record = fixture_ready_model(tmp_path)
    store.write_registry(
        tmp_path, store.ModelRegistry(schema_version=2, models=[record], legacy_unavailable=[])
    )
    (tmp_path / record.path / "tokenizer_config.json").write_text("[]")
    assert not Engine(tmp_path).available_models()[0].compatible


def test_write_refuses_future_registry_without_replacing_it(tmp_path):
    from redactio_sidecar import model_store as store

    before = b'{"schema_version":99,"models":[]}'
    (tmp_path / "manifest.json").write_bytes(before)
    with pytest.raises(EngineError, match="invalid_model_manifest"):
        store.write_registry(
            tmp_path, store.ModelRegistry(schema_version=2, models=[], legacy_unavailable=[])
        )
    assert (tmp_path / "manifest.json").read_bytes() == before


@pytest.mark.parametrize("location", ["ancestor", "root", "manifest", "model", "artifact"])
def test_windows_reparse_attributes_are_rejected_before_discovery(tmp_path, monkeypatch, location):
    import stat
    from types import SimpleNamespace

    from redactio_sidecar import model_store as store
    from redactio_sidecar.engine import Engine

    root = tmp_path / "models"
    record = fixture_ready_model(root)
    store.write_registry(
        root, store.ModelRegistry(schema_version=2, models=[record], legacy_unavailable=[])
    )
    model_path = root / record.path
    redirect = {
        "ancestor": tmp_path,
        "root": root,
        "manifest": root / "manifest.json",
        "model": model_path,
        "artifact": model_path / "config.json",
    }[location]
    original_lstat = Path.lstat

    def lstat(path, *args, **kwargs):
        attributes = original_lstat(path, *args, **kwargs)
        if path == redirect:
            return SimpleNamespace(
                st_mode=attributes.st_mode, st_file_attributes=stat.FILE_ATTRIBUTE_REPARSE_POINT
            )
        return attributes

    monkeypatch.setattr(Path, "lstat", lstat)
    if location in {"ancestor", "root", "manifest"}:
        with pytest.raises(EngineError, match="invalid_model_manifest"):
            store.read_registry(root)
    else:
        assert not Engine(root).available_models()[0].compatible
        assert not store.compatible_descriptor(model_path, record.descriptor)


@pytest.mark.parametrize("location", ["root", "ancestor", "dangling_manifest"])
def test_registry_rejects_linked_roots_ancestors_and_dangling_manifests(tmp_path, location):
    from redactio_sidecar.model_store import read_registry

    root = tmp_path / "real" / "models"
    root.mkdir(parents=True)
    if location == "dangling_manifest":
        (root / "manifest.json").symlink_to(tmp_path / "missing.json")
    else:
        link = tmp_path / "linked"
        link.symlink_to(root if location == "root" else root.parent, target_is_directory=True)
        root = link if location == "root" else link / "models"
    with pytest.raises(EngineError, match="invalid_model_manifest"):
        read_registry(root)


def test_management_boundaries_match_unicode_labels_progress_and_error_codes():
    import json

    from pydantic import TypeAdapter

    from redactio_sidecar import model_store as store
    from redactio_sidecar.schemas import EntityType

    fixture = json.loads(
        (Path(__file__).parents[3] / "tests/fixtures/model-management.json").read_text()
    )
    for schema, key in [
        (store.ModelDescriptor, "descriptor"),
        (store.ManagedModel, "managed_model"),
    ]:
        for title in ["", "line\x00break"]:
            assert schema.model_validate({**fixture[key], "title": title}).title == title
        assert schema.model_validate({**fixture[key], "name": "😀" * 257}).name == "😀" * 257
    assert TypeAdapter(EntityType).validate_python("LABEL_0") == "LABEL_0"
    for code in ["model_invalid", "a" * 128]:
        assert store.SafeError(code=code, retryable=False).code == code


@pytest.mark.parametrize(
    "schema,key,override",
    [
        ("ModelDescriptor", "descriptor", {"entity_types": ["LABEL_0"]}),
        ("ManagedModel", "managed_model", {"entity_types": ["LABEL_123"]}),
        ("ModelDescriptor", "descriptor", {"name": "😀" * 513}),
        ("ManagedModel", "managed_model", {"name": "😀" * 513}),
        ("ModelJob", "job", {"downloaded_bytes": 1025}),
        *[
            ("SafeError", "error", {"code": code})
            for code in ["", "Model", "private path", "a" * 129, "bad\n"]
        ],
    ],
)
def test_management_contract_rejection_variants(schema, key, override):
    import json

    from pydantic import ValidationError

    from redactio_sidecar import model_store as store

    fixture = json.loads(
        (Path(__file__).parents[3] / "tests/fixtures/model-management.json").read_text()
    )
    fixture["error"] = {"code": "model_invalid", "retryable": False}
    with pytest.raises(ValidationError):
        getattr(store, schema).model_validate({**fixture[key], **override})


def test_safe_path_handles_relative_roots_without_following_links(tmp_path, monkeypatch):
    from redactio_sidecar.model_store import _safe_path

    root = tmp_path / "models"
    root.mkdir()
    (tmp_path / "linked").symlink_to(root, target_is_directory=True)
    monkeypatch.chdir(tmp_path)
    assert _safe_path(Path("models"), "manifest.json") == root / "manifest.json"
    with pytest.raises(ValueError, match="links"):
        _safe_path(Path("linked"), "manifest.json")


@pytest.mark.parametrize("family", ["bert", "deberta-v2"])
@pytest.mark.parametrize(
    "options",
    [
        {"tokenizer_file": "/synthetic/outside/tokenizer.json"},
        {"tokenizer_file": "../tokenizer.json"},
        {"vocab_file": "/synthetic/outside/vocab.txt", "from_slow": True},
        {"from_slow": True},
        {"fast_tokenizer_files": ["../tokenizer.4.0.json"]},
        {"gguf_file": "/synthetic/outside/model.gguf"},
        {"sp_model_kwargs": {"model_file": "/synthetic/outside/spm.model"}},
        {"init_inputs": ["/synthetic/outside/vocab.txt"]},
        {"tokenizer_class": "GPT2Tokenizer"},
        {"tokenizer_object": {}},
    ],
)
def test_runtime_tokenizer_rejects_loader_controls_before_construction(
    tmp_path, monkeypatch, family, options
):
    import json

    import transformers

    from redactio_sidecar import model_store

    (tmp_path / "config.json").write_text(json.dumps({"model_type": family}))
    (tmp_path / "tokenizer_config.json").write_text(
        json.dumps(
            {
                "tokenizer_class": "BertTokenizer" if family == "bert" else "DebertaV2Tokenizer",
                "added_tokens_decoder": {},
                **options,
            }
        )
    )
    (tmp_path / "tokenizer.json").write_text("{}")
    for name in ("BertTokenizerFast", "DebertaV2TokenizerFast", "BertTokenizer"):
        cls = getattr(transformers, name)
        monkeypatch.setattr(cls, "__init__", lambda *a, **kw: pytest.fail("tokenizer constructed"))
    # Guard Python auxiliary reads as well as the native read inside the intercepted constructors.
    import builtins
    import io

    def guarded_open(original):
        def read(file, *args, **kwargs):
            if isinstance(file, (str, Path)):
                target = Path(file).resolve()
                if target.name in {"tokenizer.json", "vocab.txt", "spm.model", "model.gguf"}:
                    assert target.is_relative_to(tmp_path), "external tokenizer asset read"
            return original(file, *args, **kwargs)

        return read

    monkeypatch.setattr(builtins, "open", guarded_open(builtins.open))
    monkeypatch.setattr(io, "open", guarded_open(io.open))
    with pytest.raises(EngineError, match="model_incompatible"):
        model_store._load_local_tokenizer(tmp_path, family)


def test_special_token_map_cannot_inject_loader_controls(tmp_path, monkeypatch):
    import json

    from redactio_sidecar import model_store

    (tmp_path / "config.json").write_text('{"model_type":"bert"}')
    (tmp_path / "tokenizer_config.json").write_text('{"tokenizer_class":"BertTokenizer"}')
    (tmp_path / "tokenizer.json").write_text("{}")
    (tmp_path / "special_tokens_map.json").write_text(
        json.dumps({"from_slow": True, "vocab_file": "/synthetic/outside/vocab.txt"})
    )
    with pytest.raises(EngineError, match="model_incompatible"):
        model_store._load_local_tokenizer(tmp_path, "bert")


@pytest.mark.parametrize("family", ["bert", "deberta-v2"])
def test_shared_fast_loader_preserves_native_token_metadata(tmp_path, monkeypatch, family):
    import json

    import transformers

    from redactio_sidecar import model_store

    vocab = tmp_path / "vocab.txt"
    vocab.write_text("[PAD]\n[UNK]\n[CLS]\n[SEP]\n[MASK]\nhans\nende\n", encoding="utf-8")
    original = transformers.BertTokenizerFast(vocab_file=str(vocab), model_max_length=32)
    original.save_pretrained(tmp_path)
    config_path = tmp_path / "tokenizer_config.json"
    config = json.loads(config_path.read_text())
    config["tokenizer_class"] = "BertTokenizer" if family == "bert" else "DebertaV2Tokenizer"
    config["tokenizer_file"] = "tokenizer.json"
    config_path.write_text(json.dumps(config))
    (tmp_path / "config.json").write_text(json.dumps({"model_type": family}))
    # These auxiliary links must never be scanned by the fixed asset map.
    (tmp_path / "chat_templates").symlink_to(tmp_path / "outside", target_is_directory=True)
    monkeypatch.setattr(
        transformers.BertTokenizer,
        "__init__",
        lambda *a, **kw: pytest.fail("slow tokenizer constructed"),
    )
    loaded = model_store._load_local_tokenizer(tmp_path, family)
    text = "Hans Ende 😀"
    assert loaded(text, return_offsets_mapping=True) == original(text, return_offsets_mapping=True)
    assert loaded.num_special_tokens_to_add(pair=False) == 2
    assert loaded.model_max_length == 32
    assert loaded.__class__.__name__ == config["tokenizer_class"] + "Fast"


@pytest.mark.parametrize("consumer", ["worker", "inference"])
def test_worker_and_inference_use_shared_tokenizer_boundary(tmp_path, monkeypatch, consumer):
    import json

    import transformers

    from redactio_sidecar import model_manager, model_store

    (tmp_path / "config.json").write_text('{"model_type":"bert"}')
    (tmp_path / "tokenizer_config.json").write_text(
        json.dumps(
            {
                "tokenizer_class": "BertTokenizer",
                "tokenizer_file": "/synthetic/outside/tokenizer.json",
            }
        )
    )
    (tmp_path / "tokenizer.json").write_text("{}")
    monkeypatch.setattr(
        transformers.BertTokenizerFast,
        "__init__",
        lambda *a, **kw: pytest.fail("tokenizer constructed"),
    )
    monkeypatch.setattr(
        transformers.AutoModelForTokenClassification,
        "from_pretrained",
        lambda *a, **kw: pytest.fail("model constructed"),
    )
    with pytest.raises(EngineError, match="model_incompatible"):
        if consumer == "worker":
            model_manager._tokenizer_overhead(tmp_path)
        else:
            model_store._load_local_model(tmp_path, "bert", "BertForTokenClassification")


@pytest.mark.parametrize("present", [False, True])
def test_missing_or_invalid_fast_serialization_never_uses_slow_fallback(
    tmp_path, monkeypatch, present
):
    import transformers

    from redactio_sidecar import model_store

    (tmp_path / "config.json").write_text('{"model_type":"bert"}')
    (tmp_path / "tokenizer_config.json").write_text(
        '{"tokenizer_class":"BertTokenizer","added_tokens_decoder":{}}'
    )
    if present:
        (tmp_path / "tokenizer.json").write_text("{}")
    monkeypatch.setattr(
        transformers.BertTokenizer,
        "_from_pretrained",
        lambda *a, **kw: pytest.fail("slow tokenizer resolved"),
    )
    monkeypatch.setattr(
        transformers.BertTokenizer,
        "__init__",
        lambda *a, **kw: pytest.fail("slow tokenizer constructed"),
    )
    with pytest.raises(Exception):
        model_store._load_local_tokenizer(tmp_path, "bert")
