//! Procedurally generated inputs for the unit tests.
//!
//! ADR-0002's test strategy asks for inputs "generated procedurally in the
//! test, not binary fixtures committed to git", so that the repository stays
//! small and every input is auditable as code. This module is the crate-private
//! half of that; `tests/fixtures/mod.rs` holds the richer builders the
//! integration tests need.

use crate::jpeg;
use crate::pixmap::Pixmap;

/// A deterministic test image: a two-axis colour ramp with a darker grid, which
/// gives the encoder both smooth gradients and hard edges to deal with.
pub fn gradient(width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("test images are small");
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

/// Encode a pixmap at the default quality, for tests that need JPEG bytes.
pub fn jpeg_of(pixmap: Pixmap) -> Vec<u8> {
    jpeg::encode(&pixmap, crate::DEFAULT_QUALITY, crate::DEFAULT_DPI).expect("encodes")
}

/// A JPEG consisting only of markers: `SOI`, an optional Adobe `APP14`, a frame
/// header and an empty `SOS`.
///
/// Enough for anything that reads the frame header and nothing more, which is
/// what the PDF writer does. It lets the writer's handling of greyscale and of
/// Adobe-marked CMYK be tested without a four-component encoder we have
/// deliberately not written.
pub fn jpeg_header(
    sof_marker: u8,
    precision: u8,
    width: u16,
    height: u16,
    components: u8,
    adobe_transform: Option<u8>,
) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    if let Some(transform) = adobe_transform {
        let mut payload = Vec::from(&b"Adobe"[..]);
        payload.extend_from_slice(&[0x00, 0x64, 0x00, 0x00, 0x00, 0x00, transform]);
        out.extend_from_slice(&[0xFF, 0xEE]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
    }
    let mut frame = vec![precision];
    frame.extend_from_slice(&height.to_be_bytes());
    frame.extend_from_slice(&width.to_be_bytes());
    frame.push(components);
    for id in 1..=components {
        frame.extend_from_slice(&[id, 0x11, 0]);
    }
    out.extend_from_slice(&[0xFF, sof_marker]);
    out.extend_from_slice(&((frame.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&frame);
    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A four-component baseline frame header, optionally Adobe-marked.
pub fn cmyk_jpeg_header(width: u16, height: u16, adobe_transform: Option<u8>) -> Vec<u8> {
    jpeg_header(0xC0, 8, width, height, 4, adobe_transform)
}

/// A single-component baseline frame header.
pub fn grey_jpeg_header(width: u16, height: u16) -> Vec<u8> {
    jpeg_header(0xC0, 8, width, height, 1, None)
}
