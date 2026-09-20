from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from presidio_analyzer import EntityRecognizer, RecognizerResult

from .ipc import EngineError

MODEL_NAME = "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
MODEL_VERSION = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1"
HUGGINGLIL_MODEL_NAME = "pii-sensitive-ner-german"
HUGGINGLIL_MODEL_VERSION = "6af88facbb75da7be737da55d2c411c7ce79e5a1"
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
_LABEL_ID = re.compile(r"^[A-Z][A-Z0-9_]{0,63}$")


@dataclass(frozen=True)
class ModelSpec:
    name: str
    version: str
    repository: str
    model_type: str
    architecture: str
    recognizer_name: str


BIOMEDBERT = ModelSpec(
    MODEL_NAME,
    MODEL_VERSION,
    "OpenMed/" + MODEL_NAME,
    "bert",
    "BertForTokenClassification",
    "BiomedBertRecognizer",
)
HUGGINGLIL = ModelSpec(
    HUGGINGLIL_MODEL_NAME,
    HUGGINGLIL_MODEL_VERSION,
    "HuggingLil/pii-sensitive-ner-german",
    "deberta-v2",
    "DebertaV2ForTokenClassification",
    "HuggingLilRecognizer",
)
MODEL_SPECS = {spec.name: spec for spec in (BIOMEDBERT, HUGGINGLIL)}


def model_entity_types(path: Path) -> tuple[str, ...]:
    try:
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        labels = config["id2label"]
        if not isinstance(labels, dict):
            raise TypeError
        if not all(isinstance(index, str) and index.isdecimal() for index in labels):
            raise ValueError
        if not all(isinstance(label, str) for label in labels.values()):
            raise ValueError
        entity_types = {_strip_bio_prefix(label) for label in labels.values()} - {"O"}
        if any(not _LABEL_ID.fullmatch(label) for label in entity_types) or any(
            not isinstance(label, str) for label in labels.values()
        ):
            raise ValueError
        return tuple(sorted(entity_types))
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError):
        return ()


def _strip_bio_prefix(label: str) -> str:
    return label[2:] if label.startswith(("B-", "I-")) else label


def compatible_model(path: Path, name: str, model_version: str) -> bool:
    try:
        spec = MODEL_SPECS[name]
        metadata = json.loads((path / "redactio-model.json").read_text(encoding="utf-8"))
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        labels = model_entity_types(path)
        return bool(
            model_version == spec.version
            and metadata
            == {
                "name": spec.name,
                "version": spec.version,
                "repository": spec.repository,
            }
            and config["model_type"] == spec.model_type
            and (
                spec is BIOMEDBERT
                or (
                    config.get("architectures") == [spec.architecture]
                    and config["max_position_embeddings"] == 512
                )
            )
            and bool(labels)
            and all(
                (path / file).is_file()
                for file in (
                    "model.safetensors",
                    "tokenizer.json",
                    "tokenizer_config.json",
                )
            )
        )
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError):
        return False


def _load_pipeline(
    path: Path,
    model_type: str = "bert",
    architecture: str = "BertForTokenClassification",
) -> Any:
    from transformers import AutoModelForTokenClassification, AutoTokenizer, pipeline

    tokenizer = cast(Any, AutoTokenizer).from_pretrained(
        str(path),
        local_files_only=True,
        trust_remote_code=False,
        use_fast=True,
        model_max_length=512,
    )
    if not tokenizer.is_fast:
        raise ValueError("fast tokenizer required")
    model = AutoModelForTokenClassification.from_pretrained(
        str(path),
        local_files_only=True,
        trust_remote_code=False,
        use_safetensors=True,
    )
    if (
        model.config.model_type != model_type
        or model.config.max_position_embeddings != 512
        or model.config.architectures != [architecture]
    ):
        raise ValueError("incompatible model architecture")
    return pipeline(
        "token-classification",
        model=model,
        tokenizer=tokenizer,
        device=-1,
        aggregation_strategy="simple",
        stride=128,
    )


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
