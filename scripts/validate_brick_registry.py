#!/usr/bin/env python3
"""Deterministic Rust Factory brick registry validator.

Runs without third-party dependencies or network access and enforces that:

* every workspace package declares a valid ``[package.metadata.rust-factory]``
  record with a known family, role, and status;
* the workspace member list, the package directories on disk, and Cargo
  metadata agree exactly, so no package can be orphaned or unlisted;
* packages declaring ``status = "scaffolded"`` remain genuinely status-only;
* libraries under ``crates/`` own no binary target, and every ``role =
  "server"`` package is a binary under ``projects/``;
* the Living Factory Vision registry and the declared families agree in *both*
  directions, so neither the table nor the manifests can drift.
"""

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
MAKEFILE_PATH = ROOT / "Makefile"
VISION_PATH = ROOT / ".kiro" / "steering" / "living-factory-vision.md"

LIBRARY_DIR = "crates"
BINARY_DIR = "projects"

# Families that own at least one workspace package, mapped to the Vision
# registry row label that records them.
ACTIVE_FAMILIES = {
    "project": "Project authoring",
    "policy": "Policy / authorization",
    "agent": "Agent",
    "workflow": "Workflow",
    "evaluation": "Evaluation",
    "model-gateway": "Model gateway",
    "memory": "Memory",
    "sandbox": "Sandbox",
    "observability": "Observability / audit",
    "mcp-transport": "MCP transport",
}

# Capability families committed in the registry with a named future crate but
# deliberately without a package until a demonstrated consumer drives them.
# A registry row is the parking space; an empty crate is not.
DEFERRED_FAMILIES = {
    "workspace-governance": "Workspace governance",
    "identity": "Identity / authentication",
    "knowledge": "Knowledge",
    "verification": "Verification",
    "message-bus": "Message bus / events",
    "cache": "Cache",
    "graph": "Graph / provenance",
    "notification": "Notification",
}

# Active families whose only package is still a status-only core tree.
STATUS_ONLY_FAMILIES = frozenset({"model-gateway", "memory", "sandbox", "observability"})

# Registry rows carrying this taxonomy describe a capability family. Every
# other taxonomy is adapter infrastructure, a composition base, or an optional
# domain pack, none of which own a capability core. The set is closed so a
# mistyped taxonomy cannot silently opt a row out of the reverse check.
CAPABILITY_TAXONOMY = "Capability"
VALID_TAXONOMIES = frozenset(
    {
        CAPABILITY_TAXONOMY,
        "Adapter infrastructure",
        "Composition bases",
        "Optional capability/domain pack",
        "Optional domain packs",
    }
)

# The registry table is the one immediately following this heading. Scoping the
# parser keeps an unrelated table elsewhere in the document out of the registry
# namespace.
REGISTRY_HEADING = "## Brick portfolio registry"

# Recorded-state keywords. A state cell is required to *begin* with its keyword
# so prose later in the cell cannot satisfy the check by accident.
STATE_SCAFFOLDED = "Scaffolded"
STATE_DEFERRED = "Deferred"

# Defensive ceiling on any single file this validator reads.
MAX_READ_BYTES = 1 << 20

VALID_ROLES = {
    # One crate per brick: a capability plus its opt-in, feature-gated adapters.
    "brick",
    # A status-only family that has no behavior yet.
    "core",
    # Shared cross-family infrastructure that owns no capability.
    "infrastructure",
    # Roles reserved for packages a brick cannot contain: a deployable binary, a
    # peer-coordination adapter, and shared test fixtures.
    "server",
    "mesh",
    "test-support",
}

# An adapter dependency may only be named under its own feature-gated module.
# Consolidating each brick into one crate removed the crate boundary that used
# to make "the core never touches rmcp" mechanically true, so this path rule
# replaces it. serde and serde_json are absent deliberately: workflow uses
# serde_json in its core for canonical JSON.
ADAPTER_MODULES = {
    "rmcp": "mcp",
    "mcp_transport": "mcp",
    "schemars": "mcp",
    "anyhow": "mcp",
    "cap_std": "fs",
}

