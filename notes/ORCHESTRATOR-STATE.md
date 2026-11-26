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

## The rotation, and what came with it

The engine session had already started phase 2 before my rotation message reached it, and parked
the work rather than binning it. I kept it: **`claude/watermark-mark` merged to `main` as
`3ae2ee0`** — a Roboto Regular subset (21 052 bytes, 347 glyphs) with its licence and provenance,
`scripts/subset-font.py`, and `make font-subset`. I verified the part that would have been
expensive to get wrong: `googlefonts/roboto` (now `roboto-2`) is Apache-2.0 with a single LICENSE
and no OFL, and upstream `src/hinted/Roboto-Regular.ttf` is 515 100 bytes with sha256
`56a45233…664d`, matching the README exactly. The reasoning for Roboto over the better-covered OFL
families is the valuable part: a subset is a derivative work, so OFL's Reserved Font Name clause
would need reasoning about, while Apache-2.0 adds no licence family we do not already carry.
It also wrote ~540 lines of TrueType parser and **deliberately kept it out of the repo** because it
was never compiled — it sits in the notes folder labelled as notes. Right call.

### My rulings on ADR-0002's seven collisions with reality

The hand-over's section 8 found genuine defects in our own specification. Decided by me, to be
recorded by the phase-2 session as **ADR-0007**, scoped to supersede ADR-0002 on items 2, 3, 4
and 7 only:

1. One column of text at a realistic mark length **is** the intent; the horizontal gap rarely
   engages and the 6–10 repetitions are rows. Note it, change nothing.
2. **Long text wraps, then shrinks, then refuses — never clips.** Clipping "Document destiné
   exclusivement à une demande de location" into "Document destiné exclusiv" silently changes what
   the document asserts. For this product that is a correctness bug, not a cosmetic one.
3. **Blur keyed to font size**, not pixels: the spec's "at render resolution, scaled with DPI"
   contradicts itself and rounds to a no-op at its own default. Plus a test that the default blur
   demonstrably changes pixels. Still described as antialiasing, never as a defence.
4. **Per-tile jitter, not one global offset.** The ADR's letter allows the weaker version; its own
   justification ("a remover tuned to fixed tile positions fails") demands the stronger one.
5. `--seed` / `--no-jitter` exist in the spec but not in `OutputOptions`; add them in the same
   commit as the mark.
6. Add the golden-hash hook.
7. **NFC output must stay inside the font subset** — a test that the composition table's outputs
   are all in the subset's `cmap`, and runtime validation that names unsupported characters rather
   than drawing tofu.

Build-order instruction carried from the hand-over, because it is right: **rasteriser and a PGM
dump before any layout, and `é` rendering on day one** — it is a composite glyph, so composite
parsing is on the critical path, and until it renders the NFC decision is untestable.

### Also noted from the hand-over

- `LICENSE.md` may want a line recording the bundled Apache-2.0 font. That is Apache-2.0 §4
  attribution bookkeeping rather than a policy change, so it can be added — but ADR-0005 makes
  licensing Samuel's, so tell him rather than deciding the framing.

## ADR-0004 merged as Proposed (`17ba5d6`), 2026-11-26

Not `Accepted` — six open questions, four of them Samuel's, and the README holds a document at
`Proposed` until they are answered.

**The headline reverses my own expectation and theirs:** no container is needed. Our wasm build is
reproducible today for two `RUSTFLAGS` entries and no new tool — two checkouts at different
absolute paths already produce byte-identical output, because Cargo hashes a workspace member's
package id relative to the workspace root. I confirmed `trim-paths` is `-Z` only on 1.97.0, so my
premise there needed the correction they gave it.

**Three findings that bind code nobody has written yet.** Record them in `CLAUDE.md` when the
bindings land, because none of them announces itself:

1. **The wasm crate must be a workspace member.** Their first probe had `pochoir-core` as a path
   dependency outside its workspace root, an absolute path entered `-Cmetadata`, the disambiguator
   moved with the directory, every symbol was renamed and the module reordered — same size, 11 799
   bytes different. This binds ADR-0003's binding crates and nothing in the output warns you.
