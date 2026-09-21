from __future__ import annotations

import json
import os
import socket
import sys
from pathlib import Path
from types import SimpleNamespace
from uuid import uuid4

import pytest
from pydantic import ValidationError

from redactio_sidecar.engine import Engine
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.schemas import ProcessingConfig, WordRule

NAME = "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
SHA = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1"
HUGGINGLIL_NAME = "pii-sensitive-ner-german"
HUGGINGLIL_SHA = "6af88facbb75da7be737da55d2c411c7ce79e5a1"
HUGGINGLIL_LABELS = (
    "ACCOUNTNUM",
    "BUILDINGNUM",
    "CITY",
    "CREDITCARDNUMBER",
    "DATEOFBIRTH",
    "DRIVERLICENSENUM",
    "EMAIL",
    "GIVENNAME",
    "IDCARDNUM",
    "PASSWORD",
    "SOCIALNUM",
    "STREET",
    "SURNAME",
    "TAXNUM",
    "TELEPHONENUM",
    "USERNAME",
    "ZIPCODE",
    "REL",
    "ETHN",
    "SOR",
)


@pytest.fixture
def bert_root(tmp_path: Path) -> Path:
    model = tmp_path / "biomedbert-de"
    model.mkdir()
    (tmp_path / "manifest.json").write_text(
        json.dumps({"models": [{"name": NAME, "version": SHA, "path": model.name}]})
    )
    (model / "redactio-model.json").write_text(
        json.dumps(
            {
                "name": NAME,
                "version": SHA,
                "repository": "OpenMed/" + NAME,
            }
        )
    )
    (model / "config.json").write_text(
        json.dumps(
            {
                "model_type": "bert",
                "architectures": ["BertForTokenClassification"],
                "max_position_embeddings": 512,
                "id2label": {
                    "0": "O",
                    "1": "B-FIRSTNAME",
                    "2": "I-FIRSTNAME",
                    "3": "B-LASTNAME",
                    "4": "B-ZIPCODE",
                    "5": "B-AGE",
                    "6": "B-ORGANIZATION",
                },
            }
        )
    )
    for name in ("model.safetensors", "tokenizer.json", "tokenizer_config.json"):
        (model / name).write_text("{}")
    return tmp_path


@pytest.fixture
def dual_model_root(bert_root: Path) -> Path:
    model = bert_root / HUGGINGLIL_NAME
    model.mkdir()
    (bert_root / "manifest.json").write_text(
        json.dumps(
            {
                "models": [
                    {"name": NAME, "version": SHA, "path": "biomedbert-de"},
                    {"name": HUGGINGLIL_NAME, "version": HUGGINGLIL_SHA, "path": model.name},
                ]
            }
        ),
        encoding="utf-8",
    )
    (model / "redactio-model.json").write_text(
        json.dumps(
            {
                "name": HUGGINGLIL_NAME,
                "version": HUGGINGLIL_SHA,
                "repository": "HuggingLil/pii-sensitive-ner-german",
            }
        ),
        encoding="utf-8",
    )
    (model / "config.json").write_text(
        json.dumps(
            {
                "model_type": "deberta-v2",
                "architectures": ["DebertaV2ForTokenClassification"],
                "max_position_embeddings": 512,
                "id2label": {
                    **{str(index): f"I-{label}" for index, label in enumerate(HUGGINGLIL_LABELS)},
                    "20": "O",
                },
            }
        ),
        encoding="utf-8",
    )
    for name in ("model.safetensors", "tokenizer.json", "tokenizer_config.json"):
        (model / name).write_text("{}", encoding="utf-8")
    return bert_root


def test_pinned_bert_is_available_without_loading_weights(bert_root: Path) -> None:
    assert Engine(bert_root).available_models()[0].model_dump() == {
        "name": NAME,
        "version": SHA,
        "compatible": True,
        "entity_types": ["AGE", "FIRSTNAME", "LASTNAME", "ORGANIZATION", "ZIPCODE"],
    }