# Every module name a brick may feature gate. This is a superset of
# ADAPTER_MODULES' values because an adapter need not pull a dependency at all:
# a `memory` adapter is deterministic and uses only the standard library.
ADAPTER_MODULE_NAMES = frozenset({"mcp", "memory", "fs"})
VALID_STATUSES = {
    "scaffolded",
    "specified",
    "implemented",
    "migration-pending",
    "deprecated",
}
REQUIRED_METADATA_FIELDS = ("family", "role", "status")

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
INHERITED_PACKAGE_FIELDS = ("version", "edition", "license", "rust-version")

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

BINARY_TARGET_KINDS = frozenset({"bin"})


def load_toml(path: Path) -> dict[str, Any]:
    if path.stat().st_size > MAX_READ_BYTES:
        raise ValueError(f"{relative(path)}: manifest exceeds {MAX_READ_BYTES} bytes")
    with path.open("rb") as source:
        document = tomllib.load(source)
    if not isinstance(document, dict):
        raise ValueError(f"{relative(path)}: expected a TOML table")
    return document


def read_bounded(path: Path) -> str:
    if path.stat().st_size > MAX_READ_BYTES:
        raise ValueError(f"{relative(path)}: file exceeds {MAX_READ_BYTES} bytes")
    return path.read_text(encoding="utf-8")


def repo_relative(path: Path) -> Path | None:
    """Returns ``path`` relative to the repository root, or None if outside it.

    Both operands are resolved before comparison so the answer cannot be
    influenced by a symlink: an in-tree symlink pointing outside the repository
    is correctly reported as external, and a symlinked root (macOS ``/var`` to
    ``/private/var``, for example) does not make an in-tree path look external.
    """
    try:
        return path.resolve().relative_to(ROOT.resolve())
    except ValueError:
        return None


def relative(path: Path) -> str:
    within = repo_relative(path)
    return within.as_posix() if within is not None else path.as_posix()


def cargo_packages() -> list[dict[str, Any]]:
    """Returns every workspace package as reported by Cargo."""
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
    for package in packages:
        if not isinstance(package, dict) or not isinstance(
            package.get("manifest_path"), str
        ):
            raise ValueError("cargo metadata returned a malformed package")
    return packages


def factory_metadata(package: dict[str, Any], manifest_path: Path) -> dict[str, Any]:
    """Extracts and shape-checks one package's Rust Factory metadata record."""
    metadata = package.get("metadata")
    factory = metadata.get("rust-factory") if isinstance(metadata, dict) else None
    if factory is None:
        raise ValueError(
            f"{relative(manifest_path)}: missing [package.metadata.rust-factory]; "
            "every workspace package must declare family, role, and status"
        )
    if not isinstance(factory, dict):
        raise ValueError(f"{relative(manifest_path)}: invalid rust-factory metadata")
    unknown = sorted(set(factory).difference(REQUIRED_METADATA_FIELDS))
    if unknown:
        raise ValueError(
            f"{relative(manifest_path)}: unknown rust-factory metadata fields: "
            f"{', '.join(unknown)}"
        )
    missing = [field for field in REQUIRED_METADATA_FIELDS if field not in factory]
    if missing:
        raise ValueError(
            f"{relative(manifest_path)}: missing rust-factory metadata: "
            f"{', '.join(missing)}"
        )
    family = factory["family"]
    role = factory["role"]
    status = factory["status"]
    if family not in ACTIVE_FAMILIES:
        raise ValueError(
            f"{relative(manifest_path)}: family {family!r} is not an active registry "
            "family; add a Vision registry row and declare it before shipping a package"
        )
    if role not in VALID_ROLES:
        raise ValueError(f"{relative(manifest_path)}: unknown role {role!r}")
    if status not in VALID_STATUSES:
        raise ValueError(f"{relative(manifest_path)}: unknown status {status!r}")
    if role == "core" and status != "scaffolded":
        # The brick enforcement rules — adapter isolation, conditional derives,
        # and quality-gate coverage — key on role == "brick". Allowing a package
        # with behavior to declare role == "core" would let it exempt itself from
        # all three while make check stayed green.
        raise ValueError(
            f"{relative(manifest_path)}: role = \"core\" is reserved for a status-only "
            f"package, but this declares status = {status!r}. A package with behavior "
            'uses role = "brick" and is subject to the adapter isolation rules'
        )
    return factory


