# ADR-0010: The mark's colour is measured against the page

- **Status:** Accepted
- **Date:** 2026-11-27
- **Deciders:** the orchestrating session, under Samuel Bagattin's standing rule
  that he is asked only for product direction, spending, legal and irreversible
  calls. The failure was found by Samuel, on his own document.
- **Supersedes:** [ADR-0002](0002-watermark-algorithm-and-quality-modes.md), in
  respect of the **Colour** row of Decision 3 only — the fixed `#1A1A1A`
  default and the one-sided "warn when the chosen colour's luminance is too
  high". Everything else in ADR-0002 stands, including every other row of that
  table, and [ADR-0007](0007-watermark-specification-corrections.md)'s five
  corrections are untouched.

## Context

Samuel watermarked a dark-mode iPhone screenshot with the defaults. The output
PDF is visually indistinguishable from the input, and **nothing was said**.

Measured on that file, rasterised through the real pipeline at 200 dpi
(924 × 2000 px, 1 848 000 pixels):

| | value |
| --- | --- |
| p05 / p25 | 0.004 / 0.004 |
| median | 0.114 |
| p75 / p95 | 0.114 / 0.478 |
| pixels below 0.25 | 93.5% |

The mark is `#1A1A1A`, relative luminance 0.102, drawn at opacity 0.30. At full
glyph coverage it changes a pixel's luminance by `0.30 × |0.102 − 0.114|`, which
is 0.004 — one level of 255, less than JPEG rounds away. Over 93.5% of that
page the mark is not there. Measured after the fact: it is visible over **6%**
of the page, and that 6% is the white text and the green toggles it happens to
cross.

`Mark::colour_is_too_pale()` said nothing, because it tests one side of a fixed
threshold — luminance above 0.55 — and 0.102 is not above 0.55. It was never a
test about contrast. It was a test about photocopiers, on an implicit
assumption that the page is white paper.

This is the worst failure this product can have. A user believes their identity
document is protected, downloads the PDF and sends it, and there is no mark.
It is worse than refusing, because refusing is visible. And dark-mode
screenshots are not an edge case for this audience: people screenshot a bank
statement, an insurance portal or an identity document held in an app, and
phones default to dark.

Three things are fixed elsewhere and not up for debate here:

- **The mark is burned into the pixels** (ADR-0002 Decision 1). Whatever is
  decided here is a colour the compositor blends; there is no separate layer.
- **Output is deterministic and byte-identical between `wasm32` and the host**
  (ADR-0002 Decision 6), which the golden suite asserts. Anything measured from
  the image has to keep that property, and in particular may not use `powf`,
  `sin`, `cos` or anything else that comes from the platform's libm.
- **Warnings, not refusals** (ADR-0007 Decision 2). The engine draws and tells
  the caller; it does not decide for the user.

## Options considered

### Option A — keep the fixed colour, make the warning two-sided

Test contrast against the measured page rather than lightness in the abstract,
and warn in both directions. Cheap, and it fixes the silence.

It does not fix the product. The default stays wrong for the document the user
actually has, and a warning that says "your watermark is invisible, choose
another colour" on every dark screenshot is a defect reported politely.

### Option B — derive the mark's colour from the image

Samuel's own suggestion: take the page's dominant or median colour and build
the mark from it — its complement, or a shifted version of it.

The objection is legibility. The mark is **text**, and text has to stay
readable as text; that is what makes it attributable, and attributability is
most of the deterrent (ADR-0002 Decision 3). A colour derived from the page's
average is the one colour on the page that contrasts with nothing on it, and
the further the derivation moves away from the average to recover contrast, the
less the derivation is doing and the more it is an arbitrary choice dressed up
as a measurement. A complement is worse: the complement of a mid-grey page is
mid-grey, and the complement of a saturated page is a saturated mark, which is
both harder to read and — per ADR-0002's own argument for a neutral grey — the
first thing to die in a black-and-white photocopy.

### Option C — measure the page, choose between a dark mark and a light one

Two fixed candidates, `#1A1A1A` and its photometric mirror `#E5E5E5`, and a
measurement of the page that decides which. Both are neutral greys, so
ADR-0002's photocopy argument survives intact on whichever page each is used
for. The measurement decides one bit, which is the bit that was wrong.

### Option D — raise the opacity or outline the glyphs instead

