.PHONY: fmt fmt-check lint test registry-check check

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

registry-check:
	python3 -m unittest scripts/test_validate_brick_registry.py
	python3 scripts/validate_brick_registry.py

check: registry-check fmt-check lint test
