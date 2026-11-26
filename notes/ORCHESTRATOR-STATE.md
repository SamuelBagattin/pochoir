# Orchestrator state — filigrane

Maintained by the orchestrating session. Updated after every meaningful event.
Last updated: 2026-11-26.

## My role

Master orchestrator. I plan, write briefs, review diffs, merge, and keep this file.
I do not write product code or repo docs myself. Every change goes to a sub-session in its
own git worktree.

## The project

A document-watermarking tool meant to beat the French government's Filigrane Facile
(https://filigrane.beta.gouv.fr/) on privacy and output quality.

Verified facts about the incumbent: Vue 3 SPA (`watermark` package in
MTES-MCT/Dossier-Facile-Frontend) → `dossierfacile-api-watermark` (Java/Spring) → RabbitMQ →
`dossierfacile-pdf-generator` (Apache PDFBox) → Postgres `watermark_document` + object storage.
The user uploads the document. Each page is rasterised, the mark is composited into the pixels
(alpha 0.52–0.6, blur, −25°, random per-run jitter, QR-code hole), then re-encoded as a JPEG at
128 DPI fitted to A4. Several inputs are merged into one PDF. One 100-character text field, no
preview, no controls. PDF signing exists but is off by default. RGAA: "non conforme", never
audited, since 2023-09-19.

Our differentiators, in priority order:
1. Nothing leaves the device — Rust → WASM, same core crate reused as a native CLI.
2. Stop destroying the document — keep page geometry, selectable DPI, lossless option.
3. Live preview and real controls.
4. Metadata hygiene — strip EXIF/GPS/XMP.
5. N inputs → N outputs, or merge, user's choice.
Plus: audited accessibility, i18n, no analytics on the upload page, an integrator surface
(crate + CLI + npm/WASM).

Honest limits we keep stating: a local-only tool cannot produce an authority-backed signature;
large scanned PDFs are a WASM memory risk; SEO against a gouv.fr domain is hard; no revenue.

## Standing constraints

- **Lean dependencies.** Rebuild rather than depend. Every dependency needs its own ADR and a
  deliberate bump of the `deps-check` threshold in CI. No CSS framework, no UI kit.
- Rust preferred wherever viable. Astro static + Svelte islands for the UI (not SvelteKit —
  nothing to render server-side). Cloudflare Workers static assets. No backend, no database.
- `make` is the only entry point; CI runs `make check`.
- Conventional Commits. **This project has no JIRA: no ticket IDs in commits.**
- **No AI attribution in commits or PR descriptions, ever.**
- Sessions never deploy, never touch production or secrets, never push, never open PRs.

## Repository

- Path: `~/dev/samuelbagattin/filigrane`, branch `main`, `e8428a1 chore: initialise repository`
  (empty commit).
- **No git remote yet.** Sessions commit to their worktree branch; I merge locally.
- Public/private GitHub repo: **undecided, waiting on Samuel.**
- Public product name and domain: **undecided, waiting on Samuel.** Local workspace name is
  `filigrane`; renaming a crate before publication is cheap.

## Live sub-sessions

| name | id | owns | state |
|---|---|---|---|
| `scaffold` | `local_ae46d0e2-325d-4ddb-82b6-544820645fe1` | build skeleton | **merged into `main` as `ea30fcd`** |
| `adr-design` | `local_b703f80a-cb41-408f-9087-ec847b630de1` | `docs/adr/` only — design-only, must not write code | running |

File ownership is disjoint by design: `scaffold` must not touch `docs/`, `adr-design` must not
touch anything outside `docs/adr/`.

### Merge 1 review — `claude/scaffold-8112ff` → `main`, 2026-11-26

Reviewed the diff myself rather than trusting the report. 13 files, 417 lines added.
What I verified independently:

- Both pinned GitHub Action SHAs re-resolved against the GitHub API and match their stated tags
  (`actions/checkout` v7.0.1 = `3d3c42e…`, `actions/cache` v6.1.0 = `55cc834…`).
- No attribution trailers in any of the five commits (the only `claude` matches are the
  `CLAUDE.md` filename and the `/.claude/` ignore rule — both legitimate).
- `make check` passes on *merged* `main`, not just on the branch: zero warnings, 3 unit tests,
  1 doc test, wasm target builds, deps-check OK at 2/2.
- `cargo tree --workspace` shows only the two workspace crates.
- The CLI exit codes behave as reported (0 for no args / `-V` / `-h`, 2 for an unknown argument).
- Rust 1.97.0 is real, installed, and the pin resolves to it (`rustc 1.97.0 (2d8144b78 2026-07-07)`).
- The release wasm is 43 bytes — correct for a `cdylib` with nothing exported yet, not a broken
  build.

Quality notes: `#![forbid(unsafe_code)]` on both crates, `#![deny(missing_docs)]` on the core,
`core::` rather than `std::` throughout the core, hand-rolled `#[non_exhaustive]` error enum with
`Display` + `core::error::Error`. `check-deps.sh` is POSIX, `set -eu`, and its failure message
names the ADR requirement.

## Reserved numbers

- ADR-0001 stack and dependency policy — `adr-design`
- ADR-0002 watermark algorithm and output quality modes — `adr-design`
- ADR-0003 PDF input rasterisation — `adr-design`
- ADR-0004 and up: unassigned.

## Merge queue

1. ~~`scaffold` → `main`~~ — merged as `ea30fcd`, `make check` green on main.
2. `adr-design` → `main` (docs only; based on `e8428a1`, touches only `docs/adr/`, so it will
   merge clean over `ea30fcd`). Waiting on the session, then on Samuel's approval of the ADRs
   before any implementation session starts.

## Research held at orchestrator level

An orchestrator-level research subagent verified the Rust/WASM crate landscape against primary
sources on 2026-11-26. Full conclusions were forwarded to `adr-design`. The load-bearing points:

- `hayro` 0.9.0 (Apache-2.0 OR MIT) is a **pure-Rust PDF rasteriser** on `vello_cpu`, no C, no
  `-sys` crates. This reverses my earlier assumption that no such thing existed. But: 0.x,
  single-maintainer, MSRV 1.92, robustness explicitly unfinished, and **wasm32 support is
  inferred from its manifest, not advertised** — must be built and proven before we rely on it.
- `hayro::render_into` draws a page into *our own* `vello_cpu::RenderContext`, which also has
  `glyph_run` and opacity layers with fonts as in-memory bytes. One rasteriser for page and
  watermark, no pixel readback.
- `mupdf`/`mupdf-rs` and imazen's `heic` are **AGPL-3.0** — ruled out.
- `pdfium-render` works on wasm32 as a second 4.43 MiB Emscripten module via JS glue. Trap: the
  `bblanchon/pdfium-binaries` wasm builds have a non-growable heap and OOM past a few pages; use
  `paulocoutinhox/pdfium-lib`. PDFium's LICENSE file carries both BSD-3 and Apache-2.0 text.
- `krilla` 0.8.2 takes JPEG bytes as `DCTDecode` passthrough: a 30-page watermarked scan is
  ~15–30 MB instead of ~500 MB of samples. Typst's PDF backend since 0.14.0.
- **HEIC has no good answer.** Native decode is Safari 17+ only, in no version of Chrome, Edge or
  Firefox. libheif-wasm's licence is unverified (likely LGPL-3.0); `heifer` is a pure-Rust decoder
  published 2026-11-26 with 5 stars. HEVC patents apply to every implementation.
- **Privacy bug to design out:** JPEG `DCTDecode` passthrough carries EXIF including GPS into the
  output PDF. Must be stripped deliberately.
- Memory: stream page-by-page (rasterise → composite → encode → append → drop, reusing one
  buffer) for ~50–100 MB peak. wasm linear memory never shrinks. `memory64` is unusable (no
  Safari support in any version).
- Cloudflare: 25 MiB per asset, `application/wasm` compressed by default. Whether `_headers` can
  set COOP/COEP is undocumented — so avoid wasm threads; use one terminable Web Worker.

My steer to `adr-design`: the lean rule is honoured by splitting the pipeline by path. The common
path (photo in → PDF out) is hand-rolled with near-zero third-party rendering code. PDF *input* is
the one unavoidable dependency and goes behind a trait plus a lazily-loaded module.

## Decisions taken

- 2026-11-26 — Build it. The gap is real: local-only processing plus non-destructive output.
- 2026-11-26 — Rust core → WASM + CLI; Astro + Svelte islands; Cloudflare; no backend.
- 2026-11-26 — Lean dependencies, enforced mechanically by a `deps-check` threshold in CI
  starting at zero third-party crates.
- 2026-11-26 — Design-only session writes the ADRs and stops; implementation goes to fresh
  sessions, one per PR, after Samuel approves.

## Waiting on Samuel

1. Public product name and domain (`filigrane` belongs to the government service).
2. Public or private GitHub repo under `samuelbagattin`, and when to create the remote.
3. Licence — `adr-design` will recommend one in ADR-0001; the choice is his.

## Carried forward from merge 1

- **`[profile.release]` sets `panic = "abort"`** (alongside `opt-level = "z"`, `lto`,
  `codegen-units = 1`, `strip`). The scaffold session chose this for wasm size and flagged it
  itself. It is a real design decision: with `abort`, a panic traps the wasm instance and it has
  to be recreated rather than recovered from, and `cargo test --release` cannot run
  `should_panic` tests. It fits the "one terminable Web Worker" design, but it needs ADR
  coverage rather than sitting in a profile block. Forwarded to `adr-design`.
- **CI `on: push` has no branch filter**, so a pushed branch that also has a PR runs the job
  twice (the concurrency group keys on `github.ref`, which differs between the two events).
  Harmless today — no remote — but fold `branches: [main]` into the next CI change.
- `crate-type = ["lib", "cdylib"]` on the core, `resolver = "3"`, and the CLI's exit code 2 for a
  bad argument were all the session's calls, all reasonable, none worth reversing.

## Backlog (not yet briefed)

- `filigrane-core` MVP: image in → watermarked PDF out, golden-image tests, CLI.
- WASM bindings + Astro UI with live preview.
- Metadata stripping.
- N-in-N-out and zip output.
- PDF input support behind a lazily-loaded renderer module.
- Accessibility audit (RGAA AA / WCAG AA) — a deliberate differentiator, so it gets its own pass.
- Comparison and docs pages for SEO.
