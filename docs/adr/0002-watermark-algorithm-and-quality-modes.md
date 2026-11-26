# ADR-0002: Watermark algorithm and output quality modes

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** none

## Context

**Pochoir** watermarks identity and administrative documents so that a copy sent
to a landlord, an employer or an agency cannot easily be reused elsewhere. The
reference implementation is the French government's **Filigrane Facile**
(<https://filigrane.beta.gouv.fr/>), whose behaviour we know from its source:
the `watermark` package in [MTES-MCT/Dossier-Facile-Frontend] (Vue 3 SPA) and
`dossierfacile-api-watermark` / `dossierfacile-pdf-generator` in
[MTES-MCT/dossierfacile-backend] (Java/Spring → RabbitMQ → Apache PDFBox).

What that tool does, and the behaviour this ADR reacts to:

| Step | Filigrane Facile |
| --- | --- |
| Where it runs | On a server; the document is uploaded |
| Rasterisation | Every page, always |
| Resolution | Fixed **128 DPI** |
| Page geometry | Everything **forced to A4** |
| Encoding | **JPEG**, always, no quality control exposed |
| Mark | Alpha jittered 0.52–0.6, Gaussian blur, **−25°**, random per-run position offset |
| Barcodes | A hole is cut in the mark so QR codes stay scannable |
| Controls | One file picker, one 100-character text field. No preview |
| Output | Always one merged PDF |

Three facts constrain this ADR and are decided elsewhere:

- The whole pipeline runs on the user's device; nothing is uploaded
  ([ADR-0001](0001-stack-and-dependency-policy.md)).
- Dependencies are expensive. Any step that needs a library needs its own ADR
  ([ADR-0001](0001-stack-and-dependency-policy.md)).
- Turning a PDF *input* into bitmaps requires a renderer we cannot hand-roll;
  that is [ADR-0003](0003-pdf-input-rasterisation.md)'s problem, not this one.
  This ADR assumes it is given page bitmaps and page geometry.

What is open here: how the mark is composed, what we do to the document's
pixels, and what we promise the user.

## Decision 1 — The mark is burned into pixels, not added as an overlay

**We will composite the watermark into the page bitmap.** The default pipeline
rasterises, blends, re-encodes.

This is the decision the product is named after. A *pochoir* is a stencil: once
you have sprayed through one, the paint is in the paper and there is nothing to
peel off. That is the whole of our technical commitment here, and the rest of
this section is about what it costs.

A PDF watermark can also be added *non-destructively*, as an annotation or as
extra operators appended to the page content stream. We reject that as the
default because it is removable in one step by anybody:

- an annotation is deleted in any PDF editor, or by `qpdf --remove-annotations`;
- appended content-stream operators are stripped by uncompressing the stream and
  cutting the tail, which `mutool clean -d` makes trivial;
- in both cases the document underneath is recovered **bit-exact**.

A mark that is removable in one step is decoration. Once the mark is in the
pixels, removing it means reconstructing the pixels it covers — which, over
text, means knowing what the text said. That is the entire security value of
this tool, and it is the reason the official tool rasterises too.

### The honest cost

Rasterising a born-digital PDF destroys its text layer. Specifically:

- **no text selection, copy or search** in the output;
- **screen readers get nothing** — the output is a picture of a document. This
  is a real accessibility regression, and we should be careful about criticising
  Filigrane Facile's RGAA non-conformance while shipping it. We mitigate it only
  partially (see Decision 2, overlay mode) and we must say so in the UI;
- **file size often grows a lot**: a 180 KB vector payslip becomes megabytes of
  JPEG;
- vector sharpness is replaced by a fixed resolution, so zooming hits pixels.

Rasterisation also has a security *benefit* worth recording: it destroys
everything else carried by the input PDF — embedded JavaScript, embedded files,
optional-content groups, form fields, annotations, incremental-update history
containing older revisions of the page. The output is pictures plus structure we
wrote ourselves.

### One compositor, operating on an RGBA pixmap

**The watermark compositor takes an RGBA pixmap plus page geometry and knows
nothing about where the pixmap came from.** It is the same code for a decoded
JPEG and for a rasterised PDF page, and it has no dependency on the PDF
renderer.

This is worth stating because there is a tempting alternative. If we adopt a
renderer that draws into a 2-D scene context we also control — `hayro` renders a
page into a caller-supplied `vello_cpu::RenderContext`, which itself offers
`glyph_run`, `push_opacity_layer`, `set_blend_mode` and affine transforms — then
one rasteriser could draw both the page and the rotated semi-transparent mark in
a single pass.

We reject that, for three reasons:

1. **It would give us two watermark implementations**, one for image inputs and
   one for PDF inputs, which must then be kept visually identical and need two
   golden-test suites. Divergence between them would be a bug class that simply
   does not exist with one compositor.
2. **It would couple the common path to the PDF renderer's dependency tree.**
   The whole point of [ADR-0003](0003-pdf-input-rasterisation.md)'s split is that
   dropping in a photo never loads the renderer. Drawing the mark through the
   renderer's scene API destroys that.
3. **The efficiency argument is weaker than it looks.** The "no pixel readback"
   benefit applies to GPU or `<canvas>` rendering. A CPU rasteriser renders into
   a buffer in linear memory that we already own; compositing our own mark onto
   it is writing to memory we are holding anyway, not a readback.

So: the renderer's only job is to hand us pixels. The mark is ours.

## Decision 2 — Quality modes

The official tool is destructive in three independent ways at once: it drops
resolution to 128 DPI, re-encodes as JPEG, and changes the page geometry. Those
are separable, and we separate them.

### Page geometry: keep the original, never stretch

**We will keep the input's page geometry.**

- PDF input: the output `MediaBox` equals the input page's `MediaBox`, including
  landscape pages, A5, US Letter, and mixed sizes in one document.
- Image input: the page is the image's own aspect ratio, sized so that it prints
  at the chosen DPI.
- "Fit to A4" exists, but as an explicit opt-in for people who will print, and
  it **letterboxes**. We never scale the axes independently. A non-uniformly
  stretched photograph of an identity document is both ugly and forensically
  misleading about the object it depicts.

### Resolution: do not resample image inputs at all

This is the biggest single quality win over the official tool and it is free.

A phone photo of an identity card is typically 3000–4000 px on its long edge.
Filigrane Facile re-renders it at 128 DPI on A4 — about 1050 × 1485 px — and
throws away roughly 85–90% of the pixels for no reason at all; the watermark
does not need them gone.

**For bitmap inputs the default is 1:1: we composite at the source's native
pixel dimensions and never resample.** The DPI setting is then only used to
decide what physical page size those pixels represent. Resampling an image input
can only upsample (pure waste) or downsample (pure loss).

The DPI knob is meaningful only for **PDF inputs**, where we are choosing a
rasterisation resolution that did not previously exist.

### Encoding and the modes

| Mode | Raster DPI (PDF input) | Image input | Encoding | Geometry |
| --- | --- | --- | --- | --- |
| `compact` | 150 | downsample to ≤ 150 DPI equivalent | JPEG q75 | original |
| **`standard`** (default) | **200** | **native, 1:1** | **JPEG q85** | **original** |
| `high` | 300 | native, 1:1 | JPEG q92 | original |
| `lossless` | 300 | native, 1:1 | PNG (`/FlateDecode`) | original |
| `overlay` | — (not rasterised) | n/a | original PDF preserved | original |

Recommended defaults: **`standard`, 200 DPI, JPEG q85, original geometry, mark
burned in.**

Why 200 DPI and not 128 or 300: the small print on a French *carte nationale
d'identité* and the MRZ on a passport are near the limit of legibility at 128
DPI, which is why scans of them are conventionally made at 200–300. 300 DPI
costs roughly 2.2× the bytes of 200 DPI for detail that a phone photo of a
laminated card did not contain in the first place. 200 is the point where the
recipient can read every field and the file still attaches to an email.

Why JPEG by default and not lossless: at q85 the artefacts are below what the
source photo already carries, and the PNG of a 12 MP photograph is roughly an
order of magnitude larger. `lossless` exists for scanned line art and for
anybody who objects to generation loss on principle.

### `overlay` mode: we offer it, labelled as weaker

**We will offer a mode that preserves the text layer and accepts a removable
mark** — PDF input only, no rasterisation, the mark drawn into the page content
stream.

It exists because the accessibility and file-size costs of rasterisation are
real, and because some recipients genuinely need a searchable document. It must
be presented with an unambiguous warning that the mark can be removed in
seconds, and it must never be the default. It also preserves everything else in
the input PDF, including anything malicious, which is a second reason not to
default to it.

## Decision 3 — Mark appearance and defaults

| Parameter | Default | Range | Reasoning |
| --- | --- | --- | --- |
| Text | user-supplied; placeholder suggests recipient + date + purpose | up to 200 chars, up to 3 lines | The official tool's 100-char single line cannot hold "transmis à X le JJ/MM/AAAA pour une demande de location". Naming the recipient is what makes the mark attributable, and attributability is most of the deterrent |
| Font size | 3.5% of the page's **shorter** edge | 1–10% | Relative to the page, so it is identical at every DPI and on every page size. Keying it to the shorter edge keeps landscape pages sane |
| Angle | **−30°** | 0–90° | A horizontal mark competes with the document's own text lines and crops out easily; a diagonal crosses the lines, so removal must reconstruct text across the stroke. Steeper than ~40° starts to hurt the mark's own legibility |
| Opacity | **0.30** | 0.05–0.80 | The mark must not stop the recipient reading the document — that defeats its purpose. Coverage comes from tiling, not from per-mark opacity. The official tool's 0.52–0.6 is heavier than it needs to be |
| Tile density | `normal`: gaps of 1.6× text width, 4× line height | `low`/`normal`/`high` | Aims at 6–10 visible repetitions on a portrait page. Several light marks resist removal better than one heavy one, because each must be inpainted separately |
| Colour | **neutral dark grey `#1A1A1A`** | free | Load-bearing: the attacker's next move is to print and rescan, or photocopy in black and white. A dark grey mark survives greyscale reproduction and aggressive JPEG re-compression. A pale or pastel mark can vanish entirely. We allow a colour choice and warn when the chosen colour's luminance is too high |
| Blur | small, σ ≈ 0.6 px at render resolution, scaled with DPI | 0–3 px | See below |
| Per-run jitter | position only, seeded | on | See below |

### Text encoding: normalise to NFC

The mark text must be normalised to **NFC** before layout. Text arriving from
macOS filenames, from clipboard paste or from OCR is frequently
NFD-decomposed — `e` + U+0301 rather than `é`. Our glyph rasteriser does no
shaping and no mark positioning (see
[ADR-0001](0001-stack-and-dependency-policy.md)), so a decomposed string renders
as a bare letter followed by a stray floating accent. Since the target language
is French, this is not an edge case: `é è ç à ô û ï` are ordinary input. NFC
composition for the Latin-1/Latin Extended-A range we care about is a small
lookup table, not a Unicode library.

### On the official tool's jitter and blur

We looked at each separately rather than copying the bundle.

- **Random per-run position offset — keep it.** It does real work: two runs over
  the same document produce marks in different places, so an attacker holding
  two watermarked copies cannot difference them to recover the clean original,
  and a remover tuned to fixed tile positions fails. We keep it but make it
  *seeded* (Decision 6) so it stays reproducible.
- **Alpha jitter in a 0.52–0.6 band — reject it.** An 8% variation in opacity is
  invisible, confers no removal resistance, and buys nothing but
  nondeterminism. Dropped.
- **Blur — keep, small, and do not oversell it.** A hard glyph edge over a
  photograph is a crisp, locally consistent discontinuity, which is the easiest
  thing for an automated remover to find; softening it makes the mark read as
  part of the image. That is the argument, and it is a plausibility argument, not
  a measurement. **We have not measured blur against any removal tool and should
  not claim it as a defence** (see *Could not verify*). Its certain benefit is
  cosmetic: it is good antialiasing. Hence: on, small, adjustable, honestly
  described.

## Decision 4 — Barcode legibility: a draggable exclusion zone, not a detector

Preserving 2D barcodes matters more in France than it might look. Official
attestations, *avis d'imposition* and payslips carry an **ANTS 2D-Doc** barcode
whose whole function is machine verification; a watermark across it makes the
document useless to the legitimate recipient who wants to check it. The official
tool is right to cut a hole.

But the lean way to do it is not a detector:

- **2D-Doc is DataMatrix, not QR.** A QR-only detector would miss the single most
  important French case.
- A QR *finder-pattern locator* — scan lines for the 1:1:3:1:1 ratio, cluster
  candidate centres, accept three roughly equidistant centres forming a right
  angle — is genuinely hand-rollable in around 300 lines and needs no decoder.
  DataMatrix (solid L-shaped finder plus alternating timing pattern) and PDF417
  are each a separate, larger job with a worse false-positive profile.
- A detector is also simply less reliable than the human looking at the page.

**We will ship a user-draggable exclusion rectangle in the live preview as the
primary mechanism.** We can offer this precisely because we have a preview and
the official tool does not. It costs essentially no bytes, covers every
symbology including ones we have never heard of, and doubles as "do not cover
the photograph" and "do not cover the MRZ".

Constraints on it, so it cannot be used to defeat the tool: the total excluded
area is capped at **25% of the page**, and the UI warns as the cap is
approached. An exclusion zone suppresses the tiles that would overlap it; it
does not create a sharp rectangular window, because the mark is tiled and the
suppression is per-tile.

A hand-rolled QR finder-pattern locator that *pre-places* the rectangle as a
convenience is a reasonable phase-2 addition. It is a suggestion the user can
move, never an automatic hole.

**Runner-up:** a real barcode-detection dependency (`rqrr`, `bardecoder`,
`zxing-cpp`). It lost on [ADR-0001](0001-stack-and-dependency-policy.md)'s
tests: client bytes for a feature a drag handle does better, plus a parser
consuming attacker-controlled image data, which is exactly the shape of
dependency we are trying not to have.

## Decision 5 — Metadata hygiene

### What we read from the input

Exactly two things, both from EXIF, both structural:

- `Orientation` (tag `0x0112`, = 274 decimal, SHORT, default 1), to rotate the
  pixels so the page is upright. A
  phone photo that renders sideways is the single most common quality complaint
  about tools in this category.
- `XResolution` / `YResolution` / `ResolutionUnit`, if present, to derive a
  sensible physical page size.

### What we strip

Everything else. On the **burn-in path this is structural rather than a
blocklist**: we decode to pixels, composite, and re-encode, so there is no code
path along which input metadata could reach the output and no "copy metadata"
step to forget to disable.

**That guarantee does not extend to any passthrough path, and this is a named
requirement, not a footnote.** Embedding an input JPEG's bytes verbatim as a
`/DCTDecode` image XObject is the cheapest and highest-quality way to put a
photo in a PDF — and it carries the file's APP1 segment, **including GPS
coordinates**, straight into the output. The burn-in path cannot use passthrough
(compositing forces a re-encode), but two paths can:

- `overlay` mode on an image input — embed the original JPEG, draw the mark in
  the content stream;
- any future "nothing to recompress here" fast path.

On any such path we **must** walk the JPEG segment markers and drop `APP1`
(Exif/XMP), `APP2` (ICC is kept only if we are honouring the colour profile),
`APP13` (Photoshop/IPTC) and `COM` before embedding, and drop the corresponding
PNG ancillary chunks. A test must assert that a GPS-tagged input produces an
output PDF containing no `Exif\x00\x00` byte sequence, on **every** mode. This
is the easiest privacy bug in the project to ship by accident, because the
tempting optimisation and the privacy failure are the same line of code.

Named for the record, because these are the ones that matter:

- **GPS** (`GPSLatitude`, `GPSLongitude`, `GPSAltitude`, `GPSTimeStamp`) — a
  phone photo of an identity card typically carries the coordinates of the
  user's home. This alone justifies the feature.
- `DateTimeOriginal`, `Make`, `Model`, `Software`, `BodySerialNumber`,
  `LensSerialNumber`, `Artist`, `Copyright`, `MakerNote`.
- **The EXIF thumbnail.** It is a second, smaller copy of the image. Any
  pipeline that preserved EXIF would ship an **unwatermarked** thumbnail of the
  document inside the watermarked output. We do not preserve EXIF, so we do not
  have this bug — but it is worth writing down as the reason not to "just keep
  the metadata" later.
- PNG: `eXIf`, `tEXt`, `iTXt`, `zTXt`, `tIME` and any private chunks.
- PDF input: Info dictionary, XMP packet, embedded files, JavaScript,
  annotations, form fields, optional content, and any prior revisions in the
  incremental-update history — all gone, since we rasterise.

### What we write into the output PDF

| Field | Value |
| --- | --- |
| `/Producer` | `Pochoir` — constant, **no version number** |
| `/Creator` | `Pochoir` — constant |
| `/CreationDate`, `/ModDate` | **absent** |
| `/Author`, `/Title`, `/Subject`, `/Keywords` | **absent** |
| XMP packet | **absent** |
| Trailer `/ID` | derived from a hash of the content, not from randomness or a clock |

No version string, because the version tells a reader which build produced the
file and therefore which known bugs it has. No dates, because a timestamp is
both identifying and a determinism hazard (Decision 6). A content-derived `/ID`
keeps the file valid and reproducible.

**Writing nothing identifying is itself a decision, with a cost:** the output
carries no provenance. A recipient cannot tell from the file's metadata that it
was watermarked by this tool rather than altered by someone. We accept that —
the visible mark is the provenance, and in a tool whose premise is that nothing
about the user leaves the device, embedding a build fingerprint in every file
would be inconsistent.

## Decision 6 — Determinism and testability

Output must be a pure function of (input bytes, parameters). Same inputs, same
bytes, on wasm and on native, on every machine, for golden-image tests in CI.

Three concrete hazards, and what we do about each:

1. **Randomness.** All of it comes from one small seeded PRNG that we write
   ourselves (xorshift or PCG, ~20 lines, no dependency). The seed is a hash of
   the input bytes, the parameter set and the mark text. So the per-run position
   jitter of Decision 3 is reproducible for a given document, while still
   differing between different documents and different parameter sets. An
   explicit `--seed` overrides it; `--no-jitter` disables it.
2. **Clocks.** Nothing in the pipeline reads a clock. This is most of why
   `/CreationDate` is absent.
3. **Floating point across targets.** This is the subtle one. IEEE-754
   `+ - * /` and `sqrt` are exactly specified and give identical results on
   wasm32 and on native. Transcendental functions do **not**: `sin`, `cos`,
   `exp` and friends come from the platform's libm and are permitted to differ
   between wasm and native, and between libcs. Our rendering path needs `sin`
   and `cos` exactly once, for the mark's rotation. Mitigation: compute the
   rotation's sine and cosine with our own fixed polynomial (or a lookup over
   the exposed angle range), and restrict everything else in the raster path to
   the exactly-specified operations. This is cheap and it is the difference
   between golden tests that pass in CI and golden tests that pass only on the
   machine that generated them.

