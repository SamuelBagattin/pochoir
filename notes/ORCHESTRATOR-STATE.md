# Orchestrator state — Pochoir

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

## Name and repository

- **The product is called Pochoir.** Domain `pochoir.app` (available as of 2026-11-26, not yet
  registered — that is Samuel's purchase to make, along with `lepochoir.fr` / `aupochoir.fr` if he
  wants a French domain; `pochoir.fr` itself is parked by a squatter).
  The metaphor is the algorithm: spray through a stencil and the paint is *in* the paper. That is
  our one technical commitment — the mark is burned into the pixels, not a removable overlay.
- Rejected along the way: `filigrane.app` (registered 2026-06-12 and dormant, and on my advice the
  name implies state endorsement and frames us as the knock-off), Tramage (too obscure even for a
  French native), Sauf-conduit / Antivol / Apposer (all available, passed over on taste).
- **Repo: https://github.com/SamuelBagattin/pochoir, public, default branch `main`.** Note the
  account's canonical casing is `SamuelBagattin`; the lowercase URL redirects.
- `main` is pushed and **the first CI run passed in 22s** (run 45877822956), so the workflow is
  verified against GitHub, not just locally.
- Sessions still must not push and must not open PRs. They commit to their branch; I merge and
  push.
- **Crates renamed 2026-11-26:** they are now `pochoir-core` / `pochoir-cli`, the CLI binary is
  `pochoir`, and the ADRs cite those identifiers. **The folder is still
  `~/dev/samuelbagattin/filigrane` and stays that way** — historical, deliberate, not to be renamed.

## Live sub-sessions

| name | id | owns | state |
|---|---|---|---|
| `scaffold` | `local_ae46d0e2-325d-4ddb-82b6-544820645fe1` | build skeleton | **merged into `main` as `ea30fcd`** |
| `licence-rename` | `local_b18a9d99-1c33-43a8-8992-c63b54ead64a` | `LICENSE*`, the `filigrane-*` → `pochoir-*` rename, ADR status flip, `scripts/check-deps.sh` + `scripts/allowlist/` | running; **opens a PR** (the remote exists now, so sessions push and PR, and I merge) |
| `adr-design` | `local_b703f80a-cb41-408f-9087-ec847b630de1` | `docs/adr/` only — design-only | **merged into `main` as `1a8303b`**; idle, told to stand down. Keep it rather than archive it: it holds the design context if Samuel asks for ADR changes |

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

- ADR-0001 stack and dependency policy — `adr-design`, merged, `Proposed`
- ADR-0002 watermark algorithm and output quality modes — `adr-design`, merged, `Proposed`
- ADR-0003 PDF input rasterisation — `adr-design`, merged, `Proposed`
- **ADR-0004 build reproducibility and deployed-bundle attestation — reserved, not yet briefed.**
  `adr-design` made the point that forced this, and it is the sharpest thing it said: publishing
  source proves nothing about what the user executes, because they run whatever the CDN served,
  and for a web-delivered tool those bytes can change silently on any deploy. A public repo
  therefore does not satisfy the audit claim, it only sharpens three requirements — no obfuscated
  or unmappable build (reconstructible from the repo with one documented command, source maps
  shipped), reproducible builds with a published hash of the deployed bundle, and the strict CSP as
  checkable evidence. ADR-0002's determinism work earns its keep twice here: it is also what makes
  an independent rebuild comparable at all. Promoted from "someday" to near-term.
  Put the **wasm artifacts** at the centre of it: that is what a reviewer is least able to eyeball
  and where a substituted build would be hardest to notice. A hash over the HTML and JS while the
  wasm goes unattested would be security theatre.
- ADR-0005 and up: unassigned.

## Merge queue

