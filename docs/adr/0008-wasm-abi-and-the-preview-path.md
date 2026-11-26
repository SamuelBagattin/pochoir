# ADR-0008: The WebAssembly ABI, and how the preview is rendered

- **Status:** Proposed
- **Date:** 2026-11-26
- **Deciders:** the implementing session, for Samuel Bagattin's review
- **Supersedes:** nothing. It settles questions
  [ADR-0002](0002-watermark-algorithm-and-quality-modes.md) and
  [ADR-0004](0004-build-reproducibility-and-deployed-bundle-attestation.md)
  left for the binding layer, and records one contradiction inside ADR-0002
  that implementing the bindings exposed.

## Context

The engine has worked end to end since the watermark landed: a photo goes in and
a watermarked PDF comes out, byte-identical across runs. None of it was reachable
from a browser. `make wasm` built `pochoir-core`, which exports nothing but
`memory`, and produced a 43-byte module.

This ADR covers `crates/pochoir-wasm`, the crate that closes that gap, and the
decisions that could not be read off an existing document. The ABI's *shape* —
the ten exports, the packed 48-byte options struct, the six status codes — is not
among them: it was agreed with the browser side before this work began and is a
requirement here, not a choice. What follows is everything the shape did not
already settle.

Three things in the brief for this work turned out to be wrong when measured
against the code. They are recorded in Decision 2 and in the appendix, because
the way they were wrong is the useful part.

## Decision 1 — The bindings are their own crate, and it is a workspace member

**`crates/pochoir-wasm`, a `cdylib`, a member of the root workspace, with
`pochoir-core` as its only dependency.**

Two separate forces put the exports in their own crate, and either alone would
be enough:

- **`#![forbid(unsafe_code)]` in `pochoir-core`.** Under edition 2024
  `#[no_mangle]` is an unsafe attribute, spelled `#[unsafe(no_mangle)]`, and
  `forbid` — unlike `deny` — cannot be lifted by an inner `allow`. That is the
  point of `forbid`, and it is a property of the core worth keeping. So the
  exports cannot live there. This crate sets `#![deny(unsafe_code)]` at its root
  and attaches the allowance to a single module, `abi`, which does nothing but
  marshalling: slices in, status out, no pixels and no policy. Every other
  module in the crate is ordinary safe Rust and is unit tested on the host.
- **ADR-0004 Decision 1, constraint 1.** The crate must be a member of the
  workspace being built. Its finding 5 measured what happens otherwise: a
  `pochoir-core` path dependency reaching outside the workspace root puts an
  absolute path into `-Cmetadata`, which renames every symbol, reorders the
  module through LLVM, and produces two artefacts of identical size with 11 799
  bytes different — with nothing in the output to say so. The `members` line in
  the root `Cargo.toml` carries a comment saying this, and `make wasm-repro`
  asserts it rather than trusting it.

**No `wasm-bindgen`.** ADR-0001's dependency rule is *hand-roll everything that
writes; depend deliberately only for parsers of hostile input*, and generated JS
glue is neither. It would also breach ADR-0004 Decision 1's constraint 3 — no
build step that is not measured deterministic — since `wasm-bindgen` had genuine
nondeterminism in its import ordering as recently as 2024. Ten `extern "C"`
functions over linear memory cost less than the tool would.

The measured result: **the crate adds zero third-party dependencies.**
`scripts/allowlist/pochoir-wasm.txt` is byte-identical to `pochoir-core`'s —
the same eleven crates, all reached through the core. `THRESHOLD` in
`scripts/check-deps.sh` moves 15 → 16 for the new workspace member and for
nothing else.

## Decision 2 — A quality mode with no engine behind it fails; it is not approximated

**`compact` and `lossless` return `BAD_ARGUMENT`. They are not mapped onto a
nearby quality number.**

ADR-0002 Decision 2 defines four modes. Measured against the engine as it stands:

| mode | what ADR-0002 asks for | can the engine do it |
| --- | --- | --- |
| `compact` | downsample to a ≤ 150 DPI equivalent, JPEG q75 | **no** |
| `standard` | native 1:1, JPEG q85 | yes |
| `high` | native 1:1, JPEG q92 | yes |
| `lossless` | native 1:1, PNG `/FlateDecode` | **no** |

`lossless` is the simple case: `pdf::PdfWriter` exposes `add_jpeg_page` and
nothing else, and there is no PNG encoder in the crate. `pochoir-core`'s own
module documentation lists it under "Not yet here".

