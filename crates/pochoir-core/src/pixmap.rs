//! The one pixel buffer every stage of the pipeline agrees on.
//!
//! RGBA8, four bytes per pixel, rows top to bottom, no padding and no stride:
//! row `y` starts at `y * width * 4`. There is deliberately no colour-space
//! field — the pipeline treats these samples as sRGB throughout, which is what
//! both decoders hand us and what the JPEG encoder assumes.
//!
//! The buffer is caller-owned so that a multi-page job can rasterise every page
//! into the same allocation, which is the memory model
//! `docs/adr/0003-pdf-input-rasterisation.md` requires of large documents:
//! rasterise into one reused buffer, encode, append, move on.

use crate::{Error, Result};

/// Largest image we will allocate a buffer for, in pixels.
///
/// 256 megapixels is far above any camera or scanner a user will point at this
/// tool, and far below the point where `width * height * 4` could overflow a
/// 32-bit `usize` on the wasm32 target. Rejecting early turns a hostile header
/// claiming 4 billion pixels into an error rather than an allocation failure.
pub const MAX_PIXELS: u64 = 256 * 1024 * 1024;

/// An RGBA8 pixel buffer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pixmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Pixmap {
    /// A buffer with no pixels and no allocation, ready to be [`resize`]d.
    ///
    /// [`resize`]: Pixmap::resize
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            data: Vec::new(),
        }
    }

    /// Allocate a zeroed buffer of `width` by `height` pixels.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] if the pixel count exceeds
    /// [`MAX_PIXELS`].
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let mut pixmap = Self::empty();
        pixmap.resize(width, height)?;
        Ok(pixmap)
    }

    /// Wrap an existing RGBA8 buffer.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] if the pixel count exceeds
    /// [`MAX_PIXELS`], or [`Error::BufferSize`] if `data` is not exactly
    /// `width * height * 4` bytes long.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = byte_len(width, height)?;
        if data.len() != expected {
            return Err(Error::BufferSize {
                expected,
                got: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Resize in place, reusing the existing allocation where it is big enough.
    ///
    /// The contents afterwards are zeroed rather than preserved: this exists to
    /// recycle memory between pages, not to scale an image.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] if the pixel count exceeds
    /// [`MAX_PIXELS`].
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let len = byte_len(width, height)?;
        self.data.clear();
        self.data.resize(len, 0);
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// `true` when the buffer holds no pixels at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The RGBA8 samples, row-major, top row first.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The RGBA8 samples, mutably.
    #[must_use]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Consume the buffer and return its samples.
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// The four samples at `(x, y)`, or `None` if either is out of range.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let offset = self.offset(x, y)?;
        Some([
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ])
    }

    /// Write the four samples at `(x, y)`. Out-of-range writes are ignored.
    pub fn set_pixel(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if let Some(offset) = self.offset(x, y) {
            self.data[offset..offset + 4].copy_from_slice(&rgba);
        }
    }

    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        // `byte_len` already proved that width * height * 4 fits in a usize,
        // so this cannot overflow for in-range coordinates.
        Some((y as usize * self.width as usize + x as usize) * 4)
    }
}

/// Byte length of a `width` by `height` RGBA8 buffer, with the size guard.
fn byte_len(width: u32, height: u32) -> Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(Error::ImageTooLarge { width, height });
    }
    // pixels <= 2^28, so pixels * 4 <= 2^30 and fits even a 32-bit usize.
    usize::try_from(pixels * 4).map_err(|_| Error::ImageTooLarge { width, height })
}

#[cfg(test)]
mod tests {
    use super::{MAX_PIXELS, Pixmap};
    use crate::Error;

    #[test]
    fn new_allocates_a_zeroed_buffer() {
        let pixmap = Pixmap::new(3, 2).expect("3x2 is a legal size");
        assert_eq!(pixmap.width(), 3);
        assert_eq!(pixmap.height(), 2);
        assert_eq!(pixmap.data().len(), 3 * 2 * 4);
        assert!(pixmap.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn resize_reuses_the_allocation() {
        let mut pixmap = Pixmap::new(64, 64).expect("64x64 is a legal size");
        pixmap.data_mut()[0] = 0xff;
        pixmap.resize(8, 8).expect("8x8 is a legal size");
        assert_eq!(pixmap.data().len(), 8 * 8 * 4);
        assert_eq!(pixmap.data()[0], 0, "resize should zero the contents");
        pixmap.resize(128, 1).expect("128x1 is a legal size");
        assert_eq!(pixmap.data().len(), 128 * 4);
    }

    #[test]
    fn pixels_round_trip_through_accessors() {
        let mut pixmap = Pixmap::new(2, 2).expect("2x2 is a legal size");
        pixmap.set_pixel(1, 1, [1, 2, 3, 4]);
        assert_eq!(pixmap.pixel(1, 1), Some([1, 2, 3, 4]));
        assert_eq!(pixmap.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(pixmap.pixel(2, 0), None);
        assert_eq!(pixmap.pixel(0, 2), None);
    }

    #[test]
    fn an_oversized_image_is_refused_rather_than_allocated() {
        let side = 1 + (MAX_PIXELS as f64).sqrt() as u32;
        assert_eq!(
            Pixmap::new(side, side),
            Err(Error::ImageTooLarge {
                width: side,
                height: side
            })
        );
        assert_eq!(
            Pixmap::new(u32::MAX, u32::MAX),
            Err(Error::ImageTooLarge {
                width: u32::MAX,
                height: u32::MAX
            })
        );
    }

    #[test]
    fn from_rgba_checks_the_length() {
        assert!(Pixmap::from_rgba(2, 1, vec![0; 8]).is_ok());
        assert_eq!(
            Pixmap::from_rgba(2, 1, vec![0; 7]),
            Err(Error::BufferSize {
                expected: 8,
                got: 7
            })
        );
    }

    #[test]
    fn an_empty_pixmap_is_empty() {
        assert!(Pixmap::empty().is_empty());
        assert!(Pixmap::new(0, 10).expect("zero width is legal").is_empty());
    }
}
