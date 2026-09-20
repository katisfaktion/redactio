from __future__ import annotations

import json
import os
import socket
from pathlib import Path
from uuid import uuid4

import pytest
from presidio_analyzer import RecognizerRegistry

from redactio_sidecar.engine import Engine
from redactio_sidecar.ipc import EngineError
from redactio_sidecar.schemas import ProcessingConfig, RegexRule, WordRule

NAME = "OpenMed-PII-German-BiomedBERT-Large-340M-v1"


@pytest.fixture(scope="session")
def model_root() -> Path:
    root = Path(os.environ.get("REDACTIO_MODEL_DIR", Path(__file__).parents[1] / "models"))
    assert (root / "manifest.json").is_file()
    return root


@pytest.fixture(autouse=True)
def synthetic_bert_pipeline(monkeypatch: pytest.MonkeyPatch) -> None:
    from redactio_sidecar import biomedbert

    def pipeline(text: str) -> list[dict[str, object]]:
        results: list[dict[str, object]] = []
        for label, term in (
            ("FIRSTNAME", "Max"),
            ("LASTNAME", "Mustermann"),
            ("FIRSTNAME", "Anna"),
            ("CITY", "Berlin"),
        ):
            start = text.find(term)
            if start >= 0:
                results.append(
                    {"entity_group": label, "start": start, "end": start + len(term), "score": 0.99}
                )
        return results

    monkeypatch.setattr(biomedbert, "_load_pipeline", lambda _: pipeline)


@pytest.fixture(scope="session")
def configured_engine(model_root: Path) -> tuple[Engine, str, str]:
    engine = Engine(model_root)
    pair_id, revision = str(uuid4()), str(uuid4())
    engine.configure(pair_id, revision, ProcessingConfig())
    return engine, pair_id, revision


def test_switching_pair_removes_previous_custom_terms(model_root: Path) -> None:
    engine = Engine(model_root)
    pair_a, pair_b, rev_a, rev_b = [str(uuid4()) for _ in range(4)]
    rule = WordRule(
        id=str(uuid4()),
        entity_type="CUSTOM",
        enabled=True,
        kind="words",
        words=["KUNSTWORTXYZ"],
    )
    engine.configure(
        pair_a,
        rev_a,
        ProcessingConfig(enabled_entities=[], custom_rules=[rule]),
    )
    assert engine.analyze(pair_a, rev_a, "KUNSTWORTXYZ")

    engine.configure(pair_b, rev_b, ProcessingConfig(enabled_entities=[], custom_rules=[]))

    assert engine.analyze(pair_b, rev_b, "KUNSTWORTXYZ") == []
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(pair_a, rev_a, "KUNSTWORTXYZ")