def package_directory(manifest_path: Path) -> Path:
    return manifest_path.parent


def validate_location_and_targets(
    package: dict[str, Any], manifest_path: Path, role: str
) -> None:
    """Libraries live under crates/; only server binaries live under projects/."""
    directory = package_directory(manifest_path)
    within = repo_relative(directory)
    if within is None or not within.parts:
        raise ValueError(
            f"{relative(manifest_path)}: package lies outside the repository"
        )
    area = within.parts[0]
    if area not in (LIBRARY_DIR, BINARY_DIR):
        raise ValueError(
            f"{relative(manifest_path)}: packages live under {LIBRARY_DIR}/ or "
            f"{BINARY_DIR}/, not {area}/"
        )
    targets = package.get("targets")
    if not isinstance(targets, list):
        raise ValueError(f"{relative(manifest_path)}: cargo metadata returned no targets")
    has_binary = any(
        isinstance(target, dict)
        and not BINARY_TARGET_KINDS.isdisjoint(target.get("kind") or ())
        for target in targets
    )
    if area == LIBRARY_DIR and has_binary:
        raise ValueError(
            f"{relative(manifest_path)}: packages under {LIBRARY_DIR}/ are libraries "
            f"and must declare no binary target; move the binary to {BINARY_DIR}/"
        )
    if (role == "server") != (area == BINARY_DIR):
        raise ValueError(
            f"{relative(manifest_path)}: role = \"server\" and residence under "
            f"{BINARY_DIR}/ must agree"
        )
    if area == BINARY_DIR and not has_binary:
        raise ValueError(
            f"{relative(manifest_path)}: packages under {BINARY_DIR}/ must declare a "
            "binary target"
        )


def validate_package_name(
    package: dict[str, Any], manifest_path: Path, family: str, role: str
) -> None:
    """The declared crate name must match its directory, and a library its family."""
    name = package.get("name")
    if not isinstance(name, str) or not name:
        raise ValueError(f"{relative(manifest_path)}: missing package name")
    directory = package_directory(manifest_path).name
    if name != directory:
        raise ValueError(
            f"{relative(manifest_path)}: package name {name!r} must match its "
            f"directory {directory!r}"
        )
    if role == "server":
        # A composition root owns no capability and may host several brick MCP
        # surfaces, so requiring it to carry one family's prefix would force it
        # to misrepresent itself. Its residence under projects/ is the invariant
        # that matters, and that is checked separately.
        return
    if name != family and not name.startswith(f"{family}-"):
        raise ValueError(
            f"{relative(manifest_path)}: package name {name!r} must be {family!r} or "
            f"start with {family + '-'!r} to match its declared family"
        )


def source_files(package_dir: Path) -> list[Path]:
    return sorted(path for path in package_dir.rglob("*.rs") if path.is_file())


def module_of(source: Path, package_dir: Path) -> str | None:
    """Returns the top-level module a source file belongs to.

    ``src/mcp.rs`` and ``src/mcp/service.rs`` both yield ``mcp``; ``src/lib.rs``
    yields the empty string. Returns None for a target outside ``src/`` — an
    integration test, bench, or build script — which the module rules do not
    govern because such a target is separately feature-gated at file scope.
    """
    parts = source.relative_to(package_dir).parts
    if not parts or parts[0] != "src":
        return None
    if len(parts) < 2:
        return None
    if len(parts) == 2:
        return "" if parts[1] == "lib.rs" else parts[1].removesuffix(".rs")
    return parts[1]


def adapter_references(text: str, crate: str) -> bool:
    """Detects a reference to an adapter crate, including under an alias.

    A path rule that matched only ``crate::`` would be evaded by
    ``use rmcp as framework;`` followed by ``framework::Type``, so the import
    itself is matched too.
    """
    escaped = re.escape(crate)
    return bool(
        re.search(rf"\b{escaped}\s*::", text)
        or re.search(rf"^\s*(?:pub\s+)?use\s+(?:::)?{escaped}\b", text, re.MULTILINE)
        or re.search(rf"\bextern\s+crate\s+{escaped}\b", text)
    )


