from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from presidio_analyzer import EntityRecognizer, RecognizerResult

from .ipc import EngineError
from .model_store import (
    ModelDescriptor,
    Window,
    _load_local_model,
    _token_classification_pipeline,
    catalog_models,
    compatible_descriptor,
)
from .model_store import (
    model_entity_types as model_entity_types,
)
from .model_store import validate_local_model as _validate_local_model

_CATALOG = {entry.key: entry.descriptor for entry in catalog_models()}
MODEL_NAME = _CATALOG["biomedbert"].name
MODEL_VERSION = _CATALOG["biomedbert"].version
HUGGINGLIL_MODEL_NAME = _CATALOG["hugginglil"].name
HUGGINGLIL_MODEL_VERSION = _CATALOG["hugginglil"].version
_LEGACY_LABELS = {
    **dict.fromkeys(("FIRSTNAME", "MIDDLENAME", "LASTNAME"), "PERSON"),
    **dict.fromkeys(
        (
            "STREET",
            "BUILDINGNUMBER",
            "SECONDARYADDRESS",
            "ZIPCODE",
            "CITY",
            "STATE",
            "COUNTY",
            "GPSCOORDINATES",
            "ORDINALDIRECTION",
        ),
        "LOCATION",
    ),
}


@dataclass(frozen=True)
class ModelSpec:
    name: str
    version: str
    repository: str
    model_type: str
    architecture: str
    recognizer_name: str


def model_spec(descriptor: ModelDescriptor) -> ModelSpec:
    recognizer = {MODEL_NAME: "BiomedBertRecognizer", HUGGINGLIL_MODEL_NAME: "HuggingLilRecognizer"}
    return ModelSpec(
        descriptor.name,
        descriptor.version,
        descriptor.repository,
        descriptor.model_type,
        descriptor.architecture,
        recognizer.get(descriptor.name, "TransformersNerRecognizer"),
    )


BIOMEDBERT = model_spec(_CATALOG["biomedbert"])
HUGGINGLIL = model_spec(_CATALOG["hugginglil"])


def _strip_bio_prefix(label: str) -> str:
    return label[2:] if label.startswith(("B-", "I-")) else label


def compatible_model(path: Path, name: str, model_version: str) -> bool:
    descriptor = next(
        (
            entry
            for entry in _CATALOG.values()
            if (entry.name, entry.version) == (name, model_version)
        ),
        None,
    )
    return descriptor is not None and compatible_descriptor(path, descriptor, legacy=True)


def _load_pipeline(
    path: Path,
    model_type: str = "bert",
    architecture: str = "BertForTokenClassification",
) -> Any:
    tokenizer, model, window = _load_local_model(path, model_type, architecture)
    return _token_classification_pipeline(model, tokenizer, window)


def validate_local_model(path: Path, model_type: str, architecture: str) -> Window:
    return _validate_local_model(path, model_type, architecture)


class _TokenClassificationRecognizer(EntityRecognizer):
    name: str

    def __init__(self, path: Path, spec: ModelSpec, native_labels: tuple[str, ...] = ()) -> None:
        self._path = path
        self._spec = spec
        self._native_labels = frozenset(native_labels or _LEGACY_LABELS)
        self._label_mapping = {label: label for label in self._native_labels}
        self._legacy = False
        super().__init__(
            supported_entities=sorted(self._native_labels),
            supported_language="de",
            name=spec.recognizer_name,
        )

    def configure_labels(self, labels: list[str], legacy: bool) -> None:
        selected = set(labels)
        if not selected <= self._native_labels:
            raise ValueError("unsupported model label")
        self._label_mapping = {
            label: _LEGACY_LABELS[label] if legacy else label
            for label in selected
            if not legacy or label in _LEGACY_LABELS
        }
        self._legacy = legacy
        self.supported_entities = sorted(set(self._label_mapping.values()))

    def load(self) -> None:
        try:
            if self._spec is BIOMEDBERT:
                self._pipeline = _load_pipeline(self._path)
            else:
                self._pipeline = _load_pipeline(
                    self._path, self._spec.model_type, self._spec.architecture
                )
        except Exception as error:
            raise EngineError("model_incompatible") from error

    def analyze(
        self,
        text: str,
        entities: list[str],
        nlp_artifacts: Any = None,
    ) -> list[RecognizerResult]:
        if not text.strip():
            return []
        try:
            results = []
            for item in self._pipeline(text):
                raw_label = item.get("entity_group")
                entity = self._label_mapping.get(
                    _strip_bio_prefix(raw_label) if isinstance(raw_label, str) else ""
                )
                if entity not in entities or entity not in self.supported_entities:
                    continue
                start, end, score = int(item["start"]), int(item["end"]), float(item["score"])
                if not (0 <= start < end <= len(text) and math.isfinite(score) and 0 <= score <= 1):
                    raise ValueError("invalid inference result")
                results.append(RecognizerResult(entity, start, end, score))
            # The model labels name/address components separately. Join only predicted
            # components separated by address punctuation/whitespace, preserving offsets.
            merged: list[RecognizerResult] = []
            for result in sorted(results, key=lambda item: (item.start, item.end)):
                if merged and result.entity_type == merged[-1].entity_type:
                    previous = merged[-1]
                    gap = text[previous.end : result.start]
                    separators = (
                        " \t\r\n,-"
                        if not self._legacy or result.entity_type == "LOCATION"
                        else " \t-"
                    )
                    if result.start <= previous.end or (gap and all(c in separators for c in gap)):
                        previous.end = max(previous.end, result.end)
                        previous.score = min(previous.score, result.score)
                        continue
                merged.append(result)
            return merged
        except Exception as error:
            raise EngineError("internal_error") from error


class BiomedBertRecognizer(_TokenClassificationRecognizer):
    def __init__(self, path: Path, native_labels: tuple[str, ...] = ()) -> None:
        super().__init__(path, BIOMEDBERT, native_labels)


class HuggingLilRecognizer(_TokenClassificationRecognizer):
    def __init__(self, path: Path, native_labels: tuple[str, ...] = ()) -> None:
        super().__init__(path, HUGGINGLIL, native_labels)


class TransformersNerRecognizer(_TokenClassificationRecognizer):
    def __init__(self, path: Path, descriptor: ModelDescriptor) -> None:
        super().__init__(path, model_spec(descriptor), tuple(descriptor.entity_types))
