"""Read one immutable DOCX snapshot, with bounded ZIP/XML and text processing."""

from __future__ import annotations

import hashlib
import struct
import sys
from collections.abc import Iterable
from copy import copy
from dataclasses import dataclass
from io import BytesIO
from pathlib import Path
from typing import Any, cast
from xml.parsers import expat
from zipfile import ZIP_DEFLATED, ZIP_STORED, ZipFile

from docx import Document
from docx.document import Document as DocumentObject
from docx.oxml.ns import qn
from docx.parts.document import DocumentPart
from docx.styles.style import CharacterStyle
from docx.table import Table, _Cell
from docx.text.hyperlink import Hyperlink
from docx.text.paragraph import Paragraph
from docx.text.run import Run

from .ipc import EngineError

MAX_COMPRESSED_BYTES = 64 * 1024 * 1024
MAX_UNCOMPRESSED_BYTES = 256 * 1024 * 1024
MAX_EXPANSION_RATIO = 1000
MAX_TEXT_CODEPOINTS = 1_000_000
CHUNK_BYTES = 64 * 1024
WORD_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
CONTENT_NS = "http://schemas.openxmlformats.org/package/2006/content-types"


@dataclass(frozen=True)
class Extraction:
    text: str
    source_hash_sha256: str
    warnings: list[str]


def _inspect_xml(data: bytes, warnings: set[str], main: bool = False) -> dict[str, str]:
    """Expat never fetches external resources; reject DTDs before processing them."""
    content_types: dict[str, str] = {}
    parser = expat.ParserCreate(namespace_separator="}")
    parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_NEVER)

    def unsafe(*args: Any) -> None:
        raise EngineError("unsafe_xml")

    def element(name: str, attrs: dict[str, str]) -> None:
        namespace, _, local = name.rpartition("}")
        if namespace == CONTENT_NS:
            if local == "Override":
                content_types[attrs["PartName"].lstrip("/")] = attrs["ContentType"]
            elif local == "Default":
                content_types["*." + attrs["Extension"]] = attrs["ContentType"]
        if namespace == WORD_NS:
            if main and local in {"sdt", "customXml", "fldSimple"}:
                raise EngineError("unsupported_document")
            if local in {"hdr", "ftr", "headerReference", "footerReference"}:
                warnings.add("headers_footers")
            elif local in {"footnote", "endnote", "footnoteReference", "endnoteReference"}:
                warnings.add("notes")
            elif local == "txbxContent":
                warnings.add("text_boxes")
            elif local in {
                "ins",
                "del",
                "moveFrom",
                "moveTo",
                "cellIns",
                "cellDel",
                "cellMerge",
            } or local.endswith("Change"):
                warnings.add("tracked_changes")
            elif local.startswith("comment"):
                warnings.add("comments")
            elif local in {"object", "altChunk"}:
                warnings.add("embedded_objects")
            elif local in {"drawing", "pict"}:
                warnings.add("images")
        elif name in {"urn:schemas-microsoft-com:vml}textbox"}:
            warnings.add("text_boxes")
        elif name == "urn:schemas-microsoft-com:office:office}OLEObject":
            warnings.add("embedded_objects")

    parser.StartDoctypeDeclHandler = unsafe
    parser.EntityDeclHandler = unsafe
    parser.ExternalEntityRefHandler = lambda *args: 0
    parser.StartElementHandler = element
    parser.Parse(data, True)
    return content_types


