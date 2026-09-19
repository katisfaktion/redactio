"""Build-time staging of verified, package-local model data."""

import argparse
import hashlib
import json
import shutil
import zipfile
from pathlib import Path, PurePosixPath


def stage_model(wheel: Path, destination: Path, model: dict) -> None:
    with wheel.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != model["sha256"]:
            raise ValueError("Model artifact checksum mismatch")
    if destination.exists():
        raise ValueError("Model destination already exists")
    prefix = f"{model['name']}/{model['name']}-{model['version']}/"
    with zipfile.ZipFile(wheel) as archive:
        members = [entry for entry in archive.infolist() if entry.filename.startswith(prefix)]
        for entry in members:
            relative = PurePosixPath(entry.filename.removeprefix(prefix))
            if relative.is_absolute() or ".." in relative.parts or "\\" in entry.filename:
                raise ValueError("Unsafe model artifact member")
        if not any(entry.filename == prefix + "config.cfg" for entry in members):
            raise ValueError("Model data missing")
        for entry in members:
            target = destination / model["name"] / entry.filename.removeprefix(prefix)
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as source, target.open("xb") as output:
                    shutil.copyfileobj(source, output)
    metadata = json.loads((destination / model["name"] / "meta.json").read_text())
    if (f"{metadata['lang']}_{metadata['name']}", metadata["version"]) != (
        model["name"],
        model["version"],
    ):
        raise ValueError("Model metadata mismatch")
    (destination / "manifest.json").write_text(
        json.dumps(
            {
                "models": [
                    {
                        "name": model["name"],
                        "version": model["version"],
                        "path": model["name"],
                    }
                ]
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()
    inputs = json.loads((Path(__file__).parent / "build-inputs.json").read_text())
    stage_model(args.wheel, args.destination, inputs["model"])
