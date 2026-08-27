#!/usr/bin/env python3
"""Deterministic negative tests for the status-only scaffold validator."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).with_name("validate_status_only_scaffolds.py")
SPEC = importlib.util.spec_from_file_location("status_only_validator", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load status-only scaffold validator")
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)


class StatusOnlyScaffoldValidatorTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.package_dir = self.root / "crates/cache-core"
        self.package_dir.mkdir(parents=True)
        self.original_root = validator.ROOT
        self.original_root_manifest = validator.ROOT_MANIFEST
        self.original_vision_path = validator.VISION_PATH
        validator.ROOT = self.root
        validator.ROOT_MANIFEST = self.root / "Cargo.toml"
        validator.VISION_PATH = self.root / "living-factory-vision.md"
        self.write_valid_workspace()
        self.write_valid_vision()
        self.write_valid_scaffold()

    def tearDown(self) -> None:
        validator.ROOT = self.original_root
        validator.ROOT_MANIFEST = self.original_root_manifest
        validator.VISION_PATH = self.original_vision_path
        self.temporary_directory.cleanup()

    def write_valid_workspace(self) -> None:
        members = "\n".join(
            f'    "crates/{family}-core",' for family in validator.SCAFFOLDS
        )
        validator.ROOT_MANIFEST.write_text(
            f"[workspace]\nmembers = [\n{members}\n]\n",
            encoding="utf-8",
        )

    def write_valid_vision(self) -> None:
        rows = "\n".join(
            f"| {label} | `{family}-core` status-only tree | Scaffolded |"
            for family, label in validator.SCAFFOLDS.items()
        )
        validator.VISION_PATH.write_text(rows, encoding="utf-8")

    def write_valid_scaffold(self) -> None:
        (self.package_dir / "Cargo.toml").write_text(
            """[package]
name = "cache-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[package.metadata.rust-factory]
family = "cache"
role = "core"
status = "scaffolded"