def validate_adapter_isolation(package_dir: Path) -> None:
    """Keeps adapter dependencies inside their own feature-gated module.

    Also keeps core modules from reaching *into* an adapter module, since
    consuming a boundary DTO from the core is the same violation approached from
    the other side.
    """
    adapter_modules = ADAPTER_MODULE_NAMES
    for source in source_files(package_dir):
        module = module_of(source, package_dir)
        if module is None:
            continue
        text = read_bounded(source)
        for crate, required_module in ADAPTER_MODULES.items():
            if adapter_references(text, crate) and module != required_module:
                raise ValueError(
                    f"{relative(source)}: names the adapter dependency {crate!r}, "
                    f"which may appear only under the {required_module!r} module. "
                    "Adapter code belongs behind its own feature gate so the default "
                    "build stays framework-free"
                )
        if module in adapter_modules:
            continue
        for adapter_module in sorted(adapter_modules):
            if re.search(rf"\bcrate\s*::\s*{re.escape(adapter_module)}\s*::", text):
                raise ValueError(
                    f"{relative(source)}: reaches into the {adapter_module!r} adapter "
                    "module. A core module must not consume a boundary type; the "
                    "conversion belongs inside the adapter"
                )


def validate_feature_table(manifest_path: Path, manifest: dict[str, Any]) -> None:
    """No feature may be on by default, or the opt-in guarantee is not real."""
    features = manifest.get("features")
    if features is None:
        return
    if not isinstance(features, dict):
        raise ValueError(f"{relative(manifest_path)}: invalid [features] table")
    if "default" in features:
        raise ValueError(
            f"{relative(manifest_path)}: declares a `default` feature. Adapters are "
            "opt-in; a default feature puts framework dependencies back into every "
            "build"
        )


def attribute_bodies(text: str, name: str) -> list[str]:
    """Returns the argument text of each ``#[name(...)]`` attribute.

    Brackets are balanced rather than matched by regex so a nested predicate such
    as ``cfg_attr(all(feature = "mcp"), derive(Serialize))`` is captured whole.
    """
    bodies: list[str] = []
    for match in re.finditer(rf"#\s*!?\s*\[\s*{re.escape(name)}\s*\(", text):
        depth = 1
        index = match.end()
        while index < len(text) and depth:
            if text[index] == "(":
                depth += 1
            elif text[index] == ")":
                depth -= 1
            index += 1
        bodies.append(text[match.end() : index - 1])
    return bodies


def validate_conditional_derives(package_dir: Path) -> None:
    """Outside an adapter module, a feature-conditional item or attribute would
    give the crate two different public APIs depending on who else is in the
    build graph. The adapter `mod` declaration itself is the sole exception."""
    adapter_modules = ADAPTER_MODULE_NAMES
    for source in source_files(package_dir):
        module = module_of(source, package_dir)
        if module is None or module in adapter_modules:
            continue
        text = read_bounded(source)
        for body in attribute_bodies(text, "cfg_attr"):
            if re.search(r"\bfeature\s*=", body):
                raise ValueError(
                    f"{relative(source)}: uses a feature-conditional attribute outside "
                    "an adapter module. Boundary types belong in the adapter module "
                    "rather than deriving onto a domain type under cfg_attr"
                )
        # A feature-gated item in a core module is the same hazard by another
        # route. Only the adapter module declarations may be feature gated.
        for body in attribute_bodies(text, "cfg"):
            if not re.search(r"\bfeature\s*=", body):
                continue
            gated = [name for name in adapter_modules if f'"{name}"' in body]
            if len(gated) != 1 or not re.search(
                rf"#\s*\[\s*cfg\s*\([^)]*\bfeature\s*=\s*\"{gated[0]}\"[^)]*\)\s*\]\s*"
                rf"(?:pub\s+)?mod\s+{gated[0]}\s*;",
                text,
            ):
                raise ValueError(
                    f"{relative(source)}: feature-gates an item outside an adapter "
                    "module. Only the adapter `mod` declaration may be feature gated, "
                    "or the crate has two different public APIs depending on which "
                    "features another package in the build graph enables"
                )