### Test strategy

- **Inputs generated procedurally in the test**, not binary fixtures committed
  to git: a synthetic "document" of known rectangles, gradients and glyph-like
  shapes. Keeps the repository small and makes every input auditable as code.
- **Golden hash on the composited raster**, before encoding. This is the layer
  that the algorithm owns, and determinism makes an exact comparison legitimate
  — no perceptual tolerance needed.
- **Structural assertions on the PDF**, not a byte comparison: object count,
  `MediaBox` values per page, filter names (`/DCTDecode` vs `/FlateDecode`),
  absence of `/CreationDate`, absence of an XMP packet, presence of the exact
  `/Producer` string. A byte comparison of the final PDF would break every time
  the JPEG encoder changed, which tells us nothing about the watermark.
- **Encoder tests separately**: the JPEG and PNG encoders get their own
  round-trip and conformance tests, decoupled from the watermark goldens.
- **Deliberate regeneration**: goldens are regenerated behind an explicit flag,
  and the resulting diff is reviewed as part of the change, never refreshed
  automatically.
- **Both targets in CI**: the same golden suite runs native and under a wasm
  runtime, and must produce identical hashes. That assertion is what actually
  catches hazard 3.

## Decision 7 — What we do not promise

A local-only tool cannot produce an authority-backed signature, and we will say
so in the product, not only here.