[lints]
workspace = true
""",
            encoding="utf-8",
        )
        source_files = {
            "src/lib.rs": "//! Status-only scaffold.\n\nmod error;\nmod model;\nmod port;\nmod service;\nmod validation;\n",
            "src/model.rs": "//! Reserved for a future model.\n",
            "src/validation.rs": "// Reserved for validation.\n",
            "src/error.rs": "/* Reserved for errors. */\n",
            "src/port.rs": "// Reserved for ports.\n",
            "src/service.rs": "// Reserved for services.\n",
            "tests/public_contract.rs": "// Reserved for public contracts.\n",
        }
        for relative_path, contents in source_files.items():
            path = self.package_dir / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")

    def manifest(self) -> Path:
        return self.package_dir / "Cargo.toml"

    def validate_manifest(self) -> None:
        validator.validate_manifest("cache", {self.manifest().resolve()})

    def assert_rejected(self, callback: object) -> None:
        with self.assertRaises(ValueError):
            callback()

    def test_valid_scaffold_is_accepted(self) -> None:
        self.validate_manifest()
        validator.validate_workspace_members()
        validator.validate_vision()

    def test_rejects_missing_required_paths(self) -> None:
        for relative_path in ("src/model.rs", "tests/public_contract.rs"):
            with self.subTest(relative_path=relative_path):
                path = self.package_dir / relative_path
                path.unlink()
                self.assert_rejected(lambda: validator.validate_package_tree(self.package_dir))
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
                path = self.package_dir / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("// unexpected\n", encoding="utf-8")
                self.assert_rejected(lambda: validator.validate_package_tree(self.package_dir))
                path.unlink()
                if path.parent.name == "bin":
                    path.parent.rmdir()

        unexpected_directory = self.package_dir / "src/bin"
        unexpected_directory.mkdir()
        self.assert_rejected(lambda: validator.validate_package_tree(self.package_dir))

    def test_rejects_cargo_target_feature_and_dependency_configuration(self) -> None:
        for configuration in (
            'build = "build.rs"',
            "autobins = false",
            '\n[lib]\npath = "src/alternate.rs"',
            '\n[[bin]]\nname = "cache"\npath = "src/bin/main.rs"',
            '\n[[example]]\nname = "example"\npath = "examples/example.rs"',
            '\n[[bench]]\nname = "benchmark"\npath = "benches/benchmark.rs"',
            '\n[[test]]\nname = "additional"\npath = "tests/additional.rs"',
            "\n[features]\ndefault = []",
            '\n[dependencies]\nserde = "1"',
            '\n[dev-dependencies]\nserde = "1"',
            '\n[build-dependencies]\nserde = "1"',
            "\n[target.'cfg(unix)'.dependencies]\nserde = \"1\"",
        ):
            with self.subTest(configuration=configuration):
                original = self.manifest().read_text(encoding="utf-8")
                if configuration.startswith("build") or configuration.startswith("autobins"):
                    updated = original.replace(
                        "edition.workspace = true", f"edition.workspace = true\n{configuration}"
                    )
                else:
                    updated = original + configuration
                self.manifest().write_text(updated, encoding="utf-8")
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.manifest(), validator.load_toml(self.manifest())
                    )
                )
                self.manifest().write_text(original, encoding="utf-8")

    def test_rejects_missing_or_mismatched_workspace_lints(self) -> None:
        for replacement in (
            "",
            "[lints]\nworkspace = false\n",
            "[lints]\nworkspace = true\nextra = true\n",
            "[lints.rust]\nunsafe_code = \"forbid\"\n",
        ):
            with self.subTest(replacement=replacement):
                original = self.manifest().read_text(encoding="utf-8")
                updated = original.replace("[lints]\nworkspace = true\n", replacement)
                self.manifest().write_text(updated, encoding="utf-8")
                self.assert_rejected(
                    lambda: validator.validate_manifest_configuration(
                        self.manifest(), validator.load_toml(self.manifest())
                    )
                )
                self.manifest().write_text(original, encoding="utf-8")

    def test_rejects_invalid_role_or_status_metadata(self) -> None:
        for field, expected in (("role", "core"), ("status", "scaffolded")):
            with self.subTest(field=field):
                original = self.manifest().read_text(encoding="utf-8")
                self.manifest().write_text(
                    original.replace(f'{field} = "{expected}"', f'{field} = "unknown"'),
                    encoding="utf-8",
                )
                self.assert_rejected(self.validate_manifest)
                self.manifest().write_text(original, encoding="utf-8")

    def test_rejects_canonical_metadata_mismatch(self) -> None:
        original = self.manifest().read_text(encoding="utf-8")
        self.manifest().write_text(
            original.replace('family = "cache"', 'family = "memory"'), encoding="utf-8"
        )
        self.assert_rejected(self.validate_manifest)

    def test_rejects_scaffolded_package_outside_its_canonical_path(self) -> None:
        duplicate_manifest = self.root / "crates/duplicate/Cargo.toml"
        duplicate_manifest.parent.mkdir()
        duplicate_manifest.write_text(self.manifest().read_text(encoding="utf-8"), encoding="utf-8")
        self.assert_rejected(
            lambda: validator.validate_known_metadata({duplicate_manifest.resolve()})
        )

    def test_accepts_known_test_support_metadata_role(self) -> None:
        manifest = self.root / "crates/cache-test-support/Cargo.toml"
        manifest.parent.mkdir()
        manifest.write_text(
            """[package]
name = "cache-test-support"

[package.metadata.rust-factory]
family = "cache"
role = "test-support"
status = "implemented"
""",
            encoding="utf-8",
        )
        validator.validate_known_metadata({manifest.resolve()})

    def test_rejects_missing_workspace_member(self) -> None:
        validator.ROOT_MANIFEST.write_text("[workspace]\nmembers = []\n", encoding="utf-8")
        self.assert_rejected(validator.validate_workspace_members)

    def test_rejects_vision_registry_disagreement(self) -> None:
        vision = validator.VISION_PATH.read_text(encoding="utf-8")
        validator.VISION_PATH.write_text(
            vision.replace("| Cache | `cache-core` status-only tree | Scaffolded |", "| Cache |"),
            encoding="utf-8",
        )
        self.assert_rejected(validator.validate_vision)

    def test_rejects_executable_leaf_source(self) -> None:
        path = self.package_dir / "src/model.rs"
        path.write_text("pub struct Model;\n", encoding="utf-8")
        self.assert_rejected(lambda: validator.validate_source_content(self.package_dir))

    def test_rejects_public_or_noncanonical_library_items(self) -> None:
        path = self.package_dir / "src/lib.rs"
        valid = path.read_text(encoding="utf-8")
        for contents in (
            valid.replace("mod model;", "pub mod model;"),
            valid + "fn behavior() {}\n",
            valid.replace("mod model;", "#[allow(dead_code)]\nmod model;"),
            valid.replace("mod model;", "use crate::model;\nmod model;"),
            valid.replace("mod model;", "mod model;\nmod model;"),
        ):
            with self.subTest(contents=contents):
                path.write_text(contents, encoding="utf-8")
                self.assert_rejected(lambda: validator.validate_source_content(self.package_dir))
        path.write_text(valid, encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