def validate_status_only_placement(manifest_path: Path, family: str, role: str) -> None:
    if family not in STATUS_ONLY_FAMILIES:
        raise ValueError(
            f"{relative(manifest_path)}: family {family!r} is not recorded as a "
            "status-only family; give it a real contract or change its status"
        )
    if role != "core":
        raise ValueError(
            f"{relative(manifest_path)}: status-only packages must use role = \"core\""
        )
    expected = ROOT / LIBRARY_DIR / f"{family}" / "Cargo.toml"
    if repo_relative(manifest_path) != repo_relative(expected):
        raise ValueError(
            f"{relative(manifest_path)}: status-only packages must use the canonical "
            f"path {relative(expected)}"
        )


def validate_manifest_configuration(manifest_path: Path, manifest: dict[str, Any]) -> None:
    package = manifest.get("package")
    if not isinstance(package, dict):
        raise ValueError(f"{relative(manifest_path)}: missing [package]")
    target_fields = sorted(
        field for field in PACKAGE_TARGET_CONFIGURATION_FIELDS if field in package
    )
    if target_fields:
        raise ValueError(
            f"{relative(manifest_path)}: must not configure package target fields: "
            f"{', '.join(target_fields)}"
        )
    target_tables = sorted(
        table for table in TARGET_CONFIGURATION_TABLES if table in manifest
    )
    if target_tables:
        raise ValueError(
            f"{relative(manifest_path)}: must not configure Cargo target or feature "
            f"tables: {', '.join(target_tables)}"
        )
    dependency_tables = sorted(
        table for table in DEPENDENCY_TABLES if table in manifest
    )
    if dependency_tables:
        raise ValueError(
            f"{relative(manifest_path)}: must not declare dependency tables: "
            f"{', '.join(dependency_tables)}"
        )
    if manifest.get("lints") != {"workspace": True}:
        raise ValueError(
            f"{relative(manifest_path)}: must contain exactly [lints] workspace = true"
        )
    for field in INHERITED_PACKAGE_FIELDS:
        if package.get(field) != {"workspace": True}:
            raise ValueError(
                f"{relative(manifest_path)}: {field} must use the workspace value"
            )


