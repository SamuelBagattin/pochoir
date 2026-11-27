//! The exported functions: the only module in this crate that touches a
//! pointer.
//!
//! # Why `unsafe` lives here and nowhere else
//!
//! [`pochoir_core`] carries `#![forbid(unsafe_code)]`, and `forbid` cannot be
//! lifted by an inner `allow` — which is the point of it. `#[unsafe(no_mangle)]`
//! under edition 2024 is an unsafe attribute, so the exports cannot live in that
//! crate at all. They live here, in a crate whose root sets `deny` rather than
//! `forbid`, and the allowance is attached to this one module.
//!
//! Everything below is marshalling: turn a pointer and a length into a slice,
//! hand it to [`crate::render`], put the answer somewhere the caller can read
//! it. No decoding, no arithmetic on image data, no policy. If a change to this
//! module needs to think about pixels, it belongs in another module.
//!
//! # What the caller must honour
//!
//! Three obligations, which no amount of checking here can replace, because a
//! wasm module cannot tell a valid address from an invalid one:
//!
//! 1. Every `(ptr, len)` pair passed to a render function is a buffer obtained
//!    from [`pochoir_alloc`] and still live, or `(0, 0)`.
//! 2. Every handle passed to a `pochoir_result_*` function was returned by a
//!    render function and has not been freed.
//! 3. [`pochoir_free`] is given the same `len` that [`pochoir_alloc`] was.
//!
//! Breaking any of them is undefined behaviour. A `len` of zero is always safe
//! and never dereferences, which is why the result accessors answer defensively
//! on handle `0` rather than trusting it.

#![allow(
    unsafe_code,
    reason = "this module is the wasm ABI: #[unsafe(no_mangle)] exports and raw \
              pointer marshalling. Nothing else in the crate may use unsafe."
)]

use core::alloc::Layout;

use crate::render::{self, Outcome};
use crate::status::Status;

/// Largest single buffer [`pochoir_alloc`] will hand out, in bytes.
///
/// 64 MiB. The browser client caps an input at 50 MB, so this leaves room above
/// the largest file it will offer while turning a nonsense length into a null
/// return — a status the caller can report — instead of an allocation failure,
/// which aborts.
pub const MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// On the browser target a pointer is 32 bits, so every `usize` in this module
/// reaches JavaScript as an `i32`. Asserted rather than assumed: if this ever
/// stops holding, the documented ABI is wrong and the build should say so.
#[cfg(target_arch = "wasm32")]
const _: () = assert!(core::mem::size_of::<usize>() == 4);

/// The result of a render, kept alive on the heap and handed out as a handle.
///
/// The payload is owned here so that `pochoir_result_free` frees the bytes and
/// the header in one call. `bytes` is never mutated after construction, so the
/// pointer handed out by [`pochoir_result_ptr`] stays valid for the handle's
/// whole life.
struct ResultHeader {
    status: Status,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    warnings: u32,
}

impl ResultHeader {
    fn from(outcome: Outcome) -> Self {
        Self {
            status: outcome.status,
            bytes: outcome.bytes,
            width: outcome.width,
            height: outcome.height,
            warnings: outcome.warnings,
        }
    }

    /// Move onto the heap and return the handle.
    fn into_handle(self) -> usize {
        Box::into_raw(Box::new(self)) as usize
    }
}

/// Borrow a result header for the length of `f`.
///
/// Returns `None` for the null handle, which is the one invalid handle this can
/// recognise. Any other invalid handle is undefined behaviour, as documented.
fn with_header<T>(handle: usize, f: impl FnOnce(&ResultHeader) -> T) -> Option<T> {
    if handle == 0 {
        return None;
    }
    // SAFETY: obligation 2 above. `handle` came from `into_handle`, which leaked
    // a `Box<ResultHeader>`, and has not been freed; the reference does not
    // outlive this call and nothing else aliases it mutably.
    let header = unsafe { &*(handle as *const ResultHeader) };
    Some(f(header))
}