def test_hugginglil_discovery_requires_its_pinned_deberta_metadata(dual_model_root: Path) -> None:
    models = {model.name: model for model in Engine(dual_model_root).available_models()}

    assert models[HUGGINGLIL_NAME].model_dump() == {
        "name": HUGGINGLIL_NAME,
        "version": HUGGINGLIL_SHA,
        "compatible": True,
        "entity_types": sorted(HUGGINGLIL_LABELS),
    }

    model = dual_model_root / HUGGINGLIL_NAME
    metadata = json.loads((model / "redactio-model.json").read_text(encoding="utf-8"))
    metadata["version"] = "wrong"
    (model / "redactio-model.json").write_text(json.dumps(metadata), encoding="utf-8")
    assert not {entry.name: entry for entry in Engine(dual_model_root).available_models()}[
        HUGGINGLIL_NAME
    ].compatible
    metadata["version"] = HUGGINGLIL_SHA
    (model / "redactio-model.json").write_text(json.dumps(metadata), encoding="utf-8")
    base_config = json.loads((model / "config.json").read_text(encoding="utf-8"))
    for changed in (
        {"model_type": "bert", "architectures": ["DebertaV2ForTokenClassification"]},
        {"model_type": "deberta-v2", "architectures": ["BertForTokenClassification"]},
        {
            "model_type": "deberta-v2",
            "architectures": ["DebertaV2ForTokenClassification"],
            "id2label": {"0": "I-bad"},
        },
    ):
        config = base_config.copy()
        config.update(changed)
        (model / "config.json").write_text(json.dumps(config), encoding="utf-8")
        assert not {entry.name: entry for entry in Engine(dual_model_root).available_models()}[
            HUGGINGLIL_NAME
        ].compatible


def test_hugginglil_requires_native_selection_and_preserves_native_labels(
    dual_model_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from redactio_sidecar import biomedbert

    def pipeline(text: str) -> list[dict[str, object]]:
        return [
            {"entity_group": "GIVENNAME", "start": 0, "end": 4, "score": 0.99},
            {"entity_group": "REL", "start": 5, "end": 12, "score": 0.99},
        ]

    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda *_: pipeline)
    engine = Engine(dual_model_root)
    pair, revision = str(uuid4()), str(uuid4())

    with pytest.raises(EngineError, match="^invalid_configuration$"):
        engine.configure(pair, revision, ProcessingConfig(model=HUGGINGLIL_NAME))

    info = engine.configure(
        pair,
        revision,
        ProcessingConfig(
            model=HUGGINGLIL_NAME,
            enabled_entities=["EMAIL_ADDRESS"],
            model_entities=["GIVENNAME", "REL"],
        ),
    )
    detections = engine.analyze(pair, revision, "Anna Kirche anna@example.com")

    assert info.recognizers == ["HuggingLilRecognizer", "EmailRecognizer"]
    assert {(item.entity_type, item.recognizer) for item in detections} >= {
        ("GIVENNAME", "HuggingLilRecognizer"),
        ("REL", "HuggingLilRecognizer"),
    }
    assert engine.info(pair, revision).model_name == HUGGINGLIL_NAME


