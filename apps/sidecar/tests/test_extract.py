import hashlib
import socket
import struct
from io import BytesIO
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile, ZipInfo

import pytest
from docx import Document
from docx.enum.style import WD_STYLE_TYPE
from docx.opc.constants import RELATIONSHIP_TYPE as RT
from docx.oxml import OxmlElement
from docx.oxml.ns import qn

from redactio_sidecar import extract
from redactio_sidecar.extract import extract_document
from redactio_sidecar.ipc import EngineError


def test_paragraphs_and_merged_table_cells_keep_order(tmp_path: Path) -> None:
    document = Document()
    document.add_paragraph("Vorher")
    table = document.add_table(rows=1, cols=2)
    table.cell(0, 0).merge(table.cell(0, 1)).text = "Gemeinsam"
    document.add_paragraph("Nachher")
    path = tmp_path / "case.docx"
    document.save(path)
    original = path.read_bytes()
    result = extract_document(path)
    assert result.text == "Vorher\n\nGemeinsam\n\nNachher"
    assert result.warnings == []
    assert path.read_bytes() == original


def package(
    tmp_path: Path, additions: dict[str, bytes], *, compression: int = ZIP_DEFLATED
) -> Path:
    source = BytesIO()
    Document().save(source)
    target = tmp_path / "case.docx"
    with ZipFile(source) as original, ZipFile(target, "w", compression) as archive:
        for info in original.infolist():
            archive.writestr(info.filename, additions.pop(info.filename, original.read(info)))
        for name, content in additions.items():
            info = ZipInfo(name)
            # ZipInfo normalizes Windows separators; malformed fixtures need the raw name.
            info.filename = name
            info.compress_type = compression
            archive.writestr(info, content)
    return target


def error_code(path: Path, expected: str) -> None:
    before = path.read_bytes() if path.is_file() else None
    with pytest.raises(EngineError) as error:
        extract_document(path)
    assert error.value.code == expected
    assert str(error.value) == expected
    if before is not None:
        assert path.read_bytes() == before


def test_nested_tables_vertical_merges_and_hyperlink_labels(tmp_path, monkeypatch):
    def network_forbidden(*args, **kwargs):
        pytest.fail("DOCX extraction attempted network access")

    monkeypatch.setattr(socket.socket, "connect", network_forbidden)
    document = Document()
    paragraph = document.add_paragraph("Before ")
    hyperlink = OxmlElement("w:hyperlink")
    hyperlink.set(
        qn("r:id"),
        paragraph.part.relate_to(
            "https://unreachable.invalid/private-target", RT.HYPERLINK, is_external=True
        ),
    )
    run = OxmlElement("w:r")
    text = OxmlElement("w:t")
    text.text = "Visible label"
    run.append(text)
    hyperlink.append(run)
    paragraph._p.append(hyperlink)
    table = document.add_table(rows=2, cols=2)
    table.cell(0, 0).merge(table.cell(1, 0)).text = "Once"
    table.cell(0, 1).text = "Outer"
    nested = table.cell(0, 1).add_table(rows=1, cols=2)
    nested.cell(0, 0).text = "Nested A"
    nested.cell(0, 1).text = "Nested B"
    table.cell(1, 1).text = "Last"
    path = tmp_path / "nested.docx"
    document.save(path)
    result = extract_document(path)
    assert result.text == "Before Visible label\n\nOnce | Outer\n\nNested A | Nested B\nLast"
    assert result.warnings == []
    assert result.source_hash_sha256 == hashlib.sha256(path.read_bytes()).hexdigest()


def test_hidden_deleted_and_inserted_revision_text_is_not_silently_included(tmp_path):
    document = Document()
    paragraph = document.add_paragraph("Visible")
    paragraph.add_run("Hidden").font.hidden = True
    for tag in ("w:del", "w:ins", "w:moveFrom", "w:moveTo"):
        revision = OxmlElement(tag)
        run = OxmlElement("w:r")
        text = OxmlElement("w:t")
        text.text = "Revision secret"
        run.append(text)
        revision.append(run)
        paragraph._p.append(revision)
    path = tmp_path / "revisions.docx"
    document.save(path)
    result = extract_document(path)
    assert result.text == "Visible"
    assert result.warnings == ["tracked_changes"]


