import pytest

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