2. **Content-addressed asset filenames are a requirement, not a caching nicety.** Cloudflare can
   serve HTML and assets from different versions mid-rollout, and SRI fails closed when they
   disagree.
3. **`make wasm` currently emits a 43-byte empty module.** `pochoir-core` exports nothing and
   cannot carry `#[no_mangle]` while `lib.rs` has `#![forbid(unsafe_code)]` under edition 2024.
   That tension has to be resolved when the bindings land — a separate binding crate, or
   `#[unsafe(no_mangle)]` with a scoped allow. There is nothing to attest until then.

**On the mechanism I could not reproduce myself:** they measured 28 host paths surviving
`strip = true`. My own probe failed — the first had no dependencies and so no registry paths to
leak, and my later attempts did not compile. I am accepting their number, because the mechanism is
sound on its face (`core::panic::Location` strings are `file!()` in the data section, not
debuginfo, which is exactly why `strip` cannot touch them) and their note documents a reproduction
recipe. Flagged as *their* measurement, not mine.

**The one open question that could flip Decision 1** is whether a Linux x86_64 host emits the same
wasm as this Mac. Docker's daemon was down and neither of us started it unasked. It becomes nearly
free the day the bindings produce a non-empty module, because CI already runs on `ubuntu-latest` —
the measurement can be a CI job rather than a chore.

**Prior art worth remembering:** Meta's Code Verify is alive but covers JS and CSS, **not
WebAssembly**, and proves consistency rather than source-to-binary correspondence; "install it as a
PWA and you are pinned" is marketing, since service workers soft-update on navigation — the only
genuine pin is a downloaded build or the CLI, **which gives the CLI a second reason to exist**; and
WAICT (Mozilla, Cloudflare, FPF, Meta; behind a pref in Firefox Nightly) has extending SRI to
WebAssembly on its roadmap, i.e. the exact hole we are working around.

**The ceiling, which the ADR states rather than hides:** for a web app served from infrastructure
we control there is no mechanism that fully closes the gap, every root of trust we can offer
arrives through the channel it is meant to police, verification is retrospective and voluntary, and
a compelled maintainer holds every key. What a published hash changes is what can be done
*quietly*. The session looked for a legal argument that published hashes constrain a compelled
code-change order and found none — the defensible claim is purely technical.

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
- ADR-0005 licensing — merged, `Accepted`
- ADR-0006 image page size cap — merged, `Accepted`
- **ADR-0007 watermark specification corrections — reserved for the phase-2 session**
- ADR-0008 and up: unassigned.

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
5. ~~PR #3 `licence-rename` follow-up 3~~ — merged as `e000ca4`. All ten open questions closed in
   place with the decider named, the original question text re-added verbatim under *As raised:* so
   the diff is provably lossless, and the README now allows closing in place only when the answer
   does not change the decision. The only unticked box left under `docs/adr/` is in the template.
   **It declined the conditional correction I asked for, and it was right**: ADR-0001's remaining
   `88` is its own deliberate citation of the wrong figure — the worked example itself — and
   ADR-0003's `89` is a Firefox version. Declining and flagging beats complying.
   It also recorded a cost unprompted: ADR-0002's risk table lists `overlay` as part of the
   mitigation for the accessibility regression, so deferring `overlay` past v1 leaves that
   mitigation weaker in v1 than the table implies — UI disclosure alone. Does not reverse the
   decision; does stop it resurfacing later as "the ADR said we had a mitigation".
   **Lesson for me:** a decision recorded only in this file is a decision the next session cannot
   find. Record orchestrator decisions in the governing document, not just here.
6. ~~PR #4 `core-write-path`~~ — merged as `f322640`. **The engine's write path exists**: image
   in, PDF out, no watermark. 5 307 lines, zero-to-two dependencies, 118 tests.

### Merge 4 review — what I reproduced rather than read

