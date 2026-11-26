//! The two render pipelines, in safe Rust.
//!
//! Everything in this module is ordinary safe code with no pointers in it, so
//! both entry points can be unit tested on the host exactly as a browser calls
//! them. [`crate::abi`] is then only marshalling.

use pochoir_core::page::PageSource;
use pochoir_core::{ImagePage, Pixmap, image, image_to_pdf, mark};

use crate::options::{self, Request};
use crate::scale;
use crate::status::{Failure, Status};

/// What a render produced.
///
/// # Two invariants the ABI rests on
///
/// 1. **A document carries no dimensions.** `width` and `height` are the
///    bitmap's dimensions for a preview and `0` for a PDF, and `width > 0` is
///    the whole of how a caller tells the two apart. There is deliberately no
///    `kind` field — the value is already carried — so this must be an
///    invariant of the code rather than a property of the happy path. The three
///    constructors below are the only way to build an `Outcome`, each fixes the
///    dimensions for its own case, and `a_document_never_carries_dimensions`
///    pins it across every input the tests reach.
/// 2. **On failure, `bytes` is the detail payload**, not output: the structured
///    data described on [`Failure`], or empty where there is nothing to name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Zero on success.
    pub status: Status,
    /// On success the PDF bytes or the RGBA8 pixels; on failure the detail
    /// payload, which is often empty.
    pub bytes: Vec<u8>,
    /// Bitmap width, or `0` for a PDF and `0` on failure.
    pub width: u32,
    /// Bitmap height, or `0` for a PDF and `0` on failure.
    pub height: u32,
}

impl Outcome {
    /// A failure, carrying its detail payload and no dimensions.
    #[must_use]
    pub fn failed(failure: Failure) -> Self {
        Self {
            status: failure.status,
            bytes: failure.detail.into_bytes(),
            width: 0,
            height: 0,
        }
    }

    /// A failure with no payload. Shorthand for the common case.
    #[must_use]
    pub fn refused(status: Status) -> Self {
        Self::failed(Failure::bare(status))
    }

    /// A finished document: bytes, and both dimensions zero by construction.
    #[must_use]
    pub const fn document(bytes: Vec<u8>) -> Self {
        Self {
            status: Status::Ok,
            bytes,
            width: 0,
            height: 0,
        }
    }

    /// A finished bitmap.
    #[must_use]
    pub const fn bitmap(bytes: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            status: Status::Ok,
            bytes,
            width,
            height,
        }
    }
}

/// Decode `image_bytes`, burn the mark in and write a one-page PDF.
#[must_use]
pub fn render_pdf(image_bytes: &[u8], text: &str, packed_options: &[u8]) -> Outcome {
    let request = match options::parse(packed_options, text) {
        Ok(request) => request,
        Err(failure) => return Outcome::failed(failure),
    };
    match image_to_pdf(image_bytes, &request.options) {
        Ok(pdf) => Outcome::document(pdf),
        Err(error) => Outcome::failed(Failure::for_error(&error, text)),
    }
}

/// Render the same mark onto a bitmap bounded by `max_px` on its longer edge.
///
/// # Why this is not simply a lower-DPI render
///
/// For an image input the DPI decides the page's physical size and nothing
/// about its pixels, so there is no "render it smaller" to ask for. The bitmap
/// has to be resampled, and the order of the three operations is what decides
/// whether the preview is honest:
///
/// 1. **Rasterise at full resolution.** Same call, same pixels, same EXIF
///    orientation as the PDF path.
/// 2. **Derive the seed from those full-resolution pixels**, before anything is
///    thrown away. [`mark::derive_seed`] fingerprints the pixmap it is given, so
///    deriving it from the downscaled bitmap would give the preview different
///    tile positions from the document it is previewing — a preview that is
///    wrong about the one property ADR-0002 Decision 6 asks the jitter to carry.
/// 3. **Downscale, then compose.** The mark's size, tile lattice and blur are
///    all ratios of the page, so composing on the smaller bitmap reproduces the
///    same geometry at the same relative positions. Composing first and
///    shrinking afterwards would also be faithful, but it would do the full-size
///    glyph raster and blur that the preview exists to avoid.
#[must_use]
pub fn render_preview(
    image_bytes: &[u8],
    text: &str,
    packed_options: &[u8],
    max_px: u32,
) -> Outcome {
    let request = match options::parse(packed_options, text) {
        Ok(request) => request,
        Err(failure) => return Outcome::failed(failure),
    };
    match preview(&request, image_bytes, max_px) {
        Ok(outcome) => outcome,
        Err(error) => Outcome::failed(Failure::for_error(&error, text)),
    }
}

