from __future__ import annotations

import re
from collections.abc import Callable
from copy import copy
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
from presidio_analyzer.nlp_engine import NlpEngine, SpacyNlpEngine
from presidio_analyzer.predefined_recognizers import (
    DateRecognizer,
    EmailRecognizer,
    IbanRecognizer,
    IpRecognizer,
    PhoneRecognizer,
    UrlRecognizer,
)
from pydantic import ValidationError

from .biomedbert import (
    HUGGINGLIL_MODEL_NAME,
    MODEL_NAME,
    BiomedBertRecognizer,
    HuggingLilRecognizer,
    TransformersNerRecognizer,
)
from .extract import Extraction, extract_document
from .frontmatter import normalize_body, render_document
from .ipc import EngineError
from .model_store import (
    ModelDescriptor,
    _safe_path,
    catalog_models,
    compatible_descriptor,
    read_registry,
)
from .redaction import apply_redactions
from .schemas import (
    Decisions,
    Detection,
    EngineInfo,
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
_LEGACY_MODEL_ENTITIES = {
    "PERSON",
    "LOCATION",
}
_SUPPLEMENTARY_ENTITIES = {
    "EMAIL_ADDRESS",
    "PHONE_NUMBER",
    "IBAN_CODE",
    "IP_ADDRESS",
    "URL",
    "DATE_TIME",
}
_LEGACY_LABELS = {
    "PERSON": {"FIRSTNAME", "MIDDLENAME", "LASTNAME"},
    "LOCATION": {
        "STREET",
        "BUILDINGNUMBER",
        "SECONDARYADDRESS",
        "ZIPCODE",
        "CITY",
        "STATE",
        "COUNTY",
        "GPSCOORDINATES",
        "ORDINALDIRECTION",
    },
}


@dataclass(frozen=True)
class _Model:
    name: str
    version: str
    path: Path
    compatible: bool
    entity_types: tuple[str, ...]
    descriptor: ModelDescriptor


@dataclass(frozen=True)
class _Snapshot:
    pair_id: str
    revision: str
    analyzer: AnalyzerEngine | None
    entities: tuple[str, ...]
    custom_rule_ids: frozenset[str]
    recognizers: tuple[str, ...]
    manual_entities: frozenset[str]
    include_positions: bool
    info: EngineInfo


class Engine:
    def __init__(self, model_root: Path) -> None:
        self._model_root = model_root.absolute()
        self._loaded_models: dict[tuple[str, str], NlpEngine] = {}
        self._recognizers: dict[
            tuple[str, str], BiomedBertRecognizer | HuggingLilRecognizer | TransformersNerRecognizer
        ] = {}
        self._snapshot: _Snapshot | None = None

    def available_models(self) -> list[ModelInfo]:
        return [
            ModelInfo(
                name=model.name,
                version=model.version,
                compatible=model.compatible,
                entity_types=list(model.entity_types),
            )
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
        if model.name != MODEL_NAME and validated.model_entities is None:
            raise EngineError("invalid_configuration")
        if not set(validated.enabled_entities) <= (
            _SUPPLEMENTARY_ENTITIES
            | (_LEGACY_MODEL_ENTITIES | {"CUSTOM"} if validated.model_entities is None else set())
        ):
            raise EngineError("invalid_configuration")
        native_semantics = validated.model_entities is not None
        native_entities = list(validated.model_entities or ())
        if native_semantics:
            if not set(native_entities) <= set(model.entity_types):
                raise EngineError("invalid_configuration")
        else:
            native_entities = sorted(
                label
                for legacy_entity in validated.enabled_entities
                for label in _LEGACY_LABELS.get(legacy_entity, set())
                if label in model.entity_types
            )

        nlp_engine = self._nlp_engine(model)
        registry = RecognizerRegistry(supported_languages=[LANGUAGE])
        recognizer_names: list[str] = []

        model_outputs = (
            native_entities
            if native_semantics
            else [
                entity for entity in _LEGACY_MODEL_ENTITIES if entity in validated.enabled_entities
            ]
        )
        if model_outputs:
            recognizer = copy(self._recognizers[(model.name, model.version)])
            recognizer.configure_labels(native_entities, legacy=not native_semantics)
            registry.add_recognizer(recognizer)
            recognizer_names.append(recognizer.name)

        for entity_type, (recognizer_name, recognizer_type) in _AUTOMATIC_RECOGNIZERS.items():
            if entity_type not in validated.enabled_entities:
                continue
            registry.add_recognizer(
                recognizer_type(supported_language=LANGUAGE, name=recognizer_name)
            )
            recognizer_names.append(recognizer_name)

        custom_ids: set[str] = set()
        entities = [
            *model_outputs,
            *(entity for entity in validated.enabled_entities if entity in _SUPPLEMENTARY_ENTITIES),
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
            engine_version=_engine_version(
                biomedbert=True,
                native_labels=native_entities if native_semantics else None,
            ),
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
            manual_entities=frozenset(
                {*model.entity_types, *_LEGACY_MODEL_ENTITIES, *_SUPPLEMENTARY_ENTITIES, "CUSTOM"}
                | {rule.entity_type for rule in validated.custom_rules}
            ),
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
            if result.entity_type not in snapshot.entities:
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
                    entity_type=result.entity_type,
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
            or any(
                detection.origin != "manual"
                or detection.recognizer != "manual"
                or detection.entity_type not in snapshot.manual_entities
                for detection in request.decisions.manual
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
        recognizer = (
            BiomedBertRecognizer(model.path, model.entity_types)
            if model.name == MODEL_NAME
            else HuggingLilRecognizer(model.path, model.entity_types)
            if model.name == HUGGINGLIL_MODEL_NAME
            else TransformersNerRecognizer(model.path, model.descriptor)
        )
        blank_engine = SpacyNlpEngine()
        cast(Any, blank_engine).nlp = {LANGUAGE: spacy.blank(LANGUAGE)}
        self._recognizers[identity] = recognizer
        self._loaded_models[identity] = blank_engine
        return blank_engine

    def _models(self) -> list[_Model]:
        registry = read_registry(self._model_root)
        catalogs = {entry.descriptor.name: entry.descriptor for entry in catalog_models()}
        models = []
        for record in registry.models:
            if record.state != "ready" or record.path is None:
                continue
            descriptor = record.descriptor
            path = self._model_root / record.path
            try:
                path = _safe_path(self._model_root, record.path)
                compatible = compatible_descriptor(
                    path, descriptor, legacy=registry._migrated_legacy
                )
            except (OSError, ValueError):
                compatible = False
            models.append(
                _Model(
                    descriptor.name,
                    descriptor.version,
                    path,
                    compatible,
                    tuple(descriptor.entity_types) if compatible else (),
                    descriptor,
                )
            )
        for entry in registry.legacy_unavailable:
            legacy_descriptor = catalogs.get(entry.name)
            if legacy_descriptor is not None:
                models.append(
                    _Model(
                        entry.name,
                        entry.version,
                        self._model_root / entry.path,
                        False,
                        (),
                        legacy_descriptor,
                    )
                )
        return models


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


def _engine_version(biomedbert: bool = False, native_labels: list[str] | None = None) -> str:
    identities = [ENGINE_VERSION]
    dependencies: tuple[str, ...] = ("presidio-analyzer", "spacy")
    if biomedbert:
        dependencies += ("transformers", "torch")
    for dependency in dependencies:
        try:
            dependency_version = re.sub(r"[^A-Za-z0-9_.+-]", "_", version(dependency))
            identities.append(f"{dependency} {dependency_version}")
        except PackageNotFoundError:
            identities.append(f"{dependency} unknown")
    if native_labels is not None:
        identities.append("native-labels " + ",".join(sorted(native_labels)))
    return " + ".join(identities)


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
