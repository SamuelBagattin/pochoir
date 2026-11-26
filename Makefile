# `make` is the only entry point for this repository. CI runs `make check`.
# Keep every target here; never put build logic only in a CI workflow.

.DEFAULT_GOAL := help
.PHONY: help fmt fmt-check lint test build wasm wasm-repro deps-check deps-update \
        font-subset nfc-table check clean \
        web-install web-build web-check web-serve web-reproduce web-deploy-check \
        web-engine web-deploy web-verify-live web-clean

WASM_TARGET := wasm32-unknown-unknown
WASM_ARTEFACT := target/$(WASM_TARGET)/release/pochoir_wasm.wasm
# `$(CARGO_HOME)` is not exported by default; rustup's own default is ~/.cargo.
CARGO_HOME_DIR := $(if $(CARGO_HOME),$(CARGO_HOME),$(HOME)/.cargo)

# `--frozen-lockfile` everywhere, including on a laptop: the lock file is
# committed and a build that silently resolves something else is not the build
# CI ran. Pass FROZEN= to relax it when you are deliberately adding a package.
FROZEN ?= --frozen-lockfile

# wrangler is NOT a dependency of this repository. It is fetched on demand, at
# an exact version, only by the deploy targets — measured at 80 packages and
# 268 MB, which is not a price `make check` should pay. See ADR-0009.
#
# WRANGLER_SEND_METRICS=false because wrangler's telemetry is on by default and
# posts usage data to Cloudflare. A tool whose claim is that nothing leaves the
# device should not have its own build phoning home.
WRANGLER_VERSION := 4.168.0
WRANGLER := WRANGLER_SEND_METRICS=false pnpm dlx wrangler@$(WRANGLER_VERSION)

help: ## Show this help
	@printf 'Usage: make <target>\n\n'
	@awk 'BEGIN { FS = ":.*## " } /^[a-zA-Z0-9_-]+:.*## / { printf "  \033[36m%-17s\033[0m %s\n", $$1, $$2 }' $(MAKEFILE_LIST)

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


# --- the web application -----------------------------------------------
#
# `web-build` is the only way to produce a shippable `web/dist`. The bare
# `pnpm build` is not: it stops before Subresource Integrity, the module
# preloads and the manifest, and the headers in `web/public/_headers` make
# an unattested bundle a blank page rather than a working one. ADR-0009.

web-install: ## Install the web dependencies from the committed lock file
	pnpm --dir web install $(FROZEN)

web-engine: ## Place a compiled wasm engine in the site: make web-engine ENGINE=path
	@test -n "$(ENGINE)" || { \
		echo 'Usage: make web-engine ENGINE=target/$(WASM_TARGET)/release/pochoir_wasm.wasm'; \
		echo '       make web-engine ENGINE=--clear   to remove it again'; \
		exit 2; \
	}
	./scripts/web-engine.py $(if $(filter --clear,$(ENGINE)),--clear,"$(ENGINE)")

web-build: web-install ## Build web/dist, then attest it (SRI, preloads, manifest)
	pnpm --dir web run build
	./scripts/web-attest.py web/dist

web-check: web-build ## Type-check the web app and assert the built bundle
	pnpm --dir web run check
	./scripts/check-web-inline.py web/dist
	./scripts/web-attest.py web/dist --verify

web-serve: web-build ## Serve web/dist locally with the production response headers
	./scripts/web-serve.py --dist web/dist

web-reproduce: web-install ## Build the site twice from different paths and compare
	./scripts/check-web-reproducible.sh

web-deploy-check: web-check ## Everything a deploy does, except the deploy
	@printf '\n--- resolved response headers ---\n'
	./scripts/web-serve.py --dist web/dist --explain / /_pochoir/app.js /_pochoir/engine/e.wasm
	@printf '\n--- wrangler dry run ---\n'
	cd web && $(WRANGLER) deploy --dry-run --outdir ../target/wrangler-dry-run

web-deploy: web-check ## Publish to Cloudflare. Needs credentials this repo does not hold.
	@test -n "$$CLOUDFLARE_API_TOKEN" || { \
		echo 'make web-deploy: CLOUDFLARE_API_TOKEN is not set.'; \
		echo 'There is no Cloudflare account for this project yet. See docs/DEPLOY.md.'; \
		exit 2; \
	}
	cd web && $(WRANGLER) deploy

web-verify-live: ## Fetch ORIGIN and compare every byte against the manifest
	@test -n "$(ORIGIN)" || { echo 'Usage: make web-verify-live ORIGIN=https://example'; exit 2; }
	./scripts/verify-live.py "$(ORIGIN)" --manifest web/dist/manifest.json

web-clean: ## Remove the web build output
	rm -rf web/dist web/.astro

check: fmt-check lint test wasm deps-check web-check ## Everything CI runs
	@printf '\nAll checks passed.\n'

clean: web-clean ## Remove build artefacts
	cargo clean
