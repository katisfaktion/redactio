from __future__ import annotations

import json
import os
import socket
from pathlib import Path
from uuid import uuid4

import pytest

from redactio_sidecar.engine import Engine
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.schemas import ProcessingConfig, WordRule

NAME = "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
SHA = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1"


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
    (model / "config.json").write_text(json.dumps({"model_type": "bert"}))
    for name in ("model.safetensors", "tokenizer.json", "tokenizer_config.json"):
        (model / name).write_text("{}")
    return tmp_path


def test_pinned_bert_is_available_without_loading_weights(bert_root: Path) -> None:
    assert Engine(bert_root).available_models()[0].compatible


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

    results = recognizer.analyze(text, ["PERSON", "LOCATION"], None)

    assert [(r.entity_type, text[r.start : r.end]) for r in results] == [
        ("PERSON", "Jörg Müller"),
        ("LOCATION", "Hauptstraße 12, 10115 Berlin"),
        ("PERSON", "Anna"),
    ]
    assert [text[r.start : r.end] for r in recognizer.analyze(text, ["LOCATION"], None)] == [
        "Hauptstraße 12, 10115 Berlin"
    ]


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
        biomedbert.BiomedBertRecognizer(Path("unused")).analyze("Anna", ["PERSON"], None)


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


def test_real_pipeline_bounds_sentinel_tokenizer_and_covers_chunk_boundaries(tmp_path, monkeypatch):
    torch = pytest.importorskip("torch")
    transformers = pytest.importorskip("transformers")
    from redactio_sidecar.biomedbert import BiomedBertRecognizer

    # A tiny local model predicts PERSON for every token, so lost windows or bad
    # Unicode offsets leave an observable gap without requiring production weights.
    (tmp_path / "vocab.txt").write_text("[PAD]\n[UNK]\n[CLS]\n[SEP]\n[MASK]\nanna\nmüller\n")
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
        max_position_embeddings=512,
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
    recognizer = BiomedBertRecognizer(tmp_path)
    text = "😀 Müller " + "Anna " * 1200 + "Müller"
    results = recognizer.analyze(text, ["PERSON"], None)
    for index, char in enumerate(text):
        if not char.isspace():
            assert any(r.start <= index < r.end for r in results), index
    assert results[-1].end == len(text)


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
            enabled_entities=["LOCATION"],
            custom_rules=[rule],
        ),
    )
    assert engine.analyze(pair, revision, "Anna") == []
    results = engine.analyze(pair, revision, "Anna NeverMatched")
    assert [(d.start, d.end, d.recognizer) for d in results] == [(5, 17, rule.id)]
    # Reusing weights for another pair must retain its independently enabled types.
    second, second_revision = str(uuid4()), str(uuid4())
    engine.configure(
        second, second_revision, ProcessingConfig(model=NAME, enabled_entities=["PERSON"])
    )
    assert [d.entity_type for d in engine.analyze(second, second_revision, "Anna")] == ["PERSON"]


@pytest.mark.skipif(
    not os.environ.get("REDACTIO_BIOMEDBERT_MODEL_DIR"),
    reason="requires explicitly prepared pinned BiomedBERT weights",
)
def test_real_bert_offline_unicode_long_document_and_pair_switch(monkeypatch) -> None:
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
    second, second_revision = str(uuid4()), str(uuid4())
    info = engine.configure(second, second_revision, ProcessingConfig(model="de_core_news_sm"))
    assert "SpacyRecognizer" in info.recognizers
    assert "BiomedBertRecognizer" not in info.recognizers
    assert engine.analyze(second, second_revision, "Max Mustermann wohnt in Berlin.")
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(pair, revision, text)