def test_switching_from_hugginglil_to_bert_preserves_bert_legacy_output(
    dual_model_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from redactio_sidecar import biomedbert

    def pipeline(path: Path, *_: object):
        if path.name == HUGGINGLIL_NAME:
            return lambda _: [{"entity_group": "GIVENNAME", "start": 0, "end": 4, "score": 0.99}]
        return lambda _: [{"entity_group": "FIRSTNAME", "start": 0, "end": 4, "score": 0.99}]

    monkeypatch.setattr(biomedbert, "_load_pipeline", pipeline)
    engine = Engine(dual_model_root)
    pair, revision = str(uuid4()), str(uuid4())
    engine.configure(
        pair,
        revision,
        ProcessingConfig(model=HUGGINGLIL_NAME, enabled_entities=[], model_entities=["GIVENNAME"]),
    )
    assert [item.entity_type for item in engine.analyze(pair, revision, "Anna")] == ["GIVENNAME"]

    next_pair, next_revision = str(uuid4()), str(uuid4())
    info = engine.configure(next_pair, next_revision, ProcessingConfig(model=NAME))
    assert info.recognizers[0] == "BiomedBertRecognizer"
    assert [item.entity_type for item in engine.analyze(next_pair, next_revision, "Anna")] == [
        "PERSON"
    ]


def test_model_rejects_missing_or_malformed_native_labels(bert_root: Path) -> None:
    model = bert_root / "biomedbert-de"
    for labels in (
        None,
        {"0": "O", "1": "B-bad"},
        {"not-an-id": "B-FIRSTNAME"},
        ["B-FIRSTNAME"],
    ):
        config = {"model_type": "bert"}
        if labels is not None:
            config["id2label"] = labels
        (model / "config.json").write_text(json.dumps(config))
        assert not Engine(bert_root).available_models()[0].compatible


def test_native_config_rejects_invalid_and_unsupported_label_ids(bert_root: Path) -> None:
    with pytest.raises(ValidationError):
        ProcessingConfig(model_entities=["not_valid"])
    with pytest.raises(EngineError, match="^invalid_configuration$"):
        Engine(bert_root).configure(
            str(uuid4()), str(uuid4()), ProcessingConfig(model=NAME, model_entities=["PERSON"])
        )
    with pytest.raises(EngineError, match="^invalid_configuration$"):
        Engine(bert_root).configure(
            str(uuid4()),
            str(uuid4()),
            ProcessingConfig(model=NAME, enabled_entities=["CUSTOM"], model_entities=["FIRSTNAME"]),
        )


@pytest.mark.parametrize(
    "missing",
    [
        "redactio-model.json",
        "config.json",
        "model.safetensors",
        "tokenizer.json",
        "tokenizer_config.json",
    ],
)
def test_incomplete_bert_is_incompatible(bert_root: Path, missing: str) -> None:
    (bert_root / "biomedbert-de" / missing).unlink()
    assert not Engine(bert_root).available_models()[0].compatible


@pytest.mark.parametrize(
    "filename,content",
    [
        (
            "redactio-model.json",
            {"name": NAME, "version": "wrong", "repository": "OpenMed/" + NAME},
        ),
        ("config.json", {"model_type": "remote_custom_model"}),
    ],
)
def test_wrong_model_identity_is_incompatible(bert_root: Path, filename: str, content) -> None:
    (bert_root / "biomedbert-de" / filename).write_text(json.dumps(content))
    assert not Engine(bert_root).available_models()[0].compatible


def test_native_offsets_merge_only_predicted_components_and_deduplicate(monkeypatch) -> None:
    from redactio_sidecar import biomedbert

    text = "😀 Jörg Müller, Hauptstraße 12, 10115 Berlin. Danach Anna."
    spans = [
        ("FIRSTNAME", "Jörg"),
        ("LASTNAME", "Müller"),
        ("STREET", "Hauptstraße"),
        ("BUILDINGNUMBER", "12"),
        ("ZIPCODE", "10115"),
        ("CITY", "Berlin"),
        ("FIRSTNAME", "Anna"),
        ("LASTNAME", "Müller"),
    ]
    output = [
        {
            "entity_group": label,
            "start": text.index(word),
            "end": text.index(word) + len(word),
            "score": 0.9,
            "word": word,
        }
        for label, word in spans
    ]
    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda _: lambda _: output)
    recognizer = biomedbert.BiomedBertRecognizer(Path("unused"))
    recognizer.configure_labels(
        ["FIRSTNAME", "LASTNAME", "STREET", "BUILDINGNUMBER", "ZIPCODE", "CITY"],
        legacy=True,
    )

    results = recognizer.analyze(text, ["PERSON", "LOCATION"], None)

    assert [(r.entity_type, text[r.start : r.end]) for r in results] == [
        ("PERSON", "Jörg Müller"),
        ("LOCATION", "Hauptstraße 12, 10115 Berlin"),
        ("PERSON", "Anna"),
    ]
    assert [text[r.start : r.end] for r in recognizer.analyze(text, ["LOCATION"], None)] == [
        "Hauptstraße 12, 10115 Berlin"
    ]


