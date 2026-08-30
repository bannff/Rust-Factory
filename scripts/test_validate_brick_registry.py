"""Deterministic self-tests for the workspace package/brick structure validator.

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
from unittest.mock import patch

SCRIPT_PATH = Path(__file__).with_name("validate_brick_registry.py")
SPEC = importlib.util.spec_from_file_location("workspace_structure_validator", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load the workspace package/brick structure validator")
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

STATUS_ONLY_FAMILY = "widget"
IMPLEMENTED_FAMILY = "gadget"

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


class WorkspacePackageBrickStructureValidatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.originals = {
            name: getattr(validator, name)
            for name in (
                "ROOT",
                "ROOT_MANIFEST",
                "MAKEFILE_PATH",
                "STATUS_ONLY_FAMILIES",
            )
        }
        validator.ROOT = self.root
        validator.ROOT_MANIFEST = self.root / "Cargo.toml"
        validator.MAKEFILE_PATH = self.root / "Makefile"
        validator.STATUS_ONLY_FAMILIES = frozenset({STATUS_ONLY_FAMILY})
        # Registered before any fixture is written so a fixture failure cannot
        # leak patched globals into later tests.
        self.addCleanup(self.restore_globals)
        self.addCleanup(self.temporary_directory.cleanup)

        self.scaffold_dir = self.root / f"crates/{STATUS_ONLY_FAMILY}"
        self.implemented_dir = self.root / f"crates/{IMPLEMENTED_FAMILY}"
        self.write_valid_workspace()
        self.write_valid_scaffold()
        self.write_valid_implemented()
        self.write_valid_makefile()

    def restore_globals(self) -> None:
        for name, value in self.originals.items():
            setattr(validator, name, value)

    # ---- fixture builders -------------------------------------------------

    def write_valid_workspace(self, members: list[str] | None = None) -> None:
        if members is None:
            members = [
                f"crates/{STATUS_ONLY_FAMILY}",
                f"crates/{IMPLEMENTED_FAMILY}",
            ]
        rendered = "\n".join(f'    "{member}",' for member in members)
        validator.ROOT_MANIFEST.write_text(
            f"[workspace]\nmembers = [\n{rendered}\n]\n", encoding="utf-8"
        )

    def write_valid_makefile(
        self, bricks: str = IMPLEMENTED_FAMILY, recipes: str = ""
    ) -> None:
        """The quality gate the validator cross-checks brick features against."""
        validator.MAKEFILE_PATH.write_text(
            f"BRICKS := {bricks}\n\n"
            f"lint-features:\n\t@true\n{recipes}\n"
            f"test-features:\n\t@true\n{recipes}\n",
            encoding="utf-8",
        )

    def write_valid_scaffold(self) -> None:
        self.scaffold_dir.mkdir(parents=True, exist_ok=True)
        (self.scaffold_dir / "Cargo.toml").write_text(
            f"""[package]
name = "{STATUS_ONLY_FAMILY}"
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
name = "{IMPLEMENTED_FAMILY}"

