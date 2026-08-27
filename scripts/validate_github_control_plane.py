#!/usr/bin/env python3
"""Validate the repository's GitHub labels and issue-form defaults."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[1]
LABELS_PATH = ROOT / ".github" / "labels.yml"
ISSUE_TEMPLATE_DIR = ROOT / ".github" / "ISSUE_TEMPLATE"
FORM_PATHS = (
    ISSUE_TEMPLATE_DIR / "bug_report.yml",
    ISSUE_TEMPLATE_DIR / "feature_request.yml",
    ISSUE_TEMPLATE_DIR / "chore.yml",
)
REQUIRED_LABELS = {
    *(f"status/{name}" for name in ("triage", "ready", "in-progress", "blocked", "review")),
    *(f"type/{name}" for name in ("feature", "bug", "chore", "docs", "security")),
    *(f"priority/{name}" for name in ("critical", "high", "medium", "low")),
    *(
        f"area/{name}"
        for name in (
            "factory-core",
            "project",
            "agent",
            "workflow",
            "evaluation",
            "mcp",
            "fs",
            "workspace",
        )
    ),
}
COLOR_PATTERN = re.compile(r"^[0-9A-Fa-f]{6}$")


def load_yaml(path: Path) -> Any:
    try:
        with path.open(encoding="utf-8") as source:
            return yaml.safe_load(source)
    except (OSError, yaml.YAMLError) as error:
        raise ValueError(f"{path.relative_to(ROOT)}: {error}") from error


def validate_labels() -> set[str]:
    document = load_yaml(LABELS_PATH)
    if not isinstance(document, dict) or not isinstance(document.get("labels"), list):
        raise ValueError(".github/labels.yml: expected a top-level labels list")

    names: list[str] = []
    for entry in document["labels"]:
        if not isinstance(entry, dict):
            raise ValueError(".github/labels.yml: each label must be a mapping")
        name = entry.get("name")
        color = entry.get("color")
        description = entry.get("description")
        if not isinstance(name, str) or not name:
            raise ValueError(".github/labels.yml: every label needs a non-empty name")
        if not isinstance(color, str) or not COLOR_PATTERN.fullmatch(color):
            raise ValueError(f".github/labels.yml: {name!r} has an invalid color")
        if not isinstance(description, str) or not description:
            raise ValueError(f".github/labels.yml: {name!r} needs a non-empty description")
        names.append(name)

    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise ValueError(f".github/labels.yml: duplicate labels: {', '.join(duplicates)}")

    missing = sorted(REQUIRED_LABELS.difference(names))
    if missing:
        raise ValueError(f".github/labels.yml: missing required labels: {', '.join(missing)}")
    return set(names)


def validate_form_defaults(label_names: set[str]) -> None:
    for path in FORM_PATHS:
        document = load_yaml(path)
        if not isinstance(document, dict):
            raise ValueError(f"{path.relative_to(ROOT)}: expected a mapping")
        defaults = document.get("labels")
        if not isinstance(defaults, list) or not defaults or not all(
            isinstance(label, str) for label in defaults
        ):
            raise ValueError(f"{path.relative_to(ROOT)}: expected non-empty default labels")
        unknown = sorted(set(defaults).difference(label_names))
        if unknown:
            raise ValueError(
                f"{path.relative_to(ROOT)}: unknown default labels: {', '.join(unknown)}"
            )


def main() -> int:
    try:
        label_names = validate_labels()
        validate_form_defaults(label_names)
    except ValueError as error:
        print(f"GitHub control-plane validation failed: {error}", file=sys.stderr)
        return 1
    print("GitHub control-plane validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