def test_native_labels_keep_model_codes_and_merge_only_matching_labels(monkeypatch) -> None:
    from redactio_sidecar import biomedbert

    text = "Anna Muster, 10115 Berlin, 42 Jahre, Klinik Mitte"

    def span(label: str, value: str, score: float) -> dict[str, object]:
        start = text.index(value)
        return {"entity_group": label, "start": start, "end": start + len(value), "score": score}

    output = [
        span("FIRSTNAME", "Anna", 0.91),
        span("LASTNAME", "Muster", 0.92),
        span("ZIPCODE", "10115", 0.93),
        span("AGE", "42", 0.94),
        span("ORGANIZATION", "Klinik", 0.95),
        span("ORGANIZATION", "Mitte", 0.96),
    ]
    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda _: lambda _: output)

    recognizer = biomedbert.BiomedBertRecognizer(
        Path("unused"), ("FIRSTNAME", "LASTNAME", "ZIPCODE", "AGE", "ORGANIZATION")
    )
    results = recognizer.analyze(
        text, ["FIRSTNAME", "LASTNAME", "ZIPCODE", "AGE", "ORGANIZATION"], None
    )

    assert [(result.entity_type, text[result.start : result.end]) for result in results] == [
        ("FIRSTNAME", "Anna"),
        ("LASTNAME", "Muster"),
        ("ZIPCODE", "10115"),
        ("AGE", "42"),
        ("ORGANIZATION", "Klinik Mitte"),
    ]


def test_native_selection_filters_model_labels_without_disabling_supplementary_recognizers(
    bert_root: Path, monkeypatch
) -> None:
    from redactio_sidecar import biomedbert

    monkeypatch.setattr(
        biomedbert,
        "_load_pipeline",
        lambda _: (
            lambda _: [
                {"entity_group": "FIRSTNAME", "start": 0, "end": 4, "score": 0.99},
                {"entity_group": "ZIPCODE", "start": 5, "end": 10, "score": 0.99},
            ]
        ),
    )
    engine = Engine(bert_root)
    pair, revision = str(uuid4()), str(uuid4())
    engine.configure(
        pair,
        revision,
        ProcessingConfig(
            model=NAME,
            model_entities=["ZIPCODE"],
            enabled_entities=["EMAIL_ADDRESS"],
        ),
    )

    assert {item.entity_type for item in engine.analyze(pair, revision, "Anna 10115 a@b.de")} == {
        "ZIPCODE",
        "EMAIL_ADDRESS",
    }


def test_inference_failure_is_not_an_empty_success(monkeypatch) -> None:
    from redactio_sidecar import biomedbert

    def broken(_):
        raise RuntimeError("private text must not become an error message")

    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda _: broken)
    recognizer = biomedbert.BiomedBertRecognizer(Path("unused"))
    with pytest.raises(EngineError, match="^internal_error$"):
        recognizer.analyze("private", ["PERSON"], None)


@pytest.mark.parametrize("start,end,score", [(-1, 2, 0.9), (0, 100, 0.9), (0, 2, float("nan"))])
def test_invalid_inference_offsets_and_scores_fail_closed(monkeypatch, start, end, score) -> None:
    from redactio_sidecar import biomedbert

    monkeypatch.setattr(
        biomedbert,
        "_load_pipeline",
        lambda _: (
            lambda _: [{"entity_group": "FIRSTNAME", "start": start, "end": end, "score": score}]
        ),
    )
    with pytest.raises(EngineError, match="^internal_error$"):
        biomedbert.BiomedBertRecognizer(Path("unused")).analyze("Anna", ["FIRSTNAME"], None)


def test_model_load_failure_does_not_activate_pair(bert_root, monkeypatch) -> None:
    from redactio_sidecar import biomedbert

    def broken(_):
        raise OSError("missing weights")

    monkeypatch.setattr(biomedbert, "_load_pipeline", broken)
    engine = Engine(bert_root)
    pair, revision = str(uuid4()), str(uuid4())
    with pytest.raises(EngineError, match="^model_incompatible$"):
        engine.configure(pair, revision, ProcessingConfig(model=NAME))
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(pair, revision, "Anna")