`compact` is more interesting, because it is not merely unimplemented — **it
contradicts ADR-0002**. The same decision's "Resolution: do not resample image
inputs at all" says, in bold, that for bitmap inputs "we composite at the
source's native pixel dimensions and never resample", on the grounds that
resampling "can only upsample (pure waste) or downsample (pure loss)". The
mode table's image-input cell for `compact` asks for exactly the downsample the
section above it forbids. One of the two has to give, and this ADR does not
decide which: it records that `compact` cannot be implemented without reopening
that question, and refuses it in the meantime.

**Why refusal rather than approximation.** A user who picks `lossless` and is
quietly handed JPEG q92 has been told something untrue about their own document.
This is the same rule ADR-0007 Decision 2 applied to over-long mark text — "a
mark that cannot be drawn honestly is not drawn at all" — and it applies here
for the same reason: the product's whole claim is that what the document says
about itself is true.

**A corollary, and the reason `high` works.** ADR-0002's DPI column is headed
"Raster DPI (PDF input)" and its image-input column says "native, 1:1". So for
the image inputs this engine accepts, a mode chooses the **encoding only**; the
DPI comes from the options struct. `standard` and `high` therefore differ in
exactly one thing, JPEG quality 85 against 92, and both are expressible in
`OutputOptions` as it stands.

Scope for the two missing modes is Samuel's call. Implementing `compact` needs
the ADR-0002 contradiction settled first; `lossless` needs a PNG encoder, which
is a hand-rolled writer in the terms of ADR-0001 and a larger piece of work than
it sounds.

## Decision 3 — The preview is rasterise, derive the seed, downscale, compose, in that order

**The order is the decision.** Any other produces a preview that is wrong about
something.

The brief for this work said the preview needed no new code, because
`Pixmap::resize` and a lower-DPI render were already available. Neither is what
it looks like:

- **`Pixmap::resize` is not a scaler.** Its own documentation: "The contents
  afterwards are zeroed rather than preserved: this exists to recycle memory
  between pages, not to scale an image."
- **A lower DPI does not produce fewer pixels.** For an image input
  `PageSource::rasterise` ignores the DPI entirely — that is ADR-0002's
  no-resampling rule, made structural — so the DPI changes the page's size in
  points and nothing at all about the bitmap.

So there is no downscaler anywhere in the project, and the preview cannot honour
`max_px` without one. `crates/pochoir-wasm/src/scale.rs` is that downscaler: an
alpha-weighted box average, `Σ(c·a)/Σa` for colour and `Σa/n` for alpha, in `u64`
integer arithmetic so the preview is byte-identical on every target, in the same
spirit as `mark::trig`'s hand-rolled sine.

The pipeline:

1. **Rasterise at full resolution.** The same call the PDF path makes, so the
   preview sees the same pixels and the same EXIF orientation.
2. **Derive the jitter seed from those full-resolution pixels**, before anything
   is discarded. `mark::derive_seed` fingerprints the pixmap it is handed, so
   seeding from the downscaled bitmap would give the preview *different tile
   positions from the document it is previewing*. There is a test asserting the
   two fingerprints differ, which is why the ordering is load-bearing rather
   than tidy.
3. **Downscale, then compose.** Composing first and shrinking afterwards is
   equally faithful but does the full-size glyph raster and blur that a preview
   exists to avoid.

**The fidelity claim, measured.** Every geometric quantity of the mark is a
ratio of the page's shorter edge, so the preview is a scale model rather than an
approximation. Across a 64:1 range of page areas:

```
800x600   font/short 0.03500   block_width/short 0.43509
400x300   font/short 0.03500   block_width/short 0.43509
200x150   font/short 0.03500   block_width/short 0.43509
100x75    font/short 0.03500   block_width/short 0.43509
```

Identical to five decimal places, and asserted to a relative tolerance of 1e-4.

One figure is **not** invariant and is deliberately not asserted: the tile count,
which runs 21, 22, 24, 24 over those same four sizes. `mark::placements` bounds
the lattice with `.floor() - 1` and `.ceil() + 1` — a margin of one whole tile
per axis, a fixed *number* of tiles rather than a fixed fraction of the page. On
a grid three or four tiles wide a one-step rounding difference is a large
relative change in the product. Those extra candidates fall outside the page and
composite nothing, so `Composition::tiles` counts candidates, not marks the user
sees. Worth writing down because the obvious test to write is an equality on
that field, and it fails for a reason that is not a bug.

### Where the resampler belongs

It is in `pochoir-wasm` because the engine has no use for one: ADR-0002 forbids
resampling on the output path and the CLI has no preview. If a second client
ever needs a preview, or if `compact` is ever implemented, it should move to
`pochoir-core` — it is image processing, and that is the crate that owns image
processing. Recorded as a known piece of misplacement rather than discovered
later as a surprise.

