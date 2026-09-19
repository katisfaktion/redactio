import pytest

from redactio_sidecar.redaction import apply_redactions
from redactio_sidecar.schemas import Decisions, Detection


def detection(
    id: str,
    start: int,
    end: int,
    *,
    entity_type: str = "PERSON",
    confidence: float | None = 0.9,
    recognizer: str = "SpacyRecognizer",
    origin: str = "automatic",
) -> Detection:
    return Detection.model_validate(
        {
            "id": id,
            "start": start,
            "end": end,
            "entity_type": entity_type,
            "confidence": confidence,
            "recognizer": recognizer,
            "origin": origin,
        }
    )


def test_repeated_values_share_a_placeholder_and_overlaps_cover_the_union() -> None:
    body, entries = apply_redactions(
        "Anna Anna",
        [detection("a", 0, 4), detection("b", 5, 9)],
        Decisions(),
    )

    assert body == "<PERSON_1> <PERSON_1>"
    assert len(entries) == 2

    body, entries = apply_redactions(
        "abcdef",
        [detection("a", 0, 4), detection("b", 2, 6)],
        Decisions(),
    )

    assert body == "<PERSON_1>"
    assert body[entries[0].start_offset : entries[0].end_offset] == entries[0].placeholder


def test_transitive_overlaps_cover_every_contributing_span() -> None:
    body, entries = apply_redactions(
        "abcdefgh",
        [
            detection("a", 0, 3),
            detection("b", 2, 5),
            detection("c", 4, 8),
        ],
        Decisions(),
    )

    assert body == "<PERSON_1>"
    assert len(entries) == 1


def test_dismissing_a_nested_detection_keeps_the_active_detection() -> None:
    body, entries = apply_redactions(
        "abcdef",
        [detection("active", 0, 6), detection("dismissed", 2, 4)],
        Decisions(dismissed_ids=["dismissed"]),
    )

    assert body == "<PERSON_1>"
    assert len(entries) == 1


def test_manual_type_wins_a_merged_span_and_has_public_merged_metadata() -> None:
    manual = detection(
        "manual",
        2,
        6,
        entity_type="PERSON",
        confidence=None,
        recognizer="manual",
        origin="manual",
    )

    body, entries = apply_redactions(
        "abcdef",
        [detection("automatic", 0, 4, entity_type="LOCATION", confidence=0.99)],
        Decisions(manual=[manual]),
    )

    assert body == "<PERSON_1>"
    assert entries[0].model_dump() == {
        "start_offset": 0,
        "end_offset": 10,
        "entity_type": "PERSON",
        "placeholder": "<PERSON_1>",
        "confidence": None,
        "recognizer": "merged",
        "origin": "merged",
    }


def test_automatic_overlap_uses_confidence_then_type_name() -> None:
    body, entries = apply_redactions(
        "abcdef",
        [
            detection("low", 0, 4, entity_type="PERSON", confidence=0.2),
            detection("high", 2, 6, entity_type="LOCATION", confidence=0.8),
        ],
        Decisions(),
    )

    assert body == "<LOCATION_1>"
    assert entries[0].confidence == 0.8

    body, entries = apply_redactions(
        "abcdef",
        [
            detection("person", 0, 4, entity_type="PERSON", confidence=0.8),
            detection("location", 2, 6, entity_type="LOCATION", confidence=0.8),
        ],
        Decisions(),
    )

    assert body == "<LOCATION_1>"
    assert entries[0].entity_type == "LOCATION"


def test_adjacent_spans_are_not_merged() -> None:
    body, entries = apply_redactions(
        "abcdef",
        [detection("a", 0, 3), detection("b", 3, 6)],
        Decisions(),
    )

    assert body == "<PERSON_1><PERSON_2>"
    assert [entry.origin for entry in entries] == ["automatic", "automatic"]


def test_unknown_dismissed_id_is_rejected() -> None:
    with pytest.raises(ValueError, match="unknown dismissed detection ID"):
        apply_redactions("Anna", [detection("known", 0, 4)], Decisions(dismissed_ids=["other"]))


@pytest.mark.parametrize("manual", [False, True])
def test_span_past_original_text_is_rejected(manual: bool) -> None:
    span = detection(
        "bad",
        0,
        5,
        confidence=None if manual else 0.9,
        recognizer="manual" if manual else "SpacyRecognizer",
        origin="manual" if manual else "automatic",
    )
    detections = [] if manual else [span]
    decisions = Decisions(manual=[span]) if manual else Decisions()

    with pytest.raises(ValueError, match="span outside original text"):
        apply_redactions("Anna", detections, decisions)


def test_manual_span_has_no_invented_confidence() -> None:
    manual = detection(
        "manual",
        0,
        4,
        confidence=0.7,
        recognizer="stale-client-value",
        origin="manual",
    )

    body, entries = apply_redactions("Anna", [], Decisions(manual=[manual]))

    assert body == "<PERSON_1>"
    assert entries[0].confidence is None
    assert entries[0].recognizer == "manual"
    assert entries[0].origin == "manual"


def test_manual_decision_requires_manual_origin() -> None:
    inconsistent = detection("manual", 0, 4, origin="automatic")

    with pytest.raises(ValueError, match="manual decision must have manual origin"):
        apply_redactions("Anna", [], Decisions(manual=[inconsistent]))


def test_nfc_equivalent_trimmed_values_share_a_typed_placeholder() -> None:
    body, entries = apply_redactions(
        " Café  Cafe\u0301 ",
        [detection("composed", 0, 6), detection("decomposed", 7, 13)],
        Decisions(),
    )

    assert body == "<PERSON_1> <PERSON_1>"
    assert [entry.placeholder for entry in entries] == ["<PERSON_1>", "<PERSON_1>"]


def test_output_offsets_use_code_points_after_an_emoji_prefix() -> None:
    body, entries = apply_redactions(
        "🙂 Anna",
        [detection("anna", 2, 6)],
        Decisions(),
    )

    assert body == "🙂 <PERSON_1>"
    assert (entries[0].start_offset, entries[0].end_offset) == (2, 12)
    assert body[2:12] == "<PERSON_1>"


def test_literal_placeholder_text_is_not_reported_as_a_redaction() -> None:
    body, entries = apply_redactions(
        "<PERSON_1> Anna",
        [detection("anna", 11, 15)],
        Decisions(),
    )

    assert body == "<PERSON_1> <PERSON_1>"
    assert len(entries) == 1
    assert (entries[0].start_offset, entries[0].end_offset) == (11, 21)
