"""Generate synthetic packaging input; never overwrite an existing document."""

import argparse
import hashlib
import json
from pathlib import Path

from docx import Document


def generate(output: Path, count: int) -> None:
    if count != 1:
        raise ValueError("The early packaging smoke supports --count 1 only")
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        raise ValueError("The output directory must be empty")
    document = Document()
    for text in (
        "Synthetischer Fall 1.",
        "Kontakt: anna.beispiel@example.invalid",
        "Max Mustermann besucht uns. Der Termin ist in Berlin.",
        "Kontakt: max@example.com",
        "Telefon: +49 30 12345678",
        "IBAN: DE89370400440532013000",
        "Server: 192.168.1.1",
        "Webseite: https://example.de/path",
        "Termin: 19.09.2026",
    ):
        document.add_paragraph(text)
    table = document.add_table(rows=1, cols=2)
    table.cell(0, 0).text = "Kategorie"
    table.cell(0, 1).text = "Synthetischer Inhalt"
    source = output / "case-0001.docx"
    document.save(source)
    (output / "expectations.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "documents": 1,
                "source_hash_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--count", default=1, type=int)
    args = parser.parse_args()
    generate(args.output, args.count)
