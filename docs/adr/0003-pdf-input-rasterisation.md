# ADR-0003: PDF input rasterisation

- **Status:** Accepted
- **Date:** 2026-11-26
- **Deciders:** Samuel Bagattin
- **Supersedes:** none

## Context

[ADR-0002](0002-watermark-algorithm-and-quality-modes.md) burns the watermark
into pixels. For a JPEG or PNG input that is easy: decode, composite, encode.
For a **PDF input** we first have to turn pages into bitmaps, and that needs a
PDF renderer.

A PDF renderer is not a weekend project and it is not a candidate for
hand-rolling under [ADR-0001](0001-stack-and-dependency-policy.md)'s policy. ISO
32000-1 is roughly 750 pages, and a renderer that handles documents real people
actually own needs: a full content-stream interpreter; the 14 standard fonts
plus embedded Type1/CFF, TrueType, Type0/CID and Type3 fonts, with CMaps; nine
stream filters including CCITTFax, JBIG2 and JPXDecode; colour spaces including
ICCBased, Indexed, Separation and Lab; seven shading types; tiling and shading
patterns; transparency groups and soft masks; and decryption. This is the one
place in the project where a dependency is genuinely unavoidable, and saying so
plainly is more honest than pretending otherwise.

So this ADR answers three questions:

1. Which renderer?
2. How do we keep it off the common path — a photo of an identity card, JPEG or
   PNG in, watermarked PDF out — which needs none of it?
3. Does a large scanned PDF fit in a browser's memory, and what does the
   execution model have to look like?

### Evidence gathered first-hand

Unless marked otherwise, the numbers below were **measured in this session** on
macOS (`aarch64-apple-darwin` host), `rustc 1.96.0`, target
`wasm32-unknown-unknown`, in a throwaway crate outside the repository. The probe
calls `hayro::render` over every page of a loaded PDF, so the render path is not
dead-stripped. Release profile: `opt-level = "z"`, `lto = true`,
`codegen-units = 1`, `panic = "abort"`, `strip = true`. **No `wasm-opt` pass** —
`wasm-opt` is not installed here, and it would typically shave a further
10–20%.

## Options considered

### Option A — `hayro` 0.9.0 (recommended)

Pure-Rust PDF rasteriser by Laurenz Stampfl, built on `vello_cpu`,
`fearless_simd`, `kurbo` and `ab_glyph_rasterizer`. CPU only, no GPU, no C, no
`-sys` crates.

| Property | Value | How established |
| --- | --- | --- |
| Licence | Apache-2.0 **OR** MIT | crate `README.md` in the published source |
| `unsafe` | "This crate forbids unsafe code via a crate-level attribute" | crate `README.md` |
| wasm32 build | **Works.** `cargo build --target wasm32-unknown-unknown --release` succeeded in 40.9 s, no feature flags, no C toolchain | measured here |
| Size, default features | **4 471 126 B raw** (4.26 MiB); brotli -q11 **1 341 491 B** (1.28 MiB); gzip -9 1 714 730 B; zstd -19 1 444 470 B | measured here |
| Size, `default-features = false` | **3 754 881 B raw** (3.58 MiB); brotli -q11 **822 613 B** (803 KiB) | measured here |
| Transitive crates | **74** (`cargo tree -p hayro --target wasm32-unknown-unknown`, unique) | measured here |
| Correctness claim | "currently the most comprehensive and feature-complete implementation of a PDF rasterizer in pure Rust… over 1000 PDF files in our regression test suite… scraped from the `pdf.js` and `PDFBOX` test suites" | crate `README.md` |
| Performance | "no effort has been put into performance optimizations, as we are still working on implementing missing features" — the author's own words | crate `README.md` |
| Maturity | 0.9.0, 0.x, effectively single-maintainer | crates.io / repo |

Two corrections to what we believed going in, both established by reading the
published source here:

- **Encryption is supported**, contrary to the README's statement that there is
  a "lack of support for encrypted/password-protected PDF files". `hayro-syntax`
  ships `src/crypto/` with `rc4.rs`, `aes.rs`, `md5.rs`, `sha256.rs`,
  `sha384.rs`, `sha512.rs`; `crypto/mod.rs` maps `AESV2` → AES-128 and `AESV3` →
  AES-256 and handles standard-security-handler revisions 2–6. `Pdf` exposes
  both `new` and `new_with_password`. The README appears stale. This matters a
  lot for us: French administrative PDFs — bank statements, *avis
  d'imposition* — are routinely encrypted with an owner password and an empty
  user password, and a renderer that refused them would be useless.