fn preview(request: &Request, image_bytes: &[u8], max_px: u32) -> pochoir_core::Result<Outcome> {
    let Some(mark) = request.mark() else {
        // Unreachable: `options::parse` always produces a mark. Belt and braces,
        // because the alternative to this branch is an `unwrap`.
        return Err(pochoir_core::Error::InvalidParameter { name: "mark text" });
    };

    let decoded = image::decode(image_bytes)?;
    let page = ImagePage::new(decoded, request.options.dpi, request.options.page_sizing)?;

    let mut full = Pixmap::empty();
    page.rasterise(0, request.options.dpi, &mut full)?;

    // Step 2 above: the seed is the full-resolution page's, whatever size the
    // preview ends up.
    let seed = match request.options.seed {
        Some(seed) => seed,
        None => mark::derive_seed(&full, mark),
    };

    let (width, height) = scale::preview_dimensions(full.width(), full.height(), max_px)?;
    let mut canvas = if width == full.width() && height == full.height() {
        full
    } else {
        scale::downscale(&full, width, height)?
    };

    mark::compose(&mut canvas, mark, seed, request.options.jitter)?;
    Ok(Outcome::bitmap(canvas.into_data(), width, height))
}

#[cfg(test)]
mod tests {
    use super::{render_pdf, render_preview};
    use crate::Status;
    use crate::options::OPTIONS_BYTES;
    use pochoir_core::page::PageSource;
    use pochoir_core::{ImagePage, Pixmap, image, mark};

