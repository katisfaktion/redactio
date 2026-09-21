from __future__ import annotations

from pathlib import Path
from typing import Any, NamedTuple, cast

from .ipc import EngineError

_VALIDATION_TEXT = ("Hans München 😀\r\n" * 640) + "Ende"


class Window(NamedTuple):
    tokens: int
    stride: int


def processing_window(model_limit: object, tokenizer_limit: object, special_tokens: int) -> Window:
    if type(model_limit) is not int or not 0 < model_limit < 10**20:
        raise ValueError("invalid model context limit")
    if type(special_tokens) is not int or special_tokens < 0:
        raise ValueError("invalid special-token count")
    limit = model_limit
    if tokenizer_limit is not None:
        if type(tokenizer_limit) is not int or tokenizer_limit <= 0:
            raise ValueError("invalid tokenizer context limit")
        if tokenizer_limit < 10**20:
            limit = min(limit, tokenizer_limit)
    content = limit - special_tokens
    if content < 2:
        raise ValueError("unusable content window")
    return Window(limit, min(max(1, limit // 4), content - 1))


def _load_local_model(path: Path, model_type: str, architecture: str) -> tuple[Any, Any, Window]:
    from transformers import AutoModelForTokenClassification, AutoTokenizer

    tokenizer = cast(Any, AutoTokenizer).from_pretrained(
        str(path),
        local_files_only=True,
        trust_remote_code=False,
        use_fast=True,
    )
    if not tokenizer.is_fast:
        raise ValueError("fast tokenizer required")
    model, loading_info = AutoModelForTokenClassification.from_pretrained(
        str(path),
        local_files_only=True,
        trust_remote_code=False,
        use_safetensors=True,
        output_loading_info=True,
    )
    if (
        model.config.model_type != model_type
        or model.config.architectures != [architecture]
        or any(key.startswith("classifier.") for key in loading_info["missing_keys"])
        or any(key[0].startswith("classifier.") for key in loading_info["mismatched_keys"])
    ):
        raise ValueError("incompatible model architecture")
    window = processing_window(
        model.config.max_position_embeddings,
        tokenizer.model_max_length,
        tokenizer.num_special_tokens_to_add(pair=False),
    )
    tokenizer.model_max_length = window.tokens
    return tokenizer, model, window


def _token_classification_pipeline(model: Any, tokenizer: Any, window: Window) -> Any:
    from transformers import pipeline

    return pipeline(
        "token-classification",
        model=model,
        tokenizer=tokenizer,
        device=-1,
        aggregation_strategy="simple",
        stride=window.stride,
    )


def validate_local_model(path: Path, model_type: str, architecture: str) -> Window:
    try:
        tokenizer, model, window = _load_local_model(path, model_type, architecture)
        encoded = tokenizer(
            _VALIDATION_TEXT,
            truncation=True,
            max_length=window.tokens,
            stride=window.stride,
            return_overflowing_tokens=True,
            return_offsets_mapping=True,
        )
        offsets = encoded["offset_mapping"]
        spans = {
            (start, end)
            for chunk in offsets
            for start, end in chunk
            if type(start) is int and type(end) is int and 0 <= start < end <= len(_VALIDATION_TEXT)
        }
        if len(offsets) < 2 or not spans or max(end for _, end in spans) != len(_VALIDATION_TEXT):
            raise ValueError("incomplete synthetic offsets")
        starts = {start for start, _ in spans}
        ends = {end for _, end in spans}
        seen: set[tuple[int, int]] = set()
        for detection in _token_classification_pipeline(model, tokenizer, window)(_VALIDATION_TEXT):
            start, end = detection.get("start"), detection.get("end")
            if (
                type(start) is not int
                or type(end) is not int
                or not 0 <= start < end <= len(_VALIDATION_TEXT)
                or start not in starts
                or end not in ends
                or (start, end) in seen
            ):
                raise ValueError("invalid synthetic inference offsets")
            seen.add((start, end))
        return window
    except Exception as error:
        raise EngineError("model_incompatible") from error