1. ~~`scaffold` → `main`~~ — merged as `ea30fcd`, `make check` green on main.
2. ~~`adr-design` → `main`~~ — merged as `1a8303b`, then the count correction as `430256d`. ADRs are `Proposed`.
3. ~~PR #1 `licence-rename` → `main`~~ — merged as `e5a692f`. Licence texts verified against
   gnu.org / apache.org / spdx.org myself: AGPL byte-identical, Apache and MIT differing only in
   the copyright line. ADRs 0001–0003 now `Accepted`. Binary is `pochoir`. Per-crate guard live
   and passing on merged `main`.
4. ~~PR #2 `licence-rename` follow-up~~ — merged as `80de01f`. ADR-0005 Licensing is `Accepted`
   and supersedes ADR-0001 in that respect only; `LICENSE.md` and `CLAUDE.md` point at it. CI
   `push` is now filtered to `main`: verified, that branch produced exactly one run where PR #1
   produced two for one commit. ADR-0005 added a cost I had not stated and should have: **a hosted
   competitor can take the engine, the hard and valuable part, write its own front end and owe
   nothing** — §13's reach really is narrower under the split than under AGPL throughout. Its
   revisit trigger: if nobody has embedded `pochoir-core` within a year, the permissive half bought
   nothing.
5. **Open:** `licence-rename` follow-up 3 — write the ten answered open questions back into
   ADR-0001/0002/0003 and tick them. My bookkeeping failure: I settled six on 2026-11-24..26 and
   recorded them only in this file, so the ADRs still say "Needs decision from Samuel" for
   decisions the orchestrator took. Also correcting any stale 89/99 crate figures in ADR-0003 and
   allowing an open question to be closed in place after acceptance when the answer does not change
   the decision.
6. **Proposed, not started:** `core-write-path` (task_f7faf84f) — engine phase 1, image in → PDF
   out, no watermark. The harness refused a direct start because that turn was triggered by
   another session's message, so it is a card awaiting Samuel's click.

### Session ownership while both run

- `licence-rename` owns `docs/adr/` and `.github/`.
- `core-write-path` owns `crates/` and `scripts/`.
  Disjoint on purpose: `core-write-path` makes the first dependency bump, which rewrites
  `scripts/allowlist/*`, and that must not collide with a docs change.

### Merge 2 review — `claude/adr-design-238adf` → `main`, 2026-11-26

5 commits, 1 917 lines, only `docs/adr/`, no attribution trailers, scope respected. What I
verified myself rather than taking on trust:

- **hayro 0.9.0 + krilla 0.8.2 do build for `wasm32-unknown-unknown`** — I built a probe crate on
  **rustc 1.97.0**, the version this repo pins (the session measured on 1.96.0), release profile
  with `opt-level = "z"`, LTO and `panic = "abort"`: clean build in **22.99s**, no flags, no C
  toolchain. `rayon` absent from the resolved tree. This was the claim that could have invalidated
  ADR-0003, and it holds.
