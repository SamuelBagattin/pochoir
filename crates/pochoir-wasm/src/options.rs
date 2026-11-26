//! The packed 48-byte options struct, and its translation into engine types.
//!
//! The layout is documented once, at the crate root. This module is its parser.
//! Everything here is total: no indexing, no `unwrap`, no cast that can wrap,
//! and no value is clamped — an out-of-range field is
//! [`Status::BadArgument`](crate::Status::BadArgument), because a parameter
//! silently moved into range produces a document that does not say what the user
//! asked it to say.

use pochoir_core::mark::{Mark, TileDensity};
use pochoir_core::{MAX_DPI, OutputOptions, PageSizing};

use crate::Status;

/// Size of the packed struct, in bytes.
pub const OPTIONS_BYTES: usize = 48;

/// The only wire version this build understands.
pub const OPTIONS_VERSION: u32 = 1;

/// JPEG quality of ADR-0002's `standard` mode.
pub const STANDARD_QUALITY: u8 = 85;

/// JPEG quality of ADR-0002's `high` mode.
pub const HIGH_QUALITY: u8 = 92;

/// ADR-0002 Decision 2's quality modes, as the wire numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityMode {
    /// 150 DPI, downsample to a 150 DPI equivalent, JPEG q75. **Not
    /// implemented, and refused rather than approximated** — see
    /// [`Self::from_code`].
    Compact,
    /// Native pixels, JPEG q85. The default.
    Standard,
    /// Native pixels, JPEG q92.
    High,
    /// PNG `/FlateDecode` in the PDF. **Not implemented, and refused rather
    /// than approximated.**
    Lossless,
}

impl QualityMode {
    /// The mode for a wire code, or `None` for an undefined one.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Compact),
            1 => Some(Self::Standard),
            2 => Some(Self::High),
            3 => Some(Self::Lossless),
            _ => None,
        }
    }

    /// The JPEG quality this mode encodes at, or `None` if this engine cannot
    /// honour the mode at all.
    ///
    /// `compact` needs a resampler that does not exist, and that ADR-0002's own
    /// "do not resample image inputs at all" forbids for image inputs anyway;
    /// `lossless` needs a PNG image in the PDF and
    /// [`pochoir_core::pdf::PdfWriter`] can only embed a JPEG. Both return
    /// `None`, and the caller turns that into `BAD_ARGUMENT`.
    ///
    /// The alternative — mapping `lossless` onto q92 — would hand a user who
    /// explicitly asked for no generation loss a lossy file and tell them it was
    /// what they chose. This project does not do that.
    #[must_use]
    pub const fn jpeg_quality(self) -> Option<u8> {
        match self {
            Self::Standard => Some(STANDARD_QUALITY),
            Self::High => Some(HIGH_QUALITY),
            Self::Compact | Self::Lossless => None,
        }
    }
}

/// A decoded options struct, ready for the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// What the engine was asked for, mark included.
    pub options: OutputOptions,
    /// The mode the caller chose, kept so a caller-facing message can name it.
    pub mode: QualityMode,
}

impl Request {
    /// The mark, which this ABI always requires.
    ///
    /// [`OutputOptions::mark`] is an `Option` because the engine also serves a
    /// bare write path, but [`parse`] never produces `None`: an ABI whose only
    /// job is watermarking must not quietly return an unmarked document.
    #[must_use]
    pub fn mark(&self) -> Option<&Mark> {
        self.options.mark.as_ref()
    }
}

