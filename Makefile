# `make` is the only entry point for this repository. CI runs `make check`.
# Keep every target here; never put build logic only in a CI workflow.

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check lint test build wasm wasm-repro deps-check deps-update font-subset nfc-table check clean

WASM_TARGET := wasm32-unknown-unknown
WASM_ARTEFACT := target/$(WASM_TARGET)/release/pochoir_wasm.wasm
# `$(CARGO_HOME)` is not exported by default; rustup's own default is ~/.cargo.
CARGO_HOME_DIR := $(if $(CARGO_HOME),$(CARGO_HOME),$(HOME)/.cargo)

help: ## Show this help
	@printf 'Usage: make <target>\n\n'
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_-]+:.*## / { printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

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

wasm: ## Build the browser engine: crates/pochoir-wasm -> pochoir_wasm.wasm
	# `pochoir-wasm` is the artefact the browser loads; `pochoir-core` on its own
	# exports nothing but `memory`. Building this crate compiles the core for
	# wasm32 as a dependency, so the "core must keep compiling for wasm32"
	# guarantee is still checked by this target.
	#
	# `--locked` is ADR-0004 Decision 1, constraint 2: a release build must never
	# resolve a dependency the lock file does not name.
	#
	# `--remap-path-scope=object` plus the two `--remap-path-prefix` entries are
	# that decision's measured remedy. Without them the module carries
	# `core::panic::Location` strings naming the builder's home directory and
	# their crates.io registry, in the data section where `strip` does not reach.
	# Cargo excludes `--remap-path-prefix` from the metadata hash on purpose, so
	# two checkouts at different paths still agree byte for byte. Measured rather
	# than assumed: `make wasm-repro`.
	RUSTFLAGS="--remap-path-scope=object --remap-path-prefix=$(CURDIR)=/pochoir --remap-path-prefix=$(CARGO_HOME_DIR)/registry/src=/registry" \
	  cargo build -p pochoir-wasm --target $(WASM_TARGET) --release --locked
	@printf '\nwasm: %s\n' "$$(wc -c <'$(WASM_ARTEFACT)' | tr -d ' ') bytes, $(WASM_ARTEFACT)"

wasm-repro: ## Prove `make wasm` reproducible: build twice from two paths
	./scripts/check-repro.sh

deps-check: ## Fail if a crate grew a dependency outside its allowlist
	./scripts/check-deps.sh

deps-update: ## Regenerate the per-crate dependency allowlists by measurement
	./scripts/check-deps.sh --update $(if $(CRATE),$(CRATE),--all)

font-subset: ## Rebuild the bundled font subset from an upstream Roboto-Regular.ttf
	@test -n "$(FONT)" || { \
		echo 'Usage: make font-subset FONT=/path/to/Roboto-Regular.ttf'; \
		echo 'Upstream: https://github.com/googlefonts/roboto src/hinted/Roboto-Regular.ttf'; \
		exit 2; \
	}
	./scripts/subset-font.py "$(FONT)" crates/pochoir-core/assets/roboto/Roboto-Regular-subset.ttf

nfc-table: ## Print the NFC composition table for crates/pochoir-core/src/mark/nfc.rs
	@echo '# Paste between the markers in crates/pochoir-core/src/mark/nfc.rs.'
	@./scripts/gen-nfc-table.py

check: fmt-check lint test wasm deps-check ## Everything CI runs
	@printf '\nAll checks passed.\n'

clean: ## Remove build artefacts
	cargo clean
