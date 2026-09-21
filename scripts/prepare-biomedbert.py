"""Download pinned public model files during setup, never during document processing."""

import argparse
import hashlib
import json
import os
import re
import tempfile
from pathlib import Path
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]


def catalog_inputs(key: str) -> dict:
    from redactio_sidecar.model_store import catalog_models

    entry = next(entry for entry in catalog_models() if entry.key == key)
    descriptor = entry.descriptor
    return {
        "name": descriptor.name,
        "repository": descriptor.repository,
        "revision": descriptor.version,
        "license": descriptor.license,
        "directory": entry.directory,
        "files": {file.filename: file.sha256 for file in descriptor.files},
    }


def verified(path: Path, expected: str) -> bool:
    if path.is_symlink() or not path.is_file():
        return False
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest() == expected


def prepare(root: Path, inputs: dict) -> None:
    directory = inputs.get("directory", "biomedbert-de")
    if not isinstance(directory, str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", directory):
        raise ValueError("invalid model directory")
    root.mkdir(parents=True, exist_ok=True)
    manifest_path = root / "manifest.json"
    if manifest_path.is_symlink():
        raise ValueError("model manifest must not be a symlink")
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {"models": []}
    if set(manifest) != {"models"} or not isinstance(manifest["models"], list):
        raise ValueError("invalid model manifest")
    entry = {"name": inputs["name"], "version": inputs["revision"], "path": directory}
    for previous in manifest["models"]:
        if not isinstance(previous, dict) or set(previous) != {"name", "version", "path"}:
            raise ValueError("invalid model manifest entry")
        if (
            previous["name"] == entry["name"] or previous["path"] == entry["path"]
        ) and previous != entry:
            raise ValueError("a different model already uses this name or directory")
    destination = root / entry["path"]
    identity = {
        "name": inputs["name"],
        "version": inputs["revision"],
        "repository": inputs["repository"],
    }
    if destination.exists() or destination.is_symlink():
        if destination.is_symlink() or not all(
            verified(destination / name, digest) for name, digest in inputs["files"].items()
        ):
            raise ValueError("existing model checksum mismatch; choose a fresh model directory")
        if json.loads((destination / "redactio-model.json").read_text()) != identity:
            raise ValueError("existing model identity mismatch")
    else:
        with tempfile.TemporaryDirectory(prefix=".model-", dir=root) as temporary:
            staging = Path(temporary) / "model"
            staging.mkdir()
            for name, digest in inputs["files"].items():
                print(f"Preparing {name}", flush=True)
                url = f"https://huggingface.co/{inputs['repository']}/resolve/{inputs['revision']}/{name}"
                target = staging / name
                with urlopen(url, timeout=120) as response, target.open("wb") as output:
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
                if not verified(target, digest):
                    raise ValueError(f"download checksum mismatch: {name}")
            (staging / "redactio-model.json").write_text(json.dumps(identity, indent=2) + "\n")
            staging.rename(destination)
    active = [
        model
        for model in manifest["models"]
        if model["name"] not in {"de_core_news_sm", "de_core_news_md", "de_core_news_lg"}
    ]
    if entry not in active:
        active.append(entry)
    if active != manifest["models"]:
        manifest["models"] = active
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=root, delete=False
        ) as output:
            temporary_manifest = Path(output.name)
            json.dump(manifest, output, indent=2)
            output.write("\n")
        try:
            os.replace(temporary_manifest, manifest_path)
        finally:
            temporary_manifest.unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=("biomedbert", "hugginglil"), default="biomedbert")
    parser.add_argument("--model-dir", type=Path, default=ROOT / "apps/sidecar/models")
    args = parser.parse_args()
    inputs = catalog_inputs(args.model)
    prepare(args.model_dir, inputs)
    print(f"{inputs['name']} is prepared for offline use.")
