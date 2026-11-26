//! Reading the two EXIF tags we need, and nothing else.
//!
//! This is a *read*, not an EXIF library. ADR-0002 Decision 5 says we take
//! exactly two things out of the input's metadata and discard the rest
//! structurally, by re-encoding:
//!
//! - `Orientation`, so that a phone photo is not sideways in the output;
//! - `XResolution` / `YResolution` / `ResolutionUnit`, to derive a sensible
//!   physical page size.
//!
//! Everything else — GPS, `DateTimeOriginal`, `Make`, `Model`, the embedded
//! thumbnail — is never parsed, never copied and never reaches the output. We
//! walk IFD0 only: no SubIFD, no GPS IFD, no Interoperability IFD, no
//! `MakerNote`. Not following those pointers is both the privacy behaviour we
//! want and most of the reason this parser is small enough to audit.
//!
//! The byte layout is TIFF 6.0 §2 and §8, which is the normative upstream for
//! the Exif IFD0 structure: an 8-byte header giving byte order and the offset
//! of IFD0, then a count of 12-byte entries, each `tag:u16 type:u16
//! count:u32 value_or_offset:u32`.
//!
//! Every read here is bounds-checked and the parser never recurses, because
//! this is the one place in the crate that looks at attacker-controlled
//! structure rather than attacker-controlled pixels.

use crate::pixmap::Pixmap;
use crate::{Error, Result};

/// IFD0 `Orientation`, TIFF 6.0 §8: tag `0x0112` (274 decimal), type SHORT,
/// count 1, default 1, eight defined values.
const TAG_ORIENTATION: u16 = 0x0112;
/// IFD0 `XResolution`, type RATIONAL.
const TAG_X_RESOLUTION: u16 = 0x011A;
/// IFD0 `YResolution`, type RATIONAL.
const TAG_Y_RESOLUTION: u16 = 0x011B;
/// IFD0 `ResolutionUnit`, type SHORT: 1 none, 2 inch, 3 centimetre.
const TAG_RESOLUTION_UNIT: u16 = 0x0128;

const TYPE_SHORT: u16 = 3;
const TYPE_RATIONAL: u16 = 5;

/// The marker that introduces the TIFF block inside a JPEG `APP1` segment.
///
/// Also the byte sequence ADR-0002 requires a test to prove absent from every
/// output file.
pub const EXIF_HEADER: &[u8; 6] = b"Exif\x00\x00";

/// How the stored pixels must be transformed to be displayed upright.
///
/// The eight values of TIFF 6.0 §8's `Orientation`, named for where the first
/// row and first column of the stored image end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// 1 — already upright. The default when the tag is absent.
    #[default]
    TopLeft,
    /// 2 — mirrored left to right.
    TopRight,
    /// 3 — rotated 180°.
    BottomRight,
    /// 4 — mirrored top to bottom.
    BottomLeft,
    /// 5 — transposed about the main diagonal.
    LeftTop,
    /// 6 — rotated 90° clockwise to display. The common portrait phone photo.
    RightTop,
    /// 7 — transposed about the anti-diagonal.
    RightBottom,
    /// 8 — rotated 90° anticlockwise to display.
    LeftBottom,
}

impl Orientation {
    /// The orientation for a raw tag value, or `None` if it is not one of the
    /// eight defined values.
    #[must_use]
    pub const fn from_tag(value: u16) -> Option<Self> {
        Some(match value {
            1 => Self::TopLeft,
            2 => Self::TopRight,
            3 => Self::BottomRight,
            4 => Self::BottomLeft,
            5 => Self::LeftTop,
            6 => Self::RightTop,
            7 => Self::RightBottom,
            8 => Self::LeftBottom,
            _ => return None,
        })
    }

    /// The raw tag value.
    #[must_use]
    pub const fn to_tag(self) -> u16 {
        match self {
            Self::TopLeft => 1,
            Self::TopRight => 2,
            Self::BottomRight => 3,
            Self::BottomLeft => 4,
            Self::LeftTop => 5,
            Self::RightTop => 6,
            Self::RightBottom => 7,
            Self::LeftBottom => 8,
        }
    }