## Decision 4 — Two smaller calls the wire format forced

**A seed of `0` means "derive one from the document".** The struct has no
"absent" marker and the browser's default is `0`, while
`OutputOptions::seed` is an `Option<u64>` whose `None` derives per document, as
ADR-0002 Decision 6 requires. Reading `0` as a literal seed would give every
document in the world the same tile displacements — precisely the property the
jitter exists to deny an attacker holding two marked copies. The cost is that an
explicit literal seed of zero is not expressible over this ABI; any other `u64`
is, and the CLI can still ask for it.

**Empty mark text is `BAD_ARGUMENT`, not a bare render.** `OutputOptions.mark`
is an `Option` because the engine also serves an unmarked write path, but an ABI
whose only job is watermarking must not hand back an unmarked document to a user
who believes they marked it. Same rule as Decision 2.

The ABI also carries **no jitter flag**, so it cannot be switched off from the
browser. `--no-jitter` exists to reproduce a figure, which is a CLI concern, and
a browser user turning it off would weaken their own document without being told
why.

## Decision 5 — The failure payload is data for the caller to word, never a sentence

**On a non-`OK` status, `ptr` and `len` carry a short UTF-8 payload and
`width`/`height` stay zero. The payload is the bare fact; the caller writes the
sentence.**

A bare `u32` status loses the one detail that matters most often. A French user
whose mark contains a single character the font subset lacks is the motivating
case: "le calcul a échoué" is the worst possible answer, and ADR-0007 Decision 6
already asks the engine to name *every* unsupported character rather than the
first — for a caller that, until now, had no way to receive the answer.

The tempting fix is to send a human-readable message. That is wrong here, and
the reason generalises: **this product's interface is French, and a sentence
produced in Rust is a string the browser can neither display nor translate.**
So the payload carries data only:

| status | payload |
| --- | --- |
| `UNSUPPORTED_CHARACTER` | every offending character, concatenated, in order of first appearance, no duplicates, no separators, no quoting |
| `BAD_ARGUMENT` | the bare field name, a stable ASCII identifier |
| anything else | **absent** — `ptr` and `len` both zero |

An absent payload is the honest answer where the only fact is the status.
`TEXT_DOES_NOT_FIT` has nothing to add beyond "it does not fit", and inventing a
number for the UI to show would be writing a sentence by other means.

Two consequences worth recording:

- **`pochoir_result_ptr` returns `0` for an empty payload.** `Vec::as_ptr` on an
  empty `Vec` returns a non-null dangling address, which a caller checking `ptr`
  before `len` — as the browser's reader does — would read as a payload that is
  there. The emptiness is checked on this side.
- **The field names are part of the ABI.** Those the engine owns travel
  unchanged (`dpi`, `mark blur`, `mark text`, …) rather than being re-spelled
  here into a second vocabulary free to drift. Five fields exist only on the
  wire, so the engine has no name for them and this crate defines them:
  `options version`, `options length`, `quality mode`, `page sizing`,
  `mark tile density`. A caller maps what it recognises; an unrecognised name
  degrades to no detail.

The payload is capped at 4096 bytes, which is the caller's own read limit, so
exceeding it would lose the whole payload rather than truncate it. Nothing comes
close: the mark is capped at 200 characters, so the largest payload reachable is
800 bytes, and there is a test that measures it.

### Two status codes beyond the original six

| code | status | engine error |
| --- | --- | --- |
| 6 | `TEXT_DOES_NOT_FIT` | `Error::MarkDoesNotFit` |
| 7 | `UNSUPPORTED_CHARACTER` | `Error::UnsupportedCharacter` |

0–5 are unchanged. These are ADR-0007's two named refusals, and collapsing them
into `BAD_ARGUMENT` would tell a user whose sentence is merely too long that
their parameters are wrong, which sends them to the wrong fix. Each has its own
remedy — shorten the text; change those characters — so each gets its own code.
Anything outside the seven is `INTERNAL`.

## Decision 6 — The result header carries no `kind` field, and the invariant is pinned

**A caller discriminates on `width > 0`. A PDF result always has `width == 0`
and `height == 0`.**

No field is added for a value already carried. But an undocumented invariant is
a trap, so this one is written into the crate documentation, made structural —
`Outcome`'s three constructors are the only way to build a result and each fixes
the dimensions for its own case — and pinned by a test that asserts it across
every input shape the suite reaches, not just the happy path.

The preview's pixel format is specified in the same spirit: **RGBA8, row-major,
top row first, four bytes per pixel, no stride padding, straight (non-
premultiplied) alpha**, so `len` is exactly `width * height * 4`. It is stated
that precisely because every plausible alternative — BGRA, a padded stride,
premultiplied alpha — would surface as a colour or alignment bug in the picture
rather than as an error anywhere. No conversion happens: `Pixmap` is already
exactly this and the preview is a straight copy out of it.