def test_empty_snapshot_does_not_install_presidio_defaults(
    model_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def reject_defaults(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("Presidio default registry was loaded")

    monkeypatch.setattr(RecognizerRegistry, "load_predefined_recognizers", reject_defaults)
    engine = Engine(model_root)

    engine.configure(
        pair_id := str(uuid4()),
        revision := str(uuid4()),
        ProcessingConfig(enabled_entities=["CUSTOM"], custom_rules=[]),
    )

    assert engine.analyze(pair_id, revision, "KUNSTWORTXYZ") == []


@pytest.mark.parametrize(
    ("entity_type", "text"),
    [
        ("PERSON", "Max Mustermann besucht uns."),
        ("LOCATION", "Der Termin ist in Berlin."),
        ("EMAIL_ADDRESS", "Kontakt: max@example.com"),
        ("PHONE_NUMBER", "Telefon: +49 30 12345678"),
        ("IBAN_CODE", "IBAN: DE89370400440532013000"),
        ("IP_ADDRESS", "Server: 192.168.1.1"),
        ("URL", "Webseite: https://example.de/path"),
        ("DATE_TIME", "Termin: 19.09.2026"),
    ],
)
def test_default_categories_have_effective_german_recognizers(
    configured_engine: tuple[Engine, str, str], entity_type: str, text: str
) -> None:
    engine, pair_id, revision = configured_engine

    detections = engine.analyze(pair_id, revision, text)

    assert entity_type in {detection.entity_type for detection in detections}
    assert engine.info(pair_id, revision).recognizers == [
        "BiomedBertRecognizer",
        "EmailRecognizer",
        "PhoneRecognizer",
        "IbanRecognizer",
        "IpRecognizer",
        "UrlRecognizer",
        "DateRecognizer",
    ]
    assert all(
        detection.recognizer.endswith("Recognizer") or detection.recognizer.count("-") == 4
        for detection in detections
    )


def test_configure_and_analysis_do_not_use_a_downloader(
    model_root: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def deny_network(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("runtime network access attempted")

    monkeypatch.setattr(socket.socket, "connect", deny_network)
    engine = Engine(model_root)
    pair_id, revision = str(uuid4()), str(uuid4())

    engine.configure(pair_id, revision, ProcessingConfig())
    detections = engine.analyze(pair_id, revision, "Anna wohnt in Berlin.")

    assert {detection.entity_type for detection in detections} >= {"PERSON", "LOCATION"}


def test_word_rules_are_literal_deduplicated_and_use_appropriate_boundaries(
    model_root: Path,
) -> None:
    engine = Engine(model_root)
    pair_id, revision, rule_id = str(uuid4()), str(uuid4()), str(uuid4())
    rule = WordRule(
        id=rule_id,
        entity_type="CUSTOM",
        enabled=True,
        kind="words",
        words=["Anna", "Anna", "C", "C++"],
    )
    engine.configure(
        pair_id,
        revision,
        ProcessingConfig(enabled_entities=[], custom_rules=[rule]),
    )

    detections = engine.analyze(pair_id, revision, "XAnna Anna C++")

    assert [(d.start, d.end, d.confidence, d.recognizer) for d in detections] == [
        (6, 10, 1.0, rule_id),
        (11, 14, 1.0, rule_id),
    ]


def test_preview_rules_uses_the_same_pair_revision_boundary(model_root: Path) -> None:
    engine = Engine(model_root)
    pair_id, revision, rule_id = str(uuid4()), str(uuid4()), str(uuid4())
    config = ProcessingConfig(
        enabled_entities=[],
        custom_rules=[
            RegexRule(
                id=rule_id,
                entity_type="CUSTOM",
                enabled=True,
                kind="regex",
                pattern=r"ID-[0-9]+",
            )
        ],
    )
    engine.configure(pair_id, revision, config)

    assert engine.preview_rules(pair_id, revision, "ID-42") == engine.analyze(
        pair_id, revision, "ID-42"
    )
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.preview_rules(pair_id, str(uuid4()), "ID-42")


def test_engine_info_uses_the_same_pair_revision_boundary(model_root: Path) -> None:
    engine = Engine(model_root)
    pair_id, revision = str(uuid4()), str(uuid4())

    configured = engine.configure(pair_id, revision, ProcessingConfig())

    assert engine.info(pair_id, revision) == configured
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.info(str(uuid4()), revision)


@pytest.mark.parametrize(
    "invalid_rule",
    [
        RegexRule.model_construct(
            id="11111111-1111-4111-8111-111111111111",
            entity_type="CUSTOM",
            enabled=True,
            kind="regex",
            pattern="(",
        ),
        WordRule.model_construct(
            id="22222222-2222-4222-8222-222222222222",
            entity_type="CUSTOM",
            enabled=True,
            kind="words",
            words=[],
        ),
    ],
)
def test_invalid_rule_is_transactional_and_identifies_attempted_pair(
    model_root: Path, invalid_rule: RegexRule | WordRule
) -> None:
    engine = Engine(model_root)
    active_pair, active_revision = str(uuid4()), str(uuid4())
    attempted_pair, attempted_revision = str(uuid4()), str(uuid4())
    engine.configure(active_pair, active_revision, ProcessingConfig(enabled_entities=[]))
    invalid = ProcessingConfig.model_construct(
        model=NAME,
        enabled_entities=[],
        model_entities=None,
        custom_rules=[invalid_rule],
        include_positions=True,
    )

    with pytest.raises(EngineError, match="invalid_configuration"):
        engine.configure(attempted_pair, attempted_revision, invalid)

    assert engine.analyze(active_pair, active_revision, "harmless") == []
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(attempted_pair, attempted_revision, "harmless")


def test_missing_model_switch_keeps_previous_snapshot(model_root: Path) -> None:
    engine = Engine(model_root)
    active_pair, active_revision = str(uuid4()), str(uuid4())
    attempted_pair, attempted_revision = str(uuid4()), str(uuid4())
    engine.configure(active_pair, active_revision, ProcessingConfig(enabled_entities=[]))

    with pytest.raises(EngineError, match="model_not_found"):
        engine.configure(
            attempted_pair,
            attempted_revision,
            ProcessingConfig(model="de_core_news_md", enabled_entities=[]),
        )

    assert engine.analyze(active_pair, active_revision, "KUNSTWORTXYZ") == []
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.analyze(attempted_pair, attempted_revision, "KUNSTWORTXYZ")


def test_retired_model_name_is_not_silently_reinterpreted(model_root: Path) -> None:
    engine = Engine(model_root)
    first_pair, first_revision = str(uuid4()), str(uuid4())
    second_pair, second_revision = str(uuid4()), str(uuid4())
    assert engine.configure(first_pair, first_revision, ProcessingConfig()).model_name == NAME
    with pytest.raises(EngineError, match="model_not_found"):
        engine.configure(second_pair, second_revision, ProcessingConfig(model="de_core_news_sm"))
    assert engine.info(first_pair, first_revision).model_name == NAME
    with pytest.raises(EngineError, match="configuration_mismatch"):
        engine.info(second_pair, second_revision)


def test_available_models_validates_missing_incompatible_and_escaped_directories(
    tmp_path: Path,
) -> None:
    missing = tmp_path / "missing"
    incompatible = tmp_path / "incompatible"
    incompatible.mkdir()
    (incompatible / "meta.json").write_text(
        json.dumps(
            {
                "lang": "de",
                "name": "core_news_sm",
                "version": "0.0.0",
                "spacy_version": ">=99",
            }
        ),
        encoding="utf-8",
    )
    (incompatible / "config.cfg").write_text('[nlp]\nlang = "de"\n', encoding="utf-8")
    (tmp_path / "manifest.json").write_text(
        json.dumps(
            {
                "models": [
                    {"name": "missing", "version": "1", "path": missing.name},
                    {
                        "name": "de_core_news_lg",
                        "version": "3.8.0",
                        "path": incompatible.name,
                    },
                    {"name": "escaped", "version": "1", "path": "../outside"},
                ]
            }
        ),
        encoding="utf-8",
    )

    models = Engine(tmp_path).available_models()

    assert models == []
    with pytest.raises(EngineError, match="model_not_found"):
        Engine(tmp_path).configure(
            str(uuid4()),
            str(uuid4()),
            ProcessingConfig(model="de_core_news_lg"),
        )


def test_packaged_model_is_reported_compatible(model_root: Path) -> None:
    models = Engine(model_root).available_models()
    assert [model.name for model in models] == [NAME]
    assert models[0].compatible and models[0].entity_types


def test_model_manifest_must_not_repeat_names(tmp_path: Path) -> None:
    entry = {"name": "duplicate", "version": "1", "path": "missing"}
    (tmp_path / "manifest.json").write_text(
        json.dumps({"models": [entry, entry]}), encoding="utf-8"
    )

    with pytest.raises(EngineError, match="invalid_model_manifest"):
        Engine(tmp_path).available_models()


def test_detection_ids_are_stable_without_exposing_custom_terms(model_root: Path, caplog) -> None:
    engine = Engine(model_root)
    pair_id, revision, rule_id = str(uuid4()), str(uuid4()), str(uuid4())
    private_term = "CANARY_PRIVATE_CUSTOM_TERM"
    config = ProcessingConfig(
        enabled_entities=[],
        custom_rules=[
            WordRule(
                id=rule_id,
                entity_type="CUSTOM",
                enabled=True,
                kind="words",
                words=[private_term],
            )
        ],
    )

    info = engine.configure(pair_id, revision, config)
    first = engine.analyze(pair_id, revision, private_term)
    second = engine.analyze(pair_id, revision, private_term)

    assert first == second
    assert info.recognizers == [rule_id]
    assert first[0].recognizer == rule_id
    assert private_term not in caplog.text


def test_processing_identity_includes_own_version_and_dependencies(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import redactio_sidecar.engine as module

    versions = {"presidio-analyzer": "2.2.362", "spacy": "3.8.7"}
    monkeypatch.setattr(module, "version", versions.__getitem__)
    monkeypatch.setattr(module, "ENGINE_VERSION", "redactio-sidecar 0.1.0")
    before = module._engine_version()
    monkeypatch.setattr(module, "ENGINE_VERSION", "redactio-sidecar 0.2.0")
    after = module._engine_version()
    assert before != after
    assert after == module._engine_version()
    assert "redactio-sidecar 0.2.0" in after
    assert "presidio-analyzer 2.2.362" in after
    assert "spacy 3.8.7" in after
    import re

    assert re.fullmatch(r"[A-Za-z0-9_.+ -]{1,128}", after)
    versions["spacy"] = "3.8.8"
    assert after != module._engine_version()
