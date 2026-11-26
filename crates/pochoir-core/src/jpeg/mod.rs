//! A baseline JPEG encoder, written here rather than taken from a crate.
//!
//! ADR-0001's rule is "hand-roll everything that writes; depend deliberately
//! only for parsers of hostile input". An encoder processes **our** pixels, its
//! correctness is checkable against any decoder, and a bug in it is a broken
//! file rather than a compromise — so it is ours. Two further reasons are
//! recorded there: `jpeg-encoder`'s licence is the conjunctive
//! `(MIT OR Apache-2.0) AND IJG`, which carries an attribution obligation; and
//! owning the quantisation tables is what gives ADR-0002's determinism
//! requirement something to stand on.
//!
//! What this writes is baseline sequential JPEG as specified by ITU-T T.81:
//! `SOI`, a JFIF `APP0`, two `DQT` segments, `SOF0`, four `DHT` segments,
//! `SOS`, the entropy-coded scan, `EOI`. Huffman coding only, 8-bit samples,
//! three components, **no chroma subsampling**.
//!
//! # Why 4:4:4
//!
//! Full chroma resolution costs bytes — a 4:2:0 scan of the same image is
//! perhaps a fifth smaller — and buys two things this project specifically
//! wants. Identity documents carry coloured seals, stamps and guilloche
//! printing at the limit of legibility, and chroma subsampling is exactly the
//! operation that smears them. And the watermark ADR-0002 describes is a
//! diagonal stroke over document text, whose edges subsample badly. The
//! encoder is structured so that adding 4:2:0 later is a change to how blocks
//! are gathered into an MCU, not a rewrite.
//!
//! # Determinism
//!
//! Output is a pure function of (pixels, quality, dpi), on wasm32 and on
//! native. Nothing here reads a clock or a random number, and the only
//! floating-point operations used are the exactly specified ones — see
//! [`dct`] for why that matters.

pub mod dct;
pub mod huffman;
pub mod info;
pub mod quant;

pub use info::{Info, Process, parse as parse_info};

use crate::pixmap::Pixmap;
use crate::{Error, Result};
use dct::{Block, Quantiser};
use huffman::{BitWriter, Table, Tables, category};

/// Largest dimension a JPEG frame header can express: `SOF` stores the number
/// of lines and samples per line as 16-bit integers.
pub const MAX_DIMENSION: u32 = u16::MAX as u32;

/// Component identifiers, scan order. Conventional values for JFIF.
const COMPONENT_IDS: [u8; 3] = [1, 2, 3];

/// Encode an RGBA8 pixmap as a baseline JPEG datastream.
///
/// Alpha is composited over opaque white before encoding, because JPEG has no
/// alpha channel and white is the right background for a document. `dpi` is
/// recorded in the JFIF `APP0` density fields so that the datastream states its
/// own resolution; it does not change a single sample.
///
/// # Errors
///
/// - [`Error::InvalidParameter`] if `quality` is outside `1..=100` or `dpi` is
///   not a finite positive number.
/// - [`Error::ImageTooLarge`] if either dimension exceeds [`MAX_DIMENSION`].
/// - [`Error::UnsupportedFormat`] if the pixmap has no pixels.
pub fn encode(pixmap: &Pixmap, quality: u8, dpi: f32) -> Result<Vec<u8>> {
    if !(1..=100).contains(&quality) {
        return Err(Error::InvalidParameter { name: "quality" });
    }
    if !dpi.is_finite() || dpi <= 0.0 {
        return Err(Error::InvalidParameter { name: "dpi" });
    }
    if pixmap.is_empty() {
        return Err(Error::UnsupportedFormat);
    }
    let (width, height) = (pixmap.width(), pixmap.height());
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::ImageTooLarge { width, height });
    }

    let luma_table = quant::scale(&quant::LUMINANCE, quality);
    let chroma_table = quant::scale(&quant::CHROMINANCE, quality);
    let luma = Quantiser::new(&luma_table);
    let chroma = Quantiser::new(&chroma_table);
    let tables = Tables::typical();

    // A photograph at quality 85 lands near one byte per pixel; reserving a
    // quarter of that avoids most of the reallocation without over-committing
    // on a wasm heap.
    let mut out = Vec::with_capacity(pixmap.data().len() / 4 + 1024);

    out.extend_from_slice(&[0xFF, 0xD8]); // SOI
    write_jfif(&mut out, dpi);
    write_dqt(&mut out, 0, luma.dqt_bytes());
    write_dqt(&mut out, 1, chroma.dqt_bytes());
    write_sof0(&mut out, width as u16, height as u16);
    huffman::write_dht(&mut out, 0x00, &tables.dc_luma)?;
    huffman::write_dht(&mut out, 0x10, &tables.ac_luma)?;
    huffman::write_dht(&mut out, 0x01, &tables.dc_chroma)?;
    huffman::write_dht(&mut out, 0x11, &tables.ac_chroma)?;
    write_sos(&mut out);
    write_scan(&mut out, pixmap, &luma, &chroma, &tables);
    out.extend_from_slice(&[0xFF, 0xD9]); // EOI

    Ok(out)
}