- Filigrane Facile has PDF signing code (disabled by default). A server can hold
  a private key; we cannot. A key shipped to a browser is a public key. So we
  **cannot** produce a signature that attests "this mark was applied by a trusted
  party at time T". There is no clever way around this and we will not pretend
  there is.
- **A visible watermark is a deterrent, not a cryptographic guarantee.** It
  raises the cost of reuse and makes misuse attributable when the mark names the
  recipient. It is defeated by a determined attacker: generative inpainting in
  2026 removes marks from photographs well.
- The marks over *text* are the harder case for an attacker, because
  reconstructing covered text requires knowing what it said — which is the
  reason Decision 3 puts the mark diagonally across the document body rather
  than in a corner.
- It does nothing at all against someone who already holds an unmarked copy,
  and nothing against the recipient using the document for the purpose it was
  sent for.

We will also **not** add an invisible or steganographic watermark. In a tool that
runs entirely on the user's device the user can diff marked against unmarked
output and locate it, so it would be security theatre, plus extra code, plus a
claim we could not stand behind.

## Consequences

### What this gets us

Geometry, resolution and encoding all preserved or user-chosen instead of
flattened to 128 DPI A4 JPEG; real controls with a preview; GPS and camera
identity stripped from phone photos; reproducible output we can actually test.

