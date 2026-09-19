from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any, cast

from presidio_analyzer import EntityRecognizer, RecognizerResult

from .ipc import EngineError

MODEL_NAME = "OpenMed-PII-German-BiomedBERT-Large-340M-v1"
MODEL_VERSION = "ce797d58600cc20bba9a2500dafc0b7f5c3270c1"
_LABELS = {
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


def compatible_model(path: Path, name: str, model_version: str) -> bool:
    try:
        metadata = json.loads((path / "redactio-model.json").read_text(encoding="utf-8"))
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        return bool(
            name == MODEL_NAME
            and model_version == MODEL_VERSION
            and metadata
            == {
                "name": MODEL_NAME,
                "version": MODEL_VERSION,
                "repository": "OpenMed/" + MODEL_NAME,
            }
            and config["model_type"] == "bert"
            and all(
                (path / file).is_file()
                for file in (
                    "model.safetensors",
                    "tokenizer.json",
                    "tokenizer_config.json",
                )
            )
        )
    except (KeyError, OSError, TypeError, ValueError):
        return False


def _load_pipeline(path: Path) -> Any:
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
    if model.config.model_type != "bert" or model.config.max_position_embeddings != 512:
        raise ValueError("incompatible model architecture")
    return pipeline(
        "token-classification",
        model=model,
        tokenizer=tokenizer,
        device=-1,
        aggregation_strategy="simple",
        stride=128,
    )


class BiomedBertRecognizer(EntityRecognizer):
    name: str

    def __init__(self, path: Path) -> None:
        self._path = path
        super().__init__(
            supported_entities=["PERSON", "LOCATION"],
            supported_language="de",
            name="BiomedBertRecognizer",
        )

    def load(self) -> None:
        try:
            self._pipeline = _load_pipeline(self._path)
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
                entity = _LABELS.get(item["entity_group"])
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
                    separators = " \t\r\n,-" if result.entity_type == "LOCATION" else " \t-"
                    if result.start <= previous.end or (gap and all(c in separators for c in gap)):
                        previous.end = max(previous.end, result.end)
                        previous.score = min(previous.score, result.score)
                        continue
                merged.append(result)
            return merged
        except Exception as error:
            raise EngineError("internal_error") from error
