from __future__ import annotations

from pathlib import Path
from typing import Any, NamedTuple, cast

from .ipc import EngineError

_VALIDATION_UNIT = "Hans München 😀\r\n"


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


def _validation_text(tokenizer: Any, window: Window) -> str:
    input_ids = tokenizer(_VALIDATION_UNIT, add_special_tokens=False)["input_ids"]
    if not isinstance(input_ids, list) or not input_ids:
        raise ValueError("unusable synthetic tokenizer")
    return (_VALIDATION_UNIT * (window.tokens // len(input_ids) + 1)) + "Ende"


def validate_local_model(path: Path, model_type: str, architecture: str) -> Window:
    try:
        tokenizer, model, window = _load_local_model(path, model_type, architecture)
        text = _validation_text(tokenizer, window)
        encoded = tokenizer(
            text,
            truncation=True,
            max_length=window.tokens,
            stride=window.stride,
            return_overflowing_tokens=True,
            return_offsets_mapping=True,
            return_special_tokens_mask=True,
        )
        offsets = encoded["offset_mapping"]
        masks = encoded["special_tokens_mask"]
        if not isinstance(offsets, list) or not isinstance(masks, list) or len(offsets) < 2:
            raise ValueError("incomplete synthetic offsets")
        spans: set[tuple[int, int]] = set()
        previous_first = previous_last = -1
        for chunk, mask in zip(offsets, masks, strict=True):
            if not isinstance(chunk, list) or not isinstance(mask, list) or len(chunk) != len(mask):
                raise ValueError("malformed synthetic offsets")
            chunk_spans: list[tuple[int, int]] = []
            for offset, special in zip(chunk, mask, strict=True):
                if (
                    not isinstance(offset, (list, tuple))
                    or len(offset) != 2
                    or type(offset[0]) is not int
                    or type(offset[1]) is not int
                    or type(special) is not int
                    or special not in (0, 1)
                ):
                    raise ValueError("malformed synthetic offsets")
                start, end = offset
                if start == end == 0:
                    if special != 1:
                        raise ValueError("malformed synthetic offsets")
                    continue
                if special != 0 or not 0 <= start < end <= len(text):
                    raise ValueError("malformed synthetic offsets")
                chunk_spans.append((start, end))
                spans.add((start, end))
            if not chunk_spans:
                raise ValueError("incomplete synthetic offsets")
            first, last = chunk_spans[0][0], chunk_spans[-1][1]
            if first <= previous_first or last <= previous_last:
                raise ValueError("reset synthetic offsets")
            previous_first, previous_last = first, last
        if max(end for _, end in spans) != len(text):
            raise ValueError("incomplete synthetic offsets")
        covered = 0
        for start, end in sorted(spans):
            if start > covered and text[covered:start].strip():
                raise ValueError("dropped synthetic offsets")
            covered = max(covered, end)
        if text[covered:].strip():
            raise ValueError("dropped synthetic offsets")
        starts = {start for start, _ in spans}
        ends = {end for _, end in spans}
        seen: set[tuple[int, int]] = set()
        for detection in _token_classification_pipeline(model, tokenizer, window)(text):
            start, end = detection.get("start"), detection.get("end")
            if (
                type(start) is not int
                or type(end) is not int
                or not 0 <= start < end <= len(text)
                or start not in starts
                or end not in ends
                or (start, end) in seen
            ):
                raise ValueError("invalid synthetic inference offsets")
            seen.add((start, end))
        return window
    except Exception as error:
        raise EngineError("model_incompatible") from error