    /// The browser client's defaults, packed. Duplicated from the options tests
    /// on purpose: a shared helper would let a packing mistake pass both.
    fn packed(mode: u32, seed: u64) -> Vec<u8> {
        let mut b = Vec::with_capacity(OPTIONS_BYTES);
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&mode.to_le_bytes());
        b.extend_from_slice(&0_u32.to_le_bytes());
        b.extend_from_slice(&200_u32.to_le_bytes());
        b.extend_from_slice(&3.5_f32.to_le_bytes());
        b.extend_from_slice(&(-30.0_f32).to_le_bytes());
        b.extend_from_slice(&0.30_f32.to_le_bytes());
        b.extend_from_slice(&0.015_f32.to_le_bytes());
        b.extend_from_slice(&1_u32.to_le_bytes());
        b.extend_from_slice(&0x001A_1A1A_u32.to_le_bytes());
        b.extend_from_slice(&seed.to_le_bytes());
        b
    }

    fn defaults() -> Vec<u8> {
        packed(1, 0)
    }

    /// A JPEG the engine itself encoded, which is the closest a unit test gets
    /// to a camera file. The integration tests use a real photograph.
    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut pixmap = Pixmap::new(width, height).expect("allocates");
        for y in 0..height {
            for x in 0..width {
                let r = ((x * 255) / width.max(1)) as u8;
                let g = ((y * 255) / height.max(1)) as u8;
                pixmap.set_pixel(x, y, [r, g, 0x80, 0xFF]);
            }
        }
        pochoir_core::jpeg::encode(&pixmap, 90, 200.0).expect("encodes")
    }

    const TEXT: &str = "Transmis a X le 26/11/2026";

    #[test]
    fn a_jpeg_goes_in_and_pdf_bytes_come_out() {
        let outcome = render_pdf(&jpeg(320, 240), TEXT, &defaults());
        assert_eq!(outcome.status, Status::Ok);
        assert!(outcome.bytes.starts_with(b"%PDF-"));
        assert!(outcome.bytes.ends_with(b"%%EOF\n"));
        assert!(
            String::from_utf8_lossy(&outcome.bytes).contains("/Filter /DCTDecode"),
            "the page must be an embedded JPEG"
        );
        assert_eq!(
            (outcome.width, outcome.height),
            (0, 0),
            "a document must carry no dimensions: that is how the caller tells them apart"
        );
    }

    #[test]
    fn a_jpeg_goes_in_and_rgba_pixels_come_out() {
        let outcome = render_preview(&jpeg(400, 300), TEXT, &defaults(), 200);
        assert_eq!(outcome.status, Status::Ok);
        assert_eq!((outcome.width, outcome.height), (200, 150));
        assert_eq!(
            outcome.bytes.len(),
            200 * 150 * 4,
            "len must always be width * height * 4"
        );
        assert!(
            outcome.bytes.chunks_exact(4).all(|p| p[3] == 0xFF),
            "the compositor hands back opaque pixels"
        );
    }

    #[test]
    fn the_preview_actually_draws_something() {
        let image = jpeg(400, 300);
        let marked = render_preview(&image, TEXT, &defaults(), 200);
        // The same pipeline with a mark that is as close to invisible as the
        // engine permits still differs; compare against the unmarked bitmap
        // produced by the scaler alone.
        let decoded = image::decode(&image).expect("decodes");
        let page = ImagePage::new(decoded, 200.0, pochoir_core::PageSizing::CapToA4)
            .expect("builds a page");
        let mut full = Pixmap::empty();
        page.rasterise(0, 200.0, &mut full).expect("rasterises");
        let plain = crate::scale::downscale(&full, 200, 150).expect("downscales");
        assert_ne!(
            marked.bytes,
            plain.data(),
            "the mark must change pixels in the preview"
        );
    }

    #[test]
    fn both_entry_points_are_byte_identical_across_runs() {
        let image = jpeg(200, 150);
        assert_eq!(
            render_pdf(&image, TEXT, &defaults()).bytes,
            render_pdf(&image, TEXT, &defaults()).bytes
        );
        assert_eq!(
            render_preview(&image, TEXT, &defaults(), 128).bytes,
            render_preview(&image, TEXT, &defaults(), 128).bytes
        );
    }

    /// The load-bearing claim about the preview: the mark's geometry is a ratio
    /// of the page, so a smaller render is a scale model of the document rather
    /// than an approximation of it.
    ///
    /// Measured here across a 64:1 range of page areas, every quantity as a
    /// fraction of the page's shorter edge. It holds to five decimal places:
    ///
    /// ```text
    /// 800x600  font/short 0.03500  block_width/short 0.43509  tiles 21
    /// 400x300  font/short 0.03500  block_width/short 0.43509  tiles 22
    /// 200x150  font/short 0.03500  block_width/short 0.43509  tiles 24
    /// 100x75   font/short 0.03500  block_width/short 0.43509  tiles 24
    /// ```
    ///
    /// The tile *count* is the one figure that moves, and it is deliberately
    /// not asserted equal. `placements` in the engine bounds the lattice with
    /// `.floor() - 1` and `.ceil() + 1` — a margin of one whole tile on each
    /// axis, which is a fixed number of tiles rather than a fixed fraction of
    /// the page. On a grid only three or four tiles wide, a one-step difference
    /// in where that rounding lands is a large relative change in the product.
    /// Those extra candidates fall outside the page and composite nothing, so
    /// `Composition::tiles` counts candidates and is not a scale-invariant
    /// quantity. What the user sees is the ratios above.
    #[test]
    fn the_preview_is_a_faithful_scale_model_of_the_page() {
        let image = jpeg(800, 600);
        let decoded = image::decode(&image).expect("decodes");
        let page = ImagePage::new(decoded, 200.0, pochoir_core::PageSizing::CapToA4)
            .expect("builds a page");
        let mut full = Pixmap::empty();
        page.rasterise(0, 200.0, &mut full).expect("rasterises");

        let request = crate::options::parse(&packed(1, 99), TEXT).expect("parses");
        let mark = request.mark().expect("mark");

        /// Every geometric quantity of a composition, divided by the page's
        /// shorter edge. Scale invariance means these do not move.
        fn ratios(pixmap: &Pixmap, mark: &pochoir_core::Mark) -> [f64; 4] {
            let shorter = f64::from(pixmap.width().min(pixmap.height()));
            let mut canvas = pixmap.clone();
            let c = mark::compose(&mut canvas, mark, 99, true).expect("composes");
            [
                f64::from(c.layout.font_size_px) / shorter,
                f64::from(c.layout.block_width_px) / shorter,
                f64::from(c.layout.block_height_px) / shorter,
                f64::from(c.sigma_px) / shorter,
            ]
        }

        let reference = ratios(&full, mark);
        for max_px in [400_u32, 200, 100] {
            let (w, h) = crate::scale::preview_dimensions(full.width(), full.height(), max_px)
                .expect("sizes");
            let preview = crate::scale::downscale(&full, w, h).expect("downscales");
            let measured = ratios(&preview, mark);
            for (index, (got, want)) in measured.iter().zip(reference.iter()).enumerate() {
                assert!(
                    (got - want).abs() / want < 1e-4,
                    "at {w}x{h}, ratio {index} is {got:.6} but the full page gives {want:.6}: \
                     the preview is not a scale model"
                );
            }
        }
    }

    /// The seed is derived from the full-resolution page in both paths, so a
    /// preview and the document it previews agree on the jitter. If the preview
    /// derived its seed from the downscaled bitmap this would differ.
    #[test]
    fn the_preview_and_the_document_derive_the_same_seed() {
        let image = jpeg(800, 600);
        let decoded = image::decode(&image).expect("decodes");
        let page = ImagePage::new(decoded, 200.0, pochoir_core::PageSizing::CapToA4)
            .expect("builds a page");
        let mut full = Pixmap::empty();
        page.rasterise(0, 200.0, &mut full).expect("rasterises");

        let request = crate::options::parse(&defaults(), TEXT).expect("parses");
        let mark = request.mark().expect("mark");
        let from_full = mark::derive_seed(&full, mark);

        let small = crate::scale::downscale(&full, 200, 150).expect("downscales");
        let from_small = mark::derive_seed(&small, mark);
        assert_ne!(
            from_full, from_small,
            "the seed depends on the pixels, which is why the order matters"
        );

        // The preview must match the document's seed, not the bitmap's. Checked
        // behaviourally: forcing the document's derived seed explicitly produces
        // the same preview as letting the preview derive it.
        let mut explicit = packed(1, 0);
        explicit[40..48].copy_from_slice(&from_full.to_le_bytes());
        assert_eq!(
            render_preview(&image, TEXT, &explicit, 200).bytes,
            render_preview(&image, TEXT, &defaults(), 200).bytes,
            "the preview's derived seed must be the full-resolution page's"
        );
    }

    #[test]
    fn a_page_smaller_than_the_bound_is_not_enlarged() {
        let outcome = render_preview(&jpeg(120, 90), TEXT, &defaults(), 1024);
        assert_eq!(outcome.status, Status::Ok);
        assert_eq!((outcome.width, outcome.height), (120, 90));
    }

    #[test]
    fn a_zero_bound_is_a_bad_argument() {
        assert_eq!(
            render_preview(&jpeg(120, 90), TEXT, &defaults(), 0).status,
            Status::BadArgument
        );
    }

    #[test]
    fn the_input_format_decides_between_unsupported_and_decode_failed() {
        // Not a JPEG and not a PNG at all.
        for bytes in [
            b"not an image".as_slice(),
            b"".as_slice(),
            b"%PDF-1.7".as_slice(),
        ] {
            assert_eq!(
                render_pdf(bytes, TEXT, &defaults()).status,
                Status::UnsupportedFormat,
                "{:?} is no format we read",
                &bytes[..bytes.len().min(8)]
            );
        }
        // A JPEG by its magic bytes, and then nothing the decoder can use.
        let truncated = &jpeg(64, 64)[..20];
        let status = render_pdf(truncated, TEXT, &defaults()).status;
        assert_eq!(
            status,
            Status::DecodeFailed,
            "a truncated JPEG is a decode failure, not an unsupported format"
        );
    }

    #[test]
    fn bad_options_are_refused_before_the_image_is_touched() {
        // Nonsense image bytes with nonsense options: the options lose, which
        // proves nothing was decoded first.
        assert_eq!(
            render_pdf(b"not an image", TEXT, &[0_u8; OPTIONS_BYTES]).status,
            Status::BadArgument
        );
        assert_eq!(
            render_preview(b"not an image", TEXT, &[], 100).status,
            Status::BadArgument
        );
    }

    #[test]
    fn an_unimplementable_mode_fails_on_both_paths() {
        for mode in [0_u32, 3] {
            assert_eq!(
                render_pdf(&jpeg(64, 64), TEXT, &packed(mode, 0)).status,
                Status::BadArgument
            );
            assert_eq!(
                render_preview(&jpeg(64, 64), TEXT, &packed(mode, 0), 64).status,
                Status::BadArgument
            );
        }
    }

    #[test]
    fn a_failure_carries_no_bytes_and_no_dimensions() {
        let outcome = render_preview(b"not an image", TEXT, &defaults(), 100);
        assert_ne!(outcome.status, Status::Ok);
        assert!(outcome.bytes.is_empty());
        assert_eq!((outcome.width, outcome.height), (0, 0));
    }

    /// The ABI has no `kind` field: a caller discriminates on `width > 0`. That
    /// makes "a PDF always has both dimensions zero" load-bearing rather than
    /// incidental, so it is asserted over every shape of input the suite can
    /// reach — successes, each failure class, and a range of page geometries —
    /// instead of only the happy path.
    #[test]
    fn a_document_never_carries_dimensions() {
        let inputs: [&[u8]; 6] = [
            &jpeg(64, 64),
            &jpeg(300, 80),
            &jpeg(80, 300),
            b"not an image",
            &jpeg(64, 64)[..20],
            b"",
        ];
        for image in inputs {
            for text in [TEXT, "", "\u{4e2d}"] {
                for mode in [0_u32, 1, 2, 3] {
                    let outcome = render_pdf(image, text, &packed(mode, 0));
                    assert_eq!(
                        (outcome.width, outcome.height),
                        (0, 0),
                        "a PDF result must never carry dimensions, whatever happened"
                    );
                }
            }
        }
    }

    /// The mirror of the invariant above: a successful preview always carries
    /// both dimensions, so `width > 0` is never ambiguous.
    #[test]
    fn a_successful_preview_always_carries_both_dimensions() {
        for (w, h) in [(400_u32, 300_u32), (300, 400), (64, 64), (1000, 20)] {
            for max_px in [1024_u32, 200, 32] {
                let outcome = render_preview(&jpeg(w, h), TEXT, &defaults(), max_px);
                assert_eq!(outcome.status, Status::Ok, "{w}x{h} at {max_px}");
                assert!(outcome.width > 0 && outcome.height > 0);
                assert_eq!(
                    outcome.bytes.len() as u64,
                    u64::from(outcome.width) * u64::from(outcome.height) * 4,
                    "RGBA8 with no row padding: len must be width * height * 4"
                );
            }
        }
    }

    /// The two named refusals of ADR-0007 reach the caller as their own status,
    /// with the undrawable characters as the payload.
    #[test]
    fn the_named_refusals_reach_the_caller_with_their_payload() {
        let image = jpeg(200, 150);

        let glyphs = render_pdf(&image, "Pour \u{4e2d} et \u{6587}", &defaults());
        assert_eq!(glyphs.status, Status::UnsupportedCharacter);
        assert_eq!(
            String::from_utf8(glyphs.bytes).expect("the payload is UTF-8"),
            "\u{4e2d}\u{6587}"
        );

        // 200 wide characters with no space in them cannot be broken, which is
        // the one way ADR-0007's refusal path is reached. `"M".repeat(200)` is
        // the engine's own case for it; `"a".repeat(200)` still fits, because
        // the refusal depends on the advance width rather than the count.
        let unbreakable = "M".repeat(200);
        let refused = render_pdf(&image, &unbreakable, &defaults());
        assert_eq!(refused.status, Status::TextDoesNotFit);
        assert!(
            refused.bytes.is_empty(),
            "there is nothing to name beyond the status itself"
        );
    }

    #[test]
    fn a_bad_argument_names_its_field_on_both_paths() {
        let image = jpeg(64, 64);
        let bad_blur = {
            let mut o = defaults();
            o[28..32].copy_from_slice(&3.0_f32.to_le_bytes());
            o
        };
        for outcome in [
            render_pdf(&image, TEXT, &bad_blur),
            render_preview(&image, TEXT, &bad_blur, 32),
        ] {
            assert_eq!(outcome.status, Status::BadArgument);
            assert_eq!(
                String::from_utf8(outcome.bytes).expect("UTF-8"),
                "mark blur"
            );
        }
    }

    /// No payload this crate emits may exceed what the caller will read.
    #[test]
    fn no_payload_exceeds_the_readers_cap() {
        let image = jpeg(64, 64);
        // 200 distinct CJK characters: the largest payload reachable, since the
        // mark text is capped at 200 characters.
        let text: String = (0x4E00..0x4E00 + 200_u32)
            .filter_map(char::from_u32)
            .collect();
        let outcome = render_pdf(&image, &text, &defaults());
        assert_eq!(outcome.status, Status::UnsupportedCharacter);
        assert!(
            outcome.bytes.len() <= crate::status::MAX_DETAIL_BYTES,
            "payload was {} bytes",
            outcome.bytes.len()
        );
        assert!(
            String::from_utf8(outcome.bytes).is_ok(),
            "must stay valid UTF-8"
        );
    }

    #[test]
    fn a_png_with_transparency_is_accepted_on_both_paths() {
        let png = {
            let mut pixmap = Pixmap::new(80, 60).expect("allocates");
            for y in 0..60 {
                for x in 0..80 {
                    let a = if (x + y) % 2 == 0 { 0xFF } else { 0x40 };
                    pixmap.set_pixel(x, y, [0xC0, 0x30, 0x30, a]);
                }
            }
            encode_png(&pixmap)
        };
        assert_eq!(render_pdf(&png, TEXT, &defaults()).status, Status::Ok);
        let preview = render_preview(&png, TEXT, &defaults(), 40);
        assert_eq!(preview.status, Status::Ok);
        assert_eq!((preview.width, preview.height), (40, 30));
    }

    fn encode_png(pixmap: &Pixmap) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("writes a header");
            writer
                .write_image_data(pixmap.data())
                .expect("writes the pixels");
        }
        out
    }
}
