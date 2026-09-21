from pathlib import Path

import pytest

from redactio_sidecar.ipc import EngineError
from redactio_sidecar.model_store import Window, processing_window


@pytest.mark.parametrize(
    ("model", "tokenizer", "special", "expected"),
    [
        (512, 512, 2, Window(512, 128)),
        (256, 256, 2, Window(256, 64)),
        (1024, 1024, 2, Window(1024, 256)),
        (2048, 1024, 2, Window(1024, 256)),
        (1024, int(1e30), 2, Window(1024, 256)),
        (1024, None, 2, Window(1024, 256)),
        (8, 8, 6, Window(8, 1)),
    ],
)
def test_model_dependent_window(model, tokenizer, special, expected):
    assert processing_window(model, tokenizer, special) == expected


@pytest.mark.parametrize(
    ("model", "tokenizer", "special"),
    [
        (None, 512, 2),
        (0, 512, 2),
        (True, 512, 2),
        (512, -1, 2),
        (512, "512", 2),
        (2, 2, 2),
        (512, 512, -1),
    ],
)
def test_invalid_capacity_is_explicit(model, tokenizer, special):
    with pytest.raises(ValueError):
        processing_window(model, tokenizer, special)


class _ValidationTokenizer:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, object]]] = []

    def __call__(self, text: str, **kwargs: object) -> dict[str, object]:
        self.calls.append((text, kwargs))
        return {
            "offset_mapping": [
                [(0, 0), (0, 4), (5, 12)],
                [(0, 0), (len(text) - 4, len(text))],
            ]
        }


class _NoRawForward:
    def __call__(self, **_: object) -> None:
        raise AssertionError("validation must use the production pipeline")


@pytest.mark.parametrize(
    ("window", "detections"),
    [(Window(32, 8), [{"start": 0, "end": 4}]), (Window(1024, 256), [])],
)
def test_validation_runs_unicode_overflow_through_shared_pipeline(
    monkeypatch, window: Window, detections: list[dict[str, int]]
):
    from redactio_sidecar import model_store

    tokenizer = _ValidationTokenizer()
    model = _NoRawForward()
    captured: dict[str, object] = {}

    monkeypatch.setattr(
        model_store, "_load_local_model", lambda *_: (tokenizer, model, window)
    )

    def pipeline(actual_model, actual_tokenizer, actual_window):
        captured.update(model=actual_model, tokenizer=actual_tokenizer, window=actual_window)
        return lambda _: detections

    monkeypatch.setattr(model_store, "_token_classification_pipeline", pipeline)

    assert (
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")
        == window
    )
    text, kwargs = tokenizer.calls[0]
    assert "😀\r\n" in text
    assert kwargs == {
        "truncation": True,
        "max_length": window.tokens,
        "stride": window.stride,
        "return_overflowing_tokens": True,
        "return_offsets_mapping": True,
    }
    assert captured == {"model": model, "tokenizer": tokenizer, "window": window}


@pytest.mark.parametrize(
    "detections",
    [
        lambda text: [{"start": -1, "end": 4}],
        lambda text: [{"start": 0, "end": len(text) + 1}],
        lambda text: [{"start": 1, "end": 4}],
        lambda text: [{"start": 0, "end": 4}, {"start": 0, "end": 4}],
    ],
)
def test_validation_rejects_invalid_truncated_and_reset_offsets(monkeypatch, detections):
    from redactio_sidecar import model_store

    monkeypatch.setattr(
        model_store,
        "_load_local_model",
        lambda *_: (_ValidationTokenizer(), _NoRawForward(), Window(32, 8)),
    )
    monkeypatch.setattr(
        model_store,
        "_token_classification_pipeline",
        lambda *_: lambda text: detections(text),
    )

    with pytest.raises(EngineError, match="^model_incompatible$"):
        model_store.validate_local_model(Path("unused"), "bert", "BertForTokenClassification")