- **The allowlist is a true measurement.** `./scripts/check-deps.sh --update --all` run by me
  reproduces both committed files byte for byte, and `THRESHOLD=15` equals the lock count. The
  gap from ADR-0001's predicted 14 is `miniz_oxide` locked twice plus an unenabled `zlib-rs`
  flate2 backend — neither in the closure, which is why check 2 exists.
- **ISO 32000-1 verified from the Adobe copy I downloaded.** sha256 matches the session's
  (`9de0ca9e…17ff`); all three passages verbatim. **My own refinement, stronger than what the
  session claimed:** §7.4.8's footnote (a) ends **"PDF DCT Encoding shall exactly follow those
  rules established by Adobe for PostScript"** — a `shall` that incorporates Adobe TN #5116 by
  reference. So the APP14 CMYK inversion is normative *through* §7.4.8, not merely a convention
  we are right to follow. That is the sentence ADR-0001's note should cite.
  **Two corrections to my own account of this.** (a) I cited "extracted lines 3021-3022". Line
  numbers from a `pdftotext` run are not a citable location — the engine session's extraction puts
  §7.5.4 there, mine puts §7.4.8, and both are internally consistent. Cite the clause, never the
  extraction. (b) **Neither of us has read TN #5116.** §7.4.8 makes its rules binding; what those
  rules actually say about inverted CMYK storage is still inferred from the observed behaviour of
  Photoshop-written files. The incorporation is verified; the incorporated content is not. Someone
  should read the note before the ADR claims more than that.
  Bonus from the same extraction, corroborating the writer: §7.5.4 requires a cross-reference
  entry to be "exactly 20 bytes", which is what our `%010d %05d n \r\n` format satisfies and what
  a test already asserts.
- **End to end on a real 1512×806 JPEG:** byte-identical across two runs; poppler reports the
  embedded image as `1512x806 rgb 8bpc jpeg 200 ppi`, so no resampling; `pdfinfo` gives `Pochoir`
  for both strings, no dates, no metadata stream; `Exif`/`GPS`/`CreationDate`/`ModDate`/`xpacket`/
  `/Metadata` all absent. Rendered with `pdftoppm` and looked at: upright, undistorted, legible.
- **Code read, not skimmed.** The image dictionary is derived from the datastream's own frame
  header rather than the caller, so `/ColorSpace` and `/BitsPerComponent` agree by construction.
  Quantisation tables are T.81 Annex K.1/K.2 in natural order, zigzag is Figure A.6, quality
  scaling is the IJG mapping.
- I nearly accused the session of editing this file; it had not. `git diff main <branch>` compares
  trees, so it showed my own newer commit as the branch's "change". Diff from the merge base.

### Open after merge 4

- ~~**For Samuel:** the 20-inch page~~ — **decided 2026-11-26: cap image pages at A4, but never
  enlarge to it.** A 600 dpi scan of an ID card at 85 x 54 mm stays that size; capping only ever
  shrinks. PDF inputs unchanged, `max(stated, requested)` unchanged. In flight with the phase-1
  session, along with the ADR-0002 amendment. The deciding argument, worth keeping: the bytes are
  identical at every DPI (measured: 954 243 bytes for a 4032x3024 frame, only `/MediaBox` moving),
  so nothing about quality was at stake — while ADR-0002 itself says the DPI knob "is meaningful
  only for PDF inputs". We were letting a knob our own design document calls meaningless decide the
  output's most visible physical property.