- **`embed-fonts` and `embed-cmaps` are default features** and cost real bytes:
  ~700 KB raw, ~519 KB brotli, i.e. about 39% of the brotli-compressed module.
  They supply fallback fonts and CMaps for documents that reference standard
  fonts without embedding them. Turning them off is tempting and probably wrong
  for real-world administrative documents; it is a measured knob, not a free
  win.

**Production evidence, and it is the strongest single argument for hayro:** the
Typst toolchain uses it. `typst-svg` 0.15.1 (1 644 591 downloads) and
`typst-render` 0.15.1 (191 812 downloads) both depend on `hayro`, among 87
reverse dependencies. The crate itself has 6 336 345 total downloads, 886 stars,
and was created 2025-04-22 with commits through 2026-11-19.

Author-stated remaining gaps, quoted because they are the risk: **"timeouts and
out-of-memory errors are still possible on untrusted input"**, **"no glyph
caching yet"**, and the **"API surface … still being designed"**. Explicitly out
of scope: knockout groups, CMYK blending, 3D content. Its 121 open issues are
predominantly *fidelity* bugs found by its own regression and fuzz testing
(xref-repair performance, `/SMask` truncation, mesh-shading cropping) rather
than missing features — which is the signature of a project being driven by a
test corpus rather than by feature requests.

The OOM-on-untrusted-input admission matters a great deal to us, because our
input is whatever file a stranger emailed the user. It is the reason the
execution model below is non-negotiable.

### Option B — `pdfium-render` 0.9.6 + a prebuilt pdfium WASM binary

The crate is MIT OR Apache-2.0 and does support wasm32, but **not as Rust
compiled to wasm**: pdfium is C++ built with Emscripten into a *second* wasm
module, driven through JS glue, with the Rust side calling across that boundary.

Crate 0.9.6, published 2026-11-21, MIT OR Apache-2.0, 731 stars, 13 open
issues, actively released (six releases in 2026). The README is explicit that
"packaging an external build of Pdfium as a separate WASM module is essential",
and the two modules are "bound together dynamically at run time" via an exported
`initialize_pdfium_render()` — it binds to Emscripten-style exports (`malloc`,
`free`, the indirect function table).

Published artefact sizes, exact and verified:

| Artefact | Bytes | Form |
| --- | --- | --- |
| `@embedpdf/pdfium@2.15.1` `pdfium.wasm` | **4 646 932** (4.43 MiB) | uncompressed |
| `@hyzyla/pdfium@3.0.0` `pdfium.wasm` | 5 298 237 (5.05 MiB) | uncompressed |
| `bblanchon/pdfium-binaries` `pdfium-wasm.tgz` | 2 704 810 | gzip (wasm + glue) |

So pdfium is **4.43 MiB uncompressed against hayro's 4.26 MiB** — within 4%.
Size is not a reason to prefer either, which is a useful thing to have settled.

- pdfium is Chrome's own PDF engine, so it is the most battle-tested option that
  is also usable from Rust.
- **Known trap, from the crate's own README:** the `bblanchon/pdfium-binaries`
  wasm builds use a non-growable heap, so "attempting to open a PDF document
  longer than just a few pages will result in an unrecoverable out of memory
  error"; it recommends `paulocoutinhox/pdfium-lib` builds instead. bblanchon's
  own README calls its WebAssembly build "experimental". Directly relevant to our
  multi-page case.
- **Licence, now resolved:** pdfium's upstream `LICENSE` contains a 3-clause
  BSD-style grant ("Copyright 2014 The PDFium Authors") **followed by the full
  Apache-2.0 text**, and self-identifies with no SPDX name. Both are permissive
  and neither constrains us; the right label is "BSD-3-Clause + Apache-2.0". Not
  a blocker, but not tidy either.
- Architecturally it costs us: two wasm modules instead of one, a JS glue layer
  on the critical path, no Rust type safety across the boundary, and a native
  CLI that must locate and link a pdfium shared library at runtime.

### Option C — `pdf.js` / `pdfjs-dist` (Mozilla)

Apache-2.0, version 6.5.438 (2026-11-01), repository created 2011-04-26, 54 196
stars, commits the day this ADR was written, and shipped inside Firefox. It is
the most battle-tested PDF renderer on the web by a very wide margin.

It is also what the existing local-only Filigrane Facile clones reach for,
though the picture is more mixed than we had been told:

| Clone | Renderer | Writer | Licence | Last push |
| --- | --- | --- | --- | --- |
| `KazeTachinuu/filigrane-local` | `pdfjs-dist ^6.1.200` | `pdf-lib ^1.17.1` | AGPL-3.0 | 2026-08-26 |
| `CyberClarence/filigraneur` | `pdfjs-dist ^4.6.82` (vendored in `public/`) | **not `pdf-lib`** | none declared; README says "AGPL" with no version and there is no LICENSE file | 2025-08-17 |
| `Bank-Exit/filigrane` | single 890 KB inlined HTML; libraries **not identified** | — | none declared; README says GPL-3.0, no LICENSE file | 2025-06-04 |