/// View a caller buffer as a slice.
///
/// `(0, 0)` and any zero length give an empty slice without dereferencing
/// anything, so a caller that allocated nothing is safe by construction.
///
/// # Safety
///
/// Obligation 1 above: `ptr` must be a live buffer of at least `len` bytes.
unsafe fn slice_of(ptr: usize, len: usize) -> Option<&'static [u8]> {
    if len == 0 {
        return Some(&[]);
    }
    if ptr == 0 {
        return None;
    }
    // SAFETY: the caller guarantees `ptr..ptr + len` is one live allocation from
    // `pochoir_alloc`. The lifetime is `'static` only so this helper can be
    // written at all; every caller drops the slice before returning, and the
    // engine never retains it.
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) })
}

/// The layout `pochoir_alloc` and `pochoir_free` agree on: `len` bytes,
/// alignment 1.
///
/// Alignment 1 is correct for a byte buffer and is what makes a plain
/// `(ptr, len)` pair enough to free: there is no padding to reconstruct.
fn layout_of(len: usize) -> Option<Layout> {
    if len == 0 || len > MAX_BUFFER_BYTES {
        return None;
    }
    Layout::from_size_align(len, 1).ok()
}

/// Reserve `len` bytes of linear memory and return their address.
///
/// Returns `0` if `len` is `0`, if `len` exceeds [`MAX_BUFFER_BYTES`], or if the
/// allocation failed. The contents are uninitialised.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_alloc(len: usize) -> usize {
    let Some(layout) = layout_of(len) else {
        return 0;
    };
    // SAFETY: `layout` has a non-zero size, which is `alloc`'s one requirement.
    // A null return is the documented failure and is passed straight through
    // rather than reaching `handle_alloc_error`, which would abort the instance.
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    ptr as usize
}

/// Release a buffer obtained from [`pochoir_alloc`].
///
/// A `ptr` of `0` or a `len` of `0` is a no-op.
///
/// # Safety
///
/// `ptr` must come from [`pochoir_alloc`] and `len` must be the length it was
/// called with. Freeing twice, or with a different length, is undefined
/// behaviour.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_free(ptr: usize, len: usize) {
    let Some(layout) = layout_of(len) else {
        return;
    };
    if ptr == 0 {
        return;
    }
    // SAFETY: obligations 1 and 3. `ptr` was returned by `pochoir_alloc` for
    // this same `len`, so `layout` is the layout it was allocated with.
    unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) }
}

/// Burn the mark into the image and return a handle to a one-page PDF.
///
/// See the crate documentation for the buffers and the call sequence. The
/// returned handle is always non-zero and always carries a status.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_render_pdf(
    img_ptr: usize,
    img_len: usize,
    text_ptr: usize,
    text_len: usize,
    opts_ptr: usize,
    opts_len: usize,
) -> usize {
    // SAFETY: obligation 1, discharged by the caller.
    let outcome = unsafe {
        with_inputs(
            img_ptr,
            img_len,
            text_ptr,
            text_len,
            opts_ptr,
            opts_len,
            render::render_pdf,
        )
    };
    ResultHeader::from(outcome).into_handle()
}

/// Burn the mark into the image and return a handle to an RGBA8 preview bitmap
/// whose longer edge is at most `max_px`.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_render_preview(
    img_ptr: usize,
    img_len: usize,
    text_ptr: usize,
    text_len: usize,
    opts_ptr: usize,
    opts_len: usize,
    max_px: u32,
) -> usize {
    // SAFETY: obligation 1, discharged by the caller.
    let outcome = unsafe {
        with_inputs(
            img_ptr,
            img_len,
            text_ptr,
            text_len,
            opts_ptr,
            opts_len,
            |image, text, options| render::render_preview(image, text, options, max_px),
        )
    };
    ResultHeader::from(outcome).into_handle()
}

