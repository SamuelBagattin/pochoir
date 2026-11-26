//! Inputs built in code, never committed as binary blobs.
//!
//! ADR-0002's test strategy calls for "inputs generated procedurally in the
//! test, not binary fixtures committed to git: a synthetic document of known
//! rectangles, gradients and glyph-like shapes. Keeps the repository small and
//! makes every input auditable as code."
//!
//! The EXIF builder below is the part that matters most. A test that a GPS-
//! tagged photograph produces a PDF with no GPS in it is only worth something
//! if the input really does carry GPS, and the only way to be sure of that
//! without committing somebody's real holiday snap is to lay out the TIFF
//! structure here, byte by byte, where a reader can check it.

#![allow(dead_code)]

use pochoir_core::Pixmap;
use pochoir_core::exif::Orientation;

/// Latitude of a Paris address, as three EXIF rationals: 48 degrees,
/// 51 minutes, 29.6 seconds.
pub const GPS_LATITUDE: [(u32, u32); 3] = [(48, 1), (51, 1), (296, 10)];
/// The matching longitude: 2 degrees, 17 minutes, 40.5 seconds east.
pub const GPS_LONGITUDE: [(u32, u32); 3] = [(2, 1), (17, 1), (405, 10)];
/// A camera model string, standing in for `Make`.
pub const CAMERA_MAKE: &[u8; 4] = b"CAM\0";

/// Four flat-coloured quadrants, so that a geometric transform can be checked
/// by looking at which corner each colour ended up in.
///
/// Flat areas also survive JPEG quantisation nearly intact, which is what lets
/// an orientation be asserted end to end through a lossy codec.
#[must_use]
pub fn quadrants(width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("fixtures are small");
    for y in 0..height {
        for x in 0..width {
            let right = x >= width / 2;
            let bottom = y >= height / 2;
            let colour = match (right, bottom) {
                (false, false) => [220, 30, 30, 255],
                (true, false) => [30, 180, 30, 255],
                (false, true) => [30, 30, 220, 255],
                (true, true) => [230, 210, 40, 255],
            };
            pixmap.set_pixel(x, y, colour);
        }
    }
    pixmap
}

/// A two-axis ramp crossed by a darker grid: smooth gradients for the DCT and
/// hard edges to catch ringing.
#[must_use]
pub fn gradient(width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("fixtures are small");
    for y in 0..height {
        for x in 0..width {
            let on_grid = x % 8 == 0 || y % 8 == 0;
            let shade = if on_grid { 2 } else { 1 };
            let r = ((x * 255) / width.max(1)) as u8 / shade;
            let g = ((y * 255) / height.max(1)) as u8 / shade;
            let b = (((x + y) * 255) / (width + height).max(1)) as u8 / shade;
            pixmap.set_pixel(x, y, [r, g, b, 0xFF]);
        }
    }
    pixmap
}

/// The colour at the centre of each quadrant of a pixmap, clockwise from the
/// top left.
#[must_use]
pub fn quadrant_colours(pixmap: &Pixmap) -> [[u8; 3]; 4] {
    let (w, h) = (pixmap.width(), pixmap.height());
    let sample = |x: u32, y: u32| {
        let p = pixmap.pixel(x, y).expect("in range");
        [p[0], p[1], p[2]]
    };
    [
        sample(w / 4, h / 4),
        sample(w * 3 / 4, h / 4),
        sample(w * 3 / 4, h * 3 / 4),
        sample(w / 4, h * 3 / 4),
    ]
}

/// `true` when two colours are within `tolerance` on every channel.
#[must_use]
pub fn close(a: [u8; 3], b: [u8; 3], tolerance: i32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(&x, &y)| (i32::from(x) - i32::from(y)).abs() <= tolerance)
}

/// Encode a pixmap as a JPEG with our own encoder.
#[must_use]
pub fn jpeg(pixmap: &Pixmap, quality: u8) -> Vec<u8> {
    pochoir_core::jpeg::encode(pixmap, quality, 72.0).expect("fixtures encode")
}

/// Splice an `APP1` Exif segment into a JPEG, right after `SOI`.
///
/// # Panics
///
/// Panics if `jpeg` does not start with `SOI`, which would mean the fixture
/// helpers themselves are broken.
#[must_use]
pub fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "fixture is not a JPEG");
    let mut payload = Vec::from(&b"Exif\0\0"[..]);
    payload.extend_from_slice(tiff);
    let mut out = Vec::with_capacity(jpeg.len() + payload.len() + 4);
    out.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// A little-endian TIFF block for IFD0, laid out at fixed offsets.
