from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import tempfile
from importlib.resources import files
from pathlib import Path
from typing import Annotated, Any, Literal, NamedTuple, Self, cast

from pydantic import (
    AfterValidator,
    BeforeValidator,
    Field,
    PrivateAttr,
    StringConstraints,
    TypeAdapter,
    model_validator,
)

from .ipc import EngineError
from .schemas import EntityType, NonEmptyString, SafeCode, Sha256, StrictModel, UuidString

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


_TOKENIZER_FAMILIES = {
    "bert": ("BertTokenizer", "vocab.txt"),
    "deberta-v2": ("DebertaV2Tokenizer", "spm.model"),
}


def validate_tokenizer_config(
    config: dict[str, Any], tokenizer: dict[str, Any], filenames: set[str] | None = None
) -> dict[str, str]:
    """Validate publisher controls and return the only tokenizer inputs the loader may open."""
    model_type = config.get("model_type")
    if not isinstance(model_type, str) or model_type not in _TOKENIZER_FAMILIES:
        raise EngineError("model_incompatible")
    family, vocab = _TOKENIZER_FAMILIES[model_type]
    file_fields = {
        "tokenizer_file": "tokenizer.json",
        "vocab_file": vocab,
        "tokenizer_config_file": "tokenizer_config.json",
        "added_tokens_file": "added_tokens.json",
        "special_tokens_map_file": "special_tokens_map.json",
    }
    for metadata in (config, tokenizer):
        if metadata.get("auto_map"):
            raise EngineError("model_remote_code_unsupported")
        if metadata.get("tokenizer_class") not in (None, family, family + "Fast"):
            raise EngineError("model_incompatible")
        for key, value in metadata.items():
            if key in file_fields:
                if value is not None and (
                    value != file_fields[key] or filenames is not None and value not in filenames
                ):
                    raise EngineError("model_incompatible")
            elif key == "fast_tokenizer_files":
                if value not in (None, [], ["tokenizer.json"]):
                    raise EngineError("model_incompatible")
            elif key.endswith(("_file", "_files")) and value is not None:
                raise EngineError("model_incompatible")
        if (
            metadata.get("from_slow") is not None
            and metadata["from_slow"] is not False
            or metadata.get("init_inputs") not in (None, [])
            or metadata.get("sp_model_kwargs") not in (None, {})
            or metadata.get("tokenizer_object") is not None
            or metadata.get("__slow_tokenizer") is not None
        ):
            raise EngineError("model_incompatible")
    return file_fields


def _load_local_tokenizer(path: Path, model_type: str | None = None) -> Any:
    def metadata(filename: str) -> dict[str, Any]:
        source = _safe_path(path, filename)
        with source.open("rb") as stream:
            data = stream.read(2 * 1024 * 1024 + 1)
        if len(data) > 2 * 1024 * 1024:
            raise EngineError("model_incompatible")
        value = json.loads(data)
        if not isinstance(value, dict):
            raise EngineError("model_incompatible")
        return value

    config = metadata("config.json")
    if model_type is not None and config.get("model_type") != model_type:
        raise EngineError("model_incompatible")
    tokenizer = metadata("tokenizer_config.json")
    files = validate_tokenizer_config(config, tokenizer)
    family, _ = _TOKENIZER_FAMILIES[config["model_type"]]
    resolved = {}
    for key, filename in files.items():
        source = _safe_path(path, filename)
        resolved[key] = str(source) if source.is_file() else None
    if resolved["tokenizer_file"] is None:
        raise EngineError("model_incompatible")
    validate_tokenizer_config(
        config, tokenizer, {files[key] for key, value in resolved.items() if value is not None}
    )
    if resolved["special_tokens_map_file"] is not None:
        special = metadata("special_tokens_map.json")
        if not set(special) <= {
            "bos_token",
            "eos_token",
            "unk_token",
            "sep_token",
            "pad_token",
            "cls_token",
            "mask_token",
            "additional_special_tokens",
        }:
            raise EngineError("model_incompatible")
    import transformers

    # The public resolver scans auxiliary paths and can fall back to a slow tokenizer. This
    # pinned private entry point retains HF token metadata handling with only our fixed file map.
    cls = cast(Any, getattr(transformers, family + "Fast"))
    result = cls._from_pretrained(
        resolved.copy(),
        str(path),
        {},
        local_files_only=True,
        trust_remote_code=False,
        _is_local=True,
        from_slow=False,
        tokenizer_file=resolved["tokenizer_file"],
        vocab_file=resolved["vocab_file"],
    )
    if not getattr(result, "is_fast", False):
        raise EngineError("model_incompatible")
    return result