/// The JFIF `APP0` segment, carrying the resolution and nothing else.
///
/// There is no thumbnail and no other application segment. ADR-0002's metadata
/// hygiene is structural on this path — we re-encode from pixels, so there is
/// no code that *could* copy an input's `APP1` across — and this is the one
/// place a careless addition would break it.
fn write_jfif(out: &mut Vec<u8>, dpi: f32) {
    // Density is a 16-bit field; clamp rather than wrap on an absurd DPI.
    let density = (dpi + 0.5).floor().clamp(1.0, 65535.0) as u16;
    out.extend_from_slice(&[0xFF, 0xE0]);
    out.extend_from_slice(&16u16.to_be_bytes());
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[0x01, 0x02]); // version 1.02
    out.push(0x01); // density units: dots per inch
    out.extend_from_slice(&density.to_be_bytes());
    out.extend_from_slice(&density.to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00]); // no embedded thumbnail
}

/// A `DQT` segment for one 8-bit quantisation table.
fn write_dqt(out: &mut Vec<u8>, id: u8, table: &[u8; 64]) {
    out.extend_from_slice(&[0xFF, 0xDB]);
    out.extend_from_slice(&67u16.to_be_bytes());
    out.push(id); // Pq = 0 (8-bit values), Tq = id
    out.extend_from_slice(table);
}

/// The `SOF0` frame header: baseline, 8-bit, three components, 4:4:4.
fn write_sof0(out: &mut Vec<u8>, width: u16, height: u16) {
    out.extend_from_slice(&[0xFF, 0xC0]);
    out.extend_from_slice(&17u16.to_be_bytes());
    out.push(8); // sample precision
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.push(3); // components
    for (index, &id) in COMPONENT_IDS.iter().enumerate() {
        out.push(id);
        out.push(0x11); // sampling factors 1x1: no subsampling
        out.push(u8::from(index > 0)); // quantisation table 0 for Y, 1 for chroma
    }
}

/// The `SOS` scan header: all three components, one interleaved scan.
fn write_sos(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&12u16.to_be_bytes());
    out.push(3); // components in this scan
    for (index, &id) in COMPONENT_IDS.iter().enumerate() {
        out.push(id);
        // DC table in the high nibble, AC table in the low nibble.
        out.push(if index == 0 { 0x00 } else { 0x11 });
    }
    out.push(0); // Ss: first coefficient
    out.push(63); // Se: last coefficient
    out.push(0); // Ah/Al: no successive approximation in a baseline scan
}