### What this costs us

Rasterisation destroys text layers and screen-reader access on the default path
— the one cost we cannot engineer away, only disclose. Native-resolution
compositing means we handle much larger buffers than a 128 DPI pipeline, which
makes memory the binding constraint on large PDFs
([ADR-0003](0003-pdf-input-rasterisation.md)). Determinism across wasm and
native forces discipline in the raster path, including hand-rolled trigonometry.
Five quality modes and ten appearance parameters are a UI design problem the
official tool does not have.

### What this forecloses

No server-held signing key, ever, without abandoning the premise. No
barcode-aware automation beyond a hand-rolled QR hint. No metadata-preserving
mode on the rasterising path.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| Users pick `overlay` for the searchable output and get a removable mark | Medium | High | Not the default; explicit warning at selection and in the output filename suffix |
| Default opacity 0.30 proves too light to deter, or too heavy to read | Medium | Medium | Live preview makes it adjustable; revisit after real use |
| Blur is sold as a defence it has not been measured to provide | Medium | Medium | Described as cosmetic in the UI; revisit only with a measurement |
| Exclusion zone abused to blank most of the page | Low | Medium | 25% cap, UI warning |
| Golden tests pass natively and fail under wasm because of libm drift | Medium | Low | Hand-rolled trig; both targets asserted equal in CI |
| Accessibility regression draws the same criticism we level at the official tool | Medium | Medium | Disclose in the UI; `overlay` mode; revisit if a text-layer-preserving burn-in becomes feasible |