    /// `true` when displaying the image upright swaps its width and height.
    #[must_use]
    pub const fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::LeftTop | Self::RightTop | Self::RightBottom | Self::LeftBottom
        )
    }

    /// `true` when no pixels need to move.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        matches!(self, Self::TopLeft)
    }

    /// Where the stored pixel `(x, y)` of a `width` by `height` image belongs in
    /// the upright image.
    const fn map(self, x: u32, y: u32, width: u32, height: u32) -> (u32, u32) {
        match self {
            Self::TopLeft => (x, y),
            Self::TopRight => (width - 1 - x, y),
            Self::BottomRight => (width - 1 - x, height - 1 - y),
            Self::BottomLeft => (x, height - 1 - y),
            Self::LeftTop => (y, x),
            Self::RightTop => (height - 1 - y, x),
            Self::RightBottom => (height - 1 - y, width - 1 - x),
            Self::LeftBottom => (y, width - 1 - x),
        }
    }
}

/// Everything we take from an input's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Metadata {
    /// How to rotate or flip the pixels. Defaults to upright.
    pub orientation: Orientation,
    /// Horizontal resolution in dots per inch, if the input stated one.
    pub x_dpi: Option<f32>,
    /// Vertical resolution in dots per inch, if the input stated one.
    pub y_dpi: Option<f32>,
}

/// Rotate or flip `source` so that it is upright.
///
/// Returns a clone for [`Orientation::TopLeft`]; otherwise allocates the
/// transformed buffer, whose dimensions are swapped for the four transposing
/// orientations.
///
/// # Errors
///
/// Returns [`Error::ImageTooLarge`] only if the destination buffer cannot be
/// allocated, which cannot happen for a `source` that already exists.
pub fn apply(source: &Pixmap, orientation: Orientation) -> Result<Pixmap> {
    if orientation.is_identity() {
        return Ok(source.clone());
    }
    let (width, height) = (source.width(), source.height());
    let (out_width, out_height) = if orientation.swaps_axes() {
        (height, width)
    } else {
        (width, height)
    };
    let mut out = Pixmap::new(out_width, out_height)?;
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = orientation.map(x, y, width, height);
            let pixel = source.pixel(x, y).ok_or(Error::BufferSize {
                expected: 0,
                got: 0,
            })?;
            out.set_pixel(dx, dy, pixel);
        }
    }
    Ok(out)
}

/// Locate the TIFF block of the first `APP1`/Exif segment in a JPEG.
///
/// Returns the bytes **after** the `Exif\0\0` marker, which is where the TIFF
/// header begins and what offsets inside the block are relative to.
#[must_use]
pub fn find_in_jpeg(jpeg: &[u8]) -> Option<&[u8]> {
    // SOI, then a sequence of marker segments. Stop at SOS: everything after it
    // is entropy-coded scan data, in which a byte pair that looks like APP1 is
    // just data.
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    let mut at = 2usize;
    loop {
        // Skip any fill bytes; a marker may be preceded by extra 0xFF octets.
        while jpeg.get(at) == Some(&0xFF) && jpeg.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if jpeg.get(at) != Some(&0xFF) {
            return None;
        }
        let marker = *jpeg.get(at + 1)?;
        at += 2;
        match marker {
            // SOI (0xD8), SOS (0xDA) and EOI (0xD9). Reaching SOS means scan
            // data follows, in which a byte pair that looks like a segment is
            // not one; the other two mean there is nothing left to find.
            0xD8..=0xDA => return None,
            // Standalone markers with no length field.
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*jpeg.get(at)?, *jpeg.get(at + 1)?]));
        if length < 2 {
            return None;
        }
        let payload = jpeg.get(at + 2..at + length)?;
        if marker == 0xE1 && payload.starts_with(EXIF_HEADER) {
            return Some(&payload[EXIF_HEADER.len()..]);
        }
        at += length;
    }
}

/// A bounds-checked cursor over a TIFF block of known byte order.
struct Tiff<'a> {
    bytes: &'a [u8],
    big_endian: bool,
}

impl<'a> Tiff<'a> {
    fn u16_at(&self, offset: usize) -> Option<u16> {
        let raw = [*self.bytes.get(offset)?, *self.bytes.get(offset + 1)?];
        Some(if self.big_endian {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        })
    }

    fn u32_at(&self, offset: usize) -> Option<u32> {
        let raw = [
            *self.bytes.get(offset)?,
            *self.bytes.get(offset + 1)?,
            *self.bytes.get(offset + 2)?,
            *self.bytes.get(offset + 3)?,
        ];
        Some(if self.big_endian {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        })
    }

    /// A RATIONAL is two `u32`s: numerator then denominator.
    fn rational_at(&self, offset: usize) -> Option<f32> {
        let numerator = self.u32_at(offset)?;
        let denominator = self.u32_at(offset + 4)?;
        if denominator == 0 {
            return None;
        }
        Some(numerator as f32 / denominator as f32)
    }
}