def _load_local_model(path: Path, model_type: str, architecture: str) -> tuple[Any, Any, Window]:
    from transformers import AutoModelForTokenClassification

    tokenizer = _load_local_tokenizer(path, model_type)
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


_SAFE_INTEGER = 2**53 - 1
_REPOSITORY_PART = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,95}")
_RESERVED = {
    "con",
    "prn",
    "aux",
    "nul",
    *(f"com{i}" for i in range(1, 10)),
    *(f"lpt{i}" for i in range(1, 10)),
}
_FAMILIES = {"bert": "BertForTokenClassification", "deberta-v2": "DebertaV2ForTokenClassification"}


def _repository(value: str) -> str:
    parts = value.split("/")
    if len(parts) != 2 or any(
        not _REPOSITORY_PART.fullmatch(part)
        or part.endswith((".", "-"))
        or ".." in part
        or "--" in part
        for part in parts
    ):
        raise ValueError("invalid repository")
    return value


def _selection(value: str) -> str:
    if not 1 <= len(value) <= 512 or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value):
        raise ValueError("invalid selection name")
    return value


def _basename(value: str) -> str:
    if (
        not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value)
        or value.endswith(".")
        or value.split(".")[0].lower() in _RESERVED
    ):
        raise ValueError("invalid filename")
    return value


def _directory(value: str) -> str:
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,127}", value):
        raise ValueError("invalid model directory")
    return _basename(value)


def _legacy_path(value: str) -> str:
    if (
        not value
        or "\\" in value
        or ":" in value
        or any(part in ("", ".", "..") for part in value.split("/"))
    ):
        raise ValueError("invalid legacy path")
    return value


def _entities(value: list[str]) -> list[str]:
    if any(re.fullmatch(r"LABEL_[0-9]+", label) for label in value):
        raise ValueError("anonymous model labels are unsupported")
    if not value or value != sorted(set(value)):
        raise ValueError("entities must be nonempty, sorted and unique")
    return value


Repository = Annotated[str, AfterValidator(_repository)]
SelectionName = Annotated[str, AfterValidator(_selection)]
Revision = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{40}$")]
ByteCount = Annotated[int, Field(ge=0, le=_SAFE_INTEGER)]
WindowTokens = Annotated[int, Field(ge=2, le=_SAFE_INTEGER)]
Entities = Annotated[list[EntityType], AfterValidator(_entities)]
Directory = Annotated[str, AfterValidator(_directory)]
CatalogKey = Literal["biomedbert", "hugginglil"]


class UpstreamHash(StrictModel):
    algorithm: Literal["git-sha1", "sha256"]
    value: str

    @model_validator(mode="after")
    def valid_hash(self) -> Self:
        length = 40 if self.algorithm == "git-sha1" else 64
        if not re.fullmatch(r"[0-9a-f]{" + str(length) + "}", self.value):
            raise ValueError("invalid upstream hash")
        return self


class Artifact(StrictModel):
    filename: Annotated[str, AfterValidator(_basename)]
    size: ByteCount
    upstream_hash: UpstreamHash
    sha256: Sha256 | None


class ModelDescriptor(StrictModel):
    name: SelectionName
    version: Revision
    repository: Repository
    title: str
    license: str | None
    model_type: Literal["bert", "deberta-v2"]
    architecture: Literal["BertForTokenClassification", "DebertaV2ForTokenClassification"]
    entity_types: Entities
    window_tokens: WindowTokens
    stride_tokens: Annotated[int, Field(gt=0, le=_SAFE_INTEGER)]
    special_tokens: ByteCount | None
    files: list[Artifact]

    @model_validator(mode="after")
    def valid_descriptor(self) -> Self:
        _matching_name(self.name, self.repository, self.version)
        if self.architecture != _FAMILIES[self.model_type]:
            raise ValueError("incompatible architecture")
        window = processing_window(self.window_tokens, None, self.special_tokens or 0)
        if self.stride_tokens != window.stride:
            raise ValueError("invalid processing stride")
        if len({file.filename for file in self.files}) != len(self.files):
            raise ValueError("duplicate artifacts")
        return self


class CatalogEntry(StrictModel):
    key: CatalogKey
    directory: Directory
    descriptor: ModelDescriptor


def _schema_version(value: object) -> object:
    if type(value) is not int:
        raise ValueError("schema version must be an integer")
    return value


class ModelCatalog(StrictModel):
    schema_version: Annotated[Literal[1], BeforeValidator(_schema_version)]
    models: list[CatalogEntry]


