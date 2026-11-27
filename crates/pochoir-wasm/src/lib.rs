//! The WebAssembly ABI for the Pochoir engine.
//!
//! This crate is the only thing between a browser and [`pochoir_core`]. It
//! exports eleven `extern "C"` functions over linear memory and nothing else: no
//! `wasm-bindgen`, no generated glue, no JSON. A second client could be written
//! from this module's documentation alone, which is the standard this comment
//! is held to.
//!
//! The decisions behind the shape of it are
//! `docs/adr/0008-wasm-abi-and-the-preview-path.md`.
//!
//! # The module
//!
//! Built by `make wasm` into
//! `target/wasm32-unknown-unknown/release/pochoir_wasm.wasm`. It imports
//! **nothing** — instantiate it with an empty import object — and exports its
//! own `memory` plus the functions below. It is a `cdylib` with no `main`, no
//! start function and no WASI: there is nothing to initialise and no host call
//! to grant. A module that cannot import cannot phone home, which is the
//! privacy claim made structural rather than promised.
//!
//! Every entry point is **synchronous** and will occupy its thread for the
//! whole render — hundreds of milliseconds for a 12-megapixel photograph. Call
//! it from a Web Worker.
//!
//! # Pointers and integer widths
//!
//! Pointers and lengths are declared `usize`, which on `wasm32-unknown-unknown`
//! is 32 bits and reaches JavaScript as `i32`. A `number` on the JavaScript side
//! is therefore correct for every parameter and every return value, with the one
//! exception noted under `pochoir_render_*`: a returned handle must be treated
//! as an opaque token and passed back unchanged. The same signatures compile on
//! a 64-bit host, where `usize` is 64 bits, so the marshalling layer can be unit
//! tested without a browser; a static assertion in [`abi`] pins the wasm case.
//!
//! # The call sequence
//!
//! ```text
//!   ptr = pochoir_alloc(len)            for each of: image bytes, mark text,
//!                                       packed options
//!   <copy the bytes into memory at ptr>
//!   handle = pochoir_render_pdf(...)    or pochoir_render_preview(..., max_px)
//!   status = pochoir_result_status(handle)
//!   if status == 0:
//!       ptr      = pochoir_result_ptr(handle)
//!       len      = pochoir_result_len(handle)
//!       width    = pochoir_result_width(handle)
//!       height   = pochoir_result_height(handle)
//!       warnings = pochoir_result_warnings(handle)
//!       <copy the bytes out>
//!       <report the warning bits you recognise>