So only **one of three** is verifiably `pdfjs-dist` + `pdf-lib`. Two of three are
stale by over a year, and **all three have licence defects** — which incidentally
means we cannot borrow code from any of them.

Published sizes, exact, minified and **uncompressed**:

| File | Bytes |
| --- | --- |
| `build/pdf.min.mjs` | 474 284 |
| `build/pdf.worker.min.mjs` | 1 271 915 |
| **minimum render pair** | **1 746 199 (1.67 MiB)** |
| `wasm/openjpeg.wasm` (JPEG 2000) | 220 817 |
| `wasm/jbig2.wasm` | 103 671 |

Scanned PDFs frequently carry JPX or JBIG2 images, so in practice the side
modules are wanted too, taking the realistic payload past 2 MB uncompressed. No
verified gzip or brotli figure exists for the pair — jsDelivr publishes only
uncompressed sizes, and the commonly-cited 132 KB gzip figure covers the entry
point and **excludes the worker, which is 73% of the bytes**. We decline to
quote a compressed number we have not measured.

It loses on one structural fact: **it is JavaScript.** The premise in
[ADR-0001](0001-stack-and-dependency-policy.md) is one Rust core crate serving
both a browser target and a native CLI. Adopting pdf.js means the PDF path
exists *only* in the browser, and the CLI either drops PDF input entirely or
runs a different renderer. Two renderers means two different outputs for the
same input, which destroys the shared golden-image testing that
[ADR-0002](0002-watermark-algorithm-and-quality-modes.md) Decision 6 is built
on. It also requires a full-frame canvas readback per page.

It is additionally no longer purely JavaScript: 6.x ships `openjpeg.wasm`,
`jbig2.wasm`, `qcms_bg.wasm` and `quickjs-eval.wasm` as optional side modules
(`useWasm` defaults to `true`, with `*_nowasm_fallback.js` as the JS path), each
carrying its own upstream licence to review on redistribution.

The irony is worth recording: pdf.js would be the largest single dependency in a
project whose defining constraint is leanness, and it would be the only one we
could not use from the CLI.

One further argument against following the clones' example: **`pdf-lib`, which
is how two of them write their output PDF, is effectively dormant.** npm `latest`
is 1.17.1, the repository's last push was 2024-07-17, and it carries 320 open
issues against 8 699 stars. We would be adopting an unmaintained writer at the
same time as an enormous renderer.

### Option D — MuPDF (`mupdf-rs`, `mupdf.js`)

Excellent renderer. npm `mupdf@1.28.6` declares
**`license: "AGPL-3.0-or-later"`**; Artifex's licensing page confirms the
dual-licensing model — "Most of our products are dual-licensed under open source
with the GNU AGPLv3 license", and if you "can't meet the requirements of the GNU
AGPLv3 … a commercial license is required". (Note `https://mupdf.com/licensing`
returns HTTP 404; <https://artifex.com/licensing/> is the live page, and it does
not name MuPDF specifically or publish terms.)

It is also the largest option by some margin: `dist/mupdf-wasm.wasm` is
**10 416 688 B (9.93 MiB)** uncompressed, and the package ships a pre-brotlied
`mupdf-wasm.wasm.br` at **3 613 369 B (3.45 MiB)** — i.e. Artifex expects you to
serve ~3.6 MB, nearly three times our measured 1.28 MiB for hayro.

We rule this out in writing, and the reason is not ideological. A browser-only
tool still **conveys** the wasm binary to every visitor, so the AGPL's
source-distribution obligations attach to the whole combined work — the Rust
crate and the JS shell included — quite apart from the network-interaction
clause. There is no "it runs client-side so it doesn't count" escape. We intend
to be open source anyway, so that is survivable — but it would force the
**entire** project, UI included, to AGPL, and it would foreclose MIT/Apache-2.0
forever.
**That is Samuel's licensing decision to make** (ADR-0001's open question), and
a dependency must not pre-empt it. If he chooses AGPL on the merits, MuPDF
becomes available and this ADR should be revisited.

### Option E — other pure-Rust crates

None is a renderer, and it is worth being blunt about it:

- `lopdf` 0.47.0 (2026-11-04, MIT) — "a Rust library for PDF document
  manipulation"; keywords are editing/manipulation/merge and there is **no
  rendering feature** at all.
- `oxidize-pdf` 5.6.1 (2026-11-16, MIT) — "pure Rust toolkit for PDF generation,
  parsing, manipulation"; its own description does not mention rendering.
