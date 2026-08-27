.PHONY: fmt fmt-check lint test scaffold-check check

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

scaffold-check:
	python3 -m unittest scripts/test_validate_status_only_scaffolds.py
	python3 scripts/validate_status_only_scaffolds.py

check: scaffold-check fmt-check lint test
