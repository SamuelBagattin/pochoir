# `make` is the only entry point for this repository. CI runs `make check`.
# Keep every target here; never put build logic only in a CI workflow.

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check lint test build wasm deps-check check clean

WASM_TARGET := wasm32-unknown-unknown

help: ## Show this help
	@printf 'Usage: make <target>\n\n'
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_-]+:.*## / { printf "  \033[36m%-11s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

fmt: ## Format all Rust code in place
	cargo fmt --all

fmt-check: ## Fail if any Rust code is not formatted
	cargo fmt --all -- --check

lint: ## Clippy over all targets and features, warnings are errors
	cargo clippy --all-targets --all-features -- -D warnings

test: ## Run unit tests and doc tests across the workspace
	cargo test --workspace

build: ## Build the workspace for the host
	cargo build --workspace

wasm: ## Build the core engine for the browser target
	cargo build -p filigrane-core --target $(WASM_TARGET) --release

deps-check: ## Fail if Cargo.lock grew a third-party dependency
	./scripts/check-deps.sh

check: fmt-check lint test wasm deps-check ## Everything CI runs
	@printf '\nAll checks passed.\n'

clean: ## Remove build artefacts
	cargo clean
