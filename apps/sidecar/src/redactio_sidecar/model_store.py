from __future__ import annotations

from pathlib import Path
from typing import Any, NamedTuple, cast

from .ipc import EngineError


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


def validate_local_model(path: Path, model_type: str, architecture: str) -> Window:
    try:
        tokenizer, model, window = _load_local_model(path, model_type, architecture)
        model(**tokenizer("redactio validation", truncation=True, return_tensors="pt"))
        return window
    except Exception as error:
        raise EngineError("model_incompatible") from error