Make the mark heavier, or give it a contrasting halo, so that it shows on any
background. Opacity does not help: the delta is `opacity × |mark − page|`, and
on Samuel's page even the maximum 0.80 gives 0.010, still invisible. A halo
would work and is a genuinely different mark — two colours per glyph, a wider
stamp, a second raster pass, and a visual change to every document the product
has ever produced. It is not justified by a problem that one bit of measurement
solves, and it remains available later.

## Decision

**We will measure the page's luminance distribution before compositing, and by
default choose the mark's colour from it — `#1A1A1A` or `#E5E5E5`, whichever is
visible over more of the page. An explicit colour is still drawn exactly as
given, and warned about when it has poor contrast against the page that is
actually there.** Option C, with Option A's two-sided warning folded in; the
two use the same measurement, so they can never disagree.

Concretely, in `crates/pochoir-core/src/mark/tone.rs`:

1. **`PageTone::measure`** builds a 256-bin histogram of the page's relative
   luminance in one pass, before a pixel is touched. Non-opaque pixels are
   composited over white first, because `jpeg::encode` does exactly that — a
   PNG's transparent background otherwise measures as whatever sits under the
   transparency, usually black, and the page comes out white.
2. **A distribution, not an average.** The question is *over how much of this
   page would the mark be visible?*, and that is a count of pixels. A mean or a
   median answers a different question and answers it badly on a page split
   between a light half and a dark one, where the middle is the one tone that
   contrasts with neither.
3. **Visible means `MIN_VISIBLE_DELTA = 0.05`.** At full glyph coverage the mark
   changes a pixel's luminance by `opacity × |mark − page|`; a pixel counts as
   marked when that is at least 0.05, about 13 levels of 255. That is well
   clear of what JPEG at the default quality rounds away, and about a fifth of
   the 0.269 the dark mark achieves on white paper — the case the product was
   designed around and demonstrably succeeds at.
4. **The choice is an integer comparison.** Whichever candidate has the larger
   count wins. A tie goes to whichever is further from the page's median, and
   failing that to `#1A1A1A`, so that ADR-0002's photocopy argument keeps the
   default on a light page. No ratio decides anything, so no rounding can tip
   it.
5. **`Composition` carries the measurement out**: the tone, the colour actually
   drawn, whether the engine chose it, and the fraction of the page the mark is
   visible over. `has_poor_contrast()` is true below
   `MIN_CONTRAST_COVERAGE = 0.90`. This is the route ADR-0007 Decision 2
   already established for the shrink warning, and the CLI prints it the same
   way.
6. **`Mark::colour` becomes `MarkColour::{Automatic, Fixed}`**, `Automatic` by
   default. `Mark::colour_is_too_pale` and `PALE_LUMINANCE` are **removed**:
   there is no longer a context-free question about a colour to which a useful
   answer exists.

### The two candidates leave no gap

With `MIN_VISIBLE_DELTA = 0.05` at the default opacity, the dark mark fails
below page luminance 0.269 and the light mark fails above 0.731. The failure
regions do not overlap, so **every flat page is covered by one of the two** —
asserted in `tone.rs` for all 256 shades. A page that defeats both is therefore
always a page with *spread*, which is precisely the case where the honest
answer is to say so rather than to pick quietly.

### Why no linearisation

Proper sRGB relative luminance needs the transfer function, which needs `powf`.
`powf` comes from the platform's libm and is permitted to differ between
`wasm32` and native. One call would break the byte-identical outputs ADR-0002
Decision 6 requires, and nothing in CI would catch it — this crate hand-rolls
its own sine and cosine for exactly this reason. The Rec. 709 coefficients are
applied to the sRGB values directly, as `colour_is_too_pale` already did. That
is photometrically crude. It is ample for choosing between two colours 0.80
apart and for raising a warning, and it is the only measure that is identical
on both targets.

