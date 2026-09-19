from __future__ import annotations

import json
import re
from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Any, cast
from uuid import UUID, uuid5

import spacy
import tldextract
from presidio_analyzer import (
    AnalyzerEngine,
    EntityRecognizer,
    Pattern,
    PatternRecognizer,
    RecognizerRegistry,
)
from presidio_analyzer.nlp_engine import NlpEngine, NlpEngineProvider
from presidio_analyzer.predefined_recognizers import (
    DateRecognizer,
    EmailRecognizer,
    IbanRecognizer,
    IpRecognizer,
    PhoneRecognizer,
    SpacyRecognizer,
    UrlRecognizer,
)
from pydantic import ValidationError

from .extract import Extraction, extract_document
from .frontmatter import normalize_body, render_document
from .ipc import EngineError
from .redaction import apply_redactions
from .schemas import (
    Decisions,
    Detection,
    EngineInfo,
    EntityType,
    ModelInfo,
    OutputEntry,
    ProcessingConfig,
    ProcessRequest,
    ProcessResult,
    RegexRule,
    ReviewRequest,
    ReviewStatus,
    WordRule,
)

LANGUAGE = "de"
ENGINE_VERSION = "redactio-sidecar 0.1.0"
EXTRACTION_VERSION = "1"
_DETECTION_NAMESPACE = UUID("f31cdf75-cf5f-46cf-95d4-3ae885bcb84a")


class _OfflineEmailRecognizer(EmailRecognizer):
    _extract = tldextract.TLDExtract(cache_dir=None, suffix_list_urls=())

    def validate_result(self, pattern_text: str) -> bool:
        return self._extract(pattern_text).fqdn != ""


_AUTOMATIC_RECOGNIZERS: dict[str, tuple[str, Callable[..., EntityRecognizer]]] = {
    "EMAIL_ADDRESS": ("EmailRecognizer", _OfflineEmailRecognizer),
    "PHONE_NUMBER": ("PhoneRecognizer", PhoneRecognizer),
    "IBAN_CODE": ("IbanRecognizer", IbanRecognizer),
    "IP_ADDRESS": ("IpRecognizer", IpRecognizer),
    "URL": ("UrlRecognizer", UrlRecognizer),
    "DATE_TIME": ("DateRecognizer", DateRecognizer),
}
_ENTITY_TYPES = {
    "PERSON",
    "LOCATION",
    "EMAIL_ADDRESS",
    "PHONE_NUMBER",
    "IBAN_CODE",
    "IP_ADDRESS",
    "URL",
    "DATE_TIME",
    "CUSTOM",
}


@dataclass(frozen=True)
class _Model:
    name: str
    version: str
    path: Path
    compatible: bool


@dataclass(frozen=True)
class _Snapshot:
    pair_id: str
    revision: str
    analyzer: AnalyzerEngine | None
    entities: tuple[str, ...]
    custom_rule_ids: frozenset[str]
    recognizers: tuple[str, ...]
    include_positions: bool
    info: EngineInfo


