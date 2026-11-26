# ADR-0006: Cap an image input's page at A4

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** [ADR-0002](0002-watermark-algorithm-and-quality-modes.md), in respect of its page geometry for image inputs only

## Context

[ADR-0002](0002-watermark-algorithm-and-quality-modes.md) Decision 2 says, for a
bitmap input, that "the page is the image's own aspect ratio, sized so that it
prints at the chosen DPI". The write path implements that faithfully, and the
result is a page nobody wants.

A 12-megapixel phone photograph is 4032 × 3024 pixels. At the default 200 DPI
that is **512 × 384 mm — larger than A2**. The same ADR says the DPI knob "is
meaningful only for PDF inputs", because an image input is never resampled; the
knob exists only to decide what physical size the pixels are declared to
represent. So the most visible physical property of the output was being decided
by a number our own design document describes as meaningless on that path.

Three practical consequences, in rising order of how much they hurt:

1. Document properties that read as a mistake. A recipient opening the file sees
   a page size no document has.
2. A four-sheet tiled printout for anyone who prints at actual size rather than
   fit-to-page.
3. **Administrative and rental upload portals that validate page size, which
   would reject the file** — and in this project's case that is the whole point
   of the document: it is being sent to a landlord, an employer or an agency.

Nothing about image quality is at stake, and that is what makes this easy. The
encoded bytes are identical whatever the page size, because the page size is not
an input to the encoder. Measured on a 4032 × 3024 test frame at quality 85, the
embedded datastream is **837 426 bytes at 72, 150, 200, 300 and 600 DPI alike**,
and with the cap on and off; only the `/MediaBox` differs. Capping changes the
sheet, not a pixel.

There is also no fidelity argument on the other side. A French *carte nationale
d'identité* is 85.6 × 54 mm, and neither a 512 mm page nor an A4 one reproduces
it life-size, so neither option is "true to the document" in any sense a user
would recognise.

## Options considered

### Option A — cap at A4, never enlarge (chosen)

Keep the aspect ratio, size at the effective resolution, then shrink uniformly
until the page fits an A4 sheet turned to match. A page already inside A4 is
left exactly as it is.

### Option B — fit everything to A4, letterboxed

What Filigrane Facile does, and what ADR-0002 already contemplates as an opt-in.
Rejected as a default: it would enlarge as well as shrink, and a 600 DPI scan of
an identity card — 85.6 × 54 mm, its true size — would become an A4 sheet with
the card marooned in the middle of it. That is the same mistake as the A2 poster,
in the other direction.

### Option C — leave it, and document the DPI knob better

Rejected. The failure mode is a silent rejection by an upload portal, which the
user experiences as "this tool does not work" with no diagnosis available to
them. A default that fails that way is not rescued by documentation.

## Decision

**For image inputs**, the output page keeps the image's own aspect ratio, is
sized at the effective resolution exactly as before, and is then **capped at A4
— shrunk to fit, never enlarged to fill.**

- The sheet is turned to match the page: a portrait image is bounded by portrait
  A4, a landscape one by landscape A4. A square page is bounded by the short edge
  either way, so the choice does not arise.
- Scaling is uniform on both axes. ADR-0002's prohibition on stretching stands
  untouched.
- A page already inside A4 is returned unchanged. **The cap only ever shrinks.**
- `PageSizing::Native` opts out and gives the true size at the effective
  resolution, however large. On the command line that is `--page-size native`;
  the default is `--page-size a4-max`.

**For PDF inputs nothing changes.** There the DPI is a real rasterisation
resolution and the page is the input's own `MediaBox`, which ADR-0002 is right
to preserve and which this ADR does not touch.

The `max(stated, requested)` rule for the effective resolution is unchanged, and
composes with the cap rather than competing with it: a 600 DPI A4 scan already
lands on an A4 page, so the cap never engages.

Worked examples, which are also the test cases:

| Input | Effective DPI | Page before | Page after |
| --- | --- | --- | --- |
| 4032 × 3024 photo | 200 | 512 × 384 mm | **280 × 210 mm** |
| 3024 × 4032 photo | 200 | 384 × 512 mm | **210 × 280 mm** |
| 3000 × 3000 image | 200 | 381 × 381 mm | **210 × 210 mm** |
| 2022 × 1276 ID card scan | 600 | 85.6 × 54 mm | **85.6 × 54 mm, unchanged** |
| 4961 × 7016 A4 scan | 600 | 210 × 297 mm | **210 × 297 mm** |

The capped 12-megapixel photograph prints at an effective **366 DPI** — denser
than the 200 that was asked for, not coarser, because the pixels did not move
and the sheet got smaller.

One consequence of that worth stating, because it is the kind of thing that
quietly becomes a lie: the density recorded in the embedded JPEG's own JFIF
segment is the resolution the page actually prints at, not the one the caller
requested. On a capped page those differ.

## Consequences

### What this gets us

Output that opens with a page size a recipient recognises, prints on one sheet,
and passes a portal that validates page geometry. A default that is right for
the overwhelmingly common case — a phone photograph of a document — without
taking anything away from the uncommon one.

### What this costs us

A default that is no longer a pure function of the DPI knob, so "200 DPI" no
longer means "the page is the pixels divided by 200" for a large image. That is
a small loss of predictability, paid for by the knob having been meaningless on
this path to begin with.

It is also a **behaviour change for anyone who relied on the old sizing**, which
is why `Native` exists rather than the cap simply being unconditional.

### What this forecloses

Nothing. Option B remains available as a further explicit mode, exactly as
ADR-0002 describes it, and would be additive.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| A4 is the wrong sheet for a non-European user | Medium | Low | US Letter is 216 × 279 mm, so an A4-capped page fits a Letter printer's imageable area in both axes. Revisit if anyone reports a real problem, not pre-emptively |
| Someone wanted the true-size page and does not find the opt-out | Low | Low | `--page-size native`, named in `--help` |
| The cap interacts badly with the watermark's page-relative sizing | Low | Medium | ADR-0002 sizes the mark as a fraction of the page's shorter edge, which is scale-invariant, so it should not. Confirm when the compositor lands |

## Open questions

None.

## Could not verify

- The claim that administrative upload portals validate page size. It is the
  reported experience behind this decision rather than something tested against
  a named portal; the other two consequences are observable and were.
- Whether 366 DPI is the right effective density for a recipient reading an MRZ
  on screen. It is higher than the 200 ADR-0002 argues for, so the direction is
  safe, but no one has read a real document at it.

## References

- ISO 216, A4 = 210 × 297 mm
- ADR-0002, Decision 2, "Page geometry: keep the original, never stretch" and
  "Resolution: do not resample image inputs at all"
- ADR-0003, the `PageSource` trait the cap is applied behind