/// Entropy-code every minimum coded unit.
///
/// With 4:4:4 an MCU is one 8x8 block of each component, so the loop is simply
/// "for every 8x8 tile of the image, in reading order". Samples are pulled
/// straight out of the RGBA buffer a block at a time rather than through three
/// full-size planes, which keeps peak memory at the pixmap plus the output —
/// the budget ADR-0003 works to for large documents.
fn write_scan(
    out: &mut Vec<u8>,
    pixmap: &Pixmap,
    luma: &Quantiser,
    chroma: &Quantiser,
    tables: &Tables,
) {
    let blocks_x = pixmap.width().div_ceil(8);
    let blocks_y = pixmap.height().div_ceil(8);
    let mut writer = BitWriter::new(out);
    let mut previous_dc = [0i32; 3];
    let mut planes = [[0.0f32; 64]; 3];
    let mut quantised = [0i32; 64];

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            gather(pixmap, block_x, block_y, &mut planes);
            for component in 0..3usize {
                let (quantiser, dc_table, ac_table) = if component == 0 {
                    (luma, &tables.dc_luma, &tables.ac_luma)
                } else {
                    (chroma, &tables.dc_chroma, &tables.ac_chroma)
                };
                dct::forward(&mut planes[component]);
                quantiser.quantise(&planes[component], &mut quantised);
                encode_block(
                    &mut writer,
                    &quantised,
                    &mut previous_dc[component],
                    dc_table,
                    ac_table,
                );
            }
        }
    }
    writer.finish();
}

/// Read one 8x8 tile out of the pixmap as level-shifted Y, Cb and Cr.
///
/// Tiles that hang off the right or bottom edge are filled by replicating the
/// last real column and row, which is the conventional padding and leaves no
/// edge artefact at the boundary.
fn gather(pixmap: &Pixmap, block_x: u32, block_y: u32, planes: &mut [Block; 3]) {
    let max_x = pixmap.width() - 1;
    let max_y = pixmap.height() - 1;
    for row in 0..8u32 {
        let source_y = (block_y * 8 + row).min(max_y);
        for col in 0..8u32 {
            let source_x = (block_x * 8 + col).min(max_x);
            // In range by construction: both coordinates are clamped above.
            let [r, g, b, a] = pixmap.pixel(source_x, source_y).unwrap_or([0, 0, 0, 255]);
            // JPEG has no alpha, so composite over opaque white.
            let alpha = f32::from(a) / 255.0;
            let inverse = 255.0 * (1.0 - alpha);
            let r = f32::from(r) * alpha + inverse;
            let g = f32::from(g) * alpha + inverse;
            let b = f32::from(b) * alpha + inverse;

            // JFIF's RGB to YCbCr conversion, ITU-R BT.601 coefficients. The
            // 128 offsets of Cb and Cr cancel against the level shift the DCT
            // wants, so they appear in neither expression.
            let index = (row * 8 + col) as usize;
            planes[0][index] = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
            planes[1][index] = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
            planes[2][index] = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
        }
    }
}