- the `pdf` crate (pdf-rs) 0.10.0 — a PDF *reader*, with a two-year publishing
  gap (0.9.0 in 2023-11 → 0.9.1 in 2025-12).
- `pdf_render` (pdf-rs) — self-declared **"Experimental PDF viewer"** whose
  README says **"Works for some files"**, and which requires standard fonts to be
  fetched from a separate `pdf_fonts` repository. Disqualified by its own
  documentation.
- `pdfrs` 0.2.1 — claims Markdown→PDF, merge, split, validate, redact,
  rasterisation, SVG, CJK, charts and WASM with "no external dependencies", on
  **49 downloads** of the current version. A claim surface that broad with no
  corroborating corpus is not usable evidence.
- `printpdf` 0.13 — writer only.
- `resvg` 0.50.0 — an SVG renderer; does not mention PDF anywhere.
  `tiny-skia` is a Skia-subset 2-D rasteriser with no text rendering. `vello` is
  GPU-first; `vello_cpu` is the CPU backend hayro already builds on.
- `pdf-render` 2.0.1 (pdfluent) — a third-party fork of hayro that explicitly
  claims WebAssembly support, but shipped two major versions in six days
  (1.3.1 and 2.0.0 both on 2026-11-13, 2.0.1 on 2026-11-19) and reached 1.0.0
  only two months ago. Since upstream hayro demonstrably builds for wasm32
  (measured above), the fork buys us nothing.
- `karet-pdf`, `gpui-pdf` — wrappers around hayro. Nothing gained over using it
  directly.

So in late 2026 there is exactly **one** production-candidate pure-Rust PDF
renderer, and it is hayro.

## Decision

**We will use `hayro` for PDF input rasterisation, behind our own trait, in a
separately-compiled wasm module that is loaded lazily and only when the user
supplies a PDF.** `pdfium-render` is the named fallback if hayro proves
inadequate.

**Runner-up: `pdfium-render`.** It lost on three counts, none of which is size
(the two are within 4% of each other): hayro is pure Rust with no second module
and no JS glue, so the browser and the CLI run byte-identical rendering code and
our golden tests mean something; hayro is `forbid(unsafe_code)` against
untrusted input, whereas pdfium is a large C++ attack surface; and hayro is
MIT/Apache-2.0 with no licence ambiguity. It would win if hayro's correctness or
performance turns out to be inadequate on real documents — that is the explicit
trigger to revisit.

### The isolation boundary

The renderer sits behind one trait, and the watermark compositor is written
against the trait, never against hayro:

```rust
/// A source of page bitmaps. Implemented once for decoded images,
/// once for PDFs. The compositor knows only this.
pub trait PageSource {
    fn page_count(&self) -> usize;
    /// Page box in PostScript points, preserving the input's geometry.
    fn page_geometry(&self, index: usize) -> PageGeometry;
    /// Rasterise one page into a caller-owned RGBA8 buffer.
    fn rasterise(&self, index: usize, dpi: f32, out: &mut Pixmap) -> Result<(), RasterError>;
}
```

The image implementation is trivial (one page; geometry from EXIF resolution or
the chosen DPI; `rasterise` hands back the already-decoded buffer with no
resampling, per ADR-0002). The PDF implementation wraps hayro. This is also why
[ADR-0002](0002-watermark-algorithm-and-quality-modes.md) keeps the watermark
compositor path-independent rather than drawing the mark through
`vello_cpu::RenderContext`: going through the renderer's scene API would weld the
common path to this dependency tree.

### Two wasm artifacts, not one

Two crates in the workspace, both depending on the core:

| Artifact | Contains | Loaded when |
| --- | --- | --- |
| `pochoir-wasm-core` | hand-rolled codecs, glyph rasteriser, compositor, PDF writer | always, on page load |
| `pochoir-wasm-pdf` | the above plus `hayro` | **only** on the first PDF input |

`pochoir-wasm-core` must not depend on `hayro`, transitively or otherwise, and
this is enforced mechanically rather than by discipline — see *CI guard* below.
Loading is two `WebAssembly.instantiateStreaming` calls; the second fires when a
PDF is dropped.

### What lazy loading actually saves

The PDF module measures **1.28 MiB brotli** (4.26 MiB raw) with default
features, or **803 KiB brotli** (3.58 MiB raw) without embedded fonts and CMaps.
The core module does not exist yet; from the hand-rolled scope in
[ADR-0001](0001-stack-and-dependency-policy.md) a reasonable estimate is
**60–120 KiB brotli** *(estimate, not a measurement)*.

So a user who drops a photo of their identity card — which we expect to be the
large majority of use — downloads and compiles roughly **5–15% of what a
monolithic bundle would cost**, and avoids instantiating ~4 MiB of renderer they
will never call. On a mobile connection that is the difference between a tool
that feels instant and one that does not. It is also the difference between
75-odd crates of third-party code being in the trust boundary of every session
and being in the trust boundary of only the sessions that need it.