def test_all_unsupported_content_warns_once_without_leaking_text(tmp_path):
    document = Document()
    document.add_paragraph("Main")
    document.sections[0].header.paragraphs[0].text = "Header secret"
    document.sections[0].footer.paragraphs[0].text = "Footer secret"
    paragraph = document.add_paragraph()
    for tag in ("w:txbxContent", "w:txbxContent", "w:object", "w:drawing", "w:commentReference"):
        paragraph._p.append(OxmlElement(tag))
    buffer = BytesIO()
    document.save(buffer)
    path = tmp_path / "coverage.docx"
    with ZipFile(buffer) as original, ZipFile(path, "w", ZIP_DEFLATED) as archive:
        for info in original.infolist():
            archive.writestr(info.filename, original.read(info))
        for name in ("footnotes", "endnotes", "comments"):
            archive.writestr(f"word/{name}.xml", b"<unsupported>secret</unsupported>")
        archive.writestr("word/embeddings/item.bin", b"secret")
        archive.writestr("word/media/image.png", b"secret")
    result = extract_document(path)
    assert result.text == "Main"
    assert result.warnings == [
        "comments",
        "embedded_objects",
        "headers_footers",
        "images",
        "notes",
        "text_boxes",
    ]


def test_empty_document_warns(tmp_path):
    result = extract_document(package(tmp_path, {}))
    assert result.text == ""
    assert result.warnings == ["empty_document"]


@pytest.mark.parametrize(
    "name",
    ["../secret.xml", "/secret.xml", "word/../secret.xml", "word\\secret.xml", "C:/secret.xml"],
)
def test_traversal_members_rejected(tmp_path, name):
    error_code(package(tmp_path, {name: b"<x/>"}), "invalid_docx")


def test_duplicate_member_rejected(tmp_path):
    path = package(tmp_path, {})
    with ZipFile(path, "a") as archive, pytest.warns(UserWarning, match="Duplicate"):
        archive.writestr("word/document.xml", b"<fake/>")
    error_code(path, "invalid_docx")


@pytest.mark.parametrize("content", [b"not ZIP", b"PK\x03\x04broken"])
def test_corrupt_or_non_docx_rejected(tmp_path, content):
    path = tmp_path / "bad.docx"
    path.write_bytes(content)
    error_code(path, "invalid_docx")


def test_missing_docx_parts_rejected(tmp_path):
    path = tmp_path / "missing.docx"
    with ZipFile(path, "w") as archive:
        archive.writestr("arbitrary.xml", "<root/>")
    error_code(path, "invalid_docx")


def test_unreadable_document_has_safe_error(tmp_path):
    error_code(tmp_path / "private-name.docx", "unreadable_document")


def test_encrypted_member_rejected(tmp_path):
    path = package(tmp_path, {})
    data = bytearray(path.read_bytes())
    local = data.index(b"PK\x03\x04")
    central = data.index(b"PK\x01\x02")
    struct.pack_into("<H", data, local + 6, 1)
    struct.pack_into("<H", data, central + 8, 1)
    path.write_bytes(data)
    error_code(path, "unsupported_document")


def test_crc_corruption_rejected(tmp_path):
    path = package(tmp_path, {"payload.bin": b"CANARY_PAYLOAD"}, compression=ZIP_STORED)
    data = path.read_bytes().replace(b"CANARY_PAYLOAD", b"BROKEN_PAYLOAD")
    path.write_bytes(data)
    error_code(path, "invalid_docx")


@pytest.mark.parametrize("encoding", ["utf-8", "utf-16"])
def test_entity_and_doctype_in_any_package_xml_rejected(tmp_path, encoding):
    xml = '<?xml version="1.0" encoding="ENC"?><!DOCTYPE x [<!ENTITY secret SYSTEM "file:///private">]><x>&secret;</x>'
    path = package(tmp_path, {"custom/unused.xml": xml.replace("ENC", encoding).encode(encoding)})
    error_code(path, "unsafe_xml")


def test_external_doctype_rejected_without_network(tmp_path, monkeypatch):
    def forbidden(*args, **kwargs):
        pytest.fail("XML parser attempted network access")

    monkeypatch.setattr(socket.socket, "connect", forbidden)
    path = package(
        tmp_path, {"unused.xml": b'<!DOCTYPE x SYSTEM "https://unreachable.invalid/dtd"><x/>'}
    )
    error_code(path, "unsafe_xml")


def test_malformed_unused_xml_rejected(tmp_path):
    error_code(package(tmp_path, {"unused.xml": b"<broken>"}), "invalid_docx")


def test_compressed_input_limit(tmp_path, monkeypatch):
    path = package(tmp_path, {})
    monkeypatch.setattr(extract, "MAX_COMPRESSED_BYTES", len(path.read_bytes()) - 1, raising=False)
    error_code(path, "document_too_large")


def test_actual_cumulative_inflated_limit(tmp_path, monkeypatch):
    path = package(tmp_path, {})
    with ZipFile(path) as archive:
        actual = sum(len(archive.read(info)) for info in archive.infolist())
    monkeypatch.setattr(extract, "MAX_UNCOMPRESSED_BYTES", actual - 1, raising=False)
    error_code(path, "document_too_large")