class ModelRecord(StrictModel):
    descriptor: ModelDescriptor
    path: Directory | None
    state: Literal["ready", "available", "removing"]

    @model_validator(mode="after")
    def ready_is_verified(self) -> Self:
        if self.state == "ready" and (
            self.path is None
            or self.descriptor.special_tokens is None
            or any(file.sha256 is None for file in self.descriptor.files)
        ):
            raise ValueError("ready model is not verified")
        return self


class LegacyEntry(StrictModel):
    name: SelectionName
    version: NonEmptyString
    path: Annotated[str, AfterValidator(_legacy_path)]


class ModelRegistry(StrictModel):
    _migrated_legacy: bool = PrivateAttr(default=False)
    schema_version: Annotated[Literal[2], BeforeValidator(_schema_version)]
    models: list[ModelRecord]
    legacy_unavailable: list[LegacyEntry]

    @model_validator(mode="after")
    def unique_identities(self) -> Self:
        names = [model.descriptor.name for model in self.models]
        names.extend(entry.name for entry in self.legacy_unavailable)
        paths = [model.path for model in self.models if model.path is not None]
        if len(set(names)) != len(names) or len(set(paths)) != len(paths):
            raise ValueError("duplicate model identity or path")
        return self


class SafeError(StrictModel):
    code: SafeCode
    retryable: bool


class PairUse(StrictModel):
    id: UuidString
    name: str


class ManagedModel(StrictModel):
    name: SelectionName
    version: Revision
    repository: Repository
    title: str
    license: str | None
    entity_types: Entities
    window_tokens: WindowTokens | None
    download_bytes: ByteCount
    installed_bytes: ByteCount
    state: Literal["available", "ready", "invalid", "removing"]
    catalog_key: CatalogKey | None
    used_by_pairs: list[PairUse]
    error: SafeError | None

    @model_validator(mode="after")
    def valid_name(self) -> Self:
        _matching_name(self.name, self.repository, self.version)
        return self


class CatalogSource(StrictModel):
    kind: Literal["catalog"]
    key: CatalogKey


class UrlSource(StrictModel):
    kind: Literal["url"]
    url: str

    @model_validator(mode="after")
    def repository_url(self) -> Self:
        prefix = "https://huggingface.co/"
        if not self.url.startswith(prefix):
            raise ValueError("invalid repository URL")
        _repository(self.url[len(prefix) :].removesuffix("/"))
        return self


class ReceiptSource(StrictModel):
    kind: Literal["receipt"]
    name: SelectionName


ModelSource = Annotated[CatalogSource | UrlSource | ReceiptSource, Field(discriminator="kind")]


class CheckedModel(StrictModel):
    plan_id: UuidString
    model: ManagedModel


class ModelJob(StrictModel):
    job_id: UuidString
    model_name: SelectionName
    stage: Literal[
        "downloading", "validating", "ready", "cancelled", "failed", "removing", "removed"
    ]
    downloaded_bytes: ByteCount
    total_bytes: ByteCount
    error: SafeError | None

    @model_validator(mode="after")
    def progress_within_total(self) -> Self:
        if self.downloaded_bytes > self.total_bytes:
            raise ValueError("downloaded bytes exceed total")
        return self


def _matching_name(name: str, repository: str, revision: str) -> None:
    if name.startswith("hf:") and name != f"hf:{repository}@{revision}":
        raise ValueError("selection identity mismatch")


def catalog_models() -> list[CatalogEntry]:
    return ModelCatalog.model_validate_json(
        files("redactio_sidecar").joinpath("model_catalog.json").read_text(encoding="utf-8")
    ).models


def selection_name(repository: str, revision: str) -> str:
    _repository(repository)
    TypeAdapter(Revision).validate_python(revision, strict=True)
    for entry in catalog_models():
        descriptor = entry.descriptor
        if (repository, revision) == (descriptor.repository, descriptor.version):
            return descriptor.name
    return f"hf:{repository}@{revision}"


def directory_name(repository: str, revision: str) -> str:
    identity = selection_name(repository, revision)
    return "model-" + hashlib.sha256(identity.encode()).hexdigest()


def model_entity_types(path: Path) -> tuple[str, ...]:
    try:
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        labels = config["id2label"]
        if not isinstance(labels, dict) or not all(
            isinstance(index, str) and index.isdecimal() and isinstance(label, str)
            for index, label in labels.items()
        ):
            return ()
        entities = {
            label[2:] if label.startswith(("B-", "I-")) else label for label in labels.values()
        } - {"O"}
        return tuple(TypeAdapter(Entities).validate_python(sorted(entities), strict=True))
    except (KeyError, OSError, TypeError, ValueError):
        return ()