def validate_package_tree(package_dir: Path) -> None:
    actual_paths = {
        path.relative_to(package_dir).as_posix() for path in package_dir.rglob("*")
    }
    allowed_paths = ALLOWED_PACKAGE_PATHS | {"src", "tests"}
    unexpected = sorted(actual_paths.difference(allowed_paths))
    missing = sorted(ALLOWED_PACKAGE_PATHS.difference(actual_paths))
    if missing:
        raise ValueError(f"{relative(package_dir)}: missing {', '.join(missing)}")
    if unexpected:
        raise ValueError(
            f"{relative(package_dir)}: unexpected package content: "
            f"{', '.join(unexpected)}"
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
        if not source.startswith("mod ", position):
            return False
        position += 4
        terminator = source.find(";", position)
        if terminator == -1:
            return False
        name = source[position:terminator].strip()
        if name not in RESERVED_MODULES or name in declared_modules:
            return False
        declared_modules.add(name)
        position = terminator + 1


def validate_source_content(package_dir: Path) -> None:
    for relative_path in REQUIRED_PATHS:
        path = package_dir / relative_path
        source = read_bounded(path)
        if relative_path == "src/lib.rs":
            if not is_valid_lib_source(source):
                raise ValueError(
                    f"{relative(path)}: status-only lib.rs may contain only "
                    "documentation/comments and one private mod declaration per "
                    "reserved module"
                )
            continue
        if not contains_only_comments_and_whitespace(source):
            raise ValueError(
                f"{relative(path)}: status-only sources may contain only comments"
            )


def validate_quality_gate_coverage(brick_features: dict[str, set[str]]) -> None:
    """Every brick and every declared feature must appear in the quality gate.

    Adapters are feature gated, so a workspace-wide command compiles only the
    cores. That makes the Makefile's enumerations load-bearing: a brick or
    feature missing from them is code that is never linted, never tested, and
    never isolation-checked, with a green `make check`. This is the same reason
    the registry keeps an independent second statement of the family set.
    """
    makefile = MAKEFILE_PATH
    if not makefile.is_file():
        raise ValueError(f"{relative(makefile)}: missing quality gate")
    text = read_bounded(makefile)
    listed = re.search(r"^BRICKS[ \t]*:?=[ \t]*(.*)$", text, re.MULTILINE)
    if listed is None:
        raise ValueError(f"{relative(makefile)}: no BRICKS list to check bricks against")
    enumerated = set(listed.group(1).split())
    declared = set(brick_features)
    missing = sorted(declared.difference(enumerated))
    if missing:
        raise ValueError(
            f"{relative(makefile)}: BRICKS omits {', '.join(missing)}; an omitted "
            "brick is never isolation-checked"
        )
    stale = sorted(enumerated.difference(declared))
    if stale:
        raise ValueError(
            f"{relative(makefile)}: BRICKS lists {', '.join(stale)}, which declares "
            "no brick package"
        )
    for target, verb in (("lint-features", "linted"), ("test-features", "tested")):
        body = re.search(rf"^{target}:\n((?:\t.*\n)+)", text, re.MULTILINE)
        if body is None:
            raise ValueError(f"{relative(makefile)}: missing the {target} target")
        recipe = body.group(1)
        for brick, features in sorted(brick_features.items()):
            for feature in sorted(features):
                covered = any(
                    f"-p {brick} " in line and feature in _feature_list(line)
                    for line in recipe.splitlines()
                )
                if not covered:
                    raise ValueError(
                        f"{relative(makefile)}: {target} never builds {brick!r} with "
                        f"its {feature!r} feature, so that code is never {verb}"
                    )


def _feature_list(line: str) -> set[str]:
    match = re.search(r"--features\s+([\w,\-]+)", line)
    return set(match.group(1).split(",")) if match else set()


def declared_members() -> list[str]:
    workspace = load_toml(ROOT_MANIFEST).get("workspace")
    members = workspace.get("members") if isinstance(workspace, dict) else None
    if not isinstance(members, list) or not all(
        isinstance(member, str) for member in members
    ):
        raise ValueError("Cargo.toml: workspace members must be a list of strings")
    return members


def discovered_package_directories() -> set[str]:
    """Finds every directory under crates/ or projects/ holding a manifest.

    The walk is recursive so a manifest nested below a package directory cannot
    hide from the member reconciliation, and every immediate child is required
    to be a package so an empty leftover directory is reported rather than
    ignored.
    """
    found: set[str] = set()
    for area in (LIBRARY_DIR, BINARY_DIR):
        area_path = ROOT / area
        if not area_path.is_dir():
            continue
        for manifest in sorted(area_path.rglob("Cargo.toml")):
            if not manifest.is_file():
                continue
            within = repo_relative(manifest.parent)
            if within is None:
                raise ValueError(
                    f"{relative(manifest)}: manifest resolves outside the repository"
                )
            found.add(within.as_posix())
        for entry in sorted(area_path.iterdir()):
            if entry.is_dir() and not (entry / "Cargo.toml").is_file():
                raise ValueError(
                    f"{area}/{entry.name}: directory contains no Cargo.toml; remove it "
                    "or give it a manifest and register it as a workspace member"
                )
    return found


def validate_workspace_inventory(packages: list[dict[str, Any]]) -> None:
    members = declared_members()
    duplicates = sorted({member for member in members if members.count(member) > 1})
    if duplicates:
        raise ValueError(
            f"Cargo.toml: duplicate workspace members: {', '.join(duplicates)}"
        )
    declared = set(members)
    on_disk = discovered_package_directories()
    unlisted = sorted(on_disk.difference(declared))
    if unlisted:
        raise ValueError(
            f"Cargo.toml: package directories absent from workspace members: "
            f"{', '.join(unlisted)}"
        )
    phantom = sorted(declared.difference(on_disk))
    if phantom:
        raise ValueError(
            f"Cargo.toml: workspace members without a package directory: "
            f"{', '.join(phantom)}"
        )
    resolved = {
        relative(package_directory(Path(package["manifest_path"]).resolve()))
        for package in packages
    }
    missing_from_metadata = sorted(declared.difference(resolved))
    if missing_from_metadata:
        raise ValueError(
            f"Cargo.toml: members absent from cargo metadata: "
            f"{', '.join(missing_from_metadata)}"
        )
    expected_status_only = {
        f"{LIBRARY_DIR}/{family}" for family in STATUS_ONLY_FAMILIES
    }
    missing_status_only = sorted(expected_status_only.difference(declared))
    if missing_status_only:
        raise ValueError(
            f"Cargo.toml: missing status-only members: "
            f"{', '.join(missing_status_only)}"
        )


def registry_table_lines() -> list[str]:
    """Returns the pipe-delimited lines of the registry table only.

    Scanning starts at the registry heading and stops at the next heading, so a
    table elsewhere in the document cannot join the registry namespace.
    """
    lines = read_bounded(VISION_PATH).splitlines()
    try:
        start = next(
            index
            for index, line in enumerate(lines)
            if line.strip() == REGISTRY_HEADING
        )
    except StopIteration as error:
        raise ValueError(
            f"{relative(VISION_PATH)}: missing {REGISTRY_HEADING!r} heading"
        ) from error
    table: list[str] = []
    for line in lines[start + 1 :]:
        stripped = line.strip()
        if stripped.startswith("#"):
            break
        if stripped.startswith("|"):
            table.append(stripped)
        elif table:
            # The table ended; ignore trailing prose in the same section.
            break
    if not table:
        raise ValueError(f"{relative(VISION_PATH)}: registry table not found")
    return table


def registry_rows() -> dict[str, dict[str, str]]:
    """Parses the Vision registry table into ``label -> row`` records."""
    rows: dict[str, dict[str, str]] = {}
    for stripped in registry_table_lines():
        cells = [cell.strip() for cell in stripped.split("|")[1:-1]]
        label = cells[0].strip() if cells else ""
        if label == "Family" or (label and set(label) <= {"-", ":"}):
            continue
        if len(cells) != 5:
            raise ValueError(
                f"{relative(VISION_PATH)}: registry row {label!r} has {len(cells)} "
                "cells; expected 5. A cell may not contain a pipe character"
            )
        label, taxonomy, owner, _mature, state = cells
        if not label:
            raise ValueError(f"{relative(VISION_PATH)}: registry row without a family")
        if taxonomy not in VALID_TAXONOMIES:
            raise ValueError(
                f"{relative(VISION_PATH)}: registry row {label!r} has unknown taxonomy "
                f"{taxonomy!r}"
            )
        if label in rows:
            raise ValueError(
                f"{relative(VISION_PATH)}: duplicate registry row {label!r}"
            )
        rows[label] = {"taxonomy": taxonomy, "owner": owner, "state": state}
    if not rows:
        raise ValueError(f"{relative(VISION_PATH)}: no registry rows found")
    return rows


def validate_declared_families() -> dict[str, str]:
    """Checks the declared family sets are disjoint and uniquely labelled."""
    overlap = sorted(set(ACTIVE_FAMILIES).intersection(DEFERRED_FAMILIES))
    if overlap:
        raise ValueError(
            f"a family cannot be both active and deferred: {', '.join(overlap)}"
        )
    known = {**ACTIVE_FAMILIES, **DEFERRED_FAMILIES}
    labels: dict[str, str] = {}
    for family, label in known.items():
        if label in labels:
            raise ValueError(
                f"families {labels[label]!r} and {family!r} share the registry label "
                f"{label!r}; labels must be unique"
            )
        labels[label] = family
    undeclared = sorted(STATUS_ONLY_FAMILIES.difference(ACTIVE_FAMILIES))
    if undeclared:
        raise ValueError(
            f"status-only families must also be active: {', '.join(undeclared)}"
        )
    return known


def validate_registry(
    declared_statuses: dict[str, set[str]], owned_names: dict[str, set[str]]
) -> None:
    known = validate_declared_families()
    rows = registry_rows()

    # Forward: every declared family is recorded in the registry, its recorded
    # state agrees with what its packages declare, and its owner cell names a
    # crate that actually exists.
    for family, label in known.items():
        row = rows.get(label)
        if row is None:
            raise ValueError(
                f"{relative(VISION_PATH)}: missing registry row {label!r} for family "
                f"{family!r}"
            )
        if family in DEFERRED_FAMILIES:
            if f"`{family}`" not in row["owner"]:
                raise ValueError(
                    f"{relative(VISION_PATH)}: deferred family {family!r} must name "
                    f"its future owning crate `{family}`"
                )
            if not row["state"].startswith(STATE_DEFERRED):
                raise ValueError(
                    f"{relative(VISION_PATH)}: deferred family {family!r} must be "
                    f"recorded as {STATE_DEFERRED}"
                )
            continue

        owned = owned_names.get(family, set())
        if not any(f"`{name}`" in row["owner"] for name in owned):
            raise ValueError(
                f"{relative(VISION_PATH)}: family {family!r} must name one of its "
                f"packages ({', '.join(sorted(owned)) or 'none'}) in its owning-crate "
                "cell"
            )
        statuses = declared_statuses.get(family, set())
        if family in STATUS_ONLY_FAMILIES:
            if statuses != {"scaffolded"}:
                raise ValueError(
                    f"family {family!r} is recorded as status-only but its packages "
                    f"declare {sorted(statuses)}; a status-only family declares only "
                    "'scaffolded'"
                )
            if not row["state"].startswith(STATE_SCAFFOLDED):
                raise ValueError(
                    f"{relative(VISION_PATH)}: status-only family {family!r} must be "
                    f"recorded as {STATE_SCAFFOLDED}"
                )
        else:
            if "scaffolded" in statuses:
                raise ValueError(
                    f"family {family!r} declares a scaffolded package but is not "
                    "recorded as a status-only family"
                )
            if row["state"].startswith(STATE_SCAFFOLDED):
                raise ValueError(
                    f"{relative(VISION_PATH)}: family {family!r} is recorded as "
                    f"{STATE_SCAFFOLDED} but declares {sorted(statuses)}"
                )

    # Reverse: every capability row in the registry is a declared family.
    labels = set(known.values())
    for label, row in rows.items():
        if row["taxonomy"] != CAPABILITY_TAXONOMY:
            continue
        if label not in labels:
            raise ValueError(
                f"{relative(VISION_PATH)}: capability row {label!r} matches no declared "
                "family; declare it or change its taxonomy"
            )

    # Deferred families own no package and no directory.
    for family in DEFERRED_FAMILIES:
        if family in declared_statuses:
            raise ValueError(
                f"family {family!r} is recorded as deferred but owns a package"
            )
        package_dir = ROOT / LIBRARY_DIR / f"{family}"
        if package_dir.exists():
            raise ValueError(
                f"{relative(package_dir)}: deferred families own no package directory"
            )

    # Active families own at least one package.
    for family in ACTIVE_FAMILIES:
        if family not in declared_statuses:
            raise ValueError(
                f"family {family!r} is recorded as active but owns no package; move it "
                "to the deferred set or ship its package"
            )


def main() -> int:
    try:
        packages = cargo_packages()
        validate_workspace_inventory(packages)
        declared_statuses: dict[str, set[str]] = {}
        owned_names: dict[str, set[str]] = {}
        brick_features: dict[str, set[str]] = {}
        for package in packages:
            manifest_path = Path(package["manifest_path"]).resolve()
            factory = factory_metadata(package, manifest_path)
            family = factory["family"]
            role = factory["role"]
            status = factory["status"]
            declared_statuses.setdefault(family, set()).add(status)
            validate_package_name(package, manifest_path, family, role)
            owned_names.setdefault(family, set()).add(str(package["name"]))
            validate_location_and_targets(package, manifest_path, role)
            package_dir = package_directory(manifest_path)
            manifest = load_toml(manifest_path)
            validate_feature_table(manifest_path, manifest)
            if role == "brick":
                validate_adapter_isolation(package_dir)
                validate_conditional_derives(package_dir)
                features = manifest.get("features") or {}
                brick_features[str(package["name"])] = set(features)
            if status != "scaffolded":
                continue
            validate_status_only_placement(manifest_path, family, role)
            validate_package_tree(package_dir)
            validate_manifest_configuration(manifest_path, load_toml(manifest_path))
            validate_source_content(package_dir)
        validate_registry(declared_statuses, owned_names)
        validate_quality_gate_coverage(brick_features)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"Brick registry validation failed: {error}", file=sys.stderr)
        return 1
    print(
        f"Brick registry validation passed "
        f"({len(ACTIVE_FAMILIES)} active families, "
        f"{len(DEFERRED_FAMILIES)} deferred)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