@pytest.mark.parametrize("context", [256, 512, 1024])
def test_pipeline_uses_model_context_for_tokenizer_and_stride(monkeypatch, context: int) -> None:
    from redactio_sidecar import biomedbert

    tokenizer = SimpleNamespace(
        is_fast=True,
        model_max_length=10**30,
        num_special_tokens_to_add=lambda **_: 2,
    )
    model = SimpleNamespace(
        config=SimpleNamespace(
            model_type="bert",
            max_position_embeddings=context,
            architectures=["BertForTokenClassification"],
        )
    )
    captured: dict[str, object] = {}

    def pipeline(*args, **kwargs):
        captured.update(kwargs)
        return lambda _: []

    monkeypatch.setitem(
        sys.modules,
        "transformers",
        SimpleNamespace(
            AutoTokenizer=SimpleNamespace(from_pretrained=lambda *_args, **_kwargs: tokenizer),
            AutoModelForTokenClassification=SimpleNamespace(
                from_pretrained=lambda *_args, **_kwargs: (
                    model,
                    {"missing_keys": [], "mismatched_keys": []},
                )
            ),
            pipeline=pipeline,
        ),
    )

    biomedbert._load_pipeline(Path("unused"))

    assert tokenizer.model_max_length == context
    assert captured["tokenizer"] is tokenizer
    assert captured["stride"] == context // 4


def test_small_context_keeps_long_unicode_offsets(tmp_path):
    torch = pytest.importorskip("torch")
    from transformers import BertConfig, BertForTokenClassification, BertTokenizerFast

    from redactio_sidecar.biomedbert import _load_pipeline, validate_local_model
    from redactio_sidecar.model_store import Window

    vocab = tmp_path / "vocab.txt"
    vocab.write_text("[PAD]\n[UNK]\n[CLS]\n[SEP]\n[MASK]\nhans\nende\n", encoding="utf-8")
    tokenizer = BertTokenizerFast(vocab_file=str(vocab), model_max_length=32)
    tokenizer.save_pretrained(tmp_path)
    config = BertConfig(
        vocab_size=len(tokenizer),
        hidden_size=16,
        num_hidden_layers=1,
        num_attention_heads=2,
        intermediate_size=32,
        max_position_embeddings=32,
        id2label={0: "O", 1: "B-PERSON", 2: "I-PERSON"},
        label2id={"O": 0, "B-PERSON": 1, "I-PERSON": 2},
    )
    model = BertForTokenClassification(config)
    with torch.no_grad():
        model.classifier.weight.zero_()
        model.classifier.bias.copy_(torch.tensor([10.0, 0.0, 0.0]))
    model.save_pretrained(tmp_path, safe_serialization=True)
    text = ("Hans München 😀\r\n" * 30) + "Ende"
    encoded = tokenizer(
        text,
        truncation=True,
        max_length=32,
        stride=8,
        return_overflowing_tokens=True,
        return_offsets_mapping=True,
    )
    assert len(encoded["input_ids"]) > 1
    assert max(end for chunk in encoded["offset_mapping"] for _, end in chunk) == len(text)
    assert validate_local_model(tmp_path, "bert", "BertForTokenClassification") == Window(32, 8)
    with pytest.raises(EngineError, match="^model_incompatible$"):
        validate_local_model(tmp_path, "bert", "DebertaV2ForTokenClassification")
    assert _load_pipeline(tmp_path)(text) == []