/// Read [`Metadata`] out of a TIFF/Exif block.
///
/// Returns the default (upright, no resolution) for anything malformed: a
/// metadata block we cannot understand is a block we ignore, never an error
/// that stops the user watermarking their document.
#[must_use]
pub fn parse(block: &[u8]) -> Metadata {
    let mut out = Metadata::default();
    let big_endian = match block.get(..2) {
        Some(b"MM") => true,
        Some(b"II") => false,
        _ => return out,
    };
    let tiff = Tiff {
        bytes: block,
        big_endian,
    };
    // The 42 that proves we read the byte order the right way round.
    if tiff.u16_at(2) != Some(42) {
        return out;
    }
    let Some(ifd0) = tiff.u32_at(4).and_then(|o| usize::try_from(o).ok()) else {
        return out;
    };
    let Some(entries) = tiff.u16_at(ifd0) else {
        return out;
    };

    let mut unit_is_centimetres = false;
    let mut x_resolution = None;
    let mut y_resolution = None;

    for index in 0..usize::from(entries) {
        // 2 bytes of count, then 12 bytes per entry.
        let entry = ifd0 + 2 + index * 12;
        let (Some(tag), Some(kind), Some(count)) = (
            tiff.u16_at(entry),
            tiff.u16_at(entry + 2),
            tiff.u32_at(entry + 4),
        ) else {
            break;
        };
        if count != 1 {
            continue;
        }
        match (tag, kind) {
            (TAG_ORIENTATION, TYPE_SHORT) => {
                // A SHORT fits in the 4-byte value field, left-aligned, so it
                // lives at the start of the field in both byte orders. A value
                // outside the eight defined ones leaves the default in place.
                if let Some(orientation) = tiff.u16_at(entry + 8).and_then(Orientation::from_tag) {
                    out.orientation = orientation;
                }
            }
            (TAG_RESOLUTION_UNIT, TYPE_SHORT) => {
                unit_is_centimetres = tiff.u16_at(entry + 8) == Some(3);
            }
            // A RATIONAL is 8 bytes, too big for the value field, so the field
            // holds an offset from the start of the TIFF block instead.
            (TAG_X_RESOLUTION, TYPE_RATIONAL) => {
                x_resolution = tiff
                    .u32_at(entry + 8)
                    .and_then(|o| usize::try_from(o).ok())
                    .and_then(|o| tiff.rational_at(o));
            }
            (TAG_Y_RESOLUTION, TYPE_RATIONAL) => {
                y_resolution = tiff
                    .u32_at(entry + 8)
                    .and_then(|o| usize::try_from(o).ok())
                    .and_then(|o| tiff.rational_at(o));
            }
            _ => {}
        }
    }

    // ResolutionUnit 1 means "no absolute unit", so the numbers are a pixel
    // aspect ratio and not a resolution at all; 2 is inches, 3 centimetres.
    let to_dpi = |value: Option<f32>| {
        value
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| if unit_is_centimetres { v * 2.54 } else { v })
    };
    out.x_dpi = to_dpi(x_resolution);
    out.y_dpi = to_dpi(y_resolution);
    out
}

#[cfg(test)]
mod tests {
    use super::{EXIF_HEADER, Metadata, Orientation, apply, find_in_jpeg, parse};
    use crate::pixmap::Pixmap;

    /// Build a little- or big-endian TIFF block holding the given IFD0 entries,
    /// plus any out-of-line values appended after the directory.
    ///
    /// `entries` are `(tag, type, inline_value_or_offset)` triples; `trailing`
    /// is appended at a known offset so RATIONAL entries can point at it.
    fn tiff_block(
        big_endian: bool,
        entries: &[(u16, u16, u32)],
        trailing: &[u8],
    ) -> (Vec<u8>, u32) {
        let mut out = Vec::new();
        out.extend_from_slice(if big_endian { b"MM" } else { b"II" });
        let u16_bytes = |v: u16| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32_bytes = |v: u32| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        out.extend_from_slice(&u16_bytes(42));
        out.extend_from_slice(&u32_bytes(8)); // IFD0 starts right after the header
        out.extend_from_slice(&u16_bytes(entries.len() as u16));
        for &(tag, kind, value) in entries {
            out.extend_from_slice(&u16_bytes(tag));
            out.extend_from_slice(&u16_bytes(kind));
            out.extend_from_slice(&u32_bytes(1)); // count
            out.extend_from_slice(&u32_bytes(value));
        }
        out.extend_from_slice(&u32_bytes(0)); // no IFD1
        let trailing_at = out.len() as u32;
        out.extend_from_slice(trailing);
        (out, trailing_at)
    }