### CI guard

[ADR-0001](0001-stack-and-dependency-policy.md)'s `deps-check` counts
`[[package]]` entries in `Cargo.lock`, which is workspace-global and therefore
blind to *which* crate pulled a dependency in. Taking hayro raises that count by
74 in one commit, after which the global number can no longer protect the core
module.

**We therefore recommend the guard become per-crate**: keep the global count as a
blunt alarm, and add a hard assertion that the dependency closure of
`cargo tree -p pochoir-wasm-core --target wasm32-unknown-unknown` is contained
in an explicit allowlist. That assertion, not the global count, is what keeps the
common path lean once a 74-crate renderer is in the lock file. The scaffolding
session owns `scripts/check-deps.sh`; this is a recommendation to it, not a
change we make here.

### Execution model: one single-threaded Web Worker, no wasm threads

**We will not use WebAssembly threads.** Rendering runs in a single
single-threaded Web Worker that the main thread can `terminate()`.

This one choice resolves four separate problems at once:

1. **Untrusted input.** hayro's own robustness against malformed PDFs is
   self-described as unfinished, and our input is whatever file the user was
   sent. A worker we can kill on a timeout is the containment. **The renderer
   must never be called on the main thread** — a hang there freezes the tab.
2. **COOP/COEP.** Threads need `SharedArrayBuffer`, which requires the page to
   be cross-origin isolated: `Cross-Origin-Opener-Policy: same-origin` plus
   `Cross-Origin-Embedder-Policy: require-corp` (or `credentialless`). A
   **single-threaded** wasm rasteriser needs **neither header** — verified
   against MDN — so declining threads removes the whole question. That is worth
   more than it sounds: `COEP: require-corp` requires every cross-origin
   subresource to opt in via CORP, and `COOP: same-origin` severs
   `window.opener`. For us the blast radius would be near zero, but the cost of
   not needing it is zero too.
3. **Toolchain.** wasm threads mean nightly `-Zbuild-std` and
   `wasm-bindgen-rayon`. Avoiding them keeps us on stable Rust.
4. **Memory reclamation.** See below.

The cost is real: pages render serially, so a 30-page scan takes 30 page-times.
We accept it — this is a one-shot operation on a document with a progress
indicator, not an interactive loop.

