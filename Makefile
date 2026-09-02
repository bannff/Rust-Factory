.PHONY: fmt fmt-check lint lint-features test test-features isolation-check registry-check check

# Every brick is one crate. Adapters are opt-in features and nothing is enabled
# by default, so a workspace-wide command only exercises the framework-free
# cores. The -features targets below are what cover the adapter code; without
# them roughly half the workspace would go uncompiled, unlinted, and untested.

BRICKS := agent auth evaluation knowledge llm-gateway memory observability policy project sandbox storage workflow

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# Each feature is linted alone. --all-features cannot replace these: it unifies
# every feature, so it never proves that `project --features mcp` builds without
# `fs`, which is exactly the combination a consumer is most likely to pick.
lint-features:
	cargo clippy -p agent --features mcp --all-targets -- -D warnings
	cargo clippy -p auth --features biscuit --all-targets -- -D warnings
	cargo clippy -p evaluation --features mcp --all-targets -- -D warnings
	cargo clippy -p evaluation --features memory --all-targets -- -D warnings
	cargo clippy -p evaluation --features local --all-targets -- -D warnings
	cargo clippy -p evaluation --features serdes-ai-evals --all-targets -- -D warnings
	cargo clippy -p evaluation --features settings --all-targets -- -D warnings
	cargo clippy -p evaluation --features mcp,memory,local,serdes-ai-evals,settings --all-targets -- -D warnings
	cargo clippy -p knowledge --no-default-features --features static --all-targets -- -D warnings
	cargo clippy -p llm-gateway --no-default-features --features static --all-targets -- -D warnings
	cargo clippy -p llm-gateway --no-default-features --features genai --all-targets -- -D warnings
	cargo clippy -p llm-gateway --no-default-features --features tokio_cancellation --all-targets -- -D warnings
	cargo clippy -p llm-gateway --no-default-features --features static,genai --all-targets -- -D warnings
	cargo clippy -p llm-gateway --no-default-features --features static,genai,tokio_cancellation --all-targets -- -D warnings
	cargo clippy -p memory --features local --all-targets -- -D warnings
	cargo clippy -p memory --features agentic --all-targets -- -D warnings
	cargo clippy -p memory --features settings --all-targets -- -D warnings
	cargo clippy -p memory --features mcp --all-targets -- -D warnings
	cargo clippy -p memory --features agentic,local --all-targets -- -D warnings
	cargo clippy -p observability --features local --all-targets -- -D warnings
	cargo clippy -p observability --features settings --all-targets -- -D warnings
	cargo clippy -p observability --features opentelemetry --all-targets -- -D warnings
	cargo clippy -p observability --features mcp --all-targets -- -D warnings
	cargo clippy -p policy --features memory --all-targets -- -D warnings
	cargo clippy -p project --features fs --all-targets -- -D warnings
	cargo clippy -p project --features mcp --all-targets -- -D warnings
	cargo clippy -p sandbox --features docker --all-targets -- -D warnings
	cargo clippy -p sandbox --features mcp --all-targets -- -D warnings
	cargo clippy -p sandbox --features observability --all-targets -- -D warnings
	cargo clippy -p sandbox --features docker,mcp --all-targets -- -D warnings
	cargo clippy -p sandbox --features docker,mcp,observability --all-targets -- -D warnings
	cargo clippy -p storage --features local --all-targets -- -D warnings
	cargo clippy -p storage --features redb --all-targets -- -D warnings
	cargo clippy -p storage --features settings --all-targets -- -D warnings
	cargo clippy -p storage --features local,redb,settings --all-targets -- -D warnings
	cargo clippy -p workflow --features mcp --all-targets -- -D warnings
	cargo clippy -p workflow --features memory --all-targets -- -D warnings
	cargo clippy --workspace --all-features --all-targets -- -D warnings

test:
	cargo test --workspace

test-features:
	cargo test -p agent --features mcp
	cargo test -p auth --features biscuit
	cargo test -p evaluation --features mcp,memory
	cargo test -p evaluation --features local
	cargo test -p evaluation --features serdes-ai-evals
	cargo test -p evaluation --features settings
	cargo test -p evaluation --features mcp,memory,local,serdes-ai-evals,settings
	cargo test -p knowledge --no-default-features --features static
	cargo test -p llm-gateway --no-default-features --features static
	cargo test -p llm-gateway --no-default-features --features genai
	cargo test -p llm-gateway --no-default-features --features tokio_cancellation
	cargo test -p llm-gateway --no-default-features --features static,genai
	cargo test -p llm-gateway --no-default-features --features static,genai,tokio_cancellation
	cargo test -p memory --features local
	cargo test -p memory --features settings
	cargo test -p memory --features agentic,local
	cargo test -p memory --features agentic,local,settings
	cargo test -p memory --features mcp
	cargo test -p memory --features mcp,local
	cargo test -p observability --features local
	cargo test -p observability --features settings
	cargo test -p observability --features opentelemetry
	cargo test -p observability --features mcp
	cargo test -p observability --all-features
	cargo test -p policy --features memory
	cargo test -p project --features mcp,fs
	cargo test -p project --features mcp
	cargo test -p sandbox --features docker
	cargo test -p sandbox --features mcp
	cargo test -p sandbox --features observability
	cargo test -p sandbox --features docker,mcp
	cargo test -p sandbox --features docker,mcp,observability
	cargo test -p storage --features local
	cargo test -p storage --features redb
	cargo test -p storage --features settings
	cargo test -p storage --features local,redb,settings
	cargo test -p workflow --features mcp,memory
	cargo test --workspace --all-features

# Asserts that each brick's default build resolves none of the adapter
# dependencies listed below — transport, schema, error-framework, filesystem, and
# async runtime. It does not claim a brick has no third-party dependency: every
# brick still resolves sha2, and workflow resolves serde and serde_json in its
# core.
#
# Two limits worth being precise about. This checks dependency *resolution*, not
# source text; the validator's path rule is what keeps an adapter crate from
# being named outside its own module. And `cargo tree -p` resolves one crate's
# graph in isolation, so it says nothing about artifacts: Cargo unifies features
# per build graph, so a binary composing several bricks with `mcp` enabled links
# one framework-carrying build of each.
ADAPTER_DEPS := rmcp mcp-transport schemars@0.9.0 schemars@1.2.2 anyhow cap-std tokio genai reqwest futures agentic-memory redb opentelemetry serdes-ai-evals biscuit-auth prost subprocess

isolation-check:
	@for dep in $(ADAPTER_DEPS); do \
		if ! cargo tree -q --workspace --all-features --invert $$dep >/dev/null 2>&1; then \
			echo "isolation-check aborted: cannot resolve $$dep anywhere in the workspace." >&2; \
			echo "  A non-zero exit from cargo tree would otherwise be read as absence," >&2; \
			echo "  making every check below vacuously pass." >&2; \
			exit 1; \
		fi; \
	done
	@for brick in $(BRICKS); do \
		for dep in $(ADAPTER_DEPS); do \
			if cargo tree -q -p $$brick --invert $$dep >/dev/null 2>&1; then \
				echo "isolation-check failed: default build of $$brick reaches $$dep" >&2; \
				exit 1; \
			fi; \
		done; \
		echo "isolation-check: $$brick default build is framework-free"; \
	done

# Compatibility target name; validates deterministic workspace package/brick structure.
registry-check:
	python3 -m unittest scripts/test_validate_brick_registry.py
	python3 scripts/validate_brick_registry.py

check: registry-check isolation-check fmt-check lint lint-features test test-features