///
/// Six IFD0 entries: `Orientation`, `XResolution`, `YResolution`,
/// `ResolutionUnit`, `Make`, and a pointer to a GPS IFD carrying a real
/// latitude and longitude. Offsets are computed here rather than searched for,
/// so the layout is visible in one place:
///
/// ```text
///   0  "II", 42, offset of IFD0 = 8
///   8  entry count = 6, then 6 * 12 bytes of entries, then next-IFD = 0
///  86  GPS IFD: count = 4, then 4 * 12 bytes, then next-IFD = 0
/// 140  GPS latitude, three rationals
/// 164  GPS longitude, three rationals
/// 188  XResolution, then YResolution
/// ```
#[must_use]
pub fn exif_block(orientation: Orientation, dpi: u32, with_gps: bool) -> Vec<u8> {
    const IFD0_ENTRIES: u32 = 6;
    const GPS_IFD: u32 = 14 + 12 * IFD0_ENTRIES;
    const GPS_ENTRIES: u32 = 4;
    const GPS_LAT: u32 = GPS_IFD + 6 + 12 * GPS_ENTRIES;
    const GPS_LON: u32 = GPS_LAT + 24;
    const RESOLUTION: u32 = GPS_LON + 24;

    let mut out = Vec::new();
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());

    out.extend_from_slice(&(IFD0_ENTRIES as u16).to_le_bytes());
    push_entry(&mut out, 0x0112, 3, 1, u32::from(orientation.to_tag()));
    push_entry(&mut out, 0x011A, 5, 1, RESOLUTION);
    push_entry(&mut out, 0x011B, 5, 1, RESOLUTION + 8);
    push_entry(&mut out, 0x0128, 3, 1, 2); // ResolutionUnit: inches
    push_entry(&mut out, 0x010F, 2, 4, u32::from_le_bytes(*CAMERA_MAKE));
    push_entry(&mut out, 0x8825, 4, 1, GPS_IFD);
    out.extend_from_slice(&0u32.to_le_bytes()); // no IFD1
    assert_eq!(out.len() as u32, GPS_IFD, "IFD0 is not the expected size");

    out.extend_from_slice(&(GPS_ENTRIES as u16).to_le_bytes());
    push_entry(&mut out, 0x0001, 2, 2, u32::from_le_bytes(*b"N\0\0\0"));
    push_entry(&mut out, 0x0002, 5, 3, GPS_LAT);
    push_entry(&mut out, 0x0003, 2, 2, u32::from_le_bytes(*b"E\0\0\0"));
    push_entry(&mut out, 0x0004, 5, 3, GPS_LON);
    out.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        out.len() as u32,
        GPS_LAT,
        "GPS IFD is not the expected size"
    );

    for &(numerator, denominator) in &GPS_LATITUDE {
        push_rational(&mut out, numerator, denominator);
    }
    for &(numerator, denominator) in &GPS_LONGITUDE {
        push_rational(&mut out, numerator, denominator);
    }
    assert_eq!(
        out.len() as u32,
        RESOLUTION,
        "GPS data is not where expected"
    );
    push_rational(&mut out, dpi, 1);
    push_rational(&mut out, dpi, 1);

    if !with_gps {
        // Rewrite the GPS pointer to something harmless and blank the data, so
        // the only difference from the tagged fixture is the GPS itself.
        let gps_pointer_at = 10 + 5 * 12 + 8;
        out[gps_pointer_at..gps_pointer_at + 4].copy_from_slice(&0u32.to_le_bytes());
        let lat = GPS_LAT as usize;
        out[lat..lat + 48].fill(0);
    }
    out
}

/// The exact bytes a GPS coordinate occupies in [`exif_block`], for a test that
/// searches an output file for them.
#[must_use]
pub fn gps_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    for &(numerator, denominator) in &GPS_LATITUDE {
        push_rational(&mut out, numerator, denominator);
    }
    for &(numerator, denominator) in &GPS_LONGITUDE {
        push_rational(&mut out, numerator, denominator);
    }
    out
}

fn push_entry(out: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_rational(out: &mut Vec<u8>, numerator: u32, denominator: u32) {
    out.extend_from_slice(&numerator.to_le_bytes());
    out.extend_from_slice(&denominator.to_le_bytes());
}

/// Encode a pixmap as a PNG, optionally stating a resolution and a text chunk.
///
/// The text chunk exists so that a test can prove it does not survive into the
/// output: ADR-0002 names `tEXt` among the PNG chunks we drop.
#[must_use]
pub fn png(pixmap: &Pixmap, pixels_per_metre: Option<u32>, text: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some(ppu) = pixels_per_metre {
            encoder.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppu,
                yppu: ppu,
                unit: png::Unit::Meter,
            }));
        }
        if let Some(text) = text {
            encoder
                .add_text_chunk("Comment".to_string(), text.to_string())
                .expect("a tEXt chunk");
        }
        let mut writer = encoder.write_header().expect("PNG header");
        writer
            .write_image_data(pixmap.data())
            .expect("PNG image data");
        writer.finish().expect("PNG trailer");
    }
    out
}

/// Decode a JPEG with `zune-jpeg` directly, as an independent reader.
///
/// # Panics
///
/// Panics if the datastream does not decode, which is the point: our encoder's
/// output must be readable by the decoder we ship.
#[must_use]
pub fn decode_with_zune(jpeg: &[u8]) -> (u32, u32, Vec<u8>) {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let pixels = decoder.decode().expect("our own JPEG must decode");
    let info = decoder.info().expect("headers were decoded");
    (u32::from(info.width), u32::from(info.height), pixels)
}
