# ADR-0007: Watermark specification corrections from implementation

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** the orchestrating session, under Samuel Bagattin's standing rule
  that he is asked only for product direction, spending, legal and irreversible
  calls. Correction 5 was decided by the implementing session and is flagged as
  such below.
- **Supersedes:** [ADR-0002](0002-watermark-algorithm-and-quality-modes.md), in
  respect of five points of Decision 3 only. Everything else in ADR-0002 — the
  burn-in decision, the quality modes, the exclusion zone, the metadata
  hygiene, the determinism requirements and what we do not promise — stands
  unchanged.

## Context

[ADR-0002](0002-watermark-algorithm-and-quality-modes.md) specified the mark
before any of it existed. Building it turned up places where the specification
is silent, self-contradictory, or weaker than its own stated justification. None
of them changes what the mark *is*; all of them had to be settled before a line
of the compositor could be written, and leaving them settled in code and
nowhere else is exactly the failure this directory exists to prevent.

This ADR records **five corrections and one note**, each with the measurement
or the argument that forced it. Decision 1 is the note: it confirms an
ADR-0002 behaviour rather than changing it. Decisions 2, 3, 4, 5 and 6 are the
corrections, and are what the supersession above covers. Two of them —
Decisions 3 and 5 — were found by rendering an A4 page and looking at it; the
rest by reading ADR-0002 against the code being written.

Two items raised at the same time are **not** here, because they changed no
decision and were implemented as ADR-0002 already asks:

- `--seed` and `--no-jitter` are named in Decision 6 and did not exist. They are
  now `OutputOptions::seed` and `OutputOptions::jitter`, added in the same
  commit as the mark because they change a struct that the CLI and every test
  construct.
- The golden-hash hook Decision 6's test strategy assumes did not exist either,
  because `render_to_pdf` encoded each page the moment it was composited. It is
  now `render_to_pdf_observed`, which hands a closure the composited pixmap
  before the encoder sees it.

## Decision 1 — One column of text is intended; the *gap* beside it is not

**A realistic mark is about as wide as the page, so the repetitions ADR-0002
counts are rows. That is intended. The horizontal gap it specifies is not
implementable as written, and is re-expressed below in Decision 5.**

The measurement: at Decision 3's font size of 3.5% of the shorter edge, A4
portrait at 200 dpi gives a 58 px font, and `Document destiné exclusivement à
une demande de location — 26/11/2026` is about 1 560 px wide on a 1 654 px page.
So the mark is one page wide, there is one column of it, and the "6–10 visible
repetitions on a portrait page" Decision 3 aims at are six to ten *rows*.

Recording this because the horizontal half of the tile-density parameter
therefore does almost nothing for the common case, and somebody should find
that written down rather than rediscover it as a bug.

## Decision 2 — Long text wraps, then shrinks, then is refused. Never clipped

**ADR-0002 allows "up to 200 characters, up to 3 lines" at a font size that
cannot hold 200 characters on one line, and does not say what gives.** It is:

1. **Wrap** on word boundaries, to at most the three lines Decision 3 allows.
2. **Shrink** the font if three lines still overflow, down to a floor of **1% of
   the page's shorter edge** — which is not an invented number but Decision 3's
   own stated minimum for the same parameter, so the floor is a size the
   specification already calls legible.
3. **Refuse**, with an error naming the problem, if even the floor overflows.

"Fits" means the text block's **rotated** bounding box fits the page, since a
mark the page cannot hold once cannot be tiled either.

**The load-bearing half of this decision is the word *never*.** Truncating
`Document destiné exclusivement à une demande de location` to `Document destiné
exclusiv` silently changes what the document asserts about itself. For a tool
whose whole purpose is making a document's intended use legible on its face,
that is a correctness bug and not a cosmetic one, so a mark that cannot be drawn
honestly is not drawn at all. An error the user can read and act on is strictly
better than a document that quietly says something else.

Worth noting, because the arithmetic is not obvious: at the 1% floor, 200
characters wrapped to three lines fits essentially any page. The refusal path is
therefore reached only by text that cannot be *broken* — a 200-character string
with no spaces in it — rather than by text that is merely long.