## Open questions

- [ ] **Needs decision from Samuel:** default mark text. A placeholder that
      suggests recipient + date + purpose teaches users what makes a mark
      attributable, but a pre-filled field that is sent unedited is worse than an
      empty one. Recommendation: empty field, with a non-committal placeholder
      and one-click suggestions.
- [ ] **Needs decision from Samuel:** whether `overlay` mode ships in v1 at all,
      given it is the one mode that can give a user a false sense of protection.
      Recommendation: not in v1; add it once the burn-in path is solid.
- [ ] **Needs decision from Samuel:** whether the 25% exclusion-zone cap is a
      hard limit or a warning the user can override.

## Could not verify

- That blur meaningfully impedes automated watermark removal. This is a
  plausibility argument only; we have measured nothing. Decision 3 keeps blur
  for its cosmetic effect and the ADR says so.
- That a dark grey mark survives greyscale photocopying and heavy
  re-compression better than a pale one. This follows from luminance contrast
  and is near-certain, but we have not run the experiment.
- The specific legibility claim about 128 DPI being marginal for a French
  identity card's small print and a passport MRZ. It matches the 200–300 DPI
  convention for document scanning but we have not tested it on a real card.
- Lines-of-code estimates for the QR finder-pattern locator (~300) and the PRNG
  (~20) are engineering estimates, not measurements.