def _validate_package(snapshot: bytes) -> set[str]:
    warnings: set[str] = set()
    total = 0
    with ZipFile(BytesIO(snapshot)) as archive:
        infos = archive.infolist()
        names: set[str] = set()
        for info in infos:
            name = info.filename
            if (
                name in names
                or name != info.orig_filename
                or name.startswith("/")
                or "\\" in name
                or ":" in name
                or any(part in {"", ".", ".."} for part in name.rstrip("/").split("/"))
            ):
                raise EngineError("invalid_docx")
            names.add(name)
            local_flags, local_method = struct.unpack_from("<HH", snapshot, info.header_offset + 6)
            if (info.flag_bits | local_flags) & 0x41 or info.compress_type not in {
                ZIP_STORED,
                ZIP_DEFLATED,
            }:
                raise EngineError("unsupported_document")
            if local_flags != info.flag_bits or local_method != info.compress_type:
                raise EngineError("invalid_docx")
        if not {"[Content_Types].xml", "_rels/.rels", "word/document.xml"} <= names:
            raise EngineError("invalid_docx")

        # Content types can mark XML parts whose filenames have arbitrary extensions.
        infos.sort(key=lambda info: info.filename != "[Content_Types].xml")
        types: dict[str, str] = {}
        for info in infos:
            actual = 0
            data = bytearray()
            streaming_info = copy(info)
            # ZipExtFile otherwise truncates to a forged header's uncompressed size.
            # This affects its remaining-byte counter only, never allocation size.
            streaming_info.file_size = sys.maxsize
            with archive.open(streaming_info) as member:
                while chunk := member.read(CHUNK_BYTES):
                    actual += len(chunk)
                    total += len(chunk)
                    if total > MAX_UNCOMPRESSED_BYTES or actual > MAX_EXPANSION_RATIO * max(
                        info.compress_size, 1
                    ):
                        raise EngineError("document_too_large")
                    data.extend(chunk)
            if actual != info.file_size:
                raise EngineError("invalid_docx")
            name = info.filename
            content_type = types.get(name, types.get("*." + name.rsplit(".", 1)[-1], ""))
            if name.endswith((".xml", ".rels")) or content_type.endswith(("+xml", "/xml")):
                found_types = _inspect_xml(bytes(data), warnings, main=name == "word/document.xml")
                if name == "[Content_Types].xml":
                    types = found_types
            if name.startswith("word/header") or name.startswith("word/footer"):
                warnings.add("headers_footers")
            elif name in {"word/footnotes.xml", "word/endnotes.xml"}:
                warnings.add("notes")
            elif name.startswith("word/comments"):
                warnings.add("comments")
            elif name.startswith("word/embeddings/"):
                warnings.add("embedded_objects")
            elif name.startswith("word/media/"):
                warnings.add("images")
    return warnings


@dataclass
class _TextBudget:
    size: int = 0

    def count(self, text: str) -> str:
        self.size += len(text)
        if self.size > MAX_TEXT_CODEPOINTS:
            raise EngineError("document_too_large")
        return text

    def join(self, parts: Iterable[str], separator: str) -> str:
        collected: list[str] = []
        for part in parts:
            if collected:
                self.count(separator)
            collected.append(part)
        return separator.join(collected)


def _hidden(run: Run, paragraph: Paragraph) -> bool:
    if run.font.hidden is not None:
        return run.font.hidden
    defaults = cast(DocumentPart, paragraph.part).styles.element.xpath(
        "./w:docDefaults/w:rPrDefault/w:rPr/w:vanish"
    )
    hidden = bool(defaults) and defaults[0].get(qn("w:val")) not in {"0", "false", "off"}
    # Vanish is a toggle in styles, an absolute override on the run itself.
    for initial in (paragraph.style, run.style):
        style: CharacterStyle | None = initial
        seen: set[str] = set()
        while style is not None:
            if style.style_id in seen:
                raise EngineError("invalid_docx")
            seen.add(style.style_id)
            if style.font.hidden:
                hidden = not hidden
            style = style.base_style
    return hidden


def _paragraph(paragraph: Paragraph, budget: _TextBudget) -> str:
    text: list[str] = []
    for item in paragraph.iter_inner_content():
        runs = item.runs if isinstance(item, Hyperlink) else [item]
        for run in runs:
            if not _hidden(run, paragraph) and run.text:
                text.append(budget.count(run.text))
    return "".join(text)


def _table(table: Table, budget: _TextBudget) -> str:
    def rows() -> Iterable[str]:
        for row in table.rows:
            if row._tr.xpath("./w:trPr/w:del"):
                continue
            # Physical cells avoid expanding arbitrarily large gridSpan declarations.
            cells = (
                _blocks(_Cell(tc, table), budget)
                for tc in row._tr.tc_lst
                if tc.vMerge != "continue" and not tc.xpath("./w:tcPr/w:cellDel")
            )
            text = budget.join(cells, " | ")
            if text:
                yield text

    return budget.join(rows(), "\n")


def _blocks(container: DocumentObject | _Cell, budget: _TextBudget) -> str:
    def blocks() -> Iterable[str]:
        for block in container.iter_inner_content():
            text = (
                _paragraph(block, budget) if isinstance(block, Paragraph) else _table(block, budget)
            )
            if text:
                yield text

    return budget.join(blocks(), "\n\n")


def extract_document(path: Path) -> Extraction:
    try:
        with path.open("rb") as source:
            snapshot = source.read(MAX_COMPRESSED_BYTES + 1)
    except OSError:
        raise EngineError("unreadable_document") from None
    if len(snapshot) > MAX_COMPRESSED_BYTES:
        raise EngineError("document_too_large")
    try:
        warnings = _validate_package(snapshot)
        document = Document(BytesIO(snapshot))
        text = _blocks(document, _TextBudget())
    except EngineError:
        raise
    except Exception:
        # Neither library exception text nor package paths may cross the boundary.
        raise EngineError("invalid_docx") from None
    if not text.strip():
        warnings.add("empty_document")
    return Extraction(text, hashlib.sha256(snapshot).hexdigest(), sorted(warnings))