/// Turn the three caller buffers into safe arguments, or fail with a status.
///
/// The UTF-8 check is here rather than in [`crate::options`] because this is the
/// boundary where bytes become a `&str`: invalid UTF-8 is a malformed call, not
/// a bad parameter value, and either way the caller is told `BAD_ARGUMENT`.
///
/// # Safety
///
/// Obligation 1 for all three pairs.
unsafe fn with_inputs(
    img_ptr: usize,
    img_len: usize,
    text_ptr: usize,
    text_len: usize,
    opts_ptr: usize,
    opts_len: usize,
    render: impl FnOnce(&[u8], &str, &[u8]) -> Outcome,
) -> Outcome {
    // SAFETY: forwarded from this function's own contract.
    let buffers = unsafe {
        (
            slice_of(img_ptr, img_len),
            slice_of(text_ptr, text_len),
            slice_of(opts_ptr, opts_len),
        )
    };
    let (Some(image), Some(text_bytes), Some(options)) = buffers else {
        return Outcome::refused(Status::BadArgument);
    };
    let Ok(text) = core::str::from_utf8(text_bytes) else {
        return Outcome::refused(Status::BadArgument);
    };
    render(image, text, options)
}

/// The status of a result: `0` on success, otherwise one of
/// [`Status`]'s codes.
///
/// Returns [`Status::Internal`]'s code for handle `0`, which no render call
/// returns.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_status(handle: usize) -> u32 {
    with_header(handle, |header| header.status.code()).unwrap_or(Status::Internal.code())
}

/// The address of the result bytes, or `0` if there are none.
///
/// **An empty result returns `0`, never a dangling pointer.** `Vec::as_ptr` on
/// an empty `Vec` returns a non-null aligned address that points at nothing,
/// which would read as "there is a payload here" to a caller that checks `ptr`
/// before `len`. The browser's detail reader does exactly that, and an absent
/// payload has to be distinguishable from a present one, so the emptiness is
/// checked here rather than left to the caller.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_ptr(handle: usize) -> usize {
    with_header(handle, |header| {
        if header.bytes.is_empty() {
            0
        } else {
            header.bytes.as_ptr() as usize
        }
    })
    .unwrap_or(0)
}

/// The length of the result bytes in bytes, or `0`.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_len(handle: usize) -> usize {
    with_header(handle, |header| header.bytes.len()).unwrap_or(0)
}

/// The preview bitmap's width, or `0` for a PDF or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_width(handle: usize) -> u32 {
    with_header(handle, |header| header.width).unwrap_or(0)
}

/// The preview bitmap's height, or `0` for a PDF or a failure.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_height(handle: usize) -> u32 {
    with_header(handle, |header| header.height).unwrap_or(0)
}

/// What the engine wants to say about a result it nonetheless produced: a
/// bitmask of [`render::WARNING_MARK_SHRUNK`] and
/// [`render::WARNING_POOR_CONTRAST`].
///
/// `0` when there is nothing to say, on every failure, and for handle `0`.
///
/// This is the success-side channel the result header did not have, and it is
/// the whole of it. A warning is not a failure — ADR-0007 Decision 2 draws the
/// mark and says so, and ADR-0010 keeps that posture for the colour — so it
/// cannot travel as a status; and `ptr`/`len` on a success are the document or
/// the bitmap, so it cannot travel there without framing a payload that is
/// currently copied out as it stands. An accessor beside the four that already
/// exist costs one export, changes nothing an existing caller reads, and has
/// 30 bits left for the next fact of this kind.
///
/// **A caller must mask for the bits it knows** and ignore the rest, so that a
/// later engine can add one without that being a breaking change.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_warnings(handle: usize) -> u32 {
    with_header(handle, |header| header.warnings).unwrap_or(0)
}

