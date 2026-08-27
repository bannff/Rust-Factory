"""Deterministic negative self-tests for the brick registry validator.

Every fixture uses synthetic family names and a temporary repository root, so
the suite never depends on which real families happen to exist.
"""

from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

SCRIPT_PATH = Path(__file__).with_name("validate_brick_registry.py")
SPEC = importlib.util.spec_from_file_location("brick_registry_validator", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load the brick registry validator")
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

STATUS_ONLY_FAMILY = "widget"
IMPLEMENTED_FAMILY = "gadget"
DEFERRED_FAMILY = "sprocket"

ACTIVE_FAMILIES = {
    STATUS_ONLY_FAMILY: "Widget",
    IMPLEMENTED_FAMILY: "Gadget",
}
DEFERRED_FAMILIES = {DEFERRED_FAMILY: "Sprocket"}

SCAFFOLD_SOURCES = {
    "src/lib.rs": (
        "//! Status-only scaffold.\n\nmod error;\nmod model;\nmod port;\n"
        "mod service;\nmod validation;\n"
    ),
    "src/model.rs": "//! Reserved for a future model.\n",
    "src/validation.rs": "// Reserved for validation.\n",
    "src/error.rs": "/* Reserved for errors. */\n",
    "src/port.rs": "// Reserved for ports.\n",
    "src/service.rs": "// Reserved for services.\n",
    "tests/public_contract.rs": "// Reserved for public contracts.\n",
}


class BrickRegistryValidatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.originals = {
            name: getattr(validator, name)
            for name in (
                "ROOT",
                "ROOT_MANIFEST",
                "VISION_PATH",
                "ACTIVE_FAMILIES",
                "DEFERRED_FAMILIES",
                "STATUS_ONLY_FAMILIES",
            )
        }
        validator.ROOT = self.root
        validator.ROOT_MANIFEST = self.root / "Cargo.toml"
        validator.VISION_PATH = self.root / "living-factory-vision.md"
        validator.ACTIVE_FAMILIES = dict(ACTIVE_FAMILIES)
        validator.DEFERRED_FAMILIES = dict(DEFERRED_FAMILIES)
        validator.STATUS_ONLY_FAMILIES = frozenset({STATUS_ONLY_FAMILY})
        # Registered before any fixture is written so a fixture failure cannot
        # leak patched globals into later tests.
        self.addCleanup(self.restore_globals)
        self.addCleanup(self.temporary_directory.cleanup)

        self.scaffold_dir = self.root / f"crates/{STATUS_ONLY_FAMILY}-core"
        self.implemented_dir = self.root / f"crates/{IMPLEMENTED_FAMILY}-core"
        self.write_valid_workspace()
        self.write_valid_registry()
        self.write_valid_scaffold()
        self.write_valid_implemented()

    def restore_globals(self) -> None:
        for name, value in self.originals.items():
            setattr(validator, name, value)

    # ---- fixture builders -------------------------------------------------

    def write_valid_workspace(self, members: list[str] | None = None) -> None:
        if members is None:
            members = [
                f"crates/{STATUS_ONLY_FAMILY}-core",
                f"crates/{IMPLEMENTED_FAMILY}-core",
            ]
        rendered = "\n".join(f'    "{member}",' for member in members)
        validator.ROOT_MANIFEST.write_text(
            f"[workspace]\nmembers = [\n{rendered}\n]\n", encoding="utf-8"
        )

    def write_valid_registry(self, extra: str = "") -> None:
        rows = [
            validator.REGISTRY_HEADING,
            "",
            "| Family | Taxonomy | Owning crate | Mature shape | Current state |",
            "|---|---|---|---|---|",
            f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | Scaffolded |",
            f"| Gadget | Capability | `{IMPLEMENTED_FAMILY}-core` | core | Implemented |",
            f"| Sprocket | Capability | `{DEFERRED_FAMILY}-core` | core | Deferred |",
        ]
        document = "\n".join(rows) + "\n"
        if extra:
            document += extra if extra.endswith("\n") else extra + "\n"
        validator.VISION_PATH.write_text(document, encoding="utf-8")

    def write_valid_scaffold(self) -> None:
        self.scaffold_dir.mkdir(parents=True, exist_ok=True)
        (self.scaffold_dir / "Cargo.toml").write_text(
            f"""[package]
name = "{STATUS_ONLY_FAMILY}-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[package.metadata.rust-factory]
family = "{STATUS_ONLY_FAMILY}"
role = "core"
status = "scaffolded"

[lints]
workspace = true
""",
            encoding="utf-8",
        )
        for relative_path, contents in SCAFFOLD_SOURCES.items():
            path = self.scaffold_dir / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")

    def write_valid_implemented(self) -> None:
        self.implemented_dir.mkdir(parents=True, exist_ok=True)
        (self.implemented_dir / "Cargo.toml").write_text(
            f"""[package]
name = "{IMPLEMENTED_FAMILY}-core"

[package.metadata.rust-factory]
family = "{IMPLEMENTED_FAMILY}"
role = "core"
status = "implemented"
""",
            encoding="utf-8",
        )
        source = self.implemented_dir / "src/lib.rs"
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_text("pub struct Gadget;\n", encoding="utf-8")

    # ---- helpers ---------------------------------------------------------

    def scaffold_manifest(self) -> Path:
        return self.scaffold_dir / "Cargo.toml"

    def package(
        self,
        directory: Path,
        family: str,
        role: str,
        status: str,
        kinds: tuple[str, ...] = ("lib",),
        name: str | None = None,
    ) -> dict[str, object]:
        return {
            "name": name if name is not None else directory.name,
            "manifest_path": str(directory / "Cargo.toml"),
            "metadata": {
                "rust-factory": {
                    "family": family,
                    "role": role,
                    "status": status,
                }
            },
            "targets": [{"kind": [kind]} for kind in kinds],
        }

    def valid_packages(self) -> list[dict[str, object]]:
        return [
            self.package(
                self.scaffold_dir, STATUS_ONLY_FAMILY, "core", "scaffolded"
            ),
            self.package(
                self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
            ),
        ]

    def valid_statuses(self) -> dict[str, set[str]]:
        return {
            STATUS_ONLY_FAMILY: {"scaffolded"},
            IMPLEMENTED_FAMILY: {"implemented"},
        }

    def valid_owned_names(self) -> dict[str, set[str]]:
        return {
            STATUS_ONLY_FAMILY: {f"{STATUS_ONLY_FAMILY}-core"},
            IMPLEMENTED_FAMILY: {f"{IMPLEMENTED_FAMILY}-core"},
        }

    def check_registry(
        self,
        statuses: dict[str, set[str]] | None = None,
        owned: dict[str, set[str]] | None = None,
    ) -> None:
        validator.validate_registry(
            self.valid_statuses() if statuses is None else statuses,
            self.valid_owned_names() if owned is None else owned,
        )

    def run_main(self, packages: list[dict[str, object]] | None = None) -> int:
        """Drives main() end to end with cargo metadata stubbed and output captured."""
        supplied = self.valid_packages() if packages is None else packages
        original = validator.cargo_packages
        validator.cargo_packages = lambda: supplied
        sink = io.StringIO()
        try:
            with redirect_stdout(sink), redirect_stderr(sink):
                return validator.main()
        finally:
            validator.cargo_packages = original
            self.main_output = sink.getvalue()

    def metadata(self, package: dict[str, object]) -> dict[str, object]:
        return validator.factory_metadata(
            package, Path(str(package["manifest_path"]))
        )

    def assert_rejected(self, callback) -> None:
        with self.assertRaises(ValueError):
            callback()

    # ---- baseline --------------------------------------------------------

    def test_valid_workspace_is_accepted(self) -> None:
        packages = self.valid_packages()
        validator.validate_workspace_inventory(packages)
        for package in packages:
            manifest_path = Path(str(package["manifest_path"]))
            factory = self.metadata(package)
            validator.validate_location_and_targets(
                package, manifest_path, str(factory["role"])
            )
        validator.validate_package_tree(self.scaffold_dir)
        validator.validate_source_content(self.scaffold_dir)
        validator.validate_manifest_configuration(
            self.scaffold_manifest(), validator.load_toml(self.scaffold_manifest())
        )
        self.check_registry()

    # ---- metadata record -------------------------------------------------

    def test_rejects_missing_metadata_table(self) -> None:
        package = self.package(
            self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
        )
        package["metadata"] = {}
        self.assert_rejected(lambda: self.metadata(package))

    def test_rejects_missing_metadata_field(self) -> None:
        for field in validator.REQUIRED_METADATA_FIELDS:
            with self.subTest(field=field):
                package = self.package(
                    self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
                )
                del package["metadata"]["rust-factory"][field]
                self.assert_rejected(lambda: self.metadata(package))

    def test_rejects_unknown_metadata_field(self) -> None:
        package = self.package(
            self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
        )
        package["metadata"]["rust-factory"]["guarantees"] = "durable"
        self.assert_rejected(lambda: self.metadata(package))

    def test_rejects_unknown_family_role_and_status(self) -> None:
        cases = (
            ("unregistered", "core", "implemented"),
            (IMPLEMENTED_FAMILY, "unknown", "implemented"),
            (IMPLEMENTED_FAMILY, "core", "unknown"),
        )
        for family, role, status in cases:
            with self.subTest(family=family, role=role, status=status):
                package = self.package(
                    self.implemented_dir, family, role, status
                )
                self.assert_rejected(lambda: self.metadata(package))

    # ---- status-only placement ------------------------------------------

    def test_rejects_scaffolded_package_outside_canonical_path(self) -> None:
        self.assert_rejected(
            lambda: validator.validate_status_only_placement(
                self.root / "crates/duplicate/Cargo.toml",
                STATUS_ONLY_FAMILY,
                "core",
            )
        )

    def test_rejects_scaffolded_package_with_non_core_role(self) -> None:
        self.assert_rejected(
            lambda: validator.validate_status_only_placement(
                self.scaffold_manifest(), STATUS_ONLY_FAMILY, "memory"
            )
        )

    def test_rejects_scaffolded_status_for_non_status_only_family(self) -> None:
        self.assert_rejected(
            lambda: validator.validate_status_only_placement(
                self.implemented_dir / "Cargo.toml", IMPLEMENTED_FAMILY, "core"
            )
        )

    # ---- status-only tree and content -----------------------------------

    def test_rejects_missing_required_paths(self) -> None:
        for relative_path in ("src/model.rs", "tests/public_contract.rs"):
            with self.subTest(relative_path=relative_path):
                (self.scaffold_dir / relative_path).unlink()
                self.assert_rejected(
                    lambda: validator.validate_package_tree(self.scaffold_dir)
                )
                self.write_valid_scaffold()

    def test_rejects_unexpected_package_content(self) -> None:
        for relative_path in (
            "build.rs",
            "src/bin/main.rs",
            "src/extra.rs",
            "tests/extra.rs",
            "README.md",
        ):
            with self.subTest(relative_path=relative_path):
                path = self.scaffold_dir / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("// unexpected\n", encoding="utf-8")
                self.assert_rejected(
                    lambda: validator.validate_package_tree(self.scaffold_dir)
                )
                path.unlink()
                if path.parent.name == "bin":
                    path.parent.rmdir()

    def test_rejects_package_target_configuration_fields(self) -> None:
        """These must land inside [package] to exercise the intended branch."""
        anchor = "rust-version.workspace = true\n"
        for field in validator.PACKAGE_TARGET_CONFIGURATION_FIELDS:
            with self.subTest(field=field):
                original = self.scaffold_manifest().read_text(encoding="utf-8")
                self.assertIn(anchor, original)
                value = '"build.rs"' if field == "build" else "false"
                patched = original.replace(
                    anchor, f"{anchor}{field} = {value}\n", 1
                )
                self.scaffold_manifest().write_text(patched, encoding="utf-8")
                manifest = validator.load_toml(self.scaffold_manifest())
                # Guard against the field silently landing in another table.
                self.assertIn(field, manifest["package"])
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.scaffold_manifest(), manifest
                    )
                )
                self.scaffold_manifest().write_text(original, encoding="utf-8")

    def test_rejects_cargo_target_feature_and_dependency_configuration(self) -> None:
        for configuration in (
            '\n[lib]\npath = "src/alternate.rs"',
            f'\n[[bin]]\nname = "{STATUS_ONLY_FAMILY}"\npath = "src/bin/main.rs"',
            '\n[[example]]\nname = "example"\npath = "examples/example.rs"',
            '\n[[bench]]\nname = "benchmark"\npath = "benches/benchmark.rs"',
            '\n[[test]]\nname = "additional"\npath = "tests/additional.rs"',
            "\n[features]\ndefault = []",
            '\n[dependencies]\nserde = "1"',
            '\n[dev-dependencies]\nserde = "1"',
            '\n[build-dependencies]\nserde = "1"',
            '\n[target."cfg(unix)".dependencies]\nserde = "1"',
        ):
            with self.subTest(configuration=configuration):
                original = self.scaffold_manifest().read_text(encoding="utf-8")
                self.scaffold_manifest().write_text(
                    original + configuration + "\n", encoding="utf-8"
                )
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.scaffold_manifest(),
                        validator.load_toml(self.scaffold_manifest()),
                    )
                )
                self.scaffold_manifest().write_text(original, encoding="utf-8")

    def test_rejects_missing_workspace_inherited_fields(self) -> None:
        for field in validator.INHERITED_PACKAGE_FIELDS:
            with self.subTest(field=field):
                original = self.scaffold_manifest().read_text(encoding="utf-8")
                self.scaffold_manifest().write_text(
                    original.replace(f"{field}.workspace = true", f'{field} = "0.1.0"'),
                    encoding="utf-8",
                )
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.scaffold_manifest(),
                        validator.load_toml(self.scaffold_manifest()),
                    )
                )
                self.scaffold_manifest().write_text(original, encoding="utf-8")

    def test_rejects_missing_or_mismatched_workspace_lints(self) -> None:
        valid = "[lints]\nworkspace = true\n"
        for replacement in (
            "",
            "[lints]\nworkspace = false\n",
            "[lints]\nworkspace = true\nunsafe_code = \"allow\"\n",
            "[lints.rust]\nworkspace = true\n",
        ):
            with self.subTest(replacement=replacement):
                original = self.scaffold_manifest().read_text(encoding="utf-8")
                self.assertIn(valid, original)
                self.scaffold_manifest().write_text(
                    original.replace(valid, replacement), encoding="utf-8"
                )
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.scaffold_manifest(),
                        validator.load_toml(self.scaffold_manifest()),
                    )
                )
                self.scaffold_manifest().write_text(original, encoding="utf-8")

    def test_rejects_executable_leaf_source(self) -> None:
        (self.scaffold_dir / "src/model.rs").write_text(
            "pub struct Model;\n", encoding="utf-8"
        )
        self.assert_rejected(
            lambda: validator.validate_source_content(self.scaffold_dir)
        )

    def test_rejects_public_or_noncanonical_library_items(self) -> None:
        path = self.scaffold_dir / "src/lib.rs"
        valid = path.read_text(encoding="utf-8")
        for contents in (
            valid.replace("mod model;", "pub mod model;"),
            valid + "fn behavior() {}\n",
            valid.replace("mod model;", "#[allow(dead_code)]\nmod model;"),
            valid.replace("mod model;", "use crate::model;\nmod model;"),
            valid.replace("mod model;", "mod model;\nmod model;"),
            valid.replace("mod model;", ""),
        ):
            with self.subTest(contents=contents):
                path.write_text(contents, encoding="utf-8")
                self.assert_rejected(
                    lambda: validator.validate_source_content(self.scaffold_dir)
                )
        path.write_text(valid, encoding="utf-8")

    # ---- workspace inventory --------------------------------------------

    def test_rejects_unlisted_package_directory(self) -> None:
        self.write_valid_workspace(members=[f"crates/{STATUS_ONLY_FAMILY}-core"])
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_member_without_package_directory(self) -> None:
        self.write_valid_workspace(
            members=[
                f"crates/{STATUS_ONLY_FAMILY}-core",
                f"crates/{IMPLEMENTED_FAMILY}-core",
                "crates/absent-core",
            ]
        )
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_duplicate_workspace_member(self) -> None:
        self.write_valid_workspace(
            members=[
                f"crates/{STATUS_ONLY_FAMILY}-core",
                f"crates/{IMPLEMENTED_FAMILY}-core",
                f"crates/{IMPLEMENTED_FAMILY}-core",
            ]
        )
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_directory_without_manifest(self) -> None:
        (self.root / "crates/orphan").mkdir(parents=True)
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_missing_status_only_member(self) -> None:
        (self.scaffold_dir / "Cargo.toml").unlink()
        for path in sorted(
            self.scaffold_dir.rglob("*"), key=lambda item: -len(item.parts)
        ):
            path.rmdir() if path.is_dir() else path.unlink()
        self.scaffold_dir.rmdir()
        self.write_valid_workspace(members=[f"crates/{IMPLEMENTED_FAMILY}-core"])
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(
                [
                    self.package(
                        self.implemented_dir,
                        IMPLEMENTED_FAMILY,
                        "core",
                        "implemented",
                    )
                ]
            )
        )

    def test_rejects_member_absent_from_cargo_metadata(self) -> None:
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(
                [
                    self.package(
                        self.scaffold_dir, STATUS_ONLY_FAMILY, "core", "scaffolded"
                    )
                ]
            )
        )

    # ---- location and targets -------------------------------------------

    def test_rejects_binary_target_under_crates(self) -> None:
        package = self.package(
            self.implemented_dir,
            IMPLEMENTED_FAMILY,
            "core",
            "implemented",
            kinds=("lib", "bin"),
        )
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "core"
            )
        )

    def test_rejects_server_role_outside_projects(self) -> None:
        package = self.package(
            self.implemented_dir, IMPLEMENTED_FAMILY, "server", "implemented"
        )
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "server"
            )
        )

    def test_rejects_non_server_role_under_projects(self) -> None:
        directory = self.root / f"projects/{IMPLEMENTED_FAMILY}-server"
        package = self.package(
            directory, IMPLEMENTED_FAMILY, "core", "implemented", kinds=("bin",)
        )
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "core"
            )
        )

    def test_rejects_project_without_binary_target(self) -> None:
        directory = self.root / f"projects/{IMPLEMENTED_FAMILY}-server"
        package = self.package(
            directory, IMPLEMENTED_FAMILY, "server", "implemented", kinds=("lib",)
        )
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "server"
            )
        )

    def test_accepts_server_binary_under_projects(self) -> None:
        directory = self.root / f"projects/{IMPLEMENTED_FAMILY}-server"
        package = self.package(
            directory, IMPLEMENTED_FAMILY, "server", "implemented", kinds=("bin",)
        )
        validator.validate_location_and_targets(
            package, Path(str(package["manifest_path"])), "server"
        )

    def test_rejects_package_outside_known_areas(self) -> None:
        directory = self.root / f"vendor/{IMPLEMENTED_FAMILY}-core"
        package = self.package(directory, IMPLEMENTED_FAMILY, "core", "implemented")
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "core"
            )
        )

    # ---- registry agreement ---------------------------------------------

    def test_rejects_missing_registry_row(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Sprocket | Capability | `{DEFERRED_FAMILY}-core` | core | Deferred |",
                "",
            ),
            encoding="utf-8",
        )
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_capability_row_matching_no_declared_family(self) -> None:
        with validator.VISION_PATH.open("a", encoding="utf-8") as registry:
            registry.write("| Ratchet | Capability | `ratchet-core` | core | Deferred |\n")
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_accepts_non_capability_row_without_declared_family(self) -> None:
        with validator.VISION_PATH.open("a", encoding="utf-8") as registry:
            registry.write(
                "| Storage | Adapter infrastructure | No crate | ports | Prohibited |\n"
            )
        self.check_registry()

    def test_rejects_duplicate_registry_row(self) -> None:
        with validator.VISION_PATH.open("a", encoding="utf-8") as registry:
            registry.write(
                f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | Scaffolded |\n"
            )
        self.assert_rejected(validator.registry_rows)

    def test_rejects_deferred_family_without_named_crate(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(f"`{DEFERRED_FAMILY}-core`", "none"), encoding="utf-8"
        )
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_deferred_family_not_recorded_as_deferred(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Sprocket | Capability | `{DEFERRED_FAMILY}-core` | core | Deferred |",
                f"| Sprocket | Capability | `{DEFERRED_FAMILY}-core` | core | Implemented |",
            ),
            encoding="utf-8",
        )
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_status_only_family_not_recorded_as_scaffolded(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | Scaffolded |",
                f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | Implemented |",
            ),
            encoding="utf-8",
        )
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_deferred_family_owning_a_package(self) -> None:
        statuses = self.valid_statuses()
        statuses[DEFERRED_FAMILY] = {"implemented"}
        self.assert_rejected(lambda: self.check_registry(statuses))

    def test_rejects_deferred_family_with_package_directory(self) -> None:
        (self.root / f"crates/{DEFERRED_FAMILY}-core").mkdir(parents=True)
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_active_family_without_a_package(self) -> None:
        statuses = self.valid_statuses()
        del statuses[IMPLEMENTED_FAMILY]
        self.assert_rejected(lambda: self.check_registry(statuses))

    def test_rejects_scaffolded_status_for_non_status_only_declared_family(self) -> None:
        statuses = self.valid_statuses()
        statuses[IMPLEMENTED_FAMILY] = {"scaffolded"}
        self.assert_rejected(lambda: self.check_registry(statuses))

    def test_rejects_family_declared_both_active_and_deferred(self) -> None:
        validator.DEFERRED_FAMILIES = {
            **DEFERRED_FAMILIES,
            IMPLEMENTED_FAMILY: "Gadget",
        }
        self.assert_rejected(
            lambda: self.check_registry()
        )

    def test_rejects_families_sharing_a_registry_label(self) -> None:
        validator.ACTIVE_FAMILIES = {**ACTIVE_FAMILIES, "extra": "Gadget"}
        self.assert_rejected(validator.validate_declared_families)

    def test_rejects_status_only_family_absent_from_active_set(self) -> None:
        validator.STATUS_ONLY_FAMILIES = frozenset({STATUS_ONLY_FAMILY, "absent"})
        self.assert_rejected(validator.validate_declared_families)

    def test_rejects_owning_crate_cell_naming_no_owned_package(self) -> None:
        self.assert_rejected(
            lambda: self.check_registry(
                owned={
                    STATUS_ONLY_FAMILY: {f"{STATUS_ONLY_FAMILY}-core"},
                    IMPLEMENTED_FAMILY: {f"{IMPLEMENTED_FAMILY}-mcp"},
                }
            )
        )

    def test_rejects_implemented_family_recorded_as_scaffolded(self) -> None:
        self.write_valid_registry()
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Gadget | Capability | `{IMPLEMENTED_FAMILY}-core` | core | Implemented |",
                f"| Gadget | Capability | `{IMPLEMENTED_FAMILY}-core` | core | Scaffolded |",
            ),
            encoding="utf-8",
        )
        self.assert_rejected(lambda: self.check_registry())

    def test_rejects_state_keyword_that_is_not_leading(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | Scaffolded |",
                f"| Widget | Capability | `{STATUS_ONLY_FAMILY}-core` | core | "
                "No longer Scaffolded; fully Implemented |",
            ),
            encoding="utf-8",
        )
        self.assert_rejected(lambda: self.check_registry())

    def test_rejects_malformed_registry_row(self) -> None:
        self.write_valid_registry(
            extra="| Ratchet | Capability | `ratchet-core` | a | b | Deferred |"
        )
        self.assert_rejected(validator.registry_rows)

    def test_rejects_unknown_registry_taxonomy(self) -> None:
        self.write_valid_registry(
            extra="| Ratchet | capability | `ratchet-core` | core | Deferred |"
        )
        self.assert_rejected(validator.registry_rows)

    def test_rejects_missing_registry_heading(self) -> None:
        validator.VISION_PATH.write_text("no heading here\n", encoding="utf-8")
        self.assert_rejected(validator.registry_rows)

    def test_ignores_tables_outside_the_registry_section(self) -> None:
        self.write_valid_registry(
            extra=(
                "\n## Some other section\n\n"
                "| Family | Taxonomy | Owning crate | Mature shape | Current state |\n"
                "|---|---|---|---|---|\n"
                "| Widget | Capability | `elsewhere` | core | Deferred |\n"
            )
        )
        rows = validator.registry_rows()
        self.assertEqual(sorted(rows), ["Gadget", "Sprocket", "Widget"])
        self.assertIn(f"`{STATUS_ONLY_FAMILY}-core`", rows["Widget"]["owner"])

    # ---- package naming --------------------------------------------------

    def test_rejects_package_name_not_matching_directory(self) -> None:
        package = self.package(
            self.scaffold_dir,
            STATUS_ONLY_FAMILY,
            "core",
            "scaffolded",
            name="totally-unrelated",
        )
        self.assert_rejected(
            lambda: validator.validate_package_name(
                package, Path(str(package["manifest_path"])), STATUS_ONLY_FAMILY, "core"
            )
        )

    def test_rejects_package_name_not_matching_family(self) -> None:
        directory = self.root / "crates/unrelated-core"
        package = self.package(directory, IMPLEMENTED_FAMILY, "core", "implemented")
        self.assert_rejected(
            lambda: validator.validate_package_name(
                package, Path(str(package["manifest_path"])), IMPLEMENTED_FAMILY, "core"
            )
        )

    def test_rejects_missing_package_name(self) -> None:
        package = self.package(
            self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
        )
        del package["name"]
        self.assert_rejected(
            lambda: validator.validate_package_name(
                package, Path(str(package["manifest_path"])), IMPLEMENTED_FAMILY, "core"
            )
        )

    def test_accepts_family_named_package_without_core_suffix(self) -> None:
        directory = self.root / f"crates/{IMPLEMENTED_FAMILY}"
        package = self.package(directory, IMPLEMENTED_FAMILY, "infrastructure", "implemented")
        validator.validate_package_name(
            package,
            Path(str(package["manifest_path"])),
            IMPLEMENTED_FAMILY,
            "infrastructure",
        )

    def test_accepts_neutrally_named_composition_root(self) -> None:
        """A server hosts several bricks, so it carries no family prefix."""
        directory = self.root / "projects/factory-node"
        package = self.package(
            directory, IMPLEMENTED_FAMILY, "server", "implemented", kinds=("bin",)
        )
        validator.validate_package_name(
            package, Path(str(package["manifest_path"])), IMPLEMENTED_FAMILY, "server"
        )
        validator.validate_location_and_targets(
            package, Path(str(package["manifest_path"])), "server"
        )

    def test_accepts_adapter_and_infrastructure_roles(self) -> None:
        for role in ("adapter", "infrastructure", "memory", "mcp", "test-support"):
            with self.subTest(role=role):
                directory = self.root / f"crates/{IMPLEMENTED_FAMILY}-{role}"
                package = self.package(
                    directory, IMPLEMENTED_FAMILY, role, "implemented"
                )
                factory = validator.factory_metadata(
                    package, Path(str(package["manifest_path"]))
                )
                self.assertEqual(factory["role"], role)
                validator.validate_location_and_targets(
                    package, Path(str(package["manifest_path"])), role
                )

    # ---- end-to-end main() ----------------------------------------------

    def test_main_accepts_a_valid_workspace(self) -> None:
        self.assertEqual(self.run_main(), 0)

    def test_main_rejects_status_only_family_declaring_another_status(self) -> None:
        """The escape hatch: main() skips scaffold checks for other statuses."""
        packages = [
            self.package(
                self.scaffold_dir, STATUS_ONLY_FAMILY, "core", "implemented"
            ),
            self.package(
                self.implemented_dir, IMPLEMENTED_FAMILY, "core", "implemented"
            ),
        ]
        manifest = self.scaffold_manifest()
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(
                'status = "scaffolded"', 'status = "implemented"'
            )
            + '\n[dependencies]\nserde = "1"\n',
            encoding="utf-8",
        )
        (self.scaffold_dir / "src/extra.rs").write_text("pub fn x() {}\n", encoding="utf-8")
        self.assertEqual(self.run_main(packages), 1)

    def test_main_rejects_missing_metadata(self) -> None:
        packages = self.valid_packages()
        packages[1]["metadata"] = {}
        self.assertEqual(self.run_main(packages), 1)

    def test_main_rejects_unlisted_package_directory(self) -> None:
        self.write_valid_workspace(members=[f"crates/{STATUS_ONLY_FAMILY}-core"])
        self.assertEqual(self.run_main(), 1)

    def test_main_rejects_scaffold_tree_violation(self) -> None:
        (self.scaffold_dir / "src/model.rs").unlink()
        self.assertEqual(self.run_main(), 1)

    def test_main_rejects_registry_disagreement(self) -> None:
        registry = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            registry.replace(
                f"| Sprocket | Capability | `{DEFERRED_FAMILY}-core` | core | Deferred |",
                "",
            ),
            encoding="utf-8",
        )
        self.assertEqual(self.run_main(), 1)

    def test_main_rejects_binary_target_under_crates(self) -> None:
        packages = [
            self.package(
                self.scaffold_dir, STATUS_ONLY_FAMILY, "core", "scaffolded"
            ),
            self.package(
                self.implemented_dir,
                IMPLEMENTED_FAMILY,
                "core",
                "implemented",
                kinds=("lib", "bin"),
            ),
        ]
        self.assertEqual(self.run_main(packages), 1)

    def test_main_reports_failure_without_raising(self) -> None:
        validator.VISION_PATH.unlink()
        self.assertEqual(self.run_main(), 1)

    def test_rejects_nested_manifest_hidden_below_a_package(self) -> None:
        nested = self.implemented_dir / "nested"
        nested.mkdir()
        (nested / "Cargo.toml").write_text("[package]\nname = \"nested\"\n", encoding="utf-8")
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_oversized_file(self) -> None:
        original = validator.MAX_READ_BYTES
        self.addCleanup(setattr, validator, "MAX_READ_BYTES", original)
        validator.MAX_READ_BYTES = 8
        self.assert_rejected(
            lambda: validator.validate_source_content(self.scaffold_dir)
        )


if __name__ == "__main__":
    unittest.main()