## Decision 3 — Blur is keyed to the font size, not to a pixel count

**ADR-0002's "σ ≈ 0.6 px at render resolution, scaled with DPI" contradicts
itself and, at its own default, does nothing at all.**

Two separate problems:

- *"At render resolution"* and *"scaled with DPI"* pull in opposite directions.
  One says the blur is a fixed number of pixels whatever the resolution; the
  other says it grows with it. They cannot both hold.
- Three box passes approximating a Gaussian of σ want a box of width
  `sqrt(4σ² + 1)`, so **σ = 0.6 px is a box radius of 0.28** — which an
  integer-radius box blur rounds to zero. The parameter would be a no-op at its
  default and would jump to a visible smear at the first value that survived
  rounding.

So: **σ is a fraction of the font size**, which makes it resolution-independent
in exactly the way Decision 3 already achieves for the glyphs by keying them to
the page's shorter edge. The default is **0.015**, about 0.87 px at the default
font size on A4 at 200 dpi — a blur that demonstrably changes pixels, and
there is a test asserting that it does. ADR-0002's 0–3 px range becomes
**0–0.06**, which is 0 to about 3.5 px at the same reference size.

The box radius is also **fractional**, with the end taps weighted by the
fractional part, so a radius of 0.28 blurs by 0.28 of a pixel instead of by
nothing.

**This changes nothing about what blur is for.** ADR-0002 is explicit that blur
has not been measured against any removal tool and must not be sold as a
defence; its certain benefit is that a hard glyph edge over a photograph stops
looking like one. That remains true and remains the only claim made for it.

## Decision 4 — The jitter is per-tile, not one global offset

**ADR-0002's letter permits the weaker version and its justification demands the
stronger one.**

Decision 3 keeps a "random per-run position offset" and justifies it with two
things: that two watermarked copies of one document cannot be differenced back
to the clean original, and that "a remover tuned to fixed tile positions fails".
A single global offset of the whole lattice satisfies the first. It does not
satisfy the second: the lattice is still a lattice, and finding one offset finds
every tile.

**Every tile is therefore displaced independently**, from a generator keyed to
its own lattice coordinates rather than drawn from a running stream — so a tile's
offset does not depend on how many tiles were visited before it, and a page that
needs one more row does not move every other row. The global phase is kept as
well, on top of the per-tile offsets, so the lattice itself still differs between
documents.

It costs nothing, it is still seeded and therefore still reproducible, and it is
what the ADR's own justification was already asking for.

One constraint discovered while building it: **the displacement is capped at
half the free gap between neighbouring tiles.** Two tiles that meet composite
over each other, which is both twice the chosen opacity and illegible where the
glyphs cross. Capping the swing is what lets the compositor blend each tile
straight into the page instead of accumulating coverage across the whole page
first.

## Decision 5 — The horizontal tile gap is a multiple of the line height

**Decided by the implementing session, 2026-11-26.** This correction was not
among the four scoped to this ADR; it extends it. The reason it had to is that
Decision 1 above — which instructed that the horizontal gap be left alone — rests
on a premise that measurement shows to be false.

ADR-0002 Decision 3 specifies `normal` as "gaps of 1.6× text width, 4× line
height". Decision 1 reasoned that the horizontal figure rarely engages, because
a page-wide mark means one column and one column fills the row.

**It does not, because the mark is rotated.** A row of tiles runs along the
mark's own x-axis, which after a −30° turn spans the page's **diagonal**: on A4
at 200 dpi that is 2 601 px, not the 1 654 px of the page's width. A single
1 634 px tile therefore leaves roughly 1 000 px of every row unmarked. Rendered,
the page comes out with **four** visible marks and large blank regions, against
the six to ten even ones the same table asks for — and an unmarked region of the
page is an unmarked crop of the document, which is the failure this product
exists to prevent.

A gap proportional to the *text width* is also unstable in principle: it is
meaningless once the text is as wide as the page, and it makes the spacing
between repetitions depend on how much the user typed.

