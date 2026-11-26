# ADR-0001: Stack and dependency policy

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** none

## Context

**Pochoir** watermarks identity and administrative documents. It exists because
the French government's Filigrane Facile
(<https://filigrane.beta.gouv.fr/>) uploads your identity document to a server,
and because it degrades the document it returns (see
[ADR-0002](0002-watermark-algorithm-and-quality-modes.md)).

The product's single defining claim is that **nothing leaves the device**. Every
technical decision in this ADR is downstream of that claim, and the claim is
only credible if it is architecturally true rather than promised.

The owner's standing constraint, in his own words, is to be *very lean with
dependencies* — he would rather we re-build or re-code a feature than add a
library we have no control over, that has potential security issues, and that
will bloat the client. That rule is the central input to the second half of this
ADR, and taking it seriously means being concrete: a decision per capability,
with measured numbers, not a slogan.

### Direction already set

These are decided and this ADR records rather than re-opens them:

- A **Rust core crate**, compiled both to **WebAssembly** for the browser and to
  a **native CLI**.
- **Astro** with static output and **Svelte islands** for the UI — not
  SvelteKit.
- **Cloudflare Workers static assets** for hosting.
- **No backend, no database, no accounts, no analytics.**

### A note on names

The product is **Pochoir**, at `pochoir.app`, and the repository is public at
<https://github.com/SamuelBagattin/pochoir>. The workspace crates are
`pochoir-core` and `pochoir-cli`; the CLI binary is `pochoir`.

**This section previously recorded the opposite**: that the code identifiers
still read `filigrane-*`, and that the rename was deliberately deferred so it
would land as one commit rather than as churn across live branches. It landed on
2026-11-26, so the identifiers cited throughout these ADRs are now the ones in
the tree. Where "filigrane" still appears it is deliberate — the ordinary French
word for a watermark, **Filigrane Facile** (the government service this project
exists to improve on), or a third-party repository name — and none of it is a
leftover.

## Part 1 — The stack

### Rust core, two targets

One crate holds the pipeline — decode, composite, encode, write — and two thin
shells wrap it: a `wasm-bindgen` surface for the browser and a `clap`-free
argument parser for the CLI.

The reasons are worth stating because they are not just taste:

- **One implementation means one set of golden tests.** ADR-0002's determinism
  requirement depends on the browser and the CLI producing byte-identical
  rasters. Two implementations could not be held to that, and the test suite
  would stop being evidence about what ships.
- **The CLI makes the pipeline testable without a browser**, which is what makes
  CI cheap and fast.
- **The CLI is a real deliverable**, not a by-product: batch-watermarking a
  folder of scans is a thing people want and the web UI should not try to be.
- Rust gives us a memory-safe language for code that parses hostile input, with
  no runtime and no garbage collector to ship.

**Alternative considered: TypeScript everywhere**, using the browser's own
codecs (`createImageBitmap`, `OffscreenCanvas`, `canvas.convertToBlob`). It is
genuinely tempting — those cost zero bytes, since the engine already contains
them. Rejected because it gives up the CLI entirely, because canvas rendering is
not bit-reproducible across browsers (so golden tests become perceptual
comparisons), and because the owner prefers Rust. We do still use the browser's
codecs in one place, where it is the only option: HEIC (below).

### Astro + Svelte islands, not SvelteKit

**SvelteKit's value is its server.** Server-side rendering, form actions,
`+page.server.ts`, endpoints, hooks — that is what you are buying. We have no
server by design, so we would be adopting a framework whose centre of gravity is
a thing we have promised not to have, then configuring an adapter to turn it
off.

Worse, it leaves a loaded gun on the table: adding an endpoint in SvelteKit is
one file. The architecture should make the privacy promise hard to break by
accident, and a framework that makes server code trivial to add works against
that.

Astro's model is the inverse: static HTML by default, zero client JavaScript
unless a component is explicitly marked as an island. A public-good tool needs
real content pages — what it does, why it is private, a FAQ, an accessibility
statement — and those should be static HTML with no JavaScript at all. Only the
watermarking UI needs to be interactive, and that is exactly one island.

**Runner-up: plain Vite + Svelte**, with no site framework. It would work and it
is leaner still. It lost because the content pages would then either be part of
the SPA bundle or hand-maintained HTML, and because Astro's "zero JS unless you
ask" default is itself a useful guard rail. If the content pages turn out to be
a single page, this decision should be revisited — the trigger is "we have fewer
than three static pages".

### Cloudflare Workers static assets, with no Worker code

The deployment is a directory of files served from Cloudflare's edge. **We will
not write Worker code**, for two reasons: a Worker is a server, which is the
thing we promised not to have; and `_headers` does not apply to
Worker-generated responses (see
[ADR-0003](0003-pdf-input-rasterisation.md)), so adding one would silently break
our security headers.

**Alternatives considered.** GitHub Pages cannot set custom response headers at
all, which rules it out — we need a Content-Security-Policy, and a strict CSP is
part of how we make the privacy claim checkable (a `connect-src 'self'` that
forbids outbound requests is evidence, not marketing). Netlify and Vercel would
both do the job; Cloudflare wins on the owner already working with it daily and
on the free tier being sufficient for a project with no revenue. Self-hosting
adds operational burden for no gain.

### No backend, no database, no analytics

No backend is the product, not an optimisation. A server that receives identity
documents is a GDPR data controller: it needs a lawful basis, a retention
policy, breach notification, and it becomes a target holding other people's ID
cards. "We delete immediately" is a promise that cannot be verified from
outside. Not receiving the file is the only version of that promise that is
checkable.

**No analytics follows from the same logic, and we should be honest that it
hurts.** Analytics on a tool that handles identity documents is a contradiction
even when self-hosted, even when anonymised — the page that the user is on *is*
the sensitive fact. So we accept that we will not know how many people use it,
which formats fail, where users give up, or whether a release broke something.
That is a real cost, paid deliberately. What we get instead: a public issue
tracker, and an "export diagnostic report" button that composes a text blob the
**user** inspects and sends, if they choose to.

### Public auditability is now part of the claim

The repository is public. That is not incidental to the product: a privacy tool
whose source nobody can read is an assertion, not a guarantee, and "nothing
leaves your device" is exactly the kind of claim a user cannot check from the
outside.

But publishing source does not, by itself, prove anything about what the user
actually runs. **They execute whatever the CDN served them, not what is on
GitHub**, and for a web-delivered tool that gap is the real weak link — far more
so than for a downloadable binary, because the bytes can change silently on any
deploy. Being public therefore sharpens three requirements rather than
satisfying them:

1. **No obfuscated or unmappable build.** The shipped bundle must be
   reconstructible from the repository with one documented command, and we ship
   source maps.
2. **Reproducible builds, with a published hash of the deployed bundle**, so that
   a third party can rebuild from a tag and compare. This was listed below as
   "worth a future ADR"; being publicly auditable promotes it to something we
   should do early, because it is the only thing that closes the
   source-to-served-bytes gap.
3. **Determinism earns its keep twice.** ADR-0002's requirement that output be a
   pure function of input and parameters exists for golden tests, but it is also
   what makes an independent rebuild comparable at all.

The strict CSP described above belongs in the same group: a `connect-src 'self'`
that forbids outbound requests is evidence a reviewer can read off the response
headers.

**What this does *not* do is settle the licence question**, and it is worth
saying why, because the intuition runs the other way. If anything, public
auditability leans *toward* the copyleft recommendation below rather than away
from it: AGPL §13 keeps downstream network deployments auditable too, whereas a
permissive licence specifically permits a closed, non-auditable fork that keeps
our privacy language intact. The fair counter is that a permissive licence
maximises the number of people who read and reuse the code, and that the
auditability of *our own* deployment is unaffected either way. Both points are
recorded in the open question; neither changes the recommendation.

**Alternative considered: an optional server-side mode** for very large
documents or old browsers. Rejected outright. The moment a server-side mode
exists, the honest description of the product becomes "usually local", the
threat model acquires a server, and every claim needs a footnote. The
differentiator is not negotiable at this level.

## Part 2 — Dependency policy

### The tests

Every dependency is judged against the owner's three tests, plus one this
project adds:

1. **Control** — can we fix it, fork it, or understand it in an afternoon?
2. **Security surface** — does it parse attacker-controlled bytes? How much
   `unsafe`? How many transitive crates come with it?
3. **Client bytes** — what does it add to the wasm the user downloads,
   **measured in brotli, not guessed**?
4. **Privacy boundary** — could it make a network request, read a clock, or
   otherwise leak? (In practice: no build-time dependency may appear in the
   runtime wasm without passing the first three.)

### An honest correction before the table

We measured the alternative rather than assuming it. A probe crate taking the
obvious small focused crates for the whole common path — `zune-jpeg`,
`jpeg-encoder`, `png`, `ttf-parser`, `fontdue`, `pdf-writer`, `little_exif` —
built for `wasm32-unknown-unknown` with `opt-level = "z"`, LTO,
`panic = "abort"` and `strip`, exercising each crate's real code path so nothing
is dead-stripped:

| Probe | Raw | Brotli -q11 | Transitive crates |
| --- | --- | --- | --- |
| empty `cdylib` baseline | 115 B | 97 B | 0 |
| `zune-jpeg` + `jpeg-encoder` | 177 790 B | **50 706 B** | 3 |
| `png` (decode + encode) | 139 208 B | **59 336 B** | 9 |
| `ttf-parser` + `fontdue` | 135 901 B | **54 201 B** | 8 |
| `pdf-writer` | 40 954 B | **15 725 B** | 5 |
| **all of the above combined** | **420 997 B** | **149 703 B** | **34** |

*(Measured in this session: macOS `aarch64-apple-darwin` host, `rustc 1.96.0`,
target `wasm32-unknown-unknown`, no `wasm-opt` pass — `wasm-opt -Oz` would
typically shave a further 10–20%.)*

So the whole "just take the crates" option for the common path is about
**150 KiB brotli and 34 crates**. A hand-rolled equivalent might land at
60–120 KiB *(estimate — the code does not exist yet)*.

**That means the client-bytes argument for hand-rolling is worth perhaps 30–90
KiB, and we should stop pretending it is the main reason.** It is real but
modest. The arguments that actually carry weight are control and crate count:
34 crates is 34 upstreams that can break, change licence, or be compromised, for
a tool whose entire pitch is that you can trust it.

### The principle that makes this coherent

Blanket "hand-roll everything" is wrong in one specific, important way:
**hand-rolling a decoder replaces a heavily-fuzzed third-party parser with our
own less-fuzzed one.** `zune-jpeg` and `png` have been run over millions of
hostile files; our first draft will not have been. For code that eats
attacker-controlled bytes, "we wrote it ourselves" is not automatically a
security win, and claiming otherwise would be the kind of reasoning that gets
people owned.

Encoders and writers are the opposite case: they process **our own** data, their
correctness is checkable against any reader, and a bug in them is a broken file
rather than a compromise.

So the rule is:

> **Hand-roll everything that writes. Depend, deliberately, for the things that
> parse hostile input — unless the parser is small enough that we can audit all
> of it.**

This is why the table below depends for JPEG and PNG *decode* and hand-rolls
nearly everything else.

### The decisions

| Capability | Decision | Reasoning | Cost |
| --- | --- | --- | --- |
| **JPEG decode** | **Depend — `zune-jpeg`** | Parses hostile input. A correct baseline + progressive decoder is 3 000–5 000 lines and ours would be the least-tested JPEG decoder in the world. Licence `MIT OR Apache-2.0 OR Zlib` (verified from the published manifest), 1 direct dependency | 3 crates; 50.7 KiB brotli shared with the encoder probe |
| **JPEG encode** | **Hand-roll** | Processes our own pixels. Baseline encoder = forward DCT, quantisation, Huffman coding, JFIF framing — bounded and fully verifiable against any decoder. Also avoids `jpeg-encoder`'s licence, which is **`(MIT OR Apache-2.0) AND IJG`** (verified) — a *conjunctive* term carrying an attribution obligation we would have to honour. Owning the quantisation tables also gives ADR-0002 its determinism | ~700 lines |
| **PNG decode** | **Depend — `png`** | Hostile input, same reasoning as JPEG. Licence `MIT OR Apache-2.0` | 9 crates; 59.3 KiB brotli |
| **PNG encode** | **Depend — `png`** (the same crate) | A deliberate exception to the rule above, on measured marginal cost: we have already taken the crate for decode, so its encoder costs **zero additional crates and almost no additional bytes**. Hand-rolling it to satisfy a principle would be dogma. A principle should yield to a measured marginal cost of nil | included above |
| **DEFLATE / inflate** | **Depend** (arrives with `png`) | Inflate parses hostile input. Worth recording, now verified: a **stored** block (`BTYPE = 00`) is legal DEFLATE (RFC 1951 §3.2.3), each limited to 65 535 bytes (§2), and PNG mandates deflate with no preset dictionary — so a ~40-line "compressor" emitting only stored blocks produces **spec-valid** PNG and `/FlateDecode` output. We note it and decline to use it for PNG: with no LZ77 gain the output is about `width × (height × bpp + 1)` bytes, which for a full-colour photograph is a severe size regression. It remains the right trick if we ever need to emit a small `/FlateDecode` stream without a compressor | included above |
| **HEIC/HEIF decode** | **Browser only, best-effort. No Rust dependency** | See the dedicated section below | 0 lines, 0 bytes |
| **TrueType parse + glyph rasterisation** | **Hand-roll** | The clearest case on the list. We ship **one font that we choose**, in-repo, so the input is not hostile and the scope collapses: `head`, `hhea`, `hmtx`, `cmap` format 4, `loca`, `glyf`. No CFF/PostScript outlines (we pick a `glyf` font), no shaping, no variable axes, no font discovery. Plus a scanline rasteriser with analytic antialiasing over quadratic Béziers. Measured alternative: 54.2 KiB brotli and 8 crates for `ttf-parser` + `fontdue` | ~1 000 lines |
| **NFC normalisation** | **Hand-roll** | ADR-0002 requires it so that French accented input does not render as stray combining marks. We need only Latin-1 Supplement and Latin Extended-A, which is a small composition table — not a Unicode library | ~100 lines + table |
| **Alpha compositing, blur, rotation, resampling** | **Hand-roll** | Our own pixels throughout. Source-over alpha ~30 lines; a separable 3-pass box blur approximating Gaussian ~80; affine rotation with bilinear sampling ~120; box/Lanczos downsampling ~120. No dependency is justified — and for **blur there is no browser alternative either**: `OffscreenCanvasRenderingContext2D.filter` is **not supported in any Safari version** (WebKit bug 275436, still open), and `ctx.filter` is listed as disabled by default in Safari 18.0–26.5. So the hand-rolled blur is not a preference, it is the only portable option | ~350 lines |
| **PDF writing** | **Hand-roll** | Our own data, and a bounded subset of the spec: catalog, pages tree, one image XObject per page, content stream, xref, trailer. `/DCTDecode` lets JPEG bytes be embedded verbatim, so the writer never recompresses *(to be confirmed against ISO 32000-1)*. The measured alternative, `pdf-writer`, is genuinely cheap — 15.7 KiB brotli, 5 crates — so this is the closest call in the table. It goes to hand-roll because the output PDF is the artefact we most need total control over: ADR-0002's metadata hygiene (no dates, no XMP, no version string, content-derived `/ID`) is easier to guarantee in 500 lines we wrote than to police through someone else's API. **Two `/DCTDecode` passthrough traps to handle up front:** the image XObject's `/ColorSpace` and `/BitsPerComponent 8` must agree with the JPEG's own SOF marker, and CMYK JPEGs carrying an Adobe APP14 marker need `/Decode [1 0 1 0 1 0 1 0]` or they render inverted | ~500 lines |
| **EXIF read + metadata strip** | **Hand-roll** | We need exactly one tag — IFD0 `Orientation`, **`0x0112` = 274 decimal, type SHORT, count 1, default 1, eight defined values** (verified from TIFF 6.0 §8, which is the normative upstream for Exif IFD0) — plus resolution, plus walking JPEG segment markers and PNG ancillary chunks to drop everything else. `little_exif` brings **6 direct dependencies including `quick-xml` and `brotli`** (verified from its manifest): an XML parser and a compressor, to read one `u16`. This is the clearest hand-roll on the list, and it *removes* a metadata-parsing attack surface rather than adding one. We never need to *parse* XMP — only to delete it, which the byte-level walk already does | ~180–320 lines |
| **ZIP output** | **Hand-roll, stored entries only** | For ADR-0002's N-in-N-out case. Our entries are already-compressed PDFs, so compressing again is pointless work. Local file header + name + raw bytes per file, then central directory, then EOCD, plus CRC-32 (a 256-entry table and a ten-line loop). With method 0, compressed size equals uncompressed size, so sizes are known up front and no data descriptor is needed. **Two caveats to implement, not discover:** ZIP64 is required above 4 GiB total or 65 535 entries, and the UTF-8 name flag (general-purpose bit 11) must be set, since French filenames are not ASCII | ~200 lines |
| **PDF *input* rendering** | **Depend — `hayro`** | The one genuinely unavoidable dependency. Nobody hand-rolls a PDF interpreter. Fully reasoned in [ADR-0003](0003-pdf-input-rasterisation.md), and isolated in a lazily-loaded module so the common path never downloads it | 74 crates; 1.28 MiB brotli, loaded on demand only |
| **UI — state management** | **None** | Svelte 5 runes *are* the state management. Nothing to add | 0 |
| **UI — CSS framework** | **None** | Hand-written CSS with custom properties. A utility framework is bytes plus a build step for one interactive page | 0 |
| **UI — icon set** | **None** | Inline the handful of SVG paths we need, as markup | 0 |
| **UI — file drop** | **None** | `dragover` / `drop` and `DataTransfer.files` is about 30 lines | 0 |

**Totals.** Roughly **3 000–3 500 lines of Rust** that we own, plus a few hundred
lines of TypeScript and CSS. Runtime dependencies on the common path:
`zune-jpeg` and `png`, which **measure 11 unique crates / 14 `[[package]]`
entries** once their shared transitives are deduplicated (`adler2`, `bitflags`,
`cfg-if`, `crc32fast`, `fdeflate`, `flate2`, `miniz_oxide`, `png`,
`simd-adler32`, `zune-core`, `zune-jpeg`).

On the PDF path, `hayro` on top — and here a measured result worth designing
around: **adding `zune-jpeg` and `png` to `hayro` adds zero crates.** `hayro`
depends on `image`, which already pulls both, along with `flate2` and
`miniz_oxide`. The full adopted set measures the same 74 unique crates / 81
`[[package]]` entries as `hayro` alone.

The consequence is a clean property for the CI guard: **the core module's
dependency closure is a strict subset of the PDF module's.** The per-crate
allowlist therefore nests rather than diverging, and a crate appearing in the
core that is not in that 11 is unambiguously new.

### HEIC: the capability we cannot deliver well

This deserves its own treatment because every option is bad, and pretending
otherwise would be dishonest.

- **Hand-rolling is not realistic, and now we have anchors.** HEIC is
  HEVC-coded. `libde265`, the reference-grade C/C++ HEVC decoder, is
  **1 012 488 bytes across 74 files** in its core directory alone, excluding
  SIMD and encoder paths; `rust_h265`, a pure-Rust HEVC decoder, is
  **1 084 586 bytes across 28 files**. Both land around **25 000–35 000 lines**,
  before the ISOBMFF/HEIF container (`ftyp`, `meta`, `iinf`, `iloc`, `iprp`,
  `grid` tiling, `hvcC`). This is categorically unlike baseline JPEG; the
  hand-roll rule does not stretch this far.
- **`libheif-rs`'s MIT licence is misleading.** The crate is MIT, but the native
  library it wraps, libheif, is distributed under the **LGPL**, it uses
  `libde265` for decoding by default, and its encoder path pulls **x265, which
  is GPL**. Plus a C toolchain for wasm32 and a C parser for hostile input.
- **The pure-Rust crates are too young to trust.** A cluster of them appeared
  between April and November 2026: `heifer` 0.7.3 (`MIT OR Apache-2.0`,
  `#![forbid(unsafe_code)]`, ~250 KB in WASM claimed) has **6 stars and 42
  versions in seven weeks**; `heif-oxide` has 1 star. Their download counts are
  registry-mirror noise, not adoption — `heif-oxide` 0.1.0 shows 692 043
  downloads while 0.2.0 shows 127. **A weeks-old single-author decoder parsing
  attacker-controlled HEVC bitstreams, in the same address space as the user's
  identity documents, is not a dependency this project should take on trust.**
- **Browser support is poor and lopsided — verified, and worse than hoped:**

  | Mechanism | Chrome | Firefox | Safari |
  | --- | --- | --- | --- |
  | HEIC as a decodable image (`<img>`, `createImageBitmap`) | **No** — no version | **No** — no version | **Yes, 17.0+** |
  | WebCodecs `ImageDecoder` (the API) | 94 | 133 | 27.3, **beta only** as of 2026-11-26 |

  The ground truth for Chromium is Blink's own `kSupportedImageTypes` list,
  which contains JPEG, WebP, PNG, APNG, GIF, BMP, ICO, XBM, and AVIF/JXL behind
  build flags — **and no HEIC or HEIF entry at all**. WebCodecs does not change
  this: Chromium's `ImageDecoder.isTypeSupported` delegates to the same list.
  Firefox's HEIF tracking bug (1402293) is still open and marked **P5**, and a
  related bug was closed WONTFIX with "We have no plans to support images of this
  type". Notably **Chrome on macOS does not help** — Blink does not call the OS
  decoder for image resources, so the Apple pipeline is unreachable from it.
- **HEVC is patent-encumbered**, pool-licensed via Access Advance / Via LA /
  MPEG LA, and caniuse attributes browsers' reluctance to HEIC being "complex
  and expensive to license". Whether a client-side decoder in a free tool
  triggers royalty obligations is a **legal** question we have not answered.

**Decision: try the browser, and fail clearly.** We attempt
`createImageBitmap()` on the blob; if it throws, we tell the user plainly that
their browser cannot read HEIC and that converting to JPEG will work — with a
hint that iOS can be set to capture JPEG directly. In practice HEIC works on
Apple devices, which is where HEIC files come from, and fails gracefully
everywhere else. The native CLI does not support HEIC at all.

This costs zero bytes and takes zero dependencies, and it is **worse than what
Filigrane Facile offers** — their server accepts HEIC. We should say so in the
product rather than hide it.

### Where the browser's own codecs are and are not used

The browser contains a JPEG encoder, a PNG encoder, a blur filter and a
compositor, all at zero byte cost. It is reasonable to ask why we ship our own.

The answer is that **the CLI cannot reach them**, and ADR-0002's determinism
requirement needs the browser and the CLI to produce identical output. Canvas
output is not specified to be bit-identical across engines, so building on it
would mean abandoning golden-image testing — the thing that lets us claim the
algorithm works. We therefore use the browser's codecs for exactly one thing,
HEIC, where there is no alternative and where no determinism claim is possible
anyway.

One useful thing the browser does give us for free: `createImageBitmap()` takes
an `imageOrientation` option and `<img>` honours EXIF orientation, so the
browser can *apply* orientation without our help. We still read the tag
ourselves, because the CLI needs to and because ADR-0002 requires us to know
what we are stripping.

### Build-time dependencies are governed differently, and we should admit it

There is an obvious tension in being austere about Rust crates while the UI
build uses Astro, Svelte and Vite, whose npm trees run to hundreds of packages.
Pretending that is not a contradiction would undermine the rest of this
document.

The distinction we draw is **runtime versus build-time**:

- **Runtime client code** — anything in the wasm the user downloads, or the JS
  that ships — is governed by the four tests above, strictly.
- **Build-time tooling** never reaches the user's browser and never parses the
  user's documents. It is judged on supply-chain risk alone: a committed
  lockfile, pinned versions, no postinstall scripts we have not read, and
  dependency updates reviewed rather than automated.

This is a real weakening, and it is the honest place for it. A compromised build
tool could inject anything into the shipped bundle, so "build-time only" is not
"harmless" — it is "a different risk, managed differently". Reproducible builds
and a published hash of the deployed bundle are the proper mitigation, and now
that the repository is public they are a near-term requirement rather than a
nicety — they deserve their own ADR soon.

### Rule: every future dependency needs its own ADR

No dependency is added without an ADR that states:

1. the capability, and why hand-rolling was rejected against the principle
   above;
2. the **measured** brotli byte delta for the target it affects — measured, not
   estimated;
3. the transitive crate delta;
4. the full SPDX licence expression, including any **conjunctive** (`AND`)
   terms, which carry obligations rather than choices;
5. whether it parses untrusted input, and how much `unsafe` it contains;
6. which module it lands in, and whether that module is on the common path.

And, mechanically: **the `deps-check` threshold bump goes in the same commit as
the dependency**, with the ADR number in the commit message. A threshold raised
in a separate commit is a threshold raised without review.

The guard currently stands at 2 — the two workspace crates. Measured, the
decisions above move it to **14** `[[package]]` entries for the common path and
**81** once `hayro` is in the lock file. Treat both as orientation, not as
values to copy: **the allowlist and the threshold must be generated by
measurement in CI**, because they shift with feature resolution. An earlier
draft of this ADR cited 88, which came from a probe that also included `krilla`
with its default features disabled — a crate we do not adopt, measured in a
configuration we would not use. That is precisely the failure mode this
paragraph warns about. The second number is large enough that
the global count stops being a useful guard, which is why
[ADR-0003](0003-pdf-input-rasterisation.md) recommends the check become
**per-crate**: a hard assertion that `pochoir-core`'s dependency closure stays
inside an explicit allowlist, with the global count kept only as a blunt alarm.

## Decision

We adopt the stack as described — Rust core compiled to wasm and a native CLI,
Astro with Svelte islands, Cloudflare static assets, no backend, no database, no
analytics — and the dependency policy above: **hand-roll everything that writes;
depend deliberately for parsers of hostile input; one unavoidable dependency for
PDF rendering, isolated behind a lazy module; nothing at all on the UI side.**

The release profile keeps `panic = "abort"`, decided and reasoned in
[ADR-0003](0003-pdf-input-rasterisation.md) alongside the worker-based recovery
model it depends on.

## Consequences

### What this gets us

A privacy claim that is architecturally true and checkable from outside. One
implementation of the algorithm, tested once, running identically in two places.
A client bundle on the order of a couple of hundred kilobytes for the common
case. A dependency list short enough that a sceptical reader can actually audit
it — which, for a tool asking to be trusted with identity documents, is the
whole point.

### What this costs us

Three thousand-odd lines of codec, font and PDF code that we own and must
maintain, debug and keep correct — including a JPEG encoder and a glyph
rasteriser, neither of which is trivial to get right. No HEIC outside Apple
devices, which is a capability regression against the tool we are trying to
beat. Total blindness about usage and failures. And a standing obligation to
write an ADR every time someone wants to add a crate, which will feel like
bureaucracy on the day someone just wants to get something done.

### What this forecloses

Any server-side processing, ever, without a new ADR that supersedes this one.
SvelteKit-style server endpoints. Canvas-based rendering as the primary
pipeline. Analytics of any kind. A permissive licence for the project, if the
recommendation below is accepted.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| Hand-rolled JPEG encoder produces subtly wrong colour or chroma | Medium | Medium | Round-trip tests against `zune-jpeg` and against the browser's decoder; visual goldens |
| Hand-rolled glyph rasteriser looks worse than a real one | **Medium** | Medium | Compare against `fontdue` output in tests; the trigger to take a dependency is "visibly worse at the default size" |
| The 3 000-line estimate is optimistic and the project stalls | **Medium** | **High** | Build in order of risk: compositor and PDF writer first (they gate everything), encoder next, font rasteriser last — it can temporarily use a dependency |
| Hand-rolled code becomes the project's bottleneck for a contributor | Medium | Medium | Each hand-rolled module gets a doc comment citing the spec section it implements |
| HEIC gap drives users back to Filigrane Facile | Medium | Medium | Clear in-product message; revisit when `heifer` matures or Chrome ships HEIC |
| Build-time npm tree is compromised and injects into the bundle | Low | **High** | Lockfile, pinned versions, reviewed updates; future ADR on reproducible builds and published bundle hashes |
| The per-capability decisions were made before any code exists and prove wrong | **High** | Low | That is what superseding ADRs are for. Each row states the trigger |

## Open questions

**Every question in this section is closed.** They were answered on 2026-11-26,
after this ADR was accepted, and are recorded here in place rather than in a new
ADR, because none of the answers changes the decision this ADR records — see
*Closing an open question* in [README.md](README.md).

Each item names who decided it. Several were raised as "needs decision from
Samuel" and were in fact settled by the orchestrating session, under Samuel's
standing rule that he is asked only for product direction, spending, legal and
irreversible calls; those were reported to him before he approved this ADR. The
wording of each question as it was originally raised is kept beneath its answer.

- [x] **Licensing — decided by Samuel**, and superseded in this respect only by
      [ADR-0005](0005-licensing.md).

      The project is split-licensed: the crates `MIT OR Apache-2.0`, the
      application and site `AGPL-3.0-or-later`. So the recommendation below was
      **accepted for the application and declined for the engine**. ADR-0005
      carries the options, the reasoning and the costs; the reasoning below is
      left as written.

      *As raised:* **needs decision from Samuel — licensing. This is the big one,
      and we are deliberately not deciding it.**

      The scaffolding currently carries `license = "UNLICENSED"` and no `LICENSE`
      file, which means that today **nobody may legally use or contribute to the
      project**. It should be settled before the first external contribution,
      because relicensing later needs every contributor's agreement.

      **Recommendation: `AGPL-3.0-or-later`, for the whole project.** The reason
      is specific rather than ideological. The harm this project exists to
      prevent is someone handling identity documents on a server; the harm a
      permissive licence enables is someone taking this code, adding a server,
      and marketing it with our privacy language intact. AGPL §13 is the one
      licence term that addresses exactly that case — a modified version offered
      over a network must publish its source — so the licence and the product
      thesis point the same way. All dependencies chosen above are
      `MIT OR Apache-2.0`-style and compatible with it. The repository being
      public does not decide this either way — see *Public auditability* above
      for why it slightly favours copyleft rather than permissive.

      **Runner-up: `MIT OR Apache-2.0`** (the Rust ecosystem default), which
      maximises reuse and would let the core crate be adopted as a library. It
      loses because a watermarking pipeline has modest value as a library — it
      is an application — so the reuse we would be buying is small, while the
      fork-and-add-a-server scenario it permits goes to the heart of the
      product.

      **The honest cost of the recommendation:** AGPL makes the core crate
      effectively unusable in anyone's permissive project, so it will not be
      adopted by others, and AGPL crates see little uptake on crates.io. If the
      ambition for this project includes being a reusable Rust/WASM building
      block, that argues for the runner-up. *(Side note, not a reason: AGPL would
      also unblock MuPDF, which ADR-0003 rules out on licence grounds.)*

- [x] **HEIC on Apple only for v1 — accepted. Decided by the orchestrator.**

      **We ship no HEVC decoder at all.** HEIC decoding happens only where the
      browser does it natively, which in practice means Safari 17+, and every
      other browser gets an explicit "convert to JPEG and retry" message rather
      than a silent failure. The recommendation below was taken as it stood.

      This is a capability regression against Filigrane Facile, whose server
      accepts HEIC, and the product should say so rather than hide it. It is
      also the cheapest version of the gap: HEIC files come from Apple devices,
      which is where it works.

      *As raised:* whether HEIC-on-Apple-only is acceptable for v1, or whether
      the capability gap against Filigrane Facile is serious enough to reconsider
      a dependency. Recommendation: accept it for v1 and revisit when a pure-Rust
      decoder has a track record. Note this is only a gap on Chrome and Firefox;
      on the Apple devices that produce HEIC files it works.
- [x] **HEVC patent-royalty exposure — dissolved rather than answered.**

      **The question is removed by the decision above, not settled by it.** There
      is no royalty exposure in a decoder we do not ship, and we ship none. So
      this ADR does not take a position on the legal question, and nobody should
      read it as having cleared one.

      **It returns intact the moment anyone proposes bundling an HEVC decoder** —
      `libheif`, `heifer`, or any other. That would need its own ADR *and* a
      legal opinion Samuel does not currently have, obtained before the
      dependency is taken rather than after.

      *As raised:* whether shipping *any* HEVC decoder, including a pure-Rust
      one, creates a patent-royalty exposure for a free public tool. We flag it
      because the answer is legal rather than technical and because our
      recommendation above happens to avoid the question entirely (we ship no
      decoder). If a dependency is ever reconsidered, this must be answered
      first.
- [x] **Hand-roll scope — accepted as written, with one condition. Decided by
      the orchestrator.**

      The condition is the one the recommendation already implied, now binding:
      **the TrueType parser and glyph rasteriser are built last**, after the
      compositor, the PDF writer and the JPEG encoder, so that if they slip they
      can never block a release. The risk table above orders the work the same
      way.

      `ttf-parser` + `fontdue` — 54 KiB brotli, 8 crates — is the **named escape
      hatch**, not a fallback anyone may reach for quietly: taking it needs its
      own ADR under the rule in this document, like any other dependency. The
      trigger stated in the risk table stands: the hand-rolled rasteriser being
      visibly worse at the default size.

      *As raised:* whether the hand-roll scope is accepted as stated, or whether
      `ttf-parser` + `fontdue` (54 KiB brotli, 8 crates) should be taken for v1
      to get to a shipping product sooner, with the hand-rolled rasteriser as a
      follow-up. Recommendation: hand-roll, but build it last so it cannot block
      a release.

## Could not verify

**Measured or read first-hand in this session:** every byte and crate-count
figure in the probe table; the licence expressions and direct-dependency counts
of `zune-jpeg`, `jpeg-encoder`, `png`, `ttf-parser`, `fontdue`, `pdf-writer`,
`little_exif`, `fdeflate`, `miniz_oxide` and `krilla`, read from their published
manifests — including `jpeg-encoder`'s conjunctive
`(MIT OR Apache-2.0) AND IJG` and `little_exif`'s six direct dependencies.

**Verified from primary sources during research for this ADR:** that a stored
DEFLATE block (`BTYPE = 00`) is legal and capped at 65 535 bytes (RFC 1951 §2,
§3.2.3, §3.2.4) and that PNG mandates deflate with no preset dictionary; the
Exif/TIFF `Orientation` tag as `0x0112` = 274 decimal, SHORT, count 1, default 1,
eight values (TIFF 6.0 §8); browser HEIC support (caniuse HEIF, Blink's
`kSupportedImageTypes`, Firefox bug 1402293, Safari 27.3 beta release notes);
`OffscreenCanvas` support floors (Chrome 69, Firefox 105, Safari 17.0) and the
total absence of `OffscreenCanvasRenderingContext2D.filter` in Safari (WebKit bug
275436); libheif's LGPL licensing and x265's GPL; and HEVC decoder size anchors
from `libde265` and `rust_h265`.

**Still not verified — flagged rather than asserted:**

- **`/DCTDecode` embedding baseline JPEG bytes verbatim**, and `/FlateDecode`
  being RFC 1950/1951. Both are near-certain and corroborated by Adobe's own
  secondary documentation, but **no verbatim ISO 32000-1 §7.4.8 / §7.4.4 text
  was obtained** — the free Adobe-published copy exceeds our fetch limit and the
  mirrors returned 403/404. Someone should open the ISO PDF and quote both
  sections before we cite them. ADR-0002's quality modes depend on the first.
- The CIPA DC-008 (Exif 2.32) verbatim `Orientation` text. We substituted
  TIFF 6.0 §8, which is the normative upstream for Exif IFD0, read directly.
- ZIP method 0 (stored) against the PKWARE APPNOTE itself.
- Whether Safari 27.3's `ImageDecoder.isTypeSupported("image/heic")` returns
  true. This is a one-line runtime test that should be run in the beta before any
  UI copy commits to it.
- `heifer`'s conformance and its ~250 KB WASM claim, both self-reported by a
  weeks-old crate.
- Whether a client-side HEVC decoder in a free tool triggers HEVC royalty
  obligations — a legal question, raised as an open question above.
- **Every line-of-code estimate in the decision table.** These are engineering
  estimates for code that does not exist, and the totals should be read as such.
  The 60–120 KiB brotli figure for the hand-rolled core is likewise an estimate.
- ~~That the common path's dependency closure deduplicates to ~10–12 crates.~~
  **Now measured: 11 unique crates / 14 `[[package]]` entries**, and the full
  adopted set including `hayro` is 74 / 81 — unchanged by adding `zune-jpeg` and
  `png`, since `hayro`'s `image` dependency already contains them.
- That Astro's static output and Svelte's compiled islands are as lean in
  practice as claimed. Measure the shipped JS before making any public claim
  about bundle size.
- No primary-source **per-crate added WASM size** measurement exists for any of
  these crates in published form; every size number above is our own
  measurement, which is why they are stated with the build flags that produced
  them.

## References

- Filigrane Facile — <https://filigrane.beta.gouv.fr/>
- `zune-jpeg` — <https://crates.io/crates/zune-jpeg>
- `jpeg-encoder` — <https://crates.io/crates/jpeg-encoder>
- `png` — <https://crates.io/crates/png>
- `ttf-parser` — <https://crates.io/crates/ttf-parser>
- `fontdue` — <https://crates.io/crates/fontdue>
- `pdf-writer` — <https://crates.io/crates/pdf-writer>
- `little_exif` — <https://crates.io/crates/little_exif>
- `heifer` — <https://crates.io/crates/heifer>
- RFC 1951, DEFLATE — <https://www.rfc-editor.org/rfc/rfc1951>
- PNG specification — <https://www.w3.org/TR/png-3/>
- ISO 32000-1 (PDF 1.7) — <https://www.iso.org/standard/51502.html>
- PKWARE ZIP APPNOTE — <https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT>
- TIFF 6.0 specification (normative upstream for the Exif `Orientation` tag) — <https://web.archive.org/web/20210108172697/https://www.adobe.io/content/dam/udp/en/open/standards/tiff/TIFF6.pdf>
- CIPA DC-008 (Exif) — <https://www.cipa.jp/std/documents/e/DC-008-Translation-2012-E.pdf>
- ISO 32000-1:2008, free Adobe-published copy — <https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf>
- Blink supported image MIME types (`mime_util.cc`) — <https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/common/mime_util/mime_util.cc>
- Firefox HEIF tracking bug 1402293 — <https://bugzilla.mozilla.org/show_bug.cgi?id=1402293>
- WebKit bug 275436, `OffscreenCanvasRenderingContext2D.filter` — <https://bugs.webkit.org/show_bug.cgi?id=275436>
- libheif (LGPL; x265 is GPL) — <https://github.com/strukturag/libheif>
- `libde265` — <https://github.com/strukturag/libde265>
- caniuse, `OffscreenCanvas` — <https://caniuse.com/offscreencanvas>
- MDN, `OffscreenCanvas.convertToBlob` — <https://developer.mozilla.org/en-US/docs/Web/API/OffscreenCanvas/convertToBlob>
- MDN, `createImageBitmap` — <https://developer.mozilla.org/en-US/docs/Web/API/Window/createImageBitmap>
- caniuse, HEIF/HEIC image format — <https://caniuse.com/heif>
- Astro, static output — <https://docs.astro.build/en/basics/rendering-modes/>
- Cloudflare Workers static assets — <https://developers.cloudflare.com/workers/static-assets/>
- GNU AGPL-3.0 — <https://www.gnu.org/licenses/agpl-3.0.html>