class Engine:
    def __init__(self, model_root: Path) -> None:
        self._model_root = model_root.resolve()
        self._loaded_models: dict[tuple[str, str], NlpEngine] = {}
        self._snapshot: _Snapshot | None = None

    def available_models(self) -> list[ModelInfo]:
        return [
            ModelInfo(name=model.name, version=model.version, compatible=model.compatible)
            for model in self._models()
        ]

    def configure(
        self,
        pair_id: str,
        revision: str,
        config: ProcessingConfig,
    ) -> EngineInfo:
        try:
            validated = ProcessingConfig.model_validate(config.model_dump())
            _canonical_uuid(pair_id)
            _canonical_uuid(revision)
        except (AttributeError, TypeError, ValueError, ValidationError) as error:
            raise EngineError("invalid_configuration") from error

        model = next((item for item in self._models() if item.name == validated.model), None)
        if model is None:
            raise EngineError("model_not_found")
        if not model.compatible:
            raise EngineError("model_incompatible")

        nlp_engine = self._nlp_engine(model)
        registry = RecognizerRegistry(supported_languages=[LANGUAGE])
        recognizer_names: list[str] = []

        nlp_entities: list[str] = [
            entity for entity in ("PERSON", "LOCATION") if entity in validated.enabled_entities
        ]
        if nlp_entities:
            registry.add_recognizer(
                SpacyRecognizer(
                    supported_language=LANGUAGE,
                    supported_entities=nlp_entities,
                    name="SpacyRecognizer",
                )
            )
            recognizer_names.append("SpacyRecognizer")

        for entity_type, (recognizer_name, recognizer_type) in _AUTOMATIC_RECOGNIZERS.items():
            if entity_type not in validated.enabled_entities:
                continue
            registry.add_recognizer(
                recognizer_type(supported_language=LANGUAGE, name=recognizer_name)
            )
            recognizer_names.append(recognizer_name)

        custom_ids: set[str] = set()
        entities: list[str] = [
            entity for entity in validated.enabled_entities if entity != "CUSTOM"
        ]
        for rule in validated.custom_rules:
            if not rule.enabled:
                continue
            pattern = _rule_pattern(rule)
            registry.add_recognizer(
                PatternRecognizer(
                    supported_entity=rule.entity_type,
                    name=rule.id,
                    supported_language=LANGUAGE,
                    patterns=[Pattern(name=rule.id, regex=pattern, score=1.0)],
                    global_regex_flags=0,
                )
            )
            custom_ids.add(rule.id)
            recognizer_names.append(rule.id)
            if rule.entity_type not in entities:
                entities.append(rule.entity_type)

        analyzer = (
            AnalyzerEngine(
                registry=registry,
                nlp_engine=nlp_engine,
                supported_languages=[LANGUAGE],
            )
            if registry.recognizers
            else None
        )
        info = EngineInfo(
            engine_version=_engine_version(),
            model_name=model.name,
            model_version=model.version,
            recognizers=recognizer_names,
            extraction_version=EXTRACTION_VERSION,
        )
        self._snapshot = _Snapshot(
            pair_id=pair_id,
            revision=revision,
            analyzer=analyzer,
            entities=tuple(entities),
            custom_rule_ids=frozenset(custom_ids),
            recognizers=tuple(recognizer_names),
            include_positions=validated.include_positions,
            info=info,
        )
        return info.model_copy(deep=True)

    def analyze(self, pair_id: str, revision: str, text: str) -> list[Detection]:
        snapshot = self._active_snapshot(pair_id, revision)
        if not snapshot.entities:
            return []
        if snapshot.analyzer is None:
            raise EngineError("internal_error")
        results = snapshot.analyzer.analyze(
            text=text,
            language=LANGUAGE,
            entities=list(snapshot.entities),
        )
        ordered = sorted(
            results,
            key=lambda result: (
                result.start,
                result.end,
                result.entity_type,
                -result.score,
                _recognizer_name(result.recognition_metadata),
            ),
        )
        detections: list[Detection] = []
        for index, result in enumerate(ordered):
            recognizer = _recognizer_name(result.recognition_metadata)
            if (
                recognizer not in snapshot.custom_rule_ids
                and recognizer not in snapshot.recognizers
            ):
                raise EngineError("internal_error")
            if result.entity_type not in _ENTITY_TYPES:
                raise EngineError("internal_error")
            detection_id = _detection_id(
                pair_id,
                revision,
                index,
                result.start,
                result.end,
                result.entity_type,
                recognizer,
            )
            detections.append(
                Detection(
                    id=detection_id,
                    start=result.start,
                    end=result.end,
                    entity_type=cast(EntityType, result.entity_type),
                    confidence=float(result.score),
                    recognizer=recognizer,
                    origin="automatic",
                )
            )
        return detections

    def preview_rules(self, pair_id: str, revision: str, text: str) -> list[Detection]:
        return self.analyze(pair_id, revision, text)

    def info(self, pair_id: str, revision: str) -> EngineInfo:
        return self._active_snapshot(pair_id, revision).info.model_copy(deep=True)

    def process_document(self, request: ProcessRequest) -> ProcessResult:
        snapshot = self._active_snapshot(request.sync_pair_id, request.processing_revision)
        extraction = self._extract_bound(request)
        detections = self.analyze(
            request.sync_pair_id, request.processing_revision, extraction.text
        )
        try:
            body, entries = apply_redactions(extraction.text, detections, Decisions())
        except ValueError as error:
            raise EngineError("internal_error") from error
        status: ReviewStatus = "needs-rework" if extraction.warnings else "pending"
        return self._result(request, extraction, detections, body, entries, status, None, snapshot)

    def render_review(self, request: ReviewRequest) -> ProcessResult:
        snapshot = self._active_snapshot(request.sync_pair_id, request.processing_revision)
        extraction = self._extract_bound(request)
        self._validate_review(request, snapshot)
        if request.review_status == "approved" and (
            request.reviewed_at is None
            or not extraction.text.strip()
            or not set(extraction.warnings) <= set(request.acknowledged_warnings)
        ):
            raise EngineError("approval_not_allowed")
        try:
            body, entries = apply_redactions(extraction.text, request.detections, request.decisions)
        except ValueError as error:
            raise EngineError("invalid_review") from error
        status = request.review_status
        if extraction.warnings and status == "pending":
            status = "needs-rework"
        return self._result(
            request,
            extraction,
            request.detections,
            body,
            entries,
            status,
            request.reviewed_at,
            snapshot,
        )

    def _extract_bound(self, request: ProcessRequest) -> Extraction:
        extraction = extract_document(Path(request.source_path))
        if extraction.source_hash_sha256 != request.source_hash_sha256:
            raise EngineError("source_changed")
        return extraction

    def _validate_review(self, request: ReviewRequest, snapshot: _Snapshot) -> None:
        identifiers = [
            *(detection.id for detection in request.detections),
            *(detection.id for detection in request.decisions.manual),
        ]
        if (
            len(identifiers) != len(set(identifiers))
            or len(request.decisions.dismissed_ids) != len(set(request.decisions.dismissed_ids))
            or any(
                detection.origin != "automatic"
                or detection.entity_type not in snapshot.entities
                or detection.recognizer not in snapshot.recognizers
                or detection.id
                != _detection_id(
                    request.sync_pair_id,
                    request.processing_revision,
                    index,
                    detection.start,
                    detection.end,
                    detection.entity_type,
                    detection.recognizer,
                )
                for index, detection in enumerate(request.detections)
            )
        ):
            raise EngineError("invalid_review")

    def _result(
        self,
        request: ProcessRequest,
        extraction: Extraction,
        detections: list[Detection],
        body: str,
        entries: list[OutputEntry],
        status: ReviewStatus,
        reviewed_at: datetime | None,
        snapshot: _Snapshot,
    ) -> ProcessResult:
        body = normalize_body(body)
        try:
            markdown = render_document(
                request,
                snapshot.info,
                body,
                entries,
                extraction.warnings,
                status,
                reviewed_at,
                snapshot.include_positions,
            )
        except ValueError as error:
            raise EngineError("invalid_review") from error
        return ProcessResult(
            sync_pair_id=request.sync_pair_id,
            doc_id=request.doc_id,
            source_hash_sha256=request.source_hash_sha256,
            processing_revision=request.processing_revision,
            redacted_at=request.redacted_at,
            markdown=markdown,
            body=body,
            original_text=extraction.text,
            detections=detections,
            redactions=entries,
            warnings=extraction.warnings,
            body_was_empty=not extraction.text.strip(),
            review_status=status,
            engine=snapshot.info,
        )

    def _active_snapshot(self, pair_id: str, revision: str) -> _Snapshot:
        snapshot = self._snapshot
        if snapshot is None or snapshot.pair_id != pair_id or snapshot.revision != revision:
            raise EngineError("configuration_mismatch")
        return snapshot

    def _nlp_engine(self, model: _Model) -> NlpEngine:
        identity = (model.name, model.version)
        cached = self._loaded_models.get(identity)
        if cached is not None:
            return cached
        provider = NlpEngineProvider(
            nlp_configuration={
                "nlp_engine_name": "spacy",
                "models": [{"lang_code": LANGUAGE, "model_name": str(model.path)}],
                "ner_model_configuration": {
                    "model_to_presidio_entity_mapping": {
                        "PER": "PERSON",
                        "LOC": "LOCATION",
                    },
                    "labels_to_ignore": ["MISC", "ORG"],
                },
            }
        )
        try:
            loaded = provider.create_engine()
        except Exception as error:
            raise EngineError("model_incompatible") from error
        self._loaded_models[identity] = loaded
        return loaded

    def _models(self) -> list[_Model]:
        manifest_path = self._model_root / "manifest.json"
        try:
            raw = json.loads(manifest_path.read_text(encoding="utf-8"))
            entries = raw["models"]
            if not isinstance(entries, list):
                raise TypeError
            models = [self._model(entry) for entry in entries]
        except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError) as error:
            raise EngineError("invalid_model_manifest") from error
        if len({model.name for model in models}) != len(models):
            raise EngineError("invalid_model_manifest")
        return models

    def _model(self, entry: object) -> _Model:
        if not isinstance(entry, dict) or set(entry) != {"name", "version", "path"}:
            raise TypeError
        name, model_version, relative = entry["name"], entry["version"], entry["path"]
        if not all(isinstance(value, str) and value for value in (name, model_version, relative)):
            raise TypeError
        path = (self._model_root / relative).resolve()
        compatible = path.is_relative_to(self._model_root) and _compatible_model(
            path, name, model_version
        )
        return _Model(name=name, version=model_version, path=path, compatible=compatible)