def test_member_expansion_ratio(tmp_path, monkeypatch):
    path = package(tmp_path, {"padding.bin": b"x" * 100000})
    monkeypatch.setattr(extract, "MAX_EXPANSION_RATIO", 100, raising=False)
    error_code(path, "document_too_large")


def test_forged_uncompressed_sizes_cannot_hide_actual_expansion(tmp_path, monkeypatch):
    # A normal ZipExtFile read silently truncates this member to its forged size.
    path = tmp_path / "forged.docx"
    with ZipFile(path, "w", ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", b"x" * 5000)
        archive.writestr("_rels/.rels", b"<x/>")
        archive.writestr("word/document.xml", b"<x/>")
    data = bytearray(path.read_bytes())
    struct.pack_into("<I", data, 22, 1)
    central = data.index(b"PK\x01\x02")
    struct.pack_into("<I", data, central + 24, 1)
    path.write_bytes(data)
    monkeypatch.setattr(extract, "MAX_UNCOMPRESSED_BYTES", 1000, raising=False)
    error_code(path, "document_too_large")


def test_codepoint_limit_counts_separators_and_unicode(tmp_path, monkeypatch):
    document = Document()
    document.add_paragraph("ä🙂")
    document.add_paragraph("B")
    path = tmp_path / "unicode.docx"
    document.save(path)
    monkeypatch.setattr(extract, "MAX_TEXT_CODEPOINTS", 5, raising=False)
    assert extract_document(path).text == "ä🙂\n\nB"
    monkeypatch.setattr(extract, "MAX_TEXT_CODEPOINTS", 4)
    error_code(path, "document_too_large")


def test_snapshot_hash_and_extracted_text_cannot_race(tmp_path, monkeypatch):
    document = Document()
    document.add_paragraph("Original")
    path = tmp_path / "snapshot.docx"
    document.save(path)
    original_hash = hashlib.sha256(path.read_bytes()).hexdigest()
    real_document = extract.Document

    def replace_source_after_snapshot(stream):
        path.write_bytes(b"changed after bounded read")
        return real_document(stream)

    monkeypatch.setattr(extract, "Document", replace_source_after_snapshot)
    result = extract_document(path)
    assert result.text == "Original"
    assert result.source_hash_sha256 == original_hash


def test_hidden_style_inheritance_and_explicit_visibility(tmp_path):
    document = Document()
    hidden = document.styles.add_style("HiddenBase", WD_STYLE_TYPE.CHARACTER)
    hidden.font.hidden = True
    inherited = document.styles.add_style("InheritedHidden", WD_STYLE_TYPE.CHARACTER)
    inherited.base_style = hidden
    paragraph = document.add_paragraph("Visible")
    paragraph.add_run("Style secret", style=inherited)
    visible = paragraph.add_run(" shown", style=inherited)
    visible.font.hidden = False
    hidden_paragraph = document.styles.add_style("HiddenParagraph", WD_STYLE_TYPE.PARAGRAPH)
    hidden_paragraph.font.hidden = True
    document.add_paragraph("Paragraph secret", style=hidden_paragraph)
    path = tmp_path / "styles.docx"
    document.save(path)
    assert extract_document(path).text == "Visible shown"


def test_default_hidden_text_and_style_toggle(tmp_path):
    document = Document()
    defaults = document.styles.element.find(qn("w:docDefaults"))
    run_default = defaults.find(qn("w:rPrDefault")).find(qn("w:rPr"))
    run_default.append(OxmlElement("w:vanish"))
    document.add_paragraph("Default secret")
    toggled = document.styles.add_style("ToggleVisibility", WD_STYLE_TYPE.CHARACTER)
    toggled.font.hidden = True
    document.add_paragraph().add_run("Visible", style=toggled)
    path = tmp_path / "default.docx"
    document.save(path)
    assert extract_document(path).text == "Visible"


@pytest.mark.parametrize("tag", ["w:sdt", "w:customXml", "w:fldSimple"])
def test_unhandled_visible_containers_fail_explicitly(tmp_path, tag):
    document = Document()
    document.add_paragraph("Known")
    container = OxmlElement(tag)
    run = OxmlElement("w:r")
    text = OxmlElement("w:t")
    text.text = "Otherwise silently omitted"
    run.append(text)
    container.append(run)
    document.element.body.insert(0, container)
    path = tmp_path / "unsupported.docx"
    document.save(path)
    error_code(path, "unsupported_document")


def test_content_type_declared_xml_with_non_xml_extension_is_checked(tmp_path):
    buffer = BytesIO()
    Document().save(buffer)
    with ZipFile(buffer) as archive:
        types = archive.read("[Content_Types].xml").replace(
            b"</Types>",
            b'<Override PartName="/custom/part.bin" '
            b'ContentType="application/example+xml"/></Types>',
        )
    path = package(
        tmp_path,
        {
            "[Content_Types].xml": types,
            "custom/part.bin": b'<!DOCTYPE x [<!ENTITY secret "secret">]><x>&secret;</x>',
        },
    )
    error_code(path, "unsafe_xml")


def test_bounded_snapshot_read(tmp_path, monkeypatch):
    path = tmp_path / "huge.docx"
    path.write_bytes(b"x" * 100)
    real_open = Path.open

    class LimitedReader:
        def __enter__(self):
            return self

        def __exit__(self, *args):
            self.source.close()

        def __init__(self, source):
            self.source = source

        def read(self, size=-1):
            assert 0 < size <= 11, "source read was not bounded before allocation"
            return self.source.read(size)

    def open_bounded(self, *args, **kwargs):
        return LimitedReader(real_open(self, *args, **kwargs))

    monkeypatch.setattr(extract, "MAX_COMPRESSED_BYTES", 10)
    monkeypatch.setattr(Path, "open", open_bounded)
    with pytest.raises(EngineError, match="document_too_large"):
        extract_document(path)


def test_unsupported_header_containers_warn_without_blocking_main_text(tmp_path):
    document = Document()
    document.add_paragraph("Main")
    field = OxmlElement("w:fldSimple")
    document.sections[0].header.paragraphs[0]._p.append(field)
    path = tmp_path / "header.docx"
    document.save(path)
    result = extract_document(path)
    assert result.text == "Main"
    assert result.warnings == ["headers_footers"]


def test_empty_table_cells_still_enforce_separator_budget(tmp_path, monkeypatch):
    document = Document()
    document.add_table(rows=3, cols=3)
    path = tmp_path / "empty-table.docx"
    document.save(path)
    monkeypatch.setattr(extract, "MAX_TEXT_CODEPOINTS", 19)
    error_code(path, "document_too_large")


def test_deleted_table_rows_and_cells_are_not_visible(tmp_path):
    document = Document()
    table = document.add_table(rows=2, cols=2)
    table.cell(0, 0).text = "Deleted row secret"
    table.rows[0]._tr.get_or_add_trPr().append(OxmlElement("w:del"))
    table.cell(1, 0).text = "Visible"
    table.cell(1, 1).text = "Deleted cell secret"
    table.cell(1, 1)._tc.get_or_add_tcPr().append(OxmlElement("w:cellDel"))
    path = tmp_path / "deleted-table.docx"
    document.save(path)
    result = extract_document(path)
    assert result.text == "Visible"
    assert result.warnings == ["tracked_changes"]


def test_local_encryption_flag_cannot_be_hidden_by_central_directory(tmp_path):
    path = package(tmp_path, {})
    data = bytearray(path.read_bytes())
    struct.pack_into("<H", data, data.index(b"PK\x03\x04") + 6, 1)
    path.write_bytes(data)
    error_code(path, "unsupported_document")


def test_smart_tag_visible_runs_fail_explicitly(tmp_path):
    document = Document()
    paragraph = document.add_paragraph("Before ")
    wrapper = OxmlElement("w:smartTag")
    run = OxmlElement("w:r")
    text = OxmlElement("w:t")
    text.text = "Visible wrapped label"
    run.append(text)
    wrapper.append(run)
    paragraph._p.append(wrapper)
    paragraph.add_run(" After")
    path = tmp_path / "smart-tag.docx"
    document.save(path)
    error_code(path, "unsupported_document")


def test_alternate_main_part_cannot_bypass_visible_container_validation(tmp_path):
    document = Document()
    document.add_paragraph("Known")
    control = OxmlElement("w:sdt")
    content = OxmlElement("w:sdtContent")
    paragraph = OxmlElement("w:p")
    run = OxmlElement("w:r")
    text = OxmlElement("w:t")
    text.text = "Visible controlled label"
    run.append(text)
    paragraph.append(run)
    content.append(paragraph)
    control.append(content)
    document.element.body.insert(1, control)
    buffer = BytesIO()
    document.save(buffer)
    with ZipFile(buffer) as archive:
        main = archive.read("word/document.xml")
        relationships = archive.read("_rels/.rels").replace(
            b'Target="word/document.xml"', b'Target="word/alternate.xml"'
        )
        types = archive.read("[Content_Types].xml").replace(
            b'PartName="/word/document.xml"', b'PartName="/word/alternate.xml"'
        )
    # Keep a normal canonical part so the old filename-only guard accepts the ZIP.
    path = package(
        tmp_path,
        {
            "word/alternate.xml": main,
            "_rels/.rels": relationships,
            "[Content_Types].xml": types,
        },
    )
    error_code(path, "unsupported_document")