/// Entropy-code one quantised block, T.81 §F.1.2.
fn encode_block(
    writer: &mut BitWriter<'_>,
    block: &[i32; 64],
    previous_dc: &mut i32,
    dc_table: &Table,
    ac_table: &Table,
) {
    // DC: the difference from the previous block of the same component.
    let difference = block[0] - *previous_dc;
    *previous_dc = block[0];
    let (size, bits) = category(difference);
    writer.write_code(dc_table.code(size));
    writer.write(bits, u32::from(size));

    // AC: run-length of zeros, then the next non-zero coefficient, in zigzag
    // order. A run of sixteen or more is broken up with the ZRL symbol 0xF0,
    // and a tail of zeros is replaced by the end-of-block symbol 0x00.
    let mut run = 0u8;
    for &natural in &quant::ZIGZAG[1..] {
        let value = block[natural];
        if value == 0 {
            run += 1;
            continue;
        }
        while run >= 16 {
            writer.write_code(ac_table.code(0xF0));
            run -= 16;
        }
        let (size, bits) = category(value);
        writer.write_code(ac_table.code((run << 4) | size));
        writer.write(bits, u32::from(size));
        run = 0;
    }
    if run > 0 {
        writer.write_code(ac_table.code(0x00));
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_DIMENSION, encode, info};
    use crate::pixmap::Pixmap;
    use crate::{Error, test_support};

    #[test]
    fn the_output_is_a_well_formed_baseline_datastream() {
        let pixmap = test_support::gradient(37, 23);
        let jpeg = encode(&pixmap, 85, 200.0).expect("encoding a 37x23 gradient");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "no SOI");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "no EOI");
        let parsed = info::parse(&jpeg).expect("our own output must parse");
        assert_eq!(parsed.width, 37);
        assert_eq!(parsed.height, 23);
        assert_eq!(parsed.components, 3);
        assert_eq!(parsed.precision, 8);
        assert_eq!(parsed.process, info::Process::Baseline);
        assert_eq!(parsed.adobe_transform, None);
    }

    #[test]
    fn the_datastream_carries_a_jfif_segment_and_no_other_application_segment() {
        let jpeg = encode(&test_support::gradient(16, 16), 85, 300.0).expect("encodes");
        assert_eq!(&jpeg[2..4], &[0xFF, 0xE0], "APP0 should come first");
        assert_eq!(&jpeg[6..11], b"JFIF\0");
        // Density 300 dpi, units = 1 (inches).
        assert_eq!(jpeg[13], 0x01);
        assert_eq!(u16::from_be_bytes([jpeg[14], jpeg[15]]), 300);
        assert_eq!(u16::from_be_bytes([jpeg[16], jpeg[17]]), 300);
        // No APP1 anywhere in the header, which is where EXIF would live.
        let header_end = jpeg
            .windows(2)
            .position(|w| w == [0xFF, 0xDA])
            .expect("there is an SOS");
        assert!(
            !jpeg[..header_end].windows(2).any(|w| w == [0xFF, 0xE1]),
            "an APP1 segment appeared in our own output"
        );
    }

    #[test]
    fn encoding_is_byte_identical_across_runs() {
        let pixmap = test_support::gradient(64, 48);
        let first = encode(&pixmap, 85, 200.0).expect("encodes");
        let second = encode(&pixmap, 85, 200.0).expect("encodes");
        assert_eq!(first, second);
    }

    #[test]
    fn quality_changes_the_size_monotonically() {
        let pixmap = test_support::gradient(96, 96);
        let mut previous = 0;
        for quality in [20u8, 50, 75, 85, 95] {
            let size = encode(&pixmap, quality, 200.0).expect("encodes").len();
            assert!(
                size > previous,
                "quality {quality} produced {size} bytes, not more than {previous}"
            );
            previous = size;
        }
    }

    #[test]
    fn parameters_are_validated() {
        let pixmap = test_support::gradient(8, 8);
        assert_eq!(
            encode(&pixmap, 0, 200.0),
            Err(Error::InvalidParameter { name: "quality" })
        );
        assert_eq!(
            encode(&pixmap, 101, 200.0),
            Err(Error::InvalidParameter { name: "quality" })
        );
        assert_eq!(
            encode(&pixmap, 85, 0.0),
            Err(Error::InvalidParameter { name: "dpi" })
        );
        assert_eq!(
            encode(&pixmap, 85, f32::NAN),
            Err(Error::InvalidParameter { name: "dpi" })
        );
        assert_eq!(
            encode(&Pixmap::empty(), 85, 200.0),
            Err(Error::UnsupportedFormat)
        );
    }

    #[test]
    fn a_frame_too_wide_for_the_header_is_refused() {
        // One pixel past what SOF can express, kept to one row so the
        // allocation stays small.
        let width = MAX_DIMENSION + 1;
        let pixmap = Pixmap::new(width, 1).expect("65536x1 is within the pixel budget");
        assert_eq!(
            encode(&pixmap, 85, 200.0),
            Err(Error::ImageTooLarge { width, height: 1 })
        );
    }

    #[test]
    fn sizes_that_are_not_multiples_of_eight_still_encode() {
        for (width, height) in [(1u32, 1u32), (1, 9), (9, 1), (7, 7), (8, 8), (17, 33)] {
            let pixmap = test_support::gradient(width, height);
            let jpeg = encode(&pixmap, 85, 200.0)
                .unwrap_or_else(|e| panic!("{width}x{height} failed: {e}"));
            let parsed = info::parse(&jpeg).expect("parses");
            assert_eq!(
                (u32::from(parsed.width), u32::from(parsed.height)),
                (width, height)
            );
        }
    }
}
