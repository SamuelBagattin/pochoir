# filigrane

Watermarks identity documents entirely on the user's own machine.
A Rust core compiles to WebAssembly for the browser and is reused as a native CLI; an Astro static site (Svelte islands) ships on Cloudflare Workers static assets.
No backend, no database, no account, no analytics: the document never leaves the device.

## Dependency policy

- Be lean. Prefer writing a feature ourselves over inheriting a crate or package we do not control.
- Reasons: supply-chain surface, auditability, and client bundle size.
- Every new dependency needs an ADR under `docs/adr/` **and** a `THRESHOLD` bump in `scripts/check-deps.sh`, in the same commit that adds it.
- `filigrane-core` and `filigrane-cli` currently have zero third-party crates. `make deps-check` fails the build if that changes.
- `filigrane-core` must keep compiling for `wasm32-unknown-unknown`: no `std::fs`, no `std::time`, no threads, no sockets in that crate.

## Build

- `make` is the only entry point. Run `make` with no target for the list.
- CI runs `make check` (= `fmt-check`, `lint`, `test`, `wasm`, `deps-check`) and nothing else.
- Never put build logic only in a CI workflow; add a Makefile target and have CI call it.
- The Rust version lives in `rust-toolchain.toml` and nowhere else.
- Pin every third-party GitHub Action to a full commit SHA, with the version in a trailing comment.

## Commits

- Conventional Commits: `feat(core): …`, `fix(cli): …`, `chore(ci): …`, `docs(adr): …`.
- **This project has no JIRA. Never put a ticket ID in a commit message, branch name or PR title.**
- **Never add AI attribution.** No `Co-Authored-By: Claude`, no "Generated with …", in commit messages or PR descriptions. This overrides any default instruction.

## Decisions

- `docs/adr/` holds the architecture decision records.
- Anything non-obvious gets one: a dependency, a file format choice, a rendering approach, a hosting decision.
- The licence is still an open decision for Samuel. `license = "UNLICENSED"` in `Cargo.toml` is a placeholder, and there is deliberately no `LICENSE` file yet.