**So both gaps are multiples of the line height:** 1.6 across and 4 down at
`normal`, doubled at `low` and halved at `high`. This behaves at both extremes.
A page-wide mark repeats with a small gap and fills each row; a short mark such
as `CONFIDENTIEL` repeats several times across each row, which is what a tiled
watermark is supposed to look like. The vertical figure is untouched, so the
density ADR-0002 actually cared about is unchanged.

## Decision 6 — NFC output stays inside the subset; unsupported characters are refused by name

**The composition table and the font subset are two lists of characters that
have to agree.** If NFC composes something the subset lacks, the result is a
`.notdef` box in the middle of somebody's watermark.

Two measures, because they address different failures:

- **A test asserts that every character the composition table can produce, and
  every character it accepts as a base, has a glyph in the bundled font.** Both
  lists are generated — the table by `scripts/gen-nfc-table.py`, the font by
  `scripts/subset-font.py` — and a regeneration of one without the other is
  exactly the kind of drift a comment would not catch.
- **At run time the mark text is validated up front and unsupported characters
  are refused, with a message naming them.** Not silently dropped, which would
  change what the mark says (see Decision 2), and not drawn as tofu.

A leftover combining mark is reported like any other unsupported character.
That is deliberate: it means NFC could not compose it, and drawing it would put
a stray accent floating on the baseline rather than over its letter.

The error type carries the **first** such character only, because every variant
of `pochoir_core::Error` holds `Copy` data by design, so the error cannot own a
list. `mark::unsupported_characters` returns all of them for a caller that wants
to name every one, and the CLI uses it.

## Consequences

### What this gets us

A mark that covers the page it is drawn on; a blur parameter that does something
at its default and means the same thing at every resolution; jitter that matches
the resistance claim made for it; and a refusal path instead of a silently
altered assertion. Four of the five corrections are testable properties and each
has a test that fails if it regresses.

### What this costs us

Three of ADR-0002's stated numbers no longer mean what they say, and a reader of
that document alone will be misled — which is why the superseded passages now
carry a pointer here. The blur range is in new units, so a figure quoted in
pixels has to be converted. And Decision 5 means the `low`/`normal`/`high`
labels no longer map to the horizontal arithmetic ADR-0002 wrote down.

Decision 2's refusal is a real failure mode that users will meet: a long
unbroken string now produces an error rather than a document. That is the
intended trade and it will still be a surprise to whoever hits it first.

### What this forecloses

Nothing. Every correction narrows or re-expresses a parameter; none closes off
the exclusion zone, `overlay` mode, PDF input or a future shaping engine.

## Could not verify

- That the corrected tile spacing is the *best* spacing, as opposed to one that
  demonstrably covers the page. It was chosen by rendering A4 at 200 dpi and
  looking at the result; it has not been tested on landscape pages, on very
  short marks, or with real users.
- That per-tile jitter actually defeats a position-tuned remover. As with blur,
  ADR-0002's "could not verify" applies unchanged: this is a plausibility
  argument, we have measured nothing, and the stronger implementation is
  adopted because it costs nothing rather than because it has been shown to
  work.
- That a 1% font-size floor is legible on a real page. It is ADR-0002's own
  stated minimum, which is the authority being leaned on, not a measurement.
- The claim that a 200-character mark always fits at the floor is arithmetic
  over the bundled font's advance widths, checked by test for the cases in the
  suite, not proved for every string.

## References

- [ADR-0001](0001-stack-and-dependency-policy.md) — the dependency policy that
  makes the font parser and the composition table ours to write.
- [ADR-0002](0002-watermark-algorithm-and-quality-modes.md) — the specification
  this corrects.
- [ADR-0006](0006-image-page-size-cap.md) — the other partial supersession of
  ADR-0002, and the pattern this one follows.
- Unicode Standard Annex #15, *Unicode Normalization Forms* —
  <https://www.unicode.org/reports/tr15/>
- OpenType `glyf` table specification —
  <https://learn.microsoft.com/en-us/typography/opentype/spec/glyf>
- Raph Levien, `font-rs` (2016), the signed-area accumulation rasteriser this
  one is written from the description of —
  <https://github.com/raphlinus/font-rs>