    fn rational(big_endian: bool, numerator: u32, denominator: u32) -> Vec<u8> {
        let mut out = Vec::new();
        if big_endian {
            out.extend_from_slice(&numerator.to_be_bytes());
            out.extend_from_slice(&denominator.to_be_bytes());
        } else {
            out.extend_from_slice(&numerator.to_le_bytes());
            out.extend_from_slice(&denominator.to_le_bytes());
        }
        out
    }

    #[test]
    fn all_eight_orientation_values_round_trip_through_the_tag() {
        for tag in 1..=8u16 {
            let orientation = Orientation::from_tag(tag).expect("1..=8 are all defined");
            assert_eq!(orientation.to_tag(), tag);
        }
        assert_eq!(Orientation::from_tag(0), None);
        assert_eq!(Orientation::from_tag(9), None);
        assert_eq!(Orientation::default(), Orientation::TopLeft);
    }

    #[test]
    fn all_eight_orientations_are_read_in_both_byte_orders() {
        for big_endian in [false, true] {
            for tag in 1..=8u16 {
                // A SHORT is left-aligned in the 4-byte value field, so in
                // big-endian the value occupies the high half-word.
                let field = if big_endian {
                    u32::from(tag) << 16
                } else {
                    u32::from(tag)
                };
                let (block, _) = tiff_block(big_endian, &[(0x0112, 3, field)], &[]);
                assert_eq!(
                    parse(&block).orientation,
                    Orientation::from_tag(tag).expect("defined"),
                    "orientation {tag}, big_endian={big_endian}"
                );
            }
        }
    }

    #[test]
    fn resolution_is_read_and_converted_from_centimetres() {
        for big_endian in [false, true] {
            let trailing = rational(big_endian, 300, 1);
            let (block, at) = tiff_block(
                big_endian,
                &[
                    (0x011A, 5, 0),
                    (0x011B, 5, 0),
                    (0x0128, 3, if big_endian { 2 << 16 } else { 2 }),
                ],
                &trailing,
            );
            // Patch both RATIONAL offsets to point at the trailing value.
            let mut block = block;
            let offset_bytes = if big_endian {
                at.to_be_bytes()
            } else {
                at.to_le_bytes()
            };
            block[8 + 2 + 8..8 + 2 + 12].copy_from_slice(&offset_bytes);
            block[8 + 2 + 12 + 8..8 + 2 + 12 + 12].copy_from_slice(&offset_bytes);
            let metadata = parse(&block);
            assert_eq!(metadata.x_dpi, Some(300.0));
            assert_eq!(metadata.y_dpi, Some(300.0));

            // The same numbers with ResolutionUnit 3 are centimetres.
            let unit_field_at = 8 + 2 + 2 * 12 + 8;
            let three = if big_endian {
                (3u32 << 16).to_be_bytes()
            } else {
                3u32.to_le_bytes()
            };
            block[unit_field_at..unit_field_at + 4].copy_from_slice(&three);
            let metadata = parse(&block);
            assert_eq!(metadata.x_dpi, Some(300.0 * 2.54));
        }
    }

    #[test]
    fn resolution_unit_none_means_there_is_no_resolution() {
        let big_endian = false;
        let trailing = rational(big_endian, 72, 1);
        let (mut block, at) = tiff_block(big_endian, &[(0x011A, 5, 0), (0x0128, 3, 1)], &trailing);
        block[8 + 2 + 8..8 + 2 + 12].copy_from_slice(&at.to_le_bytes());
        // Unit 1 is "no absolute unit", so we keep the number as given rather
        // than inventing a conversion; it is still a dpi-shaped value.
        assert_eq!(parse(&block).x_dpi, Some(72.0));
    }

    #[test]
    fn malformed_blocks_are_ignored_rather_than_rejected() {
        for block in [
            &b""[..],
            &b"MM"[..],
            &b"XX\x00\x2a\x00\x00\x00\x08"[..],
            // Right byte order, wrong magic.
            &b"II\x2b\x00\x08\x00\x00\x00"[..],
            // IFD0 offset past the end of the block.
            &b"II\x2a\x00\xff\xff\xff\xff"[..],
            // Entry count that overruns the block.
            &b"II\x2a\x00\x08\x00\x00\x00\xff\xff"[..],
        ] {
            assert_eq!(parse(block), Metadata::default(), "{block:?}");
        }
    }