/// Release a result and its bytes. A `handle` of `0` is a no-op.
///
/// # Safety
///
/// Obligation 2. Freeing the same handle twice is undefined behaviour.
#[unsafe(no_mangle)]
pub extern "C" fn pochoir_result_free(handle: usize) {
    if handle == 0 {
        return;
    }
    // SAFETY: obligation 2. `handle` came from `into_handle`'s `Box::into_raw`
    // and has not been freed, so reconstructing the `Box` and dropping it frees
    // exactly that allocation and the `Vec` inside it.
    drop(unsafe { Box::from_raw(handle as *mut ResultHeader) });
}

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::{
        MAX_BUFFER_BYTES, pochoir_alloc, pochoir_free, pochoir_render_pdf, pochoir_render_preview,
        pochoir_result_free, pochoir_result_height, pochoir_result_len, pochoir_result_ptr,
        pochoir_result_status, pochoir_result_warnings, pochoir_result_width,
    };
    use crate::Status;
    use crate::options::{AUTOMATIC_COLOUR_WORD, OPTIONS_BYTES};
    use crate::render::{WARNING_POOR_CONTRAST, WARNINGS_KNOWN};

    /// Copy `bytes` into a module buffer, exactly as the JavaScript client does.
    fn copy_in(bytes: &[u8]) -> (usize, usize) {
        let ptr = pochoir_alloc(bytes.len());
        if bytes.is_empty() {
            return (ptr, 0);
        }
        assert_ne!(ptr, 0, "allocation of {} bytes failed", bytes.len());
        // SAFETY: `ptr` is a live buffer of exactly `bytes.len()` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
        }
        (ptr, bytes.len())
    }

    fn copy_out(handle: usize) -> Vec<u8> {
        let ptr = pochoir_result_ptr(handle);
        let len = pochoir_result_len(handle);
        if len == 0 {
            return Vec::new();
        }
        // SAFETY: `ptr..ptr + len` is the result's own buffer, live until
        // `pochoir_result_free`.
        unsafe { core::slice::from_raw_parts(ptr as *const u8, len) }.to_vec()
    }

    /// The browser's defaults with `colour` spelled explicitly, so a test can
    /// ask for the automatic choice or pin a colour without a second packer.
    fn packed(colour: u32) -> Vec<u8> {
        let mut b = Vec::with_capacity(OPTIONS_BYTES);
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&0_u32.to_le_bytes());
        b.extend_from_slice(&200_u32.to_le_bytes());
        b.extend_from_slice(&3.5_f32.to_le_bytes());
        b.extend_from_slice(&(-30.0_f32).to_le_bytes());
        b.extend_from_slice(&0.30_f32.to_le_bytes());
        b.extend_from_slice(&0.015_f32.to_le_bytes());
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&colour.to_le_bytes());
        b.extend_from_slice(&0_u64.to_le_bytes());
        b
    }

    fn defaults() -> Vec<u8> {
        packed(AUTOMATIC_COLOUR_WORD)
    }

    /// A flat near-black page: Samuel's dark-mode screenshot, reduced to the
    /// one property that defeated the old fixed default.
    fn dark_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = pochoir_core::Pixmap::new(width, height).expect("allocates");
        for y in 0..height {
            for x in 0..width {
                pixmap.set_pixel(x, y, [0x10, 0x10, 0x10, 0xFF]);
            }
        }
        pochoir_core::jpeg::encode(&pixmap, 90, 200.0).expect("encodes")
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = pochoir_core::Pixmap::new(width, height).expect("allocates");
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width.max(1)) as u8;
                pixmap.set_pixel(x, y, [r, 0x40, 0x80, 0xFF]);
            }
        }
        pochoir_core::jpeg::encode(&pixmap, 90, 200.0).expect("encodes")
    }

    const TEXT: &str = "Transmis a X";

    /// The whole documented sequence, driven exactly as `abi.ts` drives it.
    #[test]
    fn the_documented_call_sequence_produces_a_pdf() {
        let image = jpeg(240, 180);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_ne!(handle, 0, "a render must always return a handle");
        assert_eq!(pochoir_result_status(handle), Status::Ok.code());
        assert_eq!(pochoir_result_width(handle), 0);
        assert_eq!(pochoir_result_height(handle), 0);
        let pdf = copy_out(handle);
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }

    #[test]
    fn the_documented_call_sequence_produces_a_preview_bitmap() {
        let image = jpeg(400, 300);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_preview(ip, il, tp, tl, op, ol, 100);
        assert_eq!(pochoir_result_status(handle), Status::Ok.code());
        let (w, h) = (pochoir_result_width(handle), pochoir_result_height(handle));
        assert_eq!((w, h), (100, 75));
        let pixels = copy_out(handle);
        assert_eq!(pixels.len() as u32, w * h * 4);
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    /// The accessors are called on every handle including failures, so they
    /// must answer without touching bytes that are not there.
    #[test]
    fn a_failed_render_still_answers_every_accessor() {
        let (ip, il) = copy_in(b"not an image");
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_ne!(handle, 0);
        assert_eq!(
            pochoir_result_status(handle),
            Status::UnsupportedFormat.code()
        );
        assert_eq!(pochoir_result_len(handle), 0);
        assert_eq!(pochoir_result_width(handle), 0);
        assert_eq!(pochoir_result_height(handle), 0);
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    #[test]
    fn invalid_utf8_mark_text_is_a_bad_argument_not_a_panic() {
        let image = jpeg(64, 64);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in(&[0xFF, 0xFE, 0x80]);
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_eq!(pochoir_result_status(handle), Status::BadArgument.code());
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    /// A null image pointer with a non-zero length is the shape a buggy caller
    /// produces after a failed `pochoir_alloc`. It must be refused, not
    /// dereferenced.
    #[test]
    fn a_null_pointer_with_a_length_is_refused() {
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());
        let handle = pochoir_render_pdf(0, 16, tp, tl, op, ol);
        assert_eq!(pochoir_result_status(handle), Status::BadArgument.code());
        pochoir_result_free(handle);
        pochoir_free(op, ol);
        pochoir_free(tp, tl);
    }

    /// The failure payload, read exactly as the browser reads it: check `ptr`
    /// and `len` are non-zero, then decode strictly.
    #[test]
    fn a_failure_payload_is_readable_through_the_accessors() {
        let image = jpeg(64, 64);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in("Pour \u{4e2d}\u{6587}".as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_eq!(
            pochoir_result_status(handle),
            Status::UnsupportedCharacter.code()
        );
        assert_ne!(pochoir_result_ptr(handle), 0);
        assert_eq!(pochoir_result_width(handle), 0);
        assert_eq!(pochoir_result_height(handle), 0);
        let detail = String::from_utf8(copy_out(handle)).expect("the payload is UTF-8");
        assert_eq!(detail, "\u{4e2d}\u{6587}");
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    /// An absent payload must read as absent. `Vec::as_ptr` on an empty `Vec`
    /// returns a non-null dangling address, which a caller checking `ptr`
    /// before `len` would mistake for a payload that is there.
    #[test]
    fn an_absent_payload_reports_a_null_pointer_not_a_dangling_one() {
        let (ip, il) = copy_in(b"not an image");
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_eq!(
            pochoir_result_status(handle),
            Status::UnsupportedFormat.code()
        );
        assert_eq!(
            pochoir_result_ptr(handle),
            0,
            "an empty payload must report ptr 0"
        );
        assert_eq!(pochoir_result_len(handle), 0);
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    #[test]
    fn the_null_handle_answers_defensively() {
        assert_eq!(pochoir_result_status(0), Status::Internal.code());
        assert_eq!(pochoir_result_ptr(0), 0);
        assert_eq!(pochoir_result_len(0), 0);
        assert_eq!(pochoir_result_width(0), 0);
        assert_eq!(pochoir_result_height(0), 0);
        assert_eq!(pochoir_result_warnings(0), 0);
        pochoir_result_free(0); // a no-op, not a crash
    }

    /// The whole point of the wire change, driven through the exports: on a
    /// near-black page the request the browser sends by default produces a
    /// visible mark and says nothing, and the same page with `#1A1A1A` pinned
    /// produces the silent failure ADR-0010 was written about — except that it
    /// is no longer silent.
    ///
    /// Both halves matter. Without the first the automatic sentinel could be
    /// mis-decoded and nothing would notice; without the second the warning
    /// bit could be stuck at zero and nothing would notice either.
    #[test]
    fn a_dark_page_warns_on_a_pinned_dark_colour_and_not_on_the_automatic_one() {
        for (options, want, what) in [
            (defaults(), 0, "the automatic choice"),
            (
                packed(0x001A_1A1A),
                WARNING_POOR_CONTRAST,
                "a pinned #1A1A1A",
            ),
        ] {
            let image = dark_jpeg(400, 300);
            let (ip, il) = copy_in(&image);
            let (tp, tl) = copy_in(TEXT.as_bytes());
            let (op, ol) = copy_in(&options);

            let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
            assert_eq!(
                pochoir_result_status(handle),
                Status::Ok.code(),
                "{what} must still produce a document"
            );
            assert_eq!(
                pochoir_result_warnings(handle) & WARNINGS_KNOWN,
                want,
                "{what} on a near-black page"
            );
            pochoir_result_free(handle);

            // And the preview agrees with the document it is previewing: a
            // preview that said nothing where the PDF warns would put the
            // warning where nobody looks.
            let handle = pochoir_render_preview(ip, il, tp, tl, op, ol, 200);
            assert_eq!(pochoir_result_status(handle), Status::Ok.code());
            assert_eq!(
                pochoir_result_warnings(handle) & WARNINGS_KNOWN,
                want,
                "{what}, previewed"
            );
            pochoir_result_free(handle);

            pochoir_free(op, ol);
            pochoir_free(tp, tl);
            pochoir_free(ip, il);
        }
    }

    /// A warning describes a document that exists. A failure has none, and a
    /// caller that read the mask without checking the status first would
    /// otherwise report a contrast problem about a file that was never made.
    #[test]
    fn a_failure_carries_no_warning_bits() {
        let (ip, il) = copy_in(b"not an image");
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_eq!(
            pochoir_result_status(handle),
            Status::UnsupportedFormat.code()
        );
        assert_eq!(pochoir_result_warnings(handle), 0);
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    /// A colour word that means two things at once is refused rather than
    /// guessed at, and it is refused at the ABI rather than only in the
    /// parser's own tests.
    #[test]
    fn a_malformed_colour_word_is_refused_through_the_exports() {
        let image = jpeg(64, 64);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&packed(AUTOMATIC_COLOUR_WORD | 0x001A_1A1A));

        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        assert_eq!(pochoir_result_status(handle), Status::BadArgument.code());
        let detail = String::from_utf8(copy_out(handle)).expect("the payload is UTF-8");
        assert_eq!(detail, "mark colour mode");
        pochoir_result_free(handle);

        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }

    #[test]
    fn allocation_refuses_zero_and_absurd_lengths_without_aborting() {
        assert_eq!(pochoir_alloc(0), 0, "zero bytes is not an error");
        pochoir_free(0, 0);
        assert_eq!(
            pochoir_alloc(MAX_BUFFER_BYTES + 1),
            0,
            "past the cap must return null rather than abort"
        );
        assert_eq!(pochoir_alloc(usize::MAX), 0);

        let ptr = pochoir_alloc(1024);
        assert_ne!(ptr, 0);
        pochoir_free(ptr, 1024);
    }

    /// A buffer handed back and re-allocated must be reusable: the simplest
    /// check that `pochoir_free` really returns memory to the allocator rather
    /// than leaking it.
    #[test]
    fn a_buffer_can_be_allocated_and_freed_repeatedly() {
        for _ in 0..64 {
            let ptr = pochoir_alloc(1 << 16);
            assert_ne!(ptr, 0);
            pochoir_free(ptr, 1 << 16);
        }
    }

    #[test]
    fn the_result_pointer_is_stable_across_reads() {
        let image = jpeg(64, 64);
        let (ip, il) = copy_in(&image);
        let (tp, tl) = copy_in(TEXT.as_bytes());
        let (op, ol) = copy_in(&defaults());
        let handle = pochoir_render_pdf(ip, il, tp, tl, op, ol);
        let first = pochoir_result_ptr(handle);
        let _ = pochoir_result_len(handle);
        let _ = pochoir_result_status(handle);
        assert_eq!(first, pochoir_result_ptr(handle));
        pochoir_result_free(handle);
        pochoir_free(op, ol);
        pochoir_free(tp, tl);
        pochoir_free(ip, il);
    }
}