`pochoir_alloc` returns `0` both on failure and for `len == 0`, and the two are
told apart by the length rather than the pointer: a `0` for a non-zero `len` is
out of memory, a `0` for `len == 0` is nothing to allocate. `(0, 0)` is accepted
by the render functions and yields an empty slice.

## Decision 7 — Every export returns a status; a panic is an outage, not an error path

`[profile.release]` sets `panic = "abort"`. A panic therefore traps the instance:
every later call fails and the Worker has to construct a new module. So no export
panics, including on hostile input. Concretely, in this crate:

- **No `unwrap`, no `expect`, no slice indexing with a computed index.** The
  options parser reads through `get(..)` and `try_into`, so a malformed 48 bytes
  returns `BAD_ARGUMENT`. The downscaler slices one source row at a time through
  `get(..)` and walks it with `chunks_exact(4)`, which is the only place indexing
  is provably in range.
- **Checked arithmetic** on every length and offset: `checked_add`,
  `saturating_mul`, `u32::try_from`, and `u64` accumulators sized so the largest
  permitted image cannot overflow them.
- **Allocation is fallible where it can be.** `pochoir_alloc` goes through the
  raw allocator and returns `0` rather than reaching `handle_alloc_error`, and
  refuses anything over a 64 MiB cap; the engine's own `ImageTooLarge` becomes
  `OUT_OF_MEMORY`.

One case is honestly outside the guarantee and is documented at the crate root:
genuine heap exhaustion *below* the engine's image bound aborts, because Rust's
allocation-failure handler does. A caller that wants to survive it must be
prepared to rebuild the module.

The error mapping is one function, `status::status_for`, grouped by what the
caller should do rather than by where in the engine the error arose. It keeps a
catch-all arm because `pochoir_core::Error` is `#[non_exhaustive]`: a variant
added upstream surfaces as `INTERNAL`, a bug to report, rather than as a
plausible-looking lie about the user's input.

## Consequences

- The browser can call the engine. `make wasm` produces
  **`pochoir_wasm.wasm`**, not `pochoir_core.wasm`; the UI's `WASM_URL` needs
  reconciling with that, which is the orchestrator's to do.
- `make wasm` now passes `--locked` and ADR-0004 Decision 1's two
  `--remap-path-prefix` entries, and `make wasm-repro` is the measurement that
  decision asked for and could not previously run — there was no non-empty
  module to run it on.
- Reproducibility is now checkable by anyone, which is the first half of
  ADR-0004's answer to "they execute whatever the CDN served them".
- `compact` and `lossless` are reachable in the UI and will fail. The UI should
  not offer them until they exist.

## Alternatives rejected

- **Put the exports in `pochoir-core` behind a feature flag.** Requires
  downgrading its `forbid(unsafe_code)` to `deny`, which weakens a property of
  the crate that every other client benefits from, to save one small crate.
- **JSON options.** Needs a JSON parser — a parser of hostile input, so ADR-0001
  would even permit depending for one — on a path where a fixed 48-byte struct
  does the whole job. Rejected on bundle size and on attack surface.
- **Return a status and write results through out-parameters.** Fewer exports,
  but the caller then owns a buffer whose size it cannot know before the call.
  The handle keeps ownership on this side, where the allocator is.
- **Approximate the missing quality modes.** Decision 2.

## Open questions

1. **Does `compact` survive at all?** It requires resampling an image input,
   which ADR-0002 forbids three paragraphs above the table that asks for it. For
   Samuel: drop the mode, or carve an explicit exception for it?
2. **Is `lossless` worth a PNG encoder?** It is the mode with the clearest
   constituency — scanned line art, and anyone who objects to generation loss on
   principle — and the largest implementation cost.
3. **Should the resampler move into `pochoir-core`?** Decision 3's last section.
   Not urgent while this crate is its only user.
4. **A shrunken mark has no way to reach the browser.** `Composition` gained
   `shrink_ratio()` and `was_shrunk_noticeably()`, and the CLI prints a warning
   from them. This ABI cannot carry it: a warning is not a failure, so it
   cannot travel as a status, and the success payload is the document or the
   bitmap. It needs either another export, another field in the header, or a
   framing of the success payload — all three are ABI changes, and the right
   one depends on what else will eventually need to come back the same way.
   Deliberately not guessed here; left for scoping. `image_to_pdf_observed`
   and `render_to_pdf_observed`'s third closure argument are where the value
   would come from.