def _compatible_model(path: Path, name: str, model_version: str) -> bool:
    try:
        metadata: dict[str, Any] = json.loads((path / "meta.json").read_text(encoding="utf-8"))
        actual_name = f"{metadata['lang']}_{metadata['name']}"
        return bool(
            path.is_dir()
            and (path / "config.cfg").is_file()
            and actual_name == name
            and metadata["version"] == model_version
            and metadata["lang"] == LANGUAGE
            and spacy.util.is_compatible_version(version("spacy"), metadata["spacy_version"])
        )
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError):
        return False


def _rule_pattern(rule: RegexRule | WordRule) -> str:
    if isinstance(rule, RegexRule):
        try:
            re.compile(rule.pattern)
        except (TypeError, re.error) as error:
            raise EngineError("invalid_configuration") from error
        if not rule.pattern:
            raise EngineError("invalid_configuration")
        return rule.pattern

    terms = sorted(dict.fromkeys(rule.words), key=len, reverse=True)
    if not terms or any(not term for term in terms):
        raise EngineError("invalid_configuration")
    alternatives = []
    for term in terms:
        left = r"(?<!\w)" if term[0].isalnum() or term[0] == "_" else ""
        right = r"(?!\w)" if term[-1].isalnum() or term[-1] == "_" else ""
        alternatives.append(f"{left}{re.escape(term)}{right}")
    pattern = "(?:" + "|".join(alternatives) + ")"
    re.compile(pattern)
    return pattern


def _recognizer_name(metadata: object) -> str:
    if isinstance(metadata, dict):
        name = metadata.get("recognizer_name")
        if isinstance(name, str) and name:
            return name
    return "automatic"


def _engine_version() -> str:
    try:
        return f"presidio-analyzer {version('presidio-analyzer')}"
    except PackageNotFoundError:
        return ENGINE_VERSION


def _canonical_uuid(value: str) -> None:
    if str(UUID(value)) != value:
        raise ValueError


def _detection_id(
    pair_id: str,
    revision: str,
    index: int,
    start: int,
    end: int,
    entity_type: str,
    recognizer: str,
) -> str:
    return str(
        uuid5(
            _DETECTION_NAMESPACE,
            ":".join(
                (
                    pair_id,
                    revision,
                    str(index),
                    str(start),
                    str(end),
                    entity_type,
                    recognizer,
                )
            ),
        )
    )