- Filigrane Facile's exact parameters (alpha 0.52–0.6, −25°, 128 DPI, the QR
  hole) are reported from a prior reading of its source, not re-verified in
  writing this ADR.

## References

- Filigrane Facile — <https://filigrane.beta.gouv.fr/>
- [MTES-MCT/Dossier-Facile-Frontend] — <https://github.com/MTES-MCT/Dossier-Facile-Frontend>
- [MTES-MCT/dossierfacile-backend] — <https://github.com/MTES-MCT/dossierfacile-backend>
- ANTS 2D-Doc specification — <https://ants.gouv.fr/nos-missions/les-solutions-numeriques/2d-doc>
- Exif `Orientation` tag `0x0112` (= 274 decimal, SHORT, default 1, eight values);
  verified from TIFF 6.0 §8, the normative upstream for Exif IFD0 — <https://www.cipa.jp/std/documents/e/DC-008-Translation-2012-E.pdf>
- qpdf manual, `--remove-annotations` — <https://qpdf.readthedocs.io/>
- mupdf `mutool clean` — <https://mupdf.readthedocs.io/en/latest/mutool-clean.html>

[MTES-MCT/Dossier-Facile-Frontend]: https://github.com/MTES-MCT/Dossier-Facile-Frontend
[MTES-MCT/dossierfacile-backend]: https://github.com/MTES-MCT/dossierfacile-backend
