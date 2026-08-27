#!/usr/bin/env python3
"""Create or update GitHub labels from .github/labels.yml."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[1]
LABELS_PATH = ROOT / ".github" / "labels.yml"


def load_labels() -> list[dict[str, str]]:
    with LABELS_PATH.open(encoding="utf-8") as source:
        document: Any = yaml.safe_load(source)
    labels = document.get("labels") if isinstance(document, dict) else None
    if not isinstance(labels, list):
        raise ValueError(".github/labels.yml must contain a labels list")

    parsed: list[dict[str, str]] = []
    for label in labels:
        if not isinstance(label, dict) or not all(
            isinstance(label.get(field), str) and label[field]
            for field in ("name", "color", "description")
        ):
            raise ValueError("each label needs non-empty name, color, and description")
        parsed.append({field: label[field] for field in ("name", "color", "description")})
    return parsed


def main() -> int:
    repository = os.environ.get("GITHUB_REPOSITORY")
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not repository:
        print("GITHUB_REPOSITORY must be set", file=sys.stderr)
        return 1
    if not token:
        print("GH_TOKEN or GITHUB_TOKEN must be set", file=sys.stderr)
        return 1

    try:
        labels = load_labels()
    except (OSError, ValueError, yaml.YAMLError) as error:
        print(f"Unable to load {LABELS_PATH.relative_to(ROOT)}: {error}", file=sys.stderr)
        return 1

    for label in labels:
        command = [
            "gh",
            "label",
            "create",
            label["name"],
            "--repo",
            repository,
            "--color",
            label["color"],
            "--description",
            label["description"],
            "--force",
        ]
        result = subprocess.run(command, check=False)
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