Separately: `rayon` on `wasm32-unknown-unknown` is reported to be a stub that
runs closures sequentially and may never execute `spawn`ed work *(second-hand)*,
so any crate offering it must be taken with `default-features = false`. For
`hayro` this is already the case — its manifest sets `default-features = false`
on its dependencies, and our wasm32 build pulled no `rayon` into the lock file at
all (verified: `rayon` is absent from the adopted set's 81-package lock).

Two practical deployment notes that follow from the above, both verified from a
deployed comparable (`filigrane-local`, which runs on Cloudflare Pages):

- **`_headers` on Cloudflare does carry these headers in practice.** That site
  ships `Cross-Origin-Opener-Policy: same-origin` from `public/_headers` — so
  the mechanism works even though Cloudflare's documentation never names COOP or
  COEP. Notably it ships COOP *without* COEP, i.e. it is deliberately **not**
  cross-origin isolated, exactly the posture we are choosing.
- **Any WASM path needs `'wasm-unsafe-eval'` in the CSP `script-src`.** Their
  deployed policy already includes it. This is easy to forget until the module
  fails to instantiate behind a strict CSP.

### `panic = "abort"`

**We will keep `panic = "abort"`** in the release profile. The scaffolding
session set it for wasm size and rightly flagged it as a decision it should not
make alone; this ADR makes it.

The reasoning is that it costs us no recovery mechanism we were not already
building. With `abort`, a panic traps the wasm instance and it must be
recreated — but we already need exactly that capability for render *hangs* and
for OOM, and the worker-terminate-and-recreate path above provides it. One
recovery path serves all three failure modes.

It is also, counter-intuitively, better for memory: **wasm linear memory grows
and never shrinks**, so an instance that has once peaked at 1 GB holds that
address space for its lifetime. Catching a panic and continuing in the same
instance would preserve that high-water mark. Discarding the worker reclaims it.

**Alternative named and rejected:** `panic = "unwind"` with `catch_unwind` at the
wasm boundary. It yields a recoverable, nicely-reported error for a corrupt
input without discarding the instance, at the cost of bundle size and the
retained memory high-water mark. Rejected because the user-visible outcome — an
error message and a usable app — is identical, and we reach it with machinery we
need anyway.

**Consequence to handle:** Cargo ignores `panic` for the `test` and `bench`
profiles, so `cargo test` is unaffected, but `cargo test --release` cannot run
`#[should_panic]` tests. The rule is therefore: `#[should_panic]` tests run in
the default profile only, and CI must not run the suite with `--release`
expecting them to work.

## Memory

A4 is 8.27 × 11.69 in. As RGBA8:

| DPI | Pixels | Bytes per page | 30 pages resident |
| --- | --- | --- | --- |
| 150 | 1241 × 1754 | 8.7 MB | 261 MB |
| **200** (default) | 1654 × 2338 | **15.5 MB** | **464 MB** |
| 300 | 2481 × 3507 | 34.8 MB | 1.04 GB |

And for the worst case we should plan against — a 50-page 300 DPI scan — the
arithmetic is **1 739 968 000 B = 1.62 GiB** of RGBA held at once.

The ceilings, verified:

- A WebAssembly page is 65 536 B, and `WebAssembly.Memory` throws `RangeError`
  above **65 536 pages = 4 GiB**, "the maximum range that a Wasm module can
  address with 32-bit addressing". The constructor's `maximum` is only "a hint …
  the engine may ignore or clamp this reservation request".
- **Chrome/V8** raised the limit from 2 GB to **4 GB** (M83 onward). V8's own
  write-up cautions that "there just won't be enough free memory on many users'
  machines" and that signed-vs-unsigned pointer bugs surface above 2 GB.
- **Firefox** allows up to 4 GB from version 89, **on x64 and arm64 only**.
  Known interop breakage above 2 GB remains open (WebGL2 throws; WebGPU
  `writeBuffer` fails) — evidence that >2 GB is a thinly-tested regime.
- **Safari:** no primary WebKit statement on the maximum single
  `WebAssembly.Memory` could be found. **We will not assert a Safari number**,
  and mobile Safari's practical cap must be measured on a device before we
  promise any page count.

So: treat **~2 GiB as the safe working ceiling** and 4 GiB as theoretical. Note
that 1.62 GiB is already ~80% of that safe budget *before* the renderer's own
heap, the compressed source bytes and the encode buffers — so holding all pages
decoded at once is simply not viable, whichever renderer we pick.

One more ceiling worth recording, since it bounds the DPI we can offer if any
part of the pipeline ever touches a canvas: `filigrane-local` caps itself at
`MAX_CANVAS_AREA = 16 777 216` px and `MAX_CANVAS_SIDE = 8192`. A 300 DPI A4
page is 8.70 Mpx and 3508 px on its long edge, so it fits; **600 DPI
(34.8 Mpx) would not.**

**But the ceiling is only binding if we accumulate pages, and we will not.** The
pipeline is streaming, and this is a requirement, not an optimisation:

> for each page: rasterise into **one reused buffer** sized to the largest page →
> composite the mark → encode to JPEG → append the compressed bytes to the
> output → **drop the pixmap**.

Peak live memory is then one pixmap (15–35 MB) plus one encoded page plus the
accumulated *compressed* output — on the order of **50–100 MB** for ordinary
work, independent of page count. Embedding the encoded JPEG bytes directly as a
`/DCTDecode` stream (rather than raw samples) is what keeps the accumulated
output small: a watermarked 30-page scan is tens of megabytes, not the ~500 MB
its raw samples would occupy.

`memory64` would not help even if we wanted it. It ships in Chrome 133, Edge 133
and Firefox 134, but **in no shipping Safari release** — caniuse marks every
Safari desktop version through 27.3 and every iOS Safari version as unsupported,
and WebAssembly's own feature data lists Safari as flag-gated in Technology
Preview 251 only. Worse, feature *detection* has been reported to
false-positive on Safari 27.2 (detection returned true, instantiation failed
with "Memory64 is not enabled") *(third-party report)*, so any gating would have
to be a real compile-and-instantiate probe. And SpiderMonkey's own analysis notes
memory64 is **slower** than wasm32. We are staying inside wasm32, and the
streaming pipeline is what makes that fine.

## Hosting constraints (Cloudflare)

- **25 MiB per asset** on both Pages and Workers Assets *(second-hand)*. Our
  4.26 MiB release module is comfortably inside it — but a *debug* build
  carrying DWARF sections can exceed it, so `--release`, `strip = true` and a
  `wasm-opt -Oz` pass are a release-gating requirement, not a nicety.
- `application/wasm` is compressed in transit by default *(second-hand:
  Zstandard on Free, Brotli on Pro/Business)*. Our brotli measurement of
  1.28 MiB is therefore the realistic transfer size, not the 4.26 MiB on disk.
- **Workers Assets ignore `Range` requests** *(second-hand)*, so the renderer
  module is an all-or-nothing fetch. That strengthens the lazy-loading decision:
  since we cannot stream part of it, not fetching it at all is the only way to
  avoid the cost.
- `_headers` is a plain-text file in the asset directory, on both Pages and
  Workers Assets, limited to **100 header rules** and **2 000 characters per
  line**, with duplicate headers across matching rules **joined with commas**.
  The important caveat: **`_headers` does not apply to responses generated by a
  Worker** — Pages Functions, `_worker.js`, SSR, or `assets.run_worker_first`.
  Since our deployment is pure static assets this is fine, but it is a trap if
  the site ever grows a Worker.

## Consequences

### What this gets us

One renderer, written in Rust, byte-identical in the browser and in the CLI, so
golden-image tests actually test the thing that ships. No C toolchain, no
Emscripten, no JS glue, no `unsafe` in the renderer. A permissive licence that
leaves Samuel's licensing choice open. And a common path that never pays for any
of it.

### What this costs us

A 74-crate dependency closure in the lock file, which is by far the largest
concession in the project to [ADR-0001](0001-stack-and-dependency-policy.md)'s
policy — justified only because it is contained in a module most sessions never
load. A 0.x dependency on a single-maintainer crate whose author says it has had
no performance work. Serial page rendering. A two-artifact build and a loader,
instead of one wasm file. And an obligation to measure mobile-Safari memory
behaviour before promising any page count.

### What this forecloses

Rendering on the main thread, ever. wasm threads, unless we first establish that
Cloudflare can set COOP/COEP. MuPDF, unless the project goes AGPL.
`#[should_panic]` tests under `--release`.

## Risks

| Risk | Likelihood | Impact | Mitigation / trigger to revisit |
| --- | --- | --- | --- |
| hayro mis-renders real French administrative PDFs | **Medium** | **High** | Build a corpus of real documents and compare against pdfium/pdf.js output before committing. Trigger: systematic failures → switch to Option B behind the same trait |
| hayro is abandoned (0.x, one maintainer) | Medium | High | Apache-2.0 OR MIT means we can fork. The `PageSource` trait means swapping it is a contained change, not a rewrite |
| Malformed or hostile PDF hangs or OOMs the renderer | **High** | Medium | Worker with a hard timeout and `terminate()`; renderer never on the main thread. This is the mitigation that must exist on day one |
| hayro's performance is inadequate (author states no optimisation work) | Medium | Medium | Measure on a 30-page scan early. Serial rendering amplifies this |
| Mobile Safari memory cap is lower than our streaming peak | Medium | Medium | Measure on device; cap page count or offered DPI accordingly |
| Dropping `embed-fonts` to save 519 KiB breaks standard-font documents | Medium | Medium | Keep default features until measured on a real corpus |
| Taking 74 crates makes the global `deps-check` threshold meaningless | **High** | Medium | Make the guard per-crate (recommended above) in the same commit |

## Open questions

- [ ] **Needs decision from Samuel:** adopting `hayro` means accepting 74
      transitive crates for the PDF path. That is a direct, deliberate
      concession to the lean-dependency rule, made on the grounds that nobody
      hand-rolls a PDF interpreter and that the cost is confined to a module the
      common path never loads. **This is his call, not ours.** The alternative
      is narrower: ship v1 with **image inputs only** and no PDF support at all,
      keeping the whole project at a handful of dependencies, and add PDF input
      later. Recommendation: adopt hayro behind the trait, because a
      watermarking tool that cannot take a PDF cannot replace Filigrane Facile.
- [ ] **Needs decision from Samuel:** whether `embed-fonts` / `embed-cmaps` stay
      on (+519 KiB brotli). Recommendation: on, until a real-document corpus
      says otherwise.
- [ ] **Needs confirmation:** that the per-crate `deps-check` change is
      acceptable to whoever owns `scripts/check-deps.sh`.

## Could not verify

**Measured or read first-hand in this session** (solid): hayro's wasm32 build
succeeding on `rustc 1.96.0`; every hayro size figure (4 471 126 B raw /
1 341 491 B brotli with default features; 3 754 881 B / 822 613 B without); its
74-crate closure; `embed-fonts` and `embed-cmaps` being default features and
their ~700 KB raw cost; its Apache-2.0 OR MIT licence, `forbid(unsafe_code)`,
regression-suite and performance statements; the presence and scope of its
`crypto/` module (RC4, AES-128/256, revisions 2–6); and the absence of `rayon`
from our wasm32 lock file.

**Verified from primary sources** (crates.io, npm, jsDelivr, GitHub APIs, MDN,
caniuse, v8.dev, Cloudflare docs) during research for this ADR: all pdfium,
MuPDF and pdf.js artefact sizes and licences; pdfium's dual BSD+Apache `LICENSE`
file; hayro's download counts, star count, issue character and Typst reverse
dependencies; the three clones' dependencies, licences and staleness; `pdf-lib`'s
dormancy; the wasm32 4 GiB ceiling and Chrome's and Firefox's limits; memory64's
per-engine status; that single-threaded wasm needs no COOP/COEP; and Cloudflare's
`_headers` mechanism and its 100-rule / 2 000-character limits.

**Still not verified — do not rely on these without checking:**

- **Safari's maximum single `WebAssembly.Memory` for wasm32**, and mobile
  Safari's practical cap. No WebKit primary source was found. This must be
  measured on a real device before we promise any page count, and it is the
  single most load-bearing unknown in this ADR.
- The "~2 GiB safe working ceiling" figure. It is a synthesis from V8's and
  Mozilla's own cautions, not a quoted limit from any vendor.
- Cloudflare's **25 MiB per-asset limit**, default `application/wasm`
  compression and which algorithm applies per plan, and `Range` being ignored by
  Workers Assets. None of these was re-verified here.
- Whether Cloudflare explicitly *permits* `Cross-Origin-Embedder-Policy` in
  `_headers`. COOP demonstrably works in a deployed site; COEP is untested, and
  we have designed so that it does not matter.
- Gzip or brotli sizes for pdf.js's `pdf.min.mjs` + `pdf.worker.min.mjs` pair.
  Only uncompressed figures are published, and the widely-quoted ~132 KB gzip
  number excludes the worker.
- The uncompressed `.wasm` size inside `paulocoutinhox/pdfium-lib`'s `wasm.zip`,
  and any brotli figure for a pdfium wasm build.
- `rayon` being a sequential stub on `wasm32-unknown-unknown`.
- The reported memory64 false-positive feature detection on Safari 27.2
  (third-party).
- MuPDF-specific commercial licence terms; Artifex's page does not name MuPDF
  and publishes no pricing.
- Which libraries `Bank-Exit/filigrane` inlines, and how
  `CyberClarence/filigraneur` writes its output PDF without `pdf-lib`.
- The **60–120 KiB brotli estimate for `pochoir-wasm-core`**, which is an
  estimate of code that does not yet exist.

## References

- `hayro` — <https://crates.io/crates/hayro> · <https://github.com/LaurenzV/hayro> · <https://docs.rs/hayro>
- `vello_cpu` — <https://crates.io/crates/vello_cpu>
- `pdfium-render` — <https://crates.io/crates/pdfium-render> · <https://github.com/ajrcarey/pdfium-render>
- `paulocoutinhox/pdfium-lib` — <https://github.com/paulocoutinhox/pdfium-lib>
- `bblanchon/pdfium-binaries` — <https://github.com/bblanchon/pdfium-binaries>
- pdf.js — <https://github.com/mozilla/pdf.js> · <https://www.npmjs.com/package/pdfjs-dist>
- Artifex licensing (MuPDF is AGPL-3.0-or-later; `mupdf.com/licensing` is 404) — <https://artifex.com/licensing/>
- `mupdf` npm package metadata — <https://registry.npmjs.org/mupdf/latest>
- `tiny-skia` — <https://github.com/linebender/tiny-skia>
- ISO 32000-1 (PDF 1.7) — <https://www.iso.org/standard/51502.html>
- MDN, `SharedArrayBuffer` and cross-origin isolation — <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer>
- MDN, `WebAssembly.Memory` — <https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/WebAssembly/Memory>
- WebAssembly feature status — <https://webassembly.org/features/>
- V8, "Up to 4GB of memory in WebAssembly" — <https://v8.dev/blog/4gb-wasm-memory>
- Firefox large wasm memories (intent to ship) — <https://groups.google.com/a/mozilla.org/g/dev-platform/c/90hVJF-X9c4>
- caniuse, wasm memory64 — <https://caniuse.com/wf-wasm-memory64>
- SpiderMonkey, "Is Memory64 actually worth using?" — <https://spidermonkey.dev/blog/2025/01/15/is-memory64-actually-worth-using.html>
- MDN, `Window.crossOriginIsolated` — <https://developer.mozilla.org/en-US/docs/Web/API/Window/crossOriginIsolated>
- pdfium upstream LICENSE — <https://pdfium.googlesource.com/pdfium/+/refs/heads/main/LICENSE>
- `KazeTachinuu/filigrane-local` — <https://github.com/KazeTachinuu/filigrane-local>
- `pdf-lib` — <https://github.com/Hopding/pdf-lib>
- Cloudflare Workers static assets — <https://developers.cloudflare.com/workers/static-assets/>
- Cloudflare Pages `_headers` — <https://developers.cloudflare.com/pages/configuration/headers/>
