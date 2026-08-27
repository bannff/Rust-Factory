#!/usr/bin/env python3
"""Validate Rust Factory's status-only capability core scaffolds."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ROOT_MANIFEST = ROOT / "Cargo.toml"
VISION_PATH = ROOT / ".kiro" / "steering" / "living-factory-vision.md"
REQUIRED_PATHS = (
    "src/lib.rs",
    "src/model.rs",
    "src/validation.rs",
    "src/error.rs",
    "src/port.rs",
    "src/service.rs",
    "tests/public_contract.rs",
)
ALLOWED_PACKAGE_PATHS = frozenset(("Cargo.toml", *REQUIRED_PATHS))
RESERVED_MODULES = frozenset({"model", "validation", "error", "port", "service"})
SCAFFOLDS = {
    "workspace-governance": "Workspace governance",
    "identity": "Identity / authentication",
    "model-gateway": "Model gateway",
    "memory": "Memory",
    "knowledge": "Knowledge",
    "tool-execution": "Tools / test execution",
    "sandbox": "Sandbox",
    "verification": "Verification",
    "message-bus": "Message bus / events",
    "cache": "Cache",
    "graph": "Graph / provenance",
    "observability": "Observability / audit",
    "notification": "Notification",
}
DEPENDENCY_TABLES = (
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
)
TARGET_CONFIGURATION_TABLES = (
    "bin",
    "lib",
    "example",
    "bench",
    "test",
    "features",
    "target",
)
PACKAGE_TARGET_CONFIGURATION_FIELDS = (
    "build",
    "autobins",
    "autoexamples",
    "autobenches",
    "autotests",
)
VALID_ROLES = {"core", "memory", "vendor", "mcp", "server", "mesh", "test-support"}
VALID_STATUSES = {
    "scaffolded",
    "specified",
    "implemented",
    "migration-pending",
    "deprecated",
}


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as source:
        document = tomllib.load(source)
    if not isinstance(document, dict):
        raise ValueError(f"{path.relative_to(ROOT)}: expected a TOML table")
    return document


def cargo_metadata() -> set[Path]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version=1", "--no-deps", "--offline"],
        cwd=ROOT,
        capture_output=True,
        check=False,
        text=True,
    )
    if result.returncode:
        raise ValueError(f"cargo metadata failed: {result.stderr.strip()}")
    document = json.loads(result.stdout)
    packages = document.get("packages")
    if not isinstance(packages, list):
        raise ValueError("cargo metadata returned no packages")
    return {
        Path(package["manifest_path"]).resolve()
        for package in packages
        if isinstance(package, dict) and isinstance(package.get("manifest_path"), str)
    }


def validate_vision() -> None:
    vision = VISION_PATH.read_text(encoding="utf-8")
    for family, label in SCAFFOLDS.items():
        expected = f"| {label} |"
        row = next((line for line in vision.splitlines() if line.startswith(expected)), None)
        if row is None:
            raise ValueError(f"{VISION_PATH.relative_to(ROOT)}: missing {label!r} registry row")
        if f"`{family}-core` status-only tree" not in row or "Scaffolded" not in row:
            raise ValueError(
                f"{VISION_PATH.relative_to(ROOT)}: {family!r} must be recorded as Scaffolded"
            )


def validate_known_metadata(metadata_manifests: set[Path]) -> None:
    for manifest_path in sorted(metadata_manifests):
        manifest = load_toml(manifest_path)
        package = manifest.get("package")
        metadata = package.get("metadata") if isinstance(package, dict) else None
        factory = metadata.get("rust-factory") if isinstance(metadata, dict) else None
        if factory is None:
            continue
        if not isinstance(factory, dict):
            raise ValueError(f"{manifest_path.relative_to(ROOT)}: invalid rust-factory metadata")
        role = factory.get("role")
        status = factory.get("status")
        if role not in VALID_ROLES or status not in VALID_STATUSES:
            raise ValueError(f"{manifest_path.relative_to(ROOT)}: unknown role or status")
        if status == "scaffolded":
            family = factory.get("family")
            if family not in SCAFFOLDS:
                raise ValueError(f"{manifest_path.relative_to(ROOT)}: unknown scaffolded family")
            expected_manifest = ROOT / "crates" / f"{family}-core" / "Cargo.toml"
            if manifest_path.resolve() != expected_manifest.resolve():
                raise ValueError(
                    f"{manifest_path.relative_to(ROOT)}: scaffolded packages must use the canonical family path"
                )


def validate_package_tree(package_dir: Path) -> None:
    actual_paths = {
        path.relative_to(package_dir).as_posix()
        for path in package_dir.rglob("*")
    }
    allowed_paths = ALLOWED_PACKAGE_PATHS | {"src", "tests"}
    unexpected = sorted(actual_paths.difference(allowed_paths))
    missing = sorted(ALLOWED_PACKAGE_PATHS.difference(actual_paths))
    if missing:
        raise ValueError(f"{package_dir.relative_to(ROOT)}: missing {', '.join(missing)}")
    if unexpected:
        raise ValueError(
            f"{package_dir.relative_to(ROOT)}: unexpected package content: {', '.join(unexpected)}"
        )


def skip_comment(source: str, position: int) -> int | None:
    if source.startswith("//", position):
        newline = source.find("\n", position + 2)
        return len(source) if newline == -1 else newline + 1
    if not source.startswith("/*", position):
        return None

    depth = 1
    position += 2
    while depth:
        opening = source.find("/*", position)
        closing = source.find("*/", position)
        if closing == -1:
            return None
        if opening != -1 and opening < closing:
            depth += 1
            position = opening + 2
        else:
            depth -= 1
            position = closing + 2
    return position


def skip_whitespace_and_comments(source: str, position: int) -> int | None:
    while position < len(source):
        if source[position].isspace():
            position += 1
            continue
        next_position = skip_comment(source, position)
        if next_position is None:
            return position
        position = next_position
    return position


def contains_only_comments_and_whitespace(source: str) -> bool:
    return skip_whitespace_and_comments(source, 0) == len(source)


def is_valid_lib_source(source: str) -> bool:
    position = 0
    declared_modules: set[str] = set()
    while True:
        position = skip_whitespace_and_comments(source, position)
        if position is None:
            return False
        if position == len(source):
            return declared_modules == RESERVED_MODULES
        declaration = re.match(r"mod\s+(error|model|port|service|validation)\s*;", source[position:])
        if declaration is None:
            return False
        module = declaration.group(1)
        if module in declared_modules:
            return False
        declared_modules.add(module)
        position += declaration.end()


def validate_source_content(package_dir: Path) -> None:
    lib_path = package_dir / "src/lib.rs"
    if not is_valid_lib_source(lib_path.read_text(encoding="utf-8")):
        raise ValueError(
            f"{lib_path.relative_to(ROOT)}: must contain only comments and private reserved module declarations"
        )
    for relative_path in REQUIRED_PATHS:
        if relative_path == "src/lib.rs":
            continue
        path = package_dir / relative_path
        if not contains_only_comments_and_whitespace(path.read_text(encoding="utf-8")):
            raise ValueError(
                f"{path.relative_to(ROOT)}: status-only source files may contain only comments and whitespace"
            )


def validate_manifest_configuration(manifest_path: Path, manifest: dict[str, Any]) -> None:
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: missing [package]")
    configured_fields = [
        field for field in PACKAGE_TARGET_CONFIGURATION_FIELDS if field in package
    ]
    if configured_fields:
        raise ValueError(
            f"{manifest_path.relative_to(ROOT)}: scaffolded packages may not configure "
            f"package targets: {', '.join(configured_fields)}"
        )
    configured_tables = [
        table for table in TARGET_CONFIGURATION_TABLES if table in manifest
    ]
    if configured_tables:
        raise ValueError(
            f"{manifest_path.relative_to(ROOT)}: scaffolded packages may not configure "
            f"Cargo targets or features: {', '.join(configured_tables)}"
        )
    dependency_tables = [table for table in DEPENDENCY_TABLES if table in manifest]
    if dependency_tables:
        raise ValueError(
            f"{manifest_path.relative_to(ROOT)}: scaffolded packages may not declare "
            f"dependency tables: {', '.join(dependency_tables)}"
        )
    if manifest.get("lints") != {"workspace": True}:
        raise ValueError(
            f"{manifest_path.relative_to(ROOT)}: must contain exactly "
            "[lints] workspace = true"
        )


def validate_manifest(family: str, metadata_manifests: set[Path]) -> None:
    package_dir = ROOT / "crates" / f"{family}-core"
    manifest_path = package_dir / "Cargo.toml"
    validate_package_tree(package_dir)
    manifest = load_toml(manifest_path)
    validate_manifest_configuration(manifest_path, manifest)
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: missing [package]")
    if package.get("name") != f"{family}-core":
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: package name must match family")
    metadata = package.get("metadata")
    factory = metadata.get("rust-factory") if isinstance(metadata, dict) else None
    if not isinstance(factory, dict):
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: missing rust-factory metadata")
    role = factory.get("role")
    status = factory.get("status")
    if role not in VALID_ROLES or status not in VALID_STATUSES:
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: unknown role or status")
    if factory != {"family": family, "role": "core", "status": "scaffolded"}:
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: invalid scaffold metadata")
    for field in ("version", "edition", "license", "rust-version"):
        if package.get(field) != {"workspace": True}:
            raise ValueError(f"{manifest_path.relative_to(ROOT)}: {field} must use workspace value")
    if manifest_path.resolve() not in metadata_manifests:
        raise ValueError(f"{manifest_path.relative_to(ROOT)}: absent from cargo metadata")
    validate_source_content(package_dir)


def validate_workspace_members() -> None:
    workspace = load_toml(ROOT_MANIFEST).get("workspace")
    members = workspace.get("members") if isinstance(workspace, dict) else None
    if not isinstance(members, list):
        raise ValueError("Cargo.toml: workspace members must be a list")
    expected = {f"crates/{family}-core" for family in SCAFFOLDS}
    missing = sorted(expected.difference(members))
    if missing:
        raise ValueError(f"Cargo.toml: missing workspace members: {', '.join(missing)}")


def main() -> int:
    try:
        validate_workspace_members()
        metadata_manifests = cargo_metadata()
        validate_known_metadata(metadata_manifests)
        for family in SCAFFOLDS:
            validate_manifest(family, metadata_manifests)
        validate_vision()
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"Status-only scaffold validation failed: {error}", file=sys.stderr)
        return 1
    print("Status-only scaffold validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
