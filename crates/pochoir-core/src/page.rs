//! The boundary between "where the pixels came from" and "what we do to them".
//!
//! [`PageSource`] is the trait specified in
//! `docs/adr/0003-pdf-input-rasterisation.md`. Its whole purpose is that the
//! watermark compositor and the PDF writer are written against it and never
//! against a decoder: the image implementation lives in [`crate::image`], and a
//! PDF implementation wrapping a renderer can arrive behind the same three
//! methods without the common path ever learning about it.

use crate::Error;
use crate::pixmap::Pixmap;

/// The error a [`PageSource`] can fail with.
///
/// ADR-0003 writes this name into the trait signature. It is an alias rather
/// than a second error enum on purpose: one error type for the whole crate
/// keeps the wasm boundary simple, and a `PageSource` has nothing to report
/// that [`Error`] cannot carry.
pub type RasterError = Error;

/// A page box in PostScript points, 72 to the inch.
///
/// This is the physical size of the output page, not a pixel count. For image
/// inputs it is the image's own aspect ratio sized so that it prints at the
/// effective DPI; for PDF inputs it will be the input page's `MediaBox`. Either
/// way ADR-0002 forbids scaling the two axes independently, so there is no code
/// anywhere that sets these two numbers from different sources.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    /// Page width in points.
    pub width_pt: f32,
    /// Page height in points.
    pub height_pt: f32,
}

impl PageGeometry {
    /// A page sized so that `width_px` by `height_px` pixels print at
    /// `dpi_x` by `dpi_y` dots per inch.
    ///
    /// Both DPI values are assumed finite and positive; [`crate::OutputOptions`]
    /// validates that before anything reaches here.
    #[must_use]
    pub fn from_pixels(width_px: u32, height_px: u32, dpi_x: f32, dpi_y: f32) -> Self {
        Self {
            width_pt: width_px as f32 / dpi_x * 72.0,
            height_pt: height_px as f32 / dpi_y * 72.0,
        }
    }

    /// `true` when both edges are finite and strictly positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.width_pt.is_finite()
            && self.height_pt.is_finite()
            && self.width_pt > 0.0
            && self.height_pt > 0.0
    }
}

/// A source of page bitmaps.
///
/// Implemented once for decoded images and, later, once for PDFs. The
/// compositor and the writer know only this.
pub trait PageSource {
    /// How many pages this source will produce.
    fn page_count(&self) -> usize;

    /// Page box in PostScript points, preserving the input's geometry.
    ///
    /// # Panics
    ///
    /// May panic if `index >= self.page_count()`. Callers iterate
    /// `0..page_count()`.
    fn page_geometry(&self, index: usize) -> PageGeometry;

    /// Rasterise one page into a caller-owned RGBA8 buffer.
    ///
    /// The implementation resizes `out` to the page's pixel dimensions, which is
    /// what lets one allocation be reused across every page of a document.
    /// `dpi` is the rasterisation resolution and is **ignored by sources that
    /// already hold pixels**: ADR-0002 forbids resampling an image input at all.
    ///
    /// # Errors
    ///
    /// Returns an error if the page cannot be produced, or if `index` is out of
    /// range.
    fn rasterise(&self, index: usize, dpi: f32, out: &mut Pixmap) -> Result<(), RasterError>;
}

#[cfg(test)]
mod tests {
    use super::PageGeometry;

    #[test]
    fn geometry_from_pixels_is_the_aspect_ratio_at_the_given_dpi() {
        // 600x300 px at 300 dpi is 2 inches by 1 inch, i.e. 144 by 72 points.
        let geometry = PageGeometry::from_pixels(600, 300, 300.0, 300.0);
        assert_eq!(geometry.width_pt, 144.0);
        assert_eq!(geometry.height_pt, 72.0);
        assert!(geometry.is_valid());
    }

    #[test]
    fn geometry_never_stretches_one_axis() {
        // Same DPI on both axes means the point ratio equals the pixel ratio.
        for (w, h) in [(4000_u32, 3000_u32), (3000, 4000), (2048, 2048)] {
            let geometry = PageGeometry::from_pixels(w, h, 200.0, 200.0);
            let pixel_ratio = f64::from(w) / f64::from(h);
            let point_ratio = f64::from(geometry.width_pt) / f64::from(geometry.height_pt);
            assert!(
                (pixel_ratio - point_ratio).abs() < 1e-6,
                "{w}x{h}: pixel ratio {pixel_ratio} but page ratio {point_ratio}"
            );
        }
    }

    #[test]
    fn a_degenerate_geometry_is_not_valid() {
        assert!(!PageGeometry::from_pixels(0, 10, 72.0, 72.0).is_valid());
        assert!(
            !PageGeometry {
                width_pt: f32::NAN,
                height_pt: 10.0,
            }
            .is_valid()
        );
    }
}