    #[test]
    fn a_zero_denominator_does_not_divide_by_zero() {
        let trailing = rational(false, 300, 0);
        let (mut block, at) = tiff_block(false, &[(0x011A, 5, 0)], &trailing);
        block[8 + 2 + 8..8 + 2 + 12].copy_from_slice(&at.to_le_bytes());
        assert_eq!(parse(&block).x_dpi, None);
    }

    /// A 3x2 pixmap whose pixels encode their own coordinates, so a transform
    /// can be checked exactly rather than visually.
    fn marked_pixmap() -> Pixmap {
        let mut pixmap = Pixmap::new(3, 2).expect("3x2 is a legal size");
        for y in 0..2u32 {
            for x in 0..3u32 {
                pixmap.set_pixel(x, y, [x as u8, y as u8, 0, 255]);
            }
        }
        pixmap
    }

    #[test]
    fn the_eight_orientations_move_pixels_exactly_as_tiff_6_0_specifies() {
        let source = marked_pixmap();
        // Expected (width, height, and the pixel that ends up at (0, 0)) for
        // each orientation, derived from where the stored first row and first
        // column belong once the image is upright.
        let cases: [(Orientation, u32, u32, [u8; 2]); 8] = [
            (Orientation::TopLeft, 3, 2, [0, 0]),
            (Orientation::TopRight, 3, 2, [2, 0]),
            (Orientation::BottomRight, 3, 2, [2, 1]),
            (Orientation::BottomLeft, 3, 2, [0, 1]),
            (Orientation::LeftTop, 2, 3, [0, 0]),
            (Orientation::RightTop, 2, 3, [0, 1]),
            (Orientation::RightBottom, 2, 3, [2, 1]),
            (Orientation::LeftBottom, 2, 3, [2, 0]),
        ];
        for (orientation, width, height, top_left) in cases {
            let out = apply(&source, orientation).expect("transform fits");
            assert_eq!(
                (out.width(), out.height()),
                (width, height),
                "{orientation:?} dimensions"
            );
            let pixel = out.pixel(0, 0).expect("in range");
            assert_eq!(
                [pixel[0], pixel[1]],
                top_left,
                "{orientation:?} top-left pixel"
            );
            // Whatever the transform, it is a permutation: every source pixel
            // appears exactly once.
            let mut seen: Vec<[u8; 2]> = (0..out.height())
                .flat_map(|y| (0..out.width()).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let p = out.pixel(x, y).expect("in range");
                    [p[0], p[1]]
                })
                .collect();
            seen.sort_unstable();
            let mut expected: Vec<[u8; 2]> = (0..2u8)
                .flat_map(|y| (0..3u8).map(move |x| [x, y]))
                .collect();
            expected.sort_unstable();
            assert_eq!(seen, expected, "{orientation:?} is not a permutation");
        }
    }

    #[test]
    fn rotating_four_times_in_ninety_degree_steps_returns_the_original() {
        let source = marked_pixmap();
        let mut rotated = source.clone();
        for _ in 0..4 {
            rotated = apply(&rotated, Orientation::RightTop).expect("fits");
        }
        assert_eq!(rotated, source);
    }

    #[test]
    fn mirroring_twice_returns_the_original() {
        let source = marked_pixmap();
        let once = apply(&source, Orientation::TopRight).expect("fits");
        let twice = apply(&once, Orientation::TopRight).expect("fits");
        assert_eq!(twice, source);
    }

    #[test]
    fn which_orientations_swap_the_axes() {
        for tag in 1..=8u16 {
            let orientation = Orientation::from_tag(tag).expect("defined");
            assert_eq!(orientation.swaps_axes(), tag >= 5, "orientation {tag}");
        }
    }

    #[test]
    fn app1_is_found_in_a_jpeg_and_scan_data_is_not_searched() {
        let (block, _) = tiff_block(false, &[(0x0112, 3, 6)], &[]);
        let mut payload = Vec::from(&EXIF_HEADER[..]);
        payload.extend_from_slice(&block);
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&payload);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        let found = find_in_jpeg(&jpeg).expect("APP1 is there");
        assert_eq!(parse(found).orientation, Orientation::RightTop);

        // The same bytes placed after SOS are scan data, not a segment.
        let mut after_sos = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        after_sos.extend_from_slice(&jpeg[2..]);
        assert!(find_in_jpeg(&after_sos).is_none());
    }

    #[test]
    fn a_jpeg_without_exif_yields_nothing() {
        assert!(find_in_jpeg(&[]).is_none());
        assert!(find_in_jpeg(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none());
        assert!(find_in_jpeg(b"not a jpeg at all").is_none());
        // A truncated segment length must not read past the end.
        assert!(find_in_jpeg(&[0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, 0x00]).is_none());
    }
}