**Verified, not assumed.** A probe built from `pochoir-core` for both
`aarch64-apple-darwin` and `wasm32-unknown-unknown` (run under Node 24's V8)
returns identical chosen colours, identical visible-pixel counts and identical
composited-raster fingerprints over six pages — flat grey, flat dark, a
half-white/half-black split, the gradient fixture, a noisy page sitting on the
dark mark's visibility threshold, and one with a fixed colour. The threshold
case is the one that matters: it is where a one-ulp difference would change a
pixel count and flip the colour.

**Runner-up:** Option B, the derived colour, which is where Samuel started. It
loses on legibility, above. It would win if the mark stopped being text — a
tiled logo or a pattern has no legibility floor and could take an arbitrary
colour — and that is the trigger for a future ADR.

## Consequences

### What this gets us

Samuel's screenshot, watermarked with the defaults, now carries `#E5E5E5`
visible over 97.6% of the page, and the mark is plainly legible on it. The
same file with `--mark-colour 1a1a1a` is still drawn exactly as asked and now
says: *invisible over 94% of the page*.

### What this costs us

- **One pass over the pixmap per page**, integer counting into 256 bins. The
  seed derivation already makes such a pass and the JPEG encode dwarfs both,
  but it is not free and it is on the preview path too.
- **Three golden hashes moved.** `gradient-default`, `gradient-high-density`
  and `gradient-upright` use the ramp fixture, whose distribution is broad and
  sits below mid-grey; the light mark reaches 85.3% of it against the dark
  mark's 71.8%, so the default now draws them light. `flat-grey-*` is `#C8C8C8`
  and is unchanged; `gradient-black-heavy` fixes its colour and is unchanged.
- **A breaking change to the public `Mark`.** `colour` is no longer `[u8; 3]`.
  Every caller in the workspace is updated in the same commit.
- **The browser does not get this yet.** The wasm ABI has carried an explicit
  packed colour since it was written, so every request through it is
  `MarkColour::Fixed` and the engine's new default is unreachable from the web
  app. Wiring it up is a wire change and belongs with the UI control that
  exposes it; until then the deployed product still draws `#1A1A1A` on a dark
  screenshot. **This is the gap that matters most and it is not closed here.**
- **The colour can change when the opacity does.** The visibility test contains
  the opacity because the physics does, so in principle moving the opacity
  slider can flip the mark from dark to light. It takes a page near the
  threshold for this to happen, and it is surprising when it does.

### What this forecloses

Nothing permanently. A derived or user-chosen palette can be added later as
more candidates in `choose_colour`; an outline or halo (Option D) is untouched
by this and remains the answer if one colour ever stops being enough.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| A split page gets a mark that is invisible over much of it | Medium — any scan with a dark photograph on white paper | High: part of the document is effectively unmarked | `has_poor_contrast()` fires and names the situation. Revisit with Option D (outlined glyphs) if real documents hit this often |
| 0.05 is the wrong visibility floor | Medium | A mark called visible that a user cannot see, or a warning on a mark they can | It is a threshold on a crude measure, chosen against the one case we know works (0.269 on white). Revisit against real documents, not synthetic ones |
| The light mark is lost in a black-and-white photocopy | Low on a dark page, which photocopies dark | Medium | ADR-0002's argument is about white paper and still holds there, where the dark mark is still chosen. **Unverified on a real photocopier** |
| The measurement is slow on very large pages | Low | Low | One integer pass; the encode is far more expensive |
| A near-threshold page flips colour between two similar documents | Low | Low — both candidates are near-equal there by construction | The coverage warning fires whenever the margin is thin enough to matter |

## Open questions

None. The one judgement that would have needed Samuel — whether to derive the
colour from the image or choose between two — is answered above with the
legibility argument, and he is the one who reported the bug.

## Could not verify

- That `#E5E5E5` survives a black-and-white photocopy of a dark page. The
  argument is that a photocopier renders a dark-mode screenshot as a dark field
  and the light mark is what is left standing, but **no photocopy was made**.
  ADR-0002's matching claim for `#1A1A1A` on white paper is equally untested.
- That `MIN_VISIBLE_DELTA = 0.05` and `MIN_CONTRAST_COVERAGE = 0.90` are the
  right numbers for documents in general. They are calibrated against one real
  document and a handful of synthetic ones.
- The cross-target check ran `wasm32-unknown-unknown` under Node 24's V8 only.
  Other engines are specified to behave identically for these operations but
  were not measured.

## References

- ADR-0002, Decision 3 and Decision 6 — [0002-watermark-algorithm-and-quality-modes.md](0002-watermark-algorithm-and-quality-modes.md)
- ADR-0007, Decision 2, the warn-rather-than-refuse posture — [0007-watermark-specification-corrections.md](0007-watermark-specification-corrections.md)
- Rec. ITU-R BT.709-6, the luminance coefficients — <https://www.itu.int/rec/R-REC-BT.709-6-201506-I/en>
- IEEE 754 operations WebAssembly specifies exactly — <https://webassembly.github.io/spec/core/exec/numerics.html#floating-point-operations>
