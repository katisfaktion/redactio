import unicodedata

from redactio_sidecar.schemas import Decisions, Detection, OutputEntry


def apply_redactions(
    text: str,
    detections: list[Detection],
    decisions: Decisions,
) -> tuple[str, list[OutputEntry]]:
    known_ids = {detection.id for detection in detections}
    if any(dismissed_id not in known_ids for dismissed_id in decisions.dismissed_ids):
        raise ValueError("unknown dismissed detection ID")
    if any(detection.origin != "manual" for detection in decisions.manual):
        raise ValueError("manual decision must have manual origin")

    for detection in [*detections, *decisions.manual]:
        if detection.end > len(text):
            raise ValueError("span outside original text")

    dismissed_ids = set(decisions.dismissed_ids)
    active = [detection for detection in detections if detection.id not in dismissed_ids]
    active.extend(decisions.manual)
    active.sort(key=lambda detection: (detection.start, detection.end, detection.id))

    unions: list[tuple[int, int, list[Detection]]] = []
    for detection in active:
        if not unions or detection.start >= unions[-1][1]:
            unions.append((detection.start, detection.end, [detection]))
            continue
        start, end, contributors = unions[-1]
        contributors.append(detection)
        unions[-1] = (start, max(end, detection.end), contributors)

    placeholders: dict[tuple[str, str], str] = {}
    counters: dict[str, int] = {}
    pieces: list[str] = []
    entries: list[OutputEntry] = []
    cursor = 0
    output_length = 0

    for start, end, contributors in unions:
        winner = min(
            contributors,
            key=lambda detection: (
                detection.origin != "manual",
                -(detection.confidence if detection.confidence is not None else -1.0),
                detection.entity_type,
                detection.start,
                detection.end,
                detection.id,
            ),
        )
        prefix = text[cursor:start]
        pieces.append(prefix)
        output_length += len(prefix)

        key = (winner.entity_type, unicodedata.normalize("NFC", text[start:end]).strip())
        if key not in placeholders:
            counters[winner.entity_type] = counters.get(winner.entity_type, 0) + 1
            placeholders[key] = f"<{winner.entity_type}_{counters[winner.entity_type]}>"
        placeholder = placeholders[key]

        entry_start = output_length
        pieces.append(placeholder)
        output_length += len(placeholder)
        merged = len(contributors) > 1
        entries.append(
            OutputEntry(
                start_offset=entry_start,
                end_offset=output_length,
                entity_type=winner.entity_type,
                placeholder=placeholder,
                confidence=None if winner.origin == "manual" else winner.confidence,
                recognizer=(
                    "merged"
                    if merged
                    else "manual"
                    if winner.origin == "manual"
                    else winner.recognizer
                ),
                origin="merged" if merged else winner.origin,
            )
        )
        cursor = end

    pieces.append(text[cursor:])
    return "".join(pieces), entries