/// Decode `raw` and `text` into engine options.
///
/// # Errors
///
/// [`Status::BadArgument`] for a buffer of the wrong length, an unknown
/// version, an undefined enumerant, a value outside its range, or mark text
/// that is empty, over-long or undrawable. Every range check that the engine
/// already owns is left to [`OutputOptions::validate`], so there is one
/// definition of each bound rather than two that can drift.
pub fn parse(raw: &[u8], text: &str) -> Result<Request, Status> {
    if raw.len() != OPTIONS_BYTES {
        return Err(Status::BadArgument);
    }
    if u32_at(raw, 0) != Some(OPTIONS_VERSION) {
        return Err(Status::BadArgument);
    }

    let mode = u32_at(raw, 4)
        .and_then(QualityMode::from_code)
        .ok_or(Status::BadArgument)?;
    let quality = mode.jpeg_quality().ok_or(Status::BadArgument)?;

    let page_sizing = match u32_at(raw, 8) {
        Some(0) => PageSizing::CapToA4,
        Some(1) => PageSizing::Native,
        _ => return Err(Status::BadArgument),
    };

    // Range-checked as an integer *before* the cast, so an absurd figure never
    // becomes a float the engine then has to reason about. `MAX_DPI` is the
    // engine's own bound; comparing against it as a `u32` is exact for every
    // value it can hold.
    let dpi_raw = u32_at(raw, 12).ok_or(Status::BadArgument)?;
    if dpi_raw == 0 || u64::from(dpi_raw) > MAX_DPI as u64 {
        return Err(Status::BadArgument);
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "the value is bounded by MAX_DPI = 10 000, which an f32 holds exactly"
    )]
    let dpi = dpi_raw as f32;

    // The one unit conversion on the wire: the UI offers a percentage because
    // that is what a person reads off a slider; the engine keys the mark to a
    // ratio of the page's shorter edge.
    let font_size_ratio = f32_at(raw, 16).ok_or(Status::BadArgument)? / 100.0;
    let angle_degrees = f32_at(raw, 20).ok_or(Status::BadArgument)?;
    let opacity = f32_at(raw, 24).ok_or(Status::BadArgument)?;
    // Offset 28 is a ratio of the font size, not a pixel count: ADR-0007
    // Decision 3. The stale `blurPx` naming on the browser side is a rename
    // waiting to happen there, not a conversion to do here.
    let blur_ratio = f32_at(raw, 28).ok_or(Status::BadArgument)?;

    let density = match u32_at(raw, 32) {
        Some(0) => TileDensity::Low,
        Some(1) => TileDensity::Normal,
        Some(2) => TileDensity::High,
        _ => return Err(Status::BadArgument),
    };

    let packed_colour = u32_at(raw, 36).ok_or(Status::BadArgument)?;
    let colour = [
        ((packed_colour >> 16) & 0xFF) as u8,
        ((packed_colour >> 8) & 0xFF) as u8,
        (packed_colour & 0xFF) as u8,
    ];

    // Zero means "derive one from the document", which is ADR-0002 Decision 6's
    // behaviour and the engine's own default. Reading it as a literal seed would
    // give every document the same tile displacements, which is exactly what the
    // jitter exists to deny an attacker holding two marked copies.
    let seed = match u64_at(raw, 40).ok_or(Status::BadArgument)? {
        0 => None,
        value => Some(value),
    };

    let options = OutputOptions {
        quality,
        dpi,
        page_sizing,
        mark: Some(Mark {
            text: text.to_owned(),
            font_size_ratio,
            angle_degrees,
            opacity,
            colour,
            density,
            blur_ratio,
        }),
        seed,
        // The wire carries no jitter flag, so this ABI cannot turn the jitter
        // off. That is intentional: `--no-jitter` exists to reproduce a figure,
        // which is a CLI concern, and a browser user switching it off would
        // weaken their own document without being told why.
        jitter: true,
    };

    // One validation, the engine's, covering every numeric range and the mark
    // text. Re-checking the bounds here would create a second definition of
    // each one, free to drift from the first.
    options
        .validate()
        .map_err(|e| crate::status::status_for(&e))?;

    Ok(Request { options, mode })
}

fn u32_at(buffer: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes_at(buffer, offset)?))
}

fn f32_at(buffer: &[u8], offset: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes_at(buffer, offset)?))
}