- **Crate counts — settled in merge 3, and my own figure was the irrelevant one.** I first measured
  hayro + krilla at 103 unique crates against the session's 89. Both were beside the point:
  **ADR-0001 hand-rolls the PDF writer, so `krilla` is not adopted at all.** The session chased the
  discrepancy, found its probe had disabled krilla's default features, and re-measured the set we
  actually take. I verified every figure myself on rustc 1.97.0 with
  `cargo tree --target wasm32-unknown-unknown`, default features:

  | Set | Unique | `[[package]]` |
  | --- | --- | --- |
  | `zune-jpeg` + `png` (common path) | 11 | 14 |
  | `hayro` alone | 74 | 81 |
  | `hayro` + `zune-jpeg` + `png` (full adopted set) | 74 | 81 |

  **Adding the two common-path crates to `hayro` adds zero crates**, because `hayro` depends on
  `image`, which already pulls both. The common path's 11 are `adler2 bitflags cfg-if crc32fast
  fdeflate flate2 miniz_oxide png simd-adler32 zune-core zune-jpeg`, and I confirmed every one of
  them already appears in hayro's closure. So the guard goes **2 → 14 → 81**, not 88.

  The useful consequence, now recorded in ADR-0001: **the core module's dependency closure is a
  strict subset of the PDF module's.** The per-crate allowlist nests instead of diverging, so a
  crate appearing in the core that is not in that 11 is unambiguously new — a cleaner CI assertion
  than comparing two independent lists. And the allowlist and threshold must still be **generated
  by measurement in CI**, never copied from a document; ADR-0001 now cites its own retracted "88"
  as the worked example of why.
- **hayro's README is wrong about encryption, as the ADR says.** `hayro-syntax-0.9.0/src/crypto/`
  ships `aes.rs`, `rc4.rs`, `md5.rs`, `sha256/384/512.rs`, and `new_with_password` is in `pdf.rs`.
  This matters: French administrative PDFs are routinely owner-password encrypted.
- **Licences re-checked against crates.io:** `zune-jpeg` MIT/Apache-2.0/Zlib, `png` MIT/Apache-2.0,
  `hayro` and `krilla` MIT/Apache-2.0, and `jpeg-encoder` is indeed the conjunctive
  `(MIT OR Apache-2.0) AND IJG`. **No AGPL anywhere in the proposed tree**, so Samuel's licence
  choice stays unconstrained — which is exactly what ADR-0003 argued when it rejected MuPDF.

Two places the session improved on my own steer, recorded so I do not re-impose the worse version:

- Its dependency principle is better than my "hand-roll the common path" framing: **hand-roll
  everything that writes; depend deliberately for parsers of hostile input.** Hand-rolling a
  decoder swaps a heavily-fuzzed parser for a less-fuzzed one, which is not a security win.
  Encoders and writers process our own data, so a bug there is a broken file, not a compromise.
- It measured the alternative instead of assuming: the "just take small crates" option for the
  common path is **149 703 B brotli across 34 crates**, against a hand-rolled estimate of
  60–120 KiB. So hand-rolling buys 30–90 KiB, and the ADR says plainly that **client bytes are the
  weakest of the three tests here** — control and 34 upstreams are the real arguments. I had
  overweighted bundle size.
- It rejected my `hayro::render_into` + `vello_cpu` single-pass idea, correctly: it would weld the
  common path to the PDF renderer's dependency tree and leave us two watermark implementations to
  keep pixel-identical. The compositor stays path-independent on an RGBA pixmap.

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

## Decisions I took on the ADR open questions, 2026-11-26

The design session listed seven items as needing Samuel. Six of them are mine under his own
standing rule ("ask me only for real decisions: product direction, prod approvals, spending,
legal, anything irreversible"), and all six are reversible behind a trait or a flag. Taken:

1. **Adopt `hayro`** for PDF input, behind the `PageSource` trait, in a lazily-loaded second wasm
   artifact, with `pdfium-render` as the named fallback. 103 crates is a real concession to the
   lean rule, but a watermarker that cannot read a PDF cannot replace Filigrane Facile, and the
   trait plus fallback makes it reversible. The photo path never downloads it.
2. **HEIC: ship no HEVC decoder.** Browser-native decode only, which in practice means Safari 17+,
   with an explicit "convert to JPEG and retry" message everywhere else. This is also what
   dissolves the patent question — there is no royalty exposure in a decoder we do not ship — so
   that item never needs to reach Samuel as a legal decision.
3. **Hand-roll scope accepted as written** (~3 000–3 500 lines), with the TrueType glyph
   rasteriser **built last** so it can never block a release. If it slips, `ttf-parser` + `fontdue`
   is the escape hatch at 54 KiB and 8 crates, and that swap needs its own ADR.
4. **hayro's `embed-fonts` + `embed-cmaps` stay on** (+519 KiB brotli, 39% of that module) until a
   real-document corpus shows they are not needed. A PDF that renders with the wrong glyphs is a
   worse failure than a bigger lazy module.
5. **`overlay` mode does not ship in v1.** It is the weaker mode, it is the one that carries EXIF
   into the output, and shipping two security postures on day one invites the user to pick the
   wrong one.
6. **The 25% exclusion-zone cap is hard in v1**, revisited only with evidence. A mark you can
   shrink to nothing is not a mark.
7. **Default mark text: none pre-filled.** Instead, one-tap presets (location, banque, mutuelle,
   employeur, administration) that insert "Document destiné exclusivement à <purpose> — <date>".
   The government tool defaults to its own URL, which is useless to the user; an unchanged default
   is worse than a prompt.

Also accepted from the session's feedback, to be briefed with the first implementation session:

- **`scripts/check-deps.sh` becomes per-crate.** A workspace-global `[[package]]` count stops
  protecting anything the moment hayro raises it by ~100. Keep the global number as a blunt alarm,
  and add a hard assertion that the common-path crate's `cargo tree --target wasm32-unknown-unknown`
  stays inside an explicit allowlist **generated by measurement**.
- The CSP needs **`'wasm-unsafe-eval'`** in `script-src`, or the module will not instantiate.
- Someone must quote **ISO 32000-1 §7.4.8** for `/DCTDecode` verbatim-JPEG embedding. ADR-0002's
  quality modes depend on it and the citation is currently missing.
- **Mobile Safari's wasm memory ceiling must be measured on a device** before we promise any page
  count. The ~2 GiB figure in the ADR is a synthesis, not a vendor limit.

## Decisions taken

- 2026-11-26 — Build it. The gap is real: local-only processing plus non-destructive output.
- 2026-11-26 — Rust core → WASM + CLI; Astro + Svelte islands; Cloudflare; no backend.
- 2026-11-26 — Lean dependencies, enforced mechanically by a `deps-check` threshold in CI
  starting at zero third-party crates.
- 2026-11-26 — Design-only session writes the ADRs and stops; implementation goes to fresh
  sessions, one per PR, after Samuel approves.

## Waiting on Samuel

1. ~~Product name~~ — **Pochoir**, decided 2026-11-26.
2. ~~Public or private repo~~ — **public**, created and pushed 2026-11-26.
3. **Registering `pochoir.app`** — his purchase, not mine to make. Availability was verified, not
   reserved, so it can go at any time.
4. ~~Licence~~ — **decided 2026-11-26: split licensing.** `pochoir-core` and `pochoir-cli` under
   `MIT OR Apache-2.0`; the web app and site under `AGPL-3.0-or-later`. Rationale: AGPL §13 still
   catches the fork-and-host-it case, which is the harm this product exists to prevent, while the
   engine stays usable by the integrators we identified as a real gap (MTES-MCT's own `potentiel`
   repo calls the government API; a browser-extension author had to scrape it) and by anyone
   packaging the CLI — public-sector and corporate projects frequently cannot take AGPL at all.
   ADR-0001 recommended AGPL throughout; Samuel took the split instead. **Not yet implemented:**
   two `LICENSE` files, per-crate `license` fields replacing `UNLICENSED`, and a README statement.
   Until that lands the public repo is legally all-rights-reserved. Folded into the first
   implementation session as its opening commit.
5. ~~Approving the ADRs~~ — **approved 2026-11-26**, after a summary of the essentials. The
   `licence-rename` session flips all three from `Proposed` to `Accepted`.

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

- ~~**Rename `filigrane-*` → `pochoir-*`**~~ — done 2026-11-26 in the `licence-rename` session,
  together with the licensing decision and the per-crate dependency guard.
- `pochoir-core` MVP: image in → watermarked PDF out, golden-image tests, CLI.
- WASM bindings + Astro UI with live preview.
- Metadata stripping.
- N-in-N-out and zip output.
- PDF input support behind a lazily-loaded renderer module.
- Accessibility audit (RGAA AA / WCAG AA) — a deliberate differentiator, so it gets its own pass.
- Comparison and docs pages for SEO.