def _is_link(path: Path) -> bool:
    try:
        attributes = path.lstat()
    except FileNotFoundError:
        return False
    return stat.S_ISLNK(attributes.st_mode) or bool(
        getattr(attributes, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT
    )


def _safe_path(root: Path, relative: str) -> Path:
    root = root.absolute()
    path = root / relative
    for candidate in reversed((path, *path.parents)):
        if _is_link(candidate):
            raise ValueError("model path must not contain links")
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError("model path escapes store")
    return path


def compatible_descriptor(path: Path, descriptor: ModelDescriptor, *, legacy: bool = False) -> bool:
    try:
        path = _safe_path(path.parent, path.name)
        if descriptor.name != selection_name(descriptor.repository, descriptor.version):
            return False
        required = {
            "model.safetensors",
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "redactio-model.json",
        }
        if any(_is_link(path / name) or not (path / name).is_file() for name in required):
            return False
        identity = json.loads((path / "redactio-model.json").read_text(encoding="utf-8"))
        if identity != {
            "name": descriptor.name,
            "version": descriptor.version,
            "repository": descriptor.repository,
        }:
            return False
        config = json.loads((path / "config.json").read_text(encoding="utf-8"))
        tokenizer = json.loads((path / "tokenizer_config.json").read_text(encoding="utf-8"))
        if not isinstance(config, dict) or not isinstance(tokenizer, dict):
            return False
        labels = model_entity_types(path)
        window = processing_window(
            config["max_position_embeddings"],
            tokenizer.get("model_max_length"),
            descriptor.special_tokens or 0,
        )
        if (
            config["model_type"] != descriptor.model_type
            or config.get("architectures") != [descriptor.architecture]
            or not labels
            or window != Window(descriptor.window_tokens, descriptor.stride_tokens)
        ):
            return False
        if legacy:
            return True
        return (
            list(labels) == descriptor.entity_types
            and required - {"redactio-model.json"} <= {file.filename for file in descriptor.files}
            and all(
                not _is_link(path / file.filename)
                and (path / file.filename).is_file()
                and (path / file.filename).stat().st_size == file.size
                for file in descriptor.files
            )
        )
    except (KeyError, OSError, TypeError, ValueError):
        return False


def read_registry(root: Path) -> ModelRegistry:
    try:
        root = root.absolute()
        manifest = _safe_path(root, "manifest.json")
        if not manifest.exists():
            if root.exists() and not root.is_dir():
                raise ValueError("invalid store")
            return ModelRegistry(schema_version=2, models=[], legacy_unavailable=[])
        raw = json.loads(manifest.read_text(encoding="utf-8"))
        if not isinstance(raw, dict):
            raise ValueError("invalid registry")
        if "schema_version" in raw:
            return ModelRegistry.model_validate(raw)
        if set(raw) != {"models"}:
            raise ValueError("invalid legacy manifest")
        entries = TypeAdapter(list[LegacyEntry]).validate_python(raw["models"], strict=True)
        registry = ModelRegistry(schema_version=2, models=[], legacy_unavailable=[])
        catalogs = {entry.descriptor.name: entry for entry in catalog_models()}
        for entry in entries:
            catalog = catalogs.get(entry.name)
            if catalog is None or entry.version != catalog.descriptor.version:
                registry.legacy_unavailable.append(entry)
                continue
            try:
                _directory(entry.path)
                path = _safe_path(root, entry.path)
                descriptor = catalog.descriptor.model_copy(deep=True)
                if not compatible_descriptor(path, descriptor, legacy=True):
                    raise ValueError("incompatible legacy model")
                descriptor.entity_types = list(model_entity_types(path))
                registry.models.append(
                    ModelRecord(descriptor=descriptor, path=entry.path, state="ready")
                )
            except (OSError, ValueError):
                registry.legacy_unavailable.append(entry)
        registry = ModelRegistry.model_validate(registry.model_dump())
        registry._migrated_legacy = True
        return registry
    except (OSError, TypeError, ValueError) as error:
        raise EngineError("invalid_model_manifest") from error


def write_registry(root: Path, registry: ModelRegistry) -> None:
    validated = ModelRegistry.model_validate(registry.model_dump())
    root = root.absolute()
    manifest = _safe_path(root, "manifest.json")
    if manifest.exists():
        read_registry(root)
    root.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=root, delete=False
        ) as stream:
            temporary = Path(stream.name)
            stream.write(validated.model_dump_json(indent=2) + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, manifest)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
