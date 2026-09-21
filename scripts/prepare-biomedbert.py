"""Prepare a catalog model explicitly; document processing remains offline."""

import argparse
from pathlib import Path
from uuid import uuid4

from redactio_sidecar import model_manager
from redactio_sidecar.model_store import CatalogSource, ModelDescriptor

ROOT = Path(__file__).resolve().parents[1]


def prepare(root: Path, key: str) -> ModelDescriptor:
    source = CatalogSource.model_validate({"kind": "catalog", "key": key})
    descriptor = model_manager.preflight(source, root)
    return model_manager.install_model(
        root,
        descriptor,
        str(uuid4()),
        lambda job: print(f"{job.stage}: {job.downloaded_bytes}/{job.total_bytes}", flush=True),
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=("biomedbert", "hugginglil"), default="biomedbert")
    parser.add_argument("--model-dir", type=Path, default=ROOT / "apps/sidecar/models")
    args = parser.parse_args()
    descriptor = prepare(args.model_dir, args.model)
    print(f"{descriptor.name} is prepared for offline use.")