[package.metadata.rust-factory]
family = "{IMPLEMENTED_FAMILY}"
role = "brick"
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
                self.implemented_dir, IMPLEMENTED_FAMILY, "brick", "implemented"
            ),
        ]

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

    def test_cargo_metadata_is_locked_and_offline(self) -> None:
        with patch.object(validator.subprocess, "run") as run:
            run.return_value.returncode = 0
            run.return_value.stdout = '{"packages": []}'
            run.return_value.stderr = ""

            self.assertEqual(validator.cargo_packages(), [])

        run.assert_called_once_with(
            [
                "cargo",
                "metadata",
                "--locked",
                "--format-version=1",
                "--no-deps",
                "--offline",
            ],
            cwd=validator.ROOT,
            capture_output=True,
            check=False,
            text=True,
        )

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

    def test_rejects_unknown_role_and_status(self) -> None:
        cases = (
            (IMPLEMENTED_FAMILY, "unknown", "implemented"),
            (IMPLEMENTED_FAMILY, "core", "unknown"),
        )
        for family, role, status in cases:
            with self.subTest(family=family, role=role, status=status):
                package = self.package(
                    self.implemented_dir, family, role, status
                )
                self.assert_rejected(lambda: self.metadata(package))

    def test_accepts_arbitrary_nonempty_family(self) -> None:
        """The roadmap taxonomy lives in GitHub, not here, so any non-empty
        family string is accepted; only structure is enforced."""
        package = self.package(
            self.implemented_dir, "some-new-family", "brick", "implemented"
        )
        self.assertEqual(self.metadata(package)["family"], "some-new-family")

    def test_rejects_empty_or_non_string_family(self) -> None:
        for value in ("", 123):
            with self.subTest(value=value):
                package = self.package(
                    self.implemented_dir, IMPLEMENTED_FAMILY, "brick", "implemented"
                )
                package["metadata"]["rust-factory"]["family"] = value
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
        self.write_valid_workspace(members=[f"crates/{STATUS_ONLY_FAMILY}"])
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_member_without_package_directory(self) -> None:
        self.write_valid_workspace(
            members=[
                f"crates/{STATUS_ONLY_FAMILY}",
                f"crates/{IMPLEMENTED_FAMILY}",
                "crates/absent-core",
            ]
        )
        self.assert_rejected(
            lambda: validator.validate_workspace_inventory(self.valid_packages())
        )

    def test_rejects_duplicate_workspace_member(self) -> None:
        self.write_valid_workspace(
            members=[
                f"crates/{STATUS_ONLY_FAMILY}",
                f"crates/{IMPLEMENTED_FAMILY}",
                f"crates/{IMPLEMENTED_FAMILY}",
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
        self.write_valid_workspace(members=[f"crates/{IMPLEMENTED_FAMILY}"])
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
        directory = self.root / f"vendor/{IMPLEMENTED_FAMILY}"
        package = self.package(directory, IMPLEMENTED_FAMILY, "core", "implemented")
        self.assert_rejected(
            lambda: validator.validate_location_and_targets(
                package, Path(str(package["manifest_path"])), "core"
            )
        )

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

    def test_accepts_library_roles(self) -> None:
        for role in ("brick", "infrastructure", "test-support"):
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

    def test_rejects_core_role_with_behavior(self) -> None:
        """`role = "core"` would otherwise exempt a package with behavior from the
        adapter isolation, conditional derive, and quality gate rules, all of
        which key on `role == "brick"`."""
        for status in ("specified", "implemented", "migration-pending", "deprecated"):
            with self.subTest(status=status):
                package = self.package(
                    self.implemented_dir, IMPLEMENTED_FAMILY, "core", status
                )
                self.assert_rejected(lambda: self.metadata(package))

    def test_accepts_core_role_for_a_status_only_package(self) -> None:
        package = self.package(
            self.scaffold_dir, STATUS_ONLY_FAMILY, "core", "scaffolded"
        )
        self.assertEqual(self.metadata(package)["role"], "core")

    def test_rejects_retired_per_adapter_roles(self) -> None:
        """A brick is one crate, so an adapter is a module and not a package."""
        for role in ("mcp", "memory", "adapter", "vendor", "core-adapter"):
            with self.subTest(role=role):
                directory = self.root / f"crates/{IMPLEMENTED_FAMILY}-{role}"
                package = self.package(
                    directory, IMPLEMENTED_FAMILY, role, "implemented"
                )
                self.assert_rejected(
                    lambda: validator.factory_metadata(
                        package, Path(str(package["manifest_path"]))
                    )
                )

    # ---- adapter isolation ----------------------------------------------

    def write_brick_source(self, relative_path: str, body: str) -> Path:
        path = self.implemented_dir / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
        return path

    def test_accepts_adapter_dependency_inside_its_own_module(self) -> None:
        for relative_path in ("src/mcp.rs", "src/mcp/service.rs"):
            with self.subTest(relative_path=relative_path):
                path = self.write_brick_source(
                    relative_path, "use rmcp::ServerHandler;\n"
                )
                validator.validate_adapter_isolation(self.implemented_dir)
                path.unlink()

    def test_biscuit_auth_is_accepted_only_in_biscuit(self) -> None:
        path = self.write_brick_source(
            "src/biscuit.rs", "use biscuit_auth::Biscuit;\n"
        )
        validator.validate_adapter_isolation(self.implemented_dir)
        path.unlink()

        path = self.write_brick_source("src/mcp.rs", "use biscuit_auth::Biscuit;\n")
        self.assert_rejected(
            lambda: validator.validate_adapter_isolation(self.implemented_dir)
        )
        path.unlink()

    def test_prost_is_accepted_only_in_biscuit(self) -> None:
        path = self.write_brick_source("src/biscuit.rs", "use prost::Message;\n")
        validator.validate_adapter_isolation(self.implemented_dir)
        path.unlink()

        path = self.write_brick_source("src/mcp.rs", "use prost::Message;\n")
        self.assert_rejected(
            lambda: validator.validate_adapter_isolation(self.implemented_dir)
        )
        path.unlink()

    def test_redb_is_accepted_only_in_redb(self) -> None:
        path = self.write_brick_source("src/redb.rs", "use redb::Database;\n")
        validator.validate_adapter_isolation(self.implemented_dir)
        path.unlink()

        path = self.write_brick_source("src/local.rs", "use redb::Database;\n")
        self.assert_rejected(
            lambda: validator.validate_adapter_isolation(self.implemented_dir)
        )
        path.unlink()

    def test_rejects_adapter_dependency_outside_its_module(self) -> None:
        cases = (
            ("src/lib.rs", "let _ = rmcp::ServerHandler;\n"),
            ("src/model.rs", "use schemars::JsonSchema;\n"),
            ("src/service.rs", "use anyhow::Result;\n"),
            ("src/memory.rs", "use mcp_transport::BoundedStdioTransport;\n"),
            ("src/mcp.rs", "use cap_std::fs::Dir;\n"),
            ("src/lib.rs", "use cap_std::fs::Dir;\n"),
        )
        for relative_path, body in cases:
            with self.subTest(relative_path=relative_path, body=body):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source(relative_path, body)
                self.assert_rejected(
                    lambda: validator.validate_adapter_isolation(self.implemented_dir)
                )
                if relative_path != "src/lib.rs":
                    (self.implemented_dir / relative_path).unlink()
                self.write_brick_source("src/lib.rs", original)

    def test_module_of_maps_paths_to_top_level_modules(self) -> None:
        cases = {
            "src/lib.rs": "",
            "src/model.rs": "model",
            "src/mcp.rs": "mcp",
            "src/mcp/dto.rs": "mcp",
            "src/mcp/nested/deep.rs": "mcp",
            "src/fs.rs": "fs",
        }
        for relative_path, expected in cases.items():
            with self.subTest(relative_path=relative_path):
                self.assertEqual(
                    validator.module_of(
                        self.implemented_dir / relative_path, self.implemented_dir
                    ),
                    expected,
                )

    # ---- feature table and conditional attributes -----------------------

    def test_rejects_default_feature(self) -> None:
        manifest = self.implemented_dir / "Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8")
            + '\n[features]\ndefault = ["mcp"]\nmcp = []\n',
            encoding="utf-8",
        )
        self.assert_rejected(
            lambda: validator.validate_feature_table(
                manifest, validator.load_toml(manifest)
            )
        )

    def test_accepts_feature_table_without_a_default(self) -> None:
        manifest = self.implemented_dir / "Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8")
            + "\n[features]\nmcp = []\nmemory = []\n",
            encoding="utf-8",
        )
        validator.validate_feature_table(manifest, validator.load_toml(manifest))

    def test_rejects_conditional_attribute_on_a_domain_type(self) -> None:
        for relative_path in ("src/lib.rs", "src/model.rs"):
            with self.subTest(relative_path=relative_path):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source(
                    relative_path,
                    '#[cfg_attr(feature = "mcp", derive(Serialize))]\nstruct Leaky;\n',
                )
                self.assert_rejected(
                    lambda: validator.validate_conditional_derives(self.implemented_dir)
                )
                if relative_path != "src/lib.rs":
                    (self.implemented_dir / relative_path).unlink()
                self.write_brick_source("src/lib.rs", original)

    def test_accepts_conditional_attribute_inside_an_adapter_module(self) -> None:
        path = self.write_brick_source(
            "src/mcp.rs",
            '#[cfg_attr(feature = "extra", derive(Debug))]\nstruct Dto;\n',
        )
        validator.validate_conditional_derives(self.implemented_dir)
        path.unlink()

    def test_rejects_conditional_attribute_behind_a_nested_predicate(self) -> None:
        """A regex anchored on `feature =` would miss all(), any(), and not()."""
        for predicate in (
            'all(feature = "mcp")',
            'any(feature = "mcp", feature = "fs")',
            'not(feature = "mcp")',
            'all(unix, feature = "mcp")',
        ):
            with self.subTest(predicate=predicate):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source(
                    "src/lib.rs",
                    f"#[cfg_attr({predicate}, derive(Debug))]\nstruct Leaky;\n",
                )
                self.assert_rejected(
                    lambda: validator.validate_conditional_derives(self.implemented_dir)
                )
                self.write_brick_source("src/lib.rs", original)

    def test_rejects_feature_gated_item_in_a_core_module(self) -> None:
        """A cfg-gated re-export names no adapter crate and uses no cfg_attr, but
        still gives the crate two public APIs."""
        for body in (
            '#[cfg(feature = "mcp")]\npub use self::mcp::Dto;\n',
            '#[cfg(feature = "mcp")]\npub fn extra() {}\n',
            '#[cfg(feature = "memory")]\nimpl Leaky {}\n',
        ):
            with self.subTest(body=body):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source("src/lib.rs", body)
                self.assert_rejected(
                    lambda: validator.validate_conditional_derives(self.implemented_dir)
                )
                self.write_brick_source("src/lib.rs", original)

    def test_accepts_feature_gated_adapter_module_declaration(self) -> None:
        for module in sorted(validator.ADAPTER_MODULE_NAMES):
            with self.subTest(module=module):
                self.write_brick_source(
                    "src/lib.rs",
                    f'#[cfg(feature = "{module}")]\npub mod {module};\n',
                )
                self.write_brick_source(f"src/{module}.rs", "// adapter\n")
                validator.validate_conditional_derives(self.implemented_dir)
                (self.implemented_dir / f"src/{module}.rs").unlink()

    def test_rejects_adapter_dependency_reached_through_an_alias(self) -> None:
        for body in (
            "use rmcp as framework;\n",
            "pub use rmcp as framework;\n",
            "use ::rmcp as framework;\n",
            "extern crate rmcp;\n",
        ):
            with self.subTest(body=body):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source("src/lib.rs", body)
                self.assert_rejected(
                    lambda: validator.validate_adapter_isolation(self.implemented_dir)
                )
                self.write_brick_source("src/lib.rs", original)

    def test_rejects_core_module_reaching_into_an_adapter_module(self) -> None:
        for body in (
            "pub fn leak(_input: crate::mcp::Dto) {}\n",
            "use crate::mcp::Dto;\n",
            "type Alias = crate :: fs :: Writer;\n",
        ):
            with self.subTest(body=body):
                original = (self.implemented_dir / "src/lib.rs").read_text(
                    encoding="utf-8"
                )
                self.write_brick_source("src/lib.rs", body)
                self.assert_rejected(
                    lambda: validator.validate_adapter_isolation(self.implemented_dir)
                )
                self.write_brick_source("src/lib.rs", original)

    def test_allows_an_adapter_module_to_reference_its_own_path(self) -> None:
        path = self.write_brick_source(
            "src/mcp.rs", "fn convert(_d: crate::mcp::Dto) {}\n"
        )
        validator.validate_adapter_isolation(self.implemented_dir)
        path.unlink()

    def test_exempts_targets_outside_src_from_module_rules(self) -> None:
        """An integration test is feature-gated at file scope, so the module path
        rules do not govern it; otherwise no adapter could ever be tested from
        `tests/`."""
        for relative_path in ("tests/mcp_contract.rs", "benches/throughput.rs"):
            with self.subTest(relative_path=relative_path):
                path = self.write_brick_source(
                    relative_path,
                    '#![cfg(feature = "mcp")]\nfn probe() { let _ = rmcp::Thing; }\n',
                )
                validator.validate_adapter_isolation(self.implemented_dir)
                validator.validate_conditional_derives(self.implemented_dir)
                self.assertIsNone(
                    validator.module_of(path, self.implemented_dir)
                )
                path.unlink()

    # ---- quality gate coverage ------------------------------------------

    def test_accepts_a_quality_gate_that_covers_every_brick_feature(self) -> None:
        self.write_valid_makefile(
            bricks="gizmo",
            recipes="\tcargo test -p gizmo --features mcp,memory\n",
        )
        validator.validate_quality_gate_coverage({"gizmo": {"mcp", "memory"}})

    def test_rejects_a_brick_absent_from_the_isolation_list(self) -> None:
        self.write_valid_makefile(
            bricks="", recipes="\tcargo test -p gizmo --features mcp\n"
        )
        self.assert_rejected(
            lambda: validator.validate_quality_gate_coverage({"gizmo": {"mcp"}})
        )

    def test_rejects_an_isolation_list_naming_an_unknown_brick(self) -> None:
        self.write_valid_makefile(bricks="gizmo ghost")
        self.assert_rejected(
            lambda: validator.validate_quality_gate_coverage({"gizmo": set()})
        )

    def test_rejects_a_feature_no_target_ever_builds(self) -> None:
        self.write_valid_makefile(
            bricks="gizmo", recipes="\tcargo test -p gizmo --features mcp\n"
        )
        self.assert_rejected(
            lambda: validator.validate_quality_gate_coverage(
                {"gizmo": {"mcp", "memory"}}
            )
        )

    def test_rejects_a_missing_quality_gate(self) -> None:
        validator.MAKEFILE_PATH.unlink()
        self.assert_rejected(
            lambda: validator.validate_quality_gate_coverage({"gizmo": {"mcp"}})
        )

    def test_rejects_a_quality_gate_without_a_brick_list(self) -> None:
        validator.MAKEFILE_PATH.write_text(
            "lint-features:\n\t@true\ntest-features:\n\t@true\n", encoding="utf-8"
        )
        self.assert_rejected(
            lambda: validator.validate_quality_gate_coverage({"gizmo": set()})
        )

    # ---- end-to-end main() ----------------------------------------------

    def test_main_accepts_a_valid_workspace(self) -> None:
        self.assertEqual(self.run_main(), 0)

    def test_main_rejects_status_only_family_declaring_another_status(self) -> None:
        """A status-only family cannot bypass its fixed structure by relabeling."""
        packages = [
            self.package(
                self.scaffold_dir, STATUS_ONLY_FAMILY, "brick", "implemented"
            ),
            self.package(
                self.implemented_dir, IMPLEMENTED_FAMILY, "brick", "implemented"
            ),
        ]
        manifest = self.scaffold_manifest()
        manifest.write_text(
            manifest.read_text(encoding="utf-8")
            .replace('role = "core"', 'role = "brick"')
            .replace('status = "scaffolded"', 'status = "implemented"')
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
        self.write_valid_workspace(members=[f"crates/{STATUS_ONLY_FAMILY}"])
        self.assertEqual(self.run_main(), 1)

    def test_main_rejects_scaffold_tree_violation(self) -> None:
        (self.scaffold_dir / "src/model.rs").unlink()
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
        validator.ROOT_MANIFEST.unlink()
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
