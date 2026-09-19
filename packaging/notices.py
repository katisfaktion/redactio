"""Collect actual dependency notices; missing attribution stops the build."""

import argparse
import importlib.metadata
import json
import subprocess
import sys
from pathlib import Path


def license_files(root: Path) -> list[Path]:
    return sorted(
        path
        for path in root.iterdir()
        if path.is_file()
        and path.name.lower().startswith(("license", "licence", "copying", "notice"))
    )


def section(name: str, version: str, license_name: str, files: list[Path]) -> str:
    if not files:
        raise ValueError(f"Missing license text: {name} {version}")
    return f"\n{'=' * 72}\n{name} {version}\nLicense: {license_name}\n" + "\n".join(
        path.read_text(encoding="utf-8", errors="replace") for path in files
    )


def python_notices() -> str:
    text = section(
        "CPython", sys.version.split()[0], "PSF-2.0", license_files(Path(sys.base_prefix))
    )
    for distribution in sorted(
        importlib.metadata.distributions(), key=lambda d: d.metadata["Name"]
    ):
        files = [
            Path(distribution.locate_file(path))
            for path in distribution.files or []
            if Path(path).name.lower().startswith(("license", "licence", "copying", "notice"))
        ]
        if distribution.metadata["Name"] == "redactio-sidecar":
            continue  # Repository MIT notice is included by desktop_notices.
        if distribution.metadata["Name"].replace("_", "-") == "presidio-analyzer":
            files.append(Path(__file__).parent / "licenses/presidio-LICENSE.txt")
        license_name = distribution.metadata.get("License-Expression") or distribution.metadata.get(
            "License", "See license text"
        )
        text += section(distribution.metadata["Name"], distribution.version, license_name, files)
    return text


def desktop_notices(root: Path, cargo_metadata: Path) -> str:
    text = section("Redactio", "0.1.0", "MIT", [root / "LICENSE"])
    metadata = json.loads(cargo_metadata.read_text(encoding="utf-8"))
    inputs = json.loads((root / "packaging/build-inputs.json").read_text())
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "redactio":
            continue
        directory = Path(package["manifest_path"]).parent
        files = license_files(directory)
        if (directory / "LICENSES").is_dir():
            files += [path for path in (directory / "LICENSES").rglob("*") if path.is_file()]
        if package.get("license_file"):
            files.append(directory / package["license_file"])
        files += [
            root / "packaging" / entry["path"]
            for entry in inputs["notices"]
            if entry.get("package") == package["name"]
            and entry.get("version") == package["version"]
        ]
        text += section(
            package["name"], package["version"], package.get("license") or "See license text", files
        )
        text += (
            f"\nCorresponding source: https://crates.io/api/v1/crates/"
            f"{package['name']}/{package['version']}/download\n"
        )
    licenses = json.loads(
        subprocess.check_output(["pnpm", "licenses", "list", "--prod", "--json"], cwd=root)
    )
    for packages in licenses.values():
        for package in packages:
            for directory in package["paths"]:
                path = Path(directory)
                identity = json.loads((path / "package.json").read_text())
                text += section(
                    package["name"], identity["version"], package["license"], license_files(path)
                )
    return text


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--cargo-metadata", type=Path)
    args = parser.parse_args()
    text = (
        desktop_notices(Path(__file__).resolve().parents[1], args.cargo_metadata)
        if args.cargo_metadata
        else python_notices()
    )
    args.output.write_text(text, encoding="utf-8")