fn u64_at(buffer: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let slice = buffer.get(offset..end)?;
    let bytes: [u8; 8] = slice.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

/// Four bytes at `offset`, or `None` if they are not all there.
///
/// `checked_add` and `get` rather than `offset + 4` and a range index: the
/// offsets here are constants today, but a panic in the parser of a hostile
/// 48-byte buffer would trap the instance, and this is the layer that must not.
fn bytes_at(buffer: &[u8], offset: usize) -> Option<[u8; 4]> {
    let end = offset.checked_add(4)?;
    let slice = buffer.get(offset..end)?;
    slice.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        HIGH_QUALITY, OPTIONS_BYTES, OPTIONS_VERSION, QualityMode, STANDARD_QUALITY, parse,
    };
    use crate::Status;
    use pochoir_core::PageSizing;
    use pochoir_core::mark::{
        DEFAULT_ANGLE_DEGREES, DEFAULT_BLUR_RATIO, DEFAULT_COLOUR, DEFAULT_FONT_SIZE_RATIO,
        DEFAULT_OPACITY, MAX_BLUR_RATIO, TileDensity,
    };

    /// The browser client's `DEFAULT_OPTIONS`, packed. Written out field by
    /// field rather than built by a helper, because this is the wire format and
    /// a test that shares a packer with the parser tests nothing.
    fn default_bytes() -> Vec<u8> {
        let mut b = Vec::with_capacity(OPTIONS_BYTES);
        b.extend_from_slice(&OPTIONS_VERSION.to_le_bytes()); // 0: version
        b.extend_from_slice(&1_u32.to_le_bytes()); // 4: standard
        b.extend_from_slice(&0_u32.to_le_bytes()); // 8: cap to A4
        b.extend_from_slice(&200_u32.to_le_bytes()); // 12: dpi
        b.extend_from_slice(&3.5_f32.to_le_bytes()); // 16: font size, per cent
        b.extend_from_slice(&(-30.0_f32).to_le_bytes()); // 20: angle
        b.extend_from_slice(&0.30_f32.to_le_bytes()); // 24: opacity
        b.extend_from_slice(&0.015_f32.to_le_bytes()); // 28: blur ratio
        b.extend_from_slice(&1_u32.to_le_bytes()); // 32: normal density
        b.extend_from_slice(&0x001A_1A1A_u32.to_le_bytes()); // 36: colour
        b.extend_from_slice(&0_u64.to_le_bytes()); // 40: seed
        assert_eq!(b.len(), OPTIONS_BYTES);
        b
    }

    fn with(offset: usize, bytes: &[u8]) -> Vec<u8> {
        let mut b = default_bytes();
        b[offset..offset + bytes.len()].copy_from_slice(bytes);
        b
    }

    #[test]
    fn the_browser_defaults_decode_to_the_engine_defaults() {
        let request = parse(&default_bytes(), "Pour X").expect("the defaults must parse");
        let mark = request.mark().expect("a mark is always produced");

        assert_eq!(request.mode, QualityMode::Standard);
        assert_eq!(request.options.quality, STANDARD_QUALITY);
        assert_eq!(request.options.dpi, 200.0);
        assert_eq!(request.options.page_sizing, PageSizing::CapToA4);
        assert_eq!(mark.text, "Pour X");
        assert_eq!(mark.font_size_ratio, DEFAULT_FONT_SIZE_RATIO);
        assert_eq!(mark.angle_degrees, DEFAULT_ANGLE_DEGREES);
        assert_eq!(mark.opacity, DEFAULT_OPACITY);
        assert_eq!(mark.blur_ratio, DEFAULT_BLUR_RATIO);
        assert_eq!(mark.colour, DEFAULT_COLOUR);
        assert_eq!(mark.density, TileDensity::Normal);
    }

    /// The point of this one: 3.5 on the wire is a percentage and 0.035 in the
    /// engine. Getting it wrong by a factor of 100 produces a mark either
    /// invisible or the size of the page, and both parse.
    #[test]
    fn the_font_size_is_a_percentage_on_the_wire_and_a_ratio_in_the_engine() {
        let request = parse(&with(16, &7.0_f32.to_le_bytes()), "X").expect("parses");
        assert_eq!(
            request.mark().expect("mark").font_size_ratio,
            0.07,
            "7.0 per cent must become the ratio 0.07"
        );
    }

    /// Offset 28 is a ratio of the font size. The browser snapshot called it
    /// `blurPx` with a range of 0..3; read as a ratio, 3.0 is far outside the
    /// engine's 0..0.06 and must be refused rather than clamped, because a
    /// clamp would hide the unit mismatch instead of surfacing it.
    #[test]
    fn blur_is_a_ratio_and_a_pixel_count_is_refused() {
        assert_eq!(
            parse(&with(28, &MAX_BLUR_RATIO.to_le_bytes()), "X")
                .expect("the maximum ratio is valid")
                .mark()
                .expect("mark")
                .blur_ratio,
            MAX_BLUR_RATIO
        );
        for stale_pixels in [0.6_f32, 3.0] {
            assert_eq!(
                parse(&with(28, &stale_pixels.to_le_bytes()), "X").err(),
                Some(Status::BadArgument),
                "{stale_pixels} read as a ratio is out of range and must be refused"
            );
        }
    }

    #[test]
    fn standard_and_high_differ_only_in_jpeg_quality() {
        let standard = parse(&default_bytes(), "X").expect("parses");
        let high = parse(&with(4, &2_u32.to_le_bytes()), "X").expect("parses");
        assert_eq!(standard.options.quality, STANDARD_QUALITY);
        assert_eq!(high.options.quality, HIGH_QUALITY);
        assert_eq!(high.mode, QualityMode::High);
        // The DPI comes from the struct, not from the mode: ADR-0002's DPI
        // column is headed "Raster DPI (PDF input)" and its image-input column
        // says "native, 1:1".
        assert_eq!(standard.options.dpi, high.options.dpi);
    }

    #[test]
    fn the_two_unimplemented_modes_are_refused_rather_than_approximated() {
        for (code, mode) in [(0_u32, QualityMode::Compact), (3, QualityMode::Lossless)] {
            assert_eq!(mode.jpeg_quality(), None);
            assert_eq!(
                parse(&with(4, &code.to_le_bytes()), "X").err(),
                Some(Status::BadArgument),
                "mode {code} has no engine behind it and must not be mapped onto a nearby quality"
            );
        }
    }

    #[test]
    fn an_undefined_enumerant_is_refused() {
        assert_eq!(
            parse(&with(4, &4_u32.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
        assert_eq!(
            parse(&with(8, &2_u32.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
        assert_eq!(
            parse(&with(32, &3_u32.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
    }

    #[test]
    fn a_wrong_length_or_unknown_version_is_refused() {
        assert_eq!(parse(&[], "X").err(), Some(Status::BadArgument));
        assert_eq!(
            parse(&default_bytes()[..47], "X").err(),
            Some(Status::BadArgument)
        );
        let mut long = default_bytes();
        long.push(0);
        assert_eq!(parse(&long, "X").err(), Some(Status::BadArgument));
        for version in [0_u32, 2, u32::MAX] {
            assert_eq!(
                parse(&with(0, &version.to_le_bytes()), "X").err(),
                Some(Status::BadArgument),
                "version {version} is not understood by this build"
            );
        }
    }

    #[test]
    fn the_dpi_is_range_checked_as_an_integer_before_the_cast() {
        assert_eq!(
            parse(&with(12, &0_u32.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
        assert_eq!(
            parse(&with(12, &10_001_u32.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
        assert_eq!(
            parse(&with(12, &u32::MAX.to_le_bytes()), "X").err(),
            Some(Status::BadArgument)
        );
        assert_eq!(
            parse(&with(12, &10_000_u32.to_le_bytes()), "X")
                .expect("the engine's own maximum is valid")
                .options
                .dpi,
            10_000.0
        );
    }

    #[test]
    fn a_zero_seed_means_derive_one() {
        assert_eq!(
            parse(&default_bytes(), "X").expect("parses").options.seed,
            None
        );
        assert_eq!(
            parse(&with(40, &7_u64.to_le_bytes()), "X")
                .expect("parses")
                .options
                .seed,
            Some(7)
        );
    }

    #[test]
    fn the_colour_ignores_the_top_byte() {
        let request = parse(&with(36, &0xFF12_3456_u32.to_le_bytes()), "X").expect("parses");
        assert_eq!(request.mark().expect("mark").colour, [0x12, 0x34, 0x56]);
    }

    #[test]
    fn a_non_finite_float_is_refused() {
        for offset in [16_usize, 20, 24, 28] {
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(
                    parse(&with(offset, &bad.to_le_bytes()), "X").err(),
                    Some(Status::BadArgument),
                    "{bad} at offset {offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn empty_or_undrawable_mark_text_is_refused() {
        for text in ["", "   ", "\n", "\u{4e2d}\u{6587}"] {
            assert_eq!(
                parse(&default_bytes(), text).err(),
                Some(Status::BadArgument),
                "{text:?} is not a mark this engine will draw"
            );
        }
        let too_long = "a".repeat(201);
        assert_eq!(
            parse(&default_bytes(), &too_long).err(),
            Some(Status::BadArgument)
        );
    }

    /// Accented French text is the common case and must survive, decomposed or
    /// not: the engine normalises to NFC.
    #[test]
    fn accented_french_text_is_accepted_in_either_normal_form() {
        for text in ["Destiné à", "Destine\u{301} a\u{300}"] {
            assert!(
                parse(&default_bytes(), text).is_ok(),
                "{text:?} must be accepted"
            );
        }
    }

    /// The parser must never panic, whatever the 48 bytes say. Not a proof, but
    /// it covers the shapes a fuzzer finds first: all zeroes, all ones, and
    /// every single-byte perturbation of a valid struct.
    #[test]
    fn no_48_byte_input_can_panic_the_parser() {
        let _ = parse(&[0_u8; OPTIONS_BYTES], "X");
        let _ = parse(&[0xFF_u8; OPTIONS_BYTES], "X");
        for offset in 0..OPTIONS_BYTES {
            for byte in [0x00_u8, 0x01, 0x7F, 0x80, 0xFF] {
                let _ = parse(&with(offset, &[byte]), "X");
            }
        }
        for len in 0..=96_usize {
            let _ = parse(&vec![0xAB_u8; len], "X");
        }
    }
}