//!   pochoir_result_free(handle)
//!   pochoir_free(ptr, len)              for each input buffer
//! ```
//!
//! **Copy the result out before making any further call into the module.**
//! Growing the wasm memory detaches every existing `ArrayBuffer` view over it,
//! so a `Uint8Array` held across a call can silently become a view on nothing.
//!
//! # Exports
//!
//! ## `pochoir_alloc(len: usize) -> usize`
//!
//! Reserve `len` bytes and return their address. The contents are
//! uninitialised: write every byte you intend to read.
//!
//! **`0` is returned both for failure and for `len == 0`, and the two are told
//! apart by the length, not by the pointer.** A return of `0` for a non-zero
//! `len` is a failure — treat it as out of memory — and happens when the
//! allocator refuses or when `len` exceeds [`abi::MAX_BUFFER_BYTES`]. A return
//! of `0` for `len == 0` is **not** a failure: there is nothing to write, there
//! is no allocation to make, and passing `(0, 0)` to a render function is
//! accepted and yields an empty slice. So a caller checks its own length first,
//! exactly as the browser client already does.
//!
//! ## `pochoir_free(ptr: usize, len: usize)`
//!
//! Release a buffer from `pochoir_alloc`. `len` must be the length that was
//! passed to `pochoir_alloc`, because the allocator is told the layout to free
//! rather than recording it. A `ptr` of `0` or a `len` of `0` is a no-op.
//!
//! ## `pochoir_render_pdf(img_ptr, img_len, text_ptr, text_len, opts_ptr, opts_len) -> usize`
//!
//! Decode the image, burn the mark into its pixels and write a one-page PDF.
//! Returns a **result handle**. The handle is opaque; the only valid things to
//! do with it are to pass it to a `pochoir_result_*` accessor and then to
//! `pochoir_result_free`.
//!
//! On success the result carries the PDF bytes, with `width` and `height` both
//! `0`. That is how a caller tells a document from a bitmap — see
//! **Telling a document from a bitmap** below.
//!
//! ## `pochoir_render_preview(img_ptr, img_len, text_ptr, text_len, opts_ptr, opts_len, max_px) -> usize`
//!
//! The same mark, on a bitmap small enough to paint. Returns a handle whose
//! result carries the pixels described under **The preview pixel format**
//! below, with `width` and `height` set, both at least 1.
//!
//! `max_px` bounds the **longer** edge. A page already within it is returned at
//! its own size: this never enlarges an image, because upsampling a preview
//! invents detail the document does not have. `max_px == 0` is a bad argument.
//!
//! The preview is a faithful scale model of the PDF, not an approximation of
//! it. The same pixels are rasterised, the mark's geometry is a ratio of the
//! page so it scales with it, and the jitter seed is derived from the
//! **full-resolution** page, so the tiles land in the same relative places as
//! they will in the document. See ADR-0008 for why that ordering matters.
//!
//! ## `pochoir_result_status(handle) -> u32`
//!
//! `0` on success. Every other value is a failure, and the result then carries
//! a detail payload instead of output — see **The failure payload** below.
//!
//! | code | meaning |
//! | --- | --- |
//! | 0 | `OK` |
//! | 1 | `BAD_ARGUMENT` — a parameter was out of range, the options struct was malformed, or a quality mode was asked for that this engine will not fake |
//! | 2 | `DECODE_FAILED` — the bytes are a JPEG or a PNG and the decoder refused them |
//! | 3 | `UNSUPPORTED_FORMAT` — the bytes are neither a JPEG nor a PNG |
//! | 4 | `OUT_OF_MEMORY` — the image exceeds what this build will allocate for |
//! | 5 | `INTERNAL` — a bug here; please report it |
//! | 6 | `TEXT_DOES_NOT_FIT` — the mark will not fit the page at any permitted font size (ADR-0007 Decision 2) |
//! | 7 | `UNSUPPORTED_CHARACTER` — the bundled font subset cannot draw some of the mark text |
//!
//! These numbers are fixed. Anything not among the seven is `5`. Codes 6 and 7
//! exist because collapsing ADR-0007's two named refusals into `BAD_ARGUMENT`
//! leaves a user whose mark contains one undrawable character with a message
//! about their parameters, which sends them to the wrong fix.
//!
//! Calling this on handle `0` returns `5`. No `pochoir_render_*` call returns
//! `0`, so that is a defensive answer rather than a documented path; passing
//! any other value that this module did not hand out is undefined behaviour.
//!
//! ## `pochoir_result_ptr(handle) -> usize`, `pochoir_result_len(handle) -> usize`
//!
//! The result bytes, valid until `pochoir_result_free`. `ptr` is stable for the
//! lifetime of the handle, and **both are `0` when there are no bytes** — an
//! empty result never returns a dangling pointer, so a caller may check `ptr`
//! before `len`.
//!
//! ## `pochoir_result_width(handle) -> u32`, `pochoir_result_height(handle) -> u32`
//!
//! The bitmap dimensions for a preview; `0` for a PDF and `0` on failure.
//!
//! ## `pochoir_result_warnings(handle) -> u32`
//!
//! **The success-side channel: facts about a result the engine produced
//! anyway.** A bitmask, `0` when there is nothing to say, `0` on every failure
//! and `0` for handle `0`.
//!
//! | bit | meaning |
//! | --- | --- |
//! | `1 << 0` | the mark text did not fit at the size asked for and was drawn markedly smaller (ADR-0007 Decision 2) |
//! | `1 << 1` | the mark is invisible over more than 10% of the page (ADR-0010) |
//!
//! **Mask for the bits you know and ignore the rest.** A later engine may add
//! one, and that must not be a breaking change.
//!
//! Neither fact can travel any other way. A warning is not a failure — both
//! ADRs draw the mark and tell the caller rather than refusing — so it cannot
//! be a status; and on a success `ptr`/`len` are the document or the bitmap,
//! so it cannot be the payload without framing bytes that are currently
//! copied out as they stand. Which fact applies is the caller's to word: bit 1
//! on a request that asked for the automatic colour means *this page defeats
//! both candidates*, and on a request that pinned a colour it means *that
//! colour is close to this page's tone* — the caller knows which it sent, so
//! the wire does not have to carry it.
//!
//! # The preview pixel format
//!
//! **RGBA8, row-major, top row first, four bytes per pixel, no row padding and
//! no stride, non-premultiplied (straight) alpha, in that byte order: R, G, B,
//! A.** So `len` is always exactly `width * height * 4`, the pixel at `(x, y)`
//! begins at `(y * width + x) * 4`, and the buffer can be handed to
//! `ImageData`/`putImageData` as it stands.
//!
//! It is stated this precisely because every plausible alternative — BGRA, a
//! padded stride, premultiplied alpha — would show up as a colour or alignment
//! bug in the picture rather than as an error anywhere. No conversion happens
//! on this side: the engine's `Pixmap` is already exactly this, and the preview
//! is a straight copy out of it.
//!
//! In practice the compositor leaves every pixel opaque (`a = 255`), because
//! the mark blends itself into the page rather than being left for the caller
//! to flatten. A PNG input with transparency keeps its alpha in the regions the
//! mark does not touch.
//!
//! # Telling a document from a bitmap
//!
//! There is **no `kind` field**, because the value is already carried:
//!
//! > **`width > 0` means the result is a preview bitmap. A PDF always has
//! > `width == 0` and `height == 0`.**
//!
//! This is an invariant of the implementation, not a property of the happy
//! path: `Outcome`'s three constructors are the only way to build a result,
//! each fixes the dimensions for its own case, and a test asserts it across
//! every input the suite reaches. A failure also has both at `0`, so a caller
//! checks the status first.
//!
//! # The failure payload
//!
//! On a non-`OK` status, `ptr` and `len` carry **a short UTF-8 payload, at most
//! 4096 bytes**, and `width` and `height` stay `0`.
//!
//! The payload is **data for the caller to interpolate into its own wording,
//! never a sentence**. This product's interface is French, and an English
//! sentence produced here would be a string the caller can neither display nor
//! translate. So:
//!
//! | status | payload |
//! | --- | --- |
//! | `UNSUPPORTED_CHARACTER` | every offending character, concatenated, in order of first appearance, without duplicates, separators or quoting — e.g. `中文日` |
//! | `BAD_ARGUMENT` | the bare name of the offending field, a stable ASCII identifier (see below) |
//! | anything else | **absent**: `ptr` and `len` are both `0` |
//!
//! An absent payload is the right answer where the only fact is the status
//! itself. `TEXT_DOES_NOT_FIT` has nothing to add beyond "it does not fit", and
//! a caller should treat an empty payload as "no detail" rather than as an
//! error.
//!
//! The field names a `BAD_ARGUMENT` can carry are those the engine already uses
//! for the parameters it owns — `quality`, `dpi`, `mark font size`,
//! `mark angle`, `mark opacity`, `mark blur`, `mark text`, `mark tile density`,
//! `page size`, `preview size`, `max_px` — plus six for fields that exist only
//! on the wire, which the engine therefore has no name for:
//! [`options::FIELD_VERSION`], [`options::FIELD_LENGTH`],
//! [`options::FIELD_MODE`], [`options::FIELD_PAGE_SIZING`],
//! [`options::FIELD_DENSITY`] and
//! [`options::FIELD_COLOUR_MODE`]. A caller maps the ones it recognises and ignores
//! the rest; an unrecognised name must degrade to no detail, not to an error.
//!
//! ## `pochoir_result_free(handle)`
//!
//! Release the result and its bytes. A `handle` of `0` is a no-op. Calling it
//! twice on the same handle is undefined behaviour.
//!
//! # The packed options struct
//!
//! 48 bytes, little-endian, fixed layout, version 1. Not JSON: there is no JSON
//! parser in this project and there is not going to be one.
//!
//! | off | type | field | notes |
//! | --- | --- | --- | --- |
//! | 0 | `u32` | version | must be `1` |
//! | 4 | `u32` | quality mode | `0` compact, `1` standard, `2` high, `3` lossless |
//! | 8 | `u32` | page sizing | `0` cap to A4 (ADR-0006), `1` the image's native page |
//! | 12 | `u32` | dpi | `1..=10000`; for an image input this only decides the physical page size, never the pixel count |
//! | 16 | `f32` | font size | **per cent** of the page's shorter edge, `1.0..=10.0` |
//! | 20 | `f32` | angle | degrees, `-90.0..=90.0`; negative runs bottom-left to top-right |
//! | 24 | `f32` | opacity | `0.05..=0.80` |
//! | 28 | `f32` | blur | σ as a **fraction of the font size**, `0.0..=0.06` |
//! | 32 | `u32` | tile density | `0` low, `1` normal, `2` high |
//! | 36 | `u32` | colour | `0x00RRGGBB` for that exact colour, `0x01000000` for *choose one from the page*; nothing else |
//! | 40 | `u64` | jitter seed | `0` means *derive one from the document*; see below |
//!
//! Anything else — a short or long buffer, an unknown version, an out-of-range
//! value, an undefined enumerant — is `BAD_ARGUMENT`. Nothing is clamped: a
//! value silently moved into range is a document that does not say what the
//! user asked it to say.
//!
//! Four fields repay a second look, because each is a place a caller is likely
//! to guess wrong:
//!
//! - **The top byte of the colour at offset 36 is a mode byte, not padding.**
//!   It was ignored until ADR-0010 gave the engine a colour it can choose
//!   itself, and it is now the only way to ask for that: `0x00RRGGBB` is the
//!   colour as given, `0x01000000` is *measure the page and choose*, and
//!   **everything else is `BAD_ARGUMENT`** naming
//!   [`options::FIELD_COLOUR_MODE`] — including `0x01RRGGBB`, which sets the
//!   automatic flag and sends a colour, and so means two things at once. The
//!   mode byte rather than a new field because this struct is full and because
//!   a flag beside a colour is two fields that have to agree; a misread colour
//!   byte is a wrong-coloured mark rather than an error, and one field with one
//!   meaning has exactly one illegal shape. **The automatic choice is the
//!   engine's default and should be a client's**: a user who never opens the
//!   colour control must get a mark they can see, which on a dark-mode
//!   screenshot a fixed `#1A1A1A` is not.
//!
//! - **Blur at offset 28 is a ratio, not a pixel count.** ADR-0002 asked for
//!   "σ ≈ 0.6 px at render resolution, scaled with DPI", which pulls both ways
//!   at once and, at its own default, rounds to a no-op; ADR-0007 Decision 3
//!   re-keyed it to the font size. Default `0.015`, maximum `0.06`.
//! - **Font size at offset 16 is a percentage, not a ratio.** `3.5` means 3.5%
//!   of the shorter edge. It is divided by 100 here, which is the one unit
//!   conversion in the whole struct.
//! - **A seed of `0` derives the seed from the document**, as ADR-0002
//!   Decision 6 requires, rather than seeding with the literal value `0`.
//!   A fixed seed shared by every document would give every page the same tile
//!   displacements, which is precisely the property the jitter exists to deny
//!   an attacker holding two marked copies. An explicit literal seed of zero is
//!   therefore not expressible; any other `u64` is.
//!
//! The mark text is a **separate UTF-8 buffer**, not part of the struct. It is
//! normalised to NFC by the engine. Invalid UTF-8, empty or whitespace-only
//! text and text over 200 characters are `BAD_ARGUMENT`; text the bundled font
//! cannot draw is `UNSUPPORTED_CHARACTER`, carrying every offending character,
//! and text that will not fit the page is `TEXT_DOES_NOT_FIT`.
//!
//! # Quality modes this engine will not fake
//!
//! `standard` and `high` work and differ in JPEG quality, 85 against 92.
//!
//! **`compact` and `lossless` return `BAD_ARGUMENT` today.** `compact` asks for
//! a downsample to a 150 DPI equivalent, which no code here can do and which
//! ADR-0002's own "do not resample image inputs at all" forbids for image
//! inputs in any case; `lossless` asks for a PNG `/FlateDecode` image and the
//! PDF writer can only embed a JPEG. Refusing is deliberate: a user who picks
//! `lossless` and is quietly handed JPEG q92 has been told something untrue
//! about their own document.
//!
//! ADR-0002's DPI column is headed "Raster DPI (PDF input)" and its image-input
//! column says "native, 1:1", so for the image inputs this engine accepts a
//! mode chooses the **encoding only**. The DPI comes from the struct.
//!
//! # Errors are returned; panics are an outage
//!
//! `[profile.release]` sets `panic = "abort"`, so a panic traps the instance:
//! every later call fails and the Worker has to build a new one. A panic is
//! therefore not an error path, and no export has one. Each returns a status
//! instead, including for hostile input, and the marshalling layer is written
//! without `unwrap`, `expect`, slice indexing or arithmetic that can overflow.
//!
//! One case is honestly outside that guarantee: if the allocator itself is
//! exhausted inside the engine, Rust's allocation-failure handler aborts, and no
//! status can be returned. The engine refuses an oversized image up front — that
//! is `OUT_OF_MEMORY`, a status — but genuine heap exhaustion below that bound
//! ends the instance. A caller that wants to survive it must be prepared to
//! recreate the module.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod abi;
pub mod options;
pub mod render;
pub mod scale;
pub mod status;

pub use status::Status;

/// Version of this binding crate, as declared in its `Cargo.toml`.
///
/// Not written into any output: ADR-0002 keeps `/Producer` a constant with no
/// version number, because a version string tells a recipient which build made
/// the file and therefore which known bugs it has.
///
/// ```
/// assert_eq!(pochoir_wasm::version().split('.').count(), 3);
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
