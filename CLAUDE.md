# Pochoir

Watermarks identity documents entirely on the user's own machine.
A Rust core compiles to WebAssembly for the browser and is reused as a native CLI; an Astro static site (Svelte islands) ships on Cloudflare Workers static assets.
No backend, no database, no account, no analytics: the document never leaves the device.

The repository is <https://github.com/SamuelBagattin/pochoir>. The local folder is still `~/dev/samuelbagattin/filigrane` for historical reasons; that is deliberate and not to be renamed.

## Dependency policy

- Be lean. Prefer writing a feature ourselves over inheriting a crate or package we do not control.
- Reasons: supply-chain surface, auditability, and client bundle size.
- Every new dependency needs an ADR under `docs/adr/` **and** a `THRESHOLD` bump in `scripts/check-deps.sh`, in the same commit that adds it.
- `pochoir-core` and `pochoir-cli` currently have zero third-party crates. `make deps-check` fails the build if that changes.
- `pochoir-core` must keep compiling for `wasm32-unknown-unknown`: no `std::fs`, no `std::time`, no threads, no sockets in that crate.

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

## Licence

**Split-licensed. [LICENSE.md](LICENSE.md) is the decision; read it before adding a file in a new top-level directory.**

- The Rust engine and CLI — `crates/pochoir-core`, `crates/pochoir-cli` — are `MIT OR Apache-2.0`, so integrators and distribution packagers can use them. `LICENSE-MIT` and `LICENSE-APACHE` at the root.
- The web application and the site, including any future `app/` or `web/` directory, are `AGPL-3.0-or-later`. `LICENSE-AGPL` at the root. §13 is the point: anyone who forks Pochoir into a hosted service has to publish what they are running.
- `license` in `Cargo.toml` describes the crates only. It is not a statement about the application.
- Decided by Samuel on 2026-11-26. ADR-0001 had recommended AGPL throughout, with `MIT OR Apache-2.0` as runner-up; the split takes the §13 protection where it matters and keeps the reuse.

## Decisions

- `docs/adr/` holds the architecture decision records.
- Anything non-obvious gets one: a dependency, a file format choice, a rendering approach, a hosting decision.
- ADR-0001, ADR-0002 and ADR-0003 are **Accepted** as of 2026-11-26 and are binding; a divergence between the code and an accepted ADR is a bug in one or the other. ADR-0004 is reserved for build reproducibility and bundle attestation, and is not written yet.