def test_real_pipeline_bounds_sentinel_tokenizer_and_covers_chunk_boundaries(tmp_path, monkeypatch):
    torch = pytest.importorskip("torch")
    transformers = pytest.importorskip("transformers")
    from redactio_sidecar.biomedbert import BiomedBertRecognizer, validate_local_model
    from redactio_sidecar.model_store import Window

    # A tiny local model predicts PERSON for every token, so lost windows or bad
    # Unicode offsets leave an observable gap without requiring production weights.
    (tmp_path / "vocab.txt").write_text(
        "[PAD]\n[UNK]\n[CLS]\n[SEP]\n[MASK]\nanna\nmüller\n", encoding="utf-8"
    )
    tokenizer = transformers.BertTokenizerFast(
        vocab_file=str(tmp_path / "vocab.txt"),
        model_max_length=10**30,
    )
    tokenizer.save_pretrained(tmp_path)
    config = transformers.BertConfig(
        vocab_size=7,
        hidden_size=8,
        num_hidden_layers=1,
        num_attention_heads=2,
        intermediate_size=8,
        max_position_embeddings=1024,
        id2label={0: "O", 1: "B-FIRSTNAME"},
        label2id={"O": 0, "B-FIRSTNAME": 1},
    )
    model = transformers.BertForTokenClassification(config)
    with torch.no_grad():
        model.classifier.weight.zero_()
        model.classifier.bias.copy_(torch.tensor([0.0, 10.0]))
    model.save_pretrained(tmp_path, safe_serialization=True)

    def deny_network(*_args, **_kwargs):
        raise AssertionError("runtime network access attempted")

    monkeypatch.setattr(socket.socket, "connect", deny_network)
    text = "😀 Müller " + "Anna " * 1200 + "Müller"
    encoded = tokenizer(
        text,
        truncation=True,
        max_length=1024,
        stride=256,
        return_overflowing_tokens=True,
        return_offsets_mapping=True,
    )
    assert len(encoded["input_ids"]) > 1
    assert max(end for chunk in encoded["offset_mapping"] for _, end in chunk) == len(text)
    assert validate_local_model(tmp_path, "bert", "BertForTokenClassification") == Window(1024, 256)
    recognizer = BiomedBertRecognizer(tmp_path)
    recognizer.configure_labels(["FIRSTNAME"], legacy=False)
    results = recognizer.analyze(text, ["FIRSTNAME"], None)
    for index, char in enumerate(text):
        if not char.isspace():
            assert any(r.start <= index < r.end for r in results), index


def test_bert_pair_toggle_keeps_structured_recognizers(bert_root, monkeypatch) -> None:
    from redactio_sidecar import biomedbert

    monkeypatch.setattr(
        biomedbert,
        "_load_pipeline",
        lambda _: (
            lambda text: [
                {"entity_group": "FIRSTNAME", "start": 0, "end": 4, "score": 0.99, "word": "Anna"}
            ]
        ),
    )
    engine = Engine(bert_root)
    pair, revision = str(uuid4()), str(uuid4())
    info = engine.configure(pair, revision, ProcessingConfig(model=NAME))
    assert "BiomedBertRecognizer" in info.recognizers
    assert "transformers " in info.engine_version and "torch " in info.engine_version
    detections = engine.analyze(pair, revision, "Anna: anna@example.com")
    assert {d.entity_type for d in detections} >= {"PERSON", "EMAIL_ADDRESS"}

    def reject_reload(_):
        raise AssertionError("cached model weights were reloaded")

    monkeypatch.setattr(biomedbert, "_load_pipeline", reject_reload)
    other_pair, other_revision = str(uuid4()), str(uuid4())
    engine.configure(
        other_pair,
        other_revision,
        ProcessingConfig(
            model=NAME,
            enabled_entities=["EMAIL_ADDRESS"],
        ),
    )
    assert {
        d.entity_type for d in engine.analyze(other_pair, other_revision, "Anna: anna@example.com")
    } == {"EMAIL_ADDRESS"}
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(pair, revision, "Anna")
    engine.configure(pair, revision, ProcessingConfig(model=NAME, enabled_entities=["PERSON"]))
    assert [d.entity_type for d in engine.analyze(pair, revision, "Anna")] == ["PERSON"]


def test_custom_person_rule_does_not_reenable_disabled_automatic_person(bert_root, monkeypatch):
    from redactio_sidecar import biomedbert

    monkeypatch.setattr(
        biomedbert,
        "_load_pipeline",
        lambda _: (
            lambda _: [
                {"entity_group": "FIRSTNAME", "start": 0, "end": 4, "score": 0.99, "word": "Anna"}
            ]
        ),
    )
    engine = Engine(bert_root)
    pair, revision = str(uuid4()), str(uuid4())
    rule = WordRule(
        id=str(uuid4()), entity_type="PERSON", enabled=True, kind="words", words=["NeverMatched"]
    )
    engine.configure(
        pair,
        revision,
        ProcessingConfig(
            model=NAME,
            enabled_entities=[],
            model_entities=["ZIPCODE"],
            custom_rules=[rule],
        ),
    )
    assert engine.analyze(pair, revision, "Anna") == []
    results = engine.analyze(pair, revision, "Anna NeverMatched")
    assert [(d.start, d.end, d.recognizer) for d in results] == [(5, 17, rule.id)]
    # Reusing weights for another pair must retain its independently enabled types.
    second, second_revision = str(uuid4()), str(uuid4())
    engine.configure(
        second,
        second_revision,
        ProcessingConfig(model=NAME, enabled_entities=[], model_entities=["FIRSTNAME"]),
    )
    assert [d.entity_type for d in engine.analyze(second, second_revision, "Anna")] == ["FIRSTNAME"]