- **Decided 2026-11-26: output stays PDF-only for now; a user-chosen output format is a later
  feature, not current scope.** Samuel's call. The limitation is real and should not be forgotten:
  plenty of French administrative and rental portals demand an image and reject PDFs ("format JPG,
  2 Mo maximum"), so somebody needing a watermarked JPEG back cannot use Filigrane Facile *or*
  Pochoir today. The mark is burned into pixels, so the PDF is only a container and handing back
  the JPEG we already encoded is strictly less work. When it is picked up: match the input by
  default, PDF always available explicitly, PDF forced for PDF inputs and for merges; it needs its
  own metadata-stripping assertion, since a directly-emitted JPEG has no PDF wrapper standing
  between the EXIF question and the user.
- Revisit later, not now: decoder errors are reduced to `&'static str` to keep `Error: Copy`, which
  was a scaffold-era doc comment rather than an ADR requirement. Losing the decoder's message will
  cost someone a debugging session.
- ADR-0002's "same goldens on both targets" is **unwired** — nothing runs the suite under a wasm
  runtime, only the build. The determinism guarantee is half-tested, and there is real arithmetic
  in the tree now.
- The Adobe-CMYK `/Decode` path is tested only against a synthetic frame header.
- `/FlateDecode` against RFC 1950/1951 is still uncited.
- ADR-0001's `/Decode` "could not verify" bullet can now be closed, with the TN #5116 sentence.

### Running now (two sessions, disjoint files)

| session | id | owns |
|---|---|---|
| ~~engine~~ | `local_79c8214b-fbda-4eb6-adc4-d0f6590b8dbe` | **rotated out at 500k**; wrote `phase2-handover.md` (347 lines) and stood down |
| phase2-mark | `local_fb5b2705-27b3-4196-86e8-68a2a0ac87f5` — **running** | `crates/`, `scripts/`, `docs/adr/0007-*` |
| ~~adr-0004-attestation~~ | `local_082f38c0-9661-4149-b091-797afa5b08a3` | done, PR #6 merged as `17ba5d6` |

Only shared file is the ADR index README, one row each — a trivial conflict I resolve at merge.
~~Phase 2 waits for the A4 PR~~ — PR #5 merged as `a5048ad`, phase 2 briefed to the same session.

### Merge 5 review — the A4 cap

Reproduced every row through the release CLI rather than reading the table: 512 x 384 mm becomes
280.0 x 210.0 mm; `--page-size native` still gives 512.1 x 384.0; and the never-enlarge rule holds
where it counts — a 2022x1276 scan at 600 dpi comes out at **85.6 x 54.0 mm**, an identity card at
life size. **My first attempt at that case was worthless**: `sips` silently refuses to write the
DPI, so the fixture never approached the cap and the test proved nothing. Redone with `--dpi 600`.
Worth remembering — the obvious way to build that fixture does not work.

Two things the session did better than my brief. It made a change I had not asked for and should
have: **the embedded JPEG's JFIF density now follows the page rather than the request** — measured
366 dpi capped against 200 native, same 4032x3024 pixels — because on a capped page the sheet
shrinks while the pixels do not, so a datastream claiming 200 would be false. It only shows when
somebody extracts the JPEG, since a PDF reader scales by the CTM and ignores density, but then it
shows. And on the ADR route I said "your call" when the README actually decides it: a changed
decision takes a superseding ADR, not an amendment. Hence ADR-0006, scoped to image page geometry,
with ADR-0002's reasoning untouched beneath a dated pointer.

It also **declined to put my own 954 243-byte measurement in the ADR**, because it had not measured
it, and cited its own 837 426-byte fixture instead — then proved the stronger claim with a test:
capped and native decode to byte-identical pixels, the datastreams differing only in the density
field. Not quoting a number you have not measured is the right instinct, and that is three times
now a session has been right to push back.

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

## Context budget (checked 2026-11-26, after Samuel reminded me)

| session | context | action |
|---|---|---|
| orchestrator (me) | 499 157 / 1 000 000 — **50%** | watch; messages are 40% of it |
| phase-2 engine | 499 649 / 1 000 000 — **50%** | **rotating now**, before phase 2 rather than during it |
| adr-0004-attestation | 239 279 / 1 000 000 — 24% | fine, finishing a docs task |

The standing rule is rotate at ~300k. I had never looked at the number until Samuel asked, and the
engine session was at 500k with the project's largest implementation just briefed to it. Rotation
is cheap at a boundary and expensive mid-task, so: the engine session writes
`phase2-handover.md` to the shared notes folder and stands down; a fresh session takes phase 2
from that note. **Check this after every couple of merges, and always before briefing a big
phase.**

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