@pytest.mark.skipif(
    not os.environ.get("REDACTIO_BIOMEDBERT_MODEL_DIR"),
    reason="requires explicitly prepared pinned BiomedBERT weights",
)
def test_real_bert_offline_unicode_long_document(monkeypatch) -> None:
    def deny_network(*_args, **_kwargs):
        raise AssertionError("runtime network access attempted")

    monkeypatch.setattr(socket.socket, "connect", deny_network)
    engine = Engine(Path(os.environ["REDACTIO_BIOMEDBERT_MODEL_DIR"]))
    pair, revision = str(uuid4()), str(uuid4())
    engine.configure(pair, revision, ProcessingConfig(model=NAME))
    sentence = "😀 Der Patient Jörg Müller wohnt in der Hauptstraße 12, 10115 Berlin. "
    # Places names both near the 512-token window boundary and well after it.
    text = (
        sentence + "Befund unauffällig. " * 125 + sentence + "Befund unauffällig. " * 160 + sentence
    )
    detections = engine.analyze(pair, revision, text)
    for component in ("Jörg", "Müller", "10115", "Berlin"):
        start = 0
        while (start := text.find(component, start)) >= 0:
            assert any(d.start <= start and d.end >= start + len(component) for d in detections), (
                component
            )
            start += len(component)
    # Known recall limitation of these pinned weights: raw logits label every
    # Hauptstraße subword O; neither simple/first/max aggregation recovers it.
    # Keep this visible rather than implying the successful canary covers streets.
    for component in ("Hauptstraße", "12"):
        start = text.index(component)
        assert not any(d.start <= start and d.end >= start + len(component) for d in detections)
    assert all(0 <= d.start < d.end <= len(text) for d in detections)
    assert len({(d.start, d.end, d.entity_type) for d in detections}) == len(detections)
    with pytest.raises(EngineError, match="model_not_found"):
        engine.configure(str(uuid4()), str(uuid4()), ProcessingConfig(model="de_core_news_sm"))
    assert engine.info(pair, revision).model_name == NAME


@pytest.mark.skipif(
    not os.environ.get("REDACTIO_HUGGINGLIL_MODEL_DIR"),
    reason="requires explicitly prepared pinned HuggingLil weights",
)
def test_real_hugginglil_offline_long_document(monkeypatch) -> None:
    def deny_network(*_args, **_kwargs):
        raise AssertionError("runtime network access attempted")

    monkeypatch.setattr(socket.socket, "connect", deny_network)
    engine = Engine(Path(os.environ["REDACTIO_HUGGINGLIL_MODEL_DIR"]))
    pair, revision = str(uuid4()), str(uuid4())
    engine.configure(
        pair,
        revision,
        ProcessingConfig(
            model=HUGGINGLIL_NAME,
            enabled_entities=[],
            model_entities=["GIVENNAME", "SURNAME", "CITY", "REL", "ETHN"],
        ),
    )
    prefix = "Befund unauffällig. " * 300
    tail = "Elena Petrov ist Kosovarin und lebt in Berlin. Weihnachten."
    text = prefix + tail
    detections = engine.analyze(pair, revision, text)

    # The model's rare labels are context-sensitive beyond a window boundary;
    # keep the canary about stride coverage using labels it recalls there.
    for entity, term in (("SURNAME", "Petrov"), ("ETHN", "Kosovarin")):
        start = text.rindex(term)
        assert any(
            detection.entity_type == entity and detection.start <= start < detection.end
            for detection in detections
        ), (entity, term)
