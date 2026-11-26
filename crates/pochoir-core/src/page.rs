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

/// How big the page for a bitmap input is allowed to get.
///
/// ADR-0006 supersedes ADR-0002 on this point for image inputs only. A page
/// sized purely by the chosen DPI is unbounded: a 12-megapixel phone photo at
/// 200 dpi is 512 by 384 millimetres, larger than A2, which reads as a mistake
/// in a viewer's document properties, tiles across four sheets if anybody
/// prints it at actual size, and is the kind of thing an upload portal that
/// validates page size rejects without saying why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSizing {
    /// Natural size at the effective resolution, shrunk uniformly to fit an A4
    /// sheet when it would otherwise be larger. **Never enlarged.** The
    /// default.
    #[default]
    CapToA4,
    /// Natural size at the effective resolution, however large that is.
    ///
    /// The opt-out, for somebody who genuinely wants the page to be the
    /// physical size the resolution implies.
    Native,
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

    /// The shorter edge of an A4 sheet, 210 mm, in points.
    ///
    /// Written as the division rather than as `595.2756` so that the number a
    /// reader can check — the ISO 216 millimetre figure — is the one in the
    /// source. Both operations are exactly specified in IEEE-754, so this is
    /// identical on wasm32 and native.
    pub const A4_SHORT_PT: f32 = 210.0 / 25.4 * 72.0;

    /// The longer edge of an A4 sheet, 297 mm, in points.
    pub const A4_LONG_PT: f32 = 297.0 / 25.4 * 72.0;

    /// Shrink uniformly until the page fits an A4 sheet, if it does not already.
    ///
    /// The sheet is turned to match the page, so a portrait image is capped
    /// against portrait A4 and a landscape one against landscape A4; a square
    /// page is bounded by the short edge either way, so the choice does not
    /// matter and portrait is used.
    ///
    /// **This only ever shrinks.** Enlarging to fill the sheet would be the
    /// same mistake in the other direction: a 600 dpi scan of an identity card
    /// is 85.6 by 54 millimetres, and that is its true size, not a page that
    /// needs filling. A page already inside A4 is returned untouched, and so is
    /// a degenerate one, which has no meaningful scale factor.
    #[must_use]
    pub fn capped_to_a4(self) -> Self {
        if !self.is_valid() {
            return self;
        }
        let (limit_width, limit_height) = if self.width_pt > self.height_pt {
            (Self::A4_LONG_PT, Self::A4_SHORT_PT)
        } else {
            (Self::A4_SHORT_PT, Self::A4_LONG_PT)
        };
        let scale = (limit_width / self.width_pt)
            .min(limit_height / self.height_pt)
            .min(1.0);
        Self {
            width_pt: self.width_pt * scale,
            height_pt: self.height_pt * scale,
        }
    }

    /// Apply a [`PageSizing`] to a natural page size.
    #[must_use]
    pub fn sized(self, sizing: PageSizing) -> Self {
        match sizing {
            PageSizing::CapToA4 => self.capped_to_a4(),
            PageSizing::Native => self,
        }
    }

    /// The resolution this page actually prints at, horizontally, given the
    /// pixel width of the image on it.
    ///
    /// Not the same as the DPI the caller asked for once the page has been
    /// capped: the pixels are unchanged and the sheet is smaller, so they are
    /// packed more densely. A 12-megapixel photo requested at 200 dpi prints at
    /// about 366. This is the honest number, and it is what goes into the
    /// embedded JPEG's own density field.
    #[must_use]
    pub fn print_dpi(&self, width_px: u32) -> Option<f32> {
        if !self.is_valid() || width_px == 0 {
            return None;
        }
        let inches = self.width_pt / 72.0;
        let dpi = width_px as f32 / inches;
        (dpi.is_finite() && dpi > 0.0).then_some(dpi)
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
    use super::{PageGeometry, PageSizing};

    /// Millimetres, for assertions that read like the spec they come from.
    fn mm(geometry: PageGeometry) -> (f64, f64) {
        (
            f64::from(geometry.width_pt) / 72.0 * 25.4,
            f64::from(geometry.height_pt) / 72.0 * 25.4,
        )
    }

    fn close(actual: (f64, f64), expected: (f64, f64), tolerance: f64) -> bool {
        (actual.0 - expected.0).abs() < tolerance && (actual.1 - expected.1).abs() < tolerance
    }

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

    #[test]
    fn the_a4_constants_are_iso_216() {
        assert!((f64::from(PageGeometry::A4_SHORT_PT) - 595.275_590_5).abs() < 1e-3);
        assert!((f64::from(PageGeometry::A4_LONG_PT) - 841.889_763_8).abs() < 1e-3);
    }

    #[test]
    fn a_twelve_megapixel_landscape_photo_is_capped_to_landscape_a4() {
        let natural = PageGeometry::from_pixels(4032, 3024, 200.0, 200.0);
        assert!(close(mm(natural), (512.1, 384.0), 0.1), "{:?}", mm(natural));
        let capped = natural.capped_to_a4();
        assert!(close(mm(capped), (280.0, 210.0), 0.1), "{:?}", mm(capped));
        // The short edge is exactly A4's, and the aspect ratio is untouched.
        assert!((capped.height_pt - PageGeometry::A4_SHORT_PT).abs() < 0.01);
        assert!(
            (f64::from(natural.width_pt / natural.height_pt)
                - f64::from(capped.width_pt / capped.height_pt))
            .abs()
                < 1e-5
        );
    }

    #[test]
    fn a_twelve_megapixel_portrait_photo_is_capped_to_portrait_a4() {
        let capped = PageGeometry::from_pixels(3024, 4032, 200.0, 200.0).capped_to_a4();
        assert!(close(mm(capped), (210.0, 280.0), 0.1), "{:?}", mm(capped));
        assert!((capped.width_pt - PageGeometry::A4_SHORT_PT).abs() < 0.01);
    }

    #[test]
    fn a_square_image_is_bounded_by_the_short_edge() {
        let capped = PageGeometry::from_pixels(3000, 3000, 200.0, 200.0).capped_to_a4();
        assert!(close(mm(capped), (210.0, 210.0), 0.1), "{:?}", mm(capped));
        assert_eq!(capped.width_pt, capped.height_pt);
    }

    #[test]
    fn a_page_already_inside_a4_is_returned_untouched() {
        // A 600 dpi scan of an identity card: 85.6 by 54 mm, its true size.
        let natural = PageGeometry::from_pixels(2022, 1276, 600.0, 600.0);
        assert!(close(mm(natural), (85.6, 54.0), 0.1), "{:?}", mm(natural));
        assert_eq!(
            natural.capped_to_a4(),
            natural,
            "the cap enlarged a page it should have left alone"
        );
    }

    #[test]
    fn a_six_hundred_dpi_a4_scan_stays_a4() {
        // 210 mm at 600 dpi is 4960.6 px, so a real scanner writes 4961 and the
        // page comes out a hair over A4. The cap takes that hair off and
        // nothing more.
        let natural = PageGeometry::from_pixels(4961, 7016, 600.0, 600.0);
        let capped = natural.capped_to_a4();
        assert!(close(mm(capped), (210.0, 297.0), 0.05), "{:?}", mm(capped));
        let shrink = f64::from(capped.width_pt / natural.width_pt);
        assert!(shrink > 0.9999, "shrank by {:.4}%", (1.0 - shrink) * 100.0);
    }

    #[test]
    fn nothing_is_ever_enlarged() {
        for (w, h, dpi) in [
            (10u32, 10u32, 600.0f32),
            (100, 2000, 600.0),
            (2000, 100, 600.0),
            (1, 1, 72.0),
        ] {
            let natural = PageGeometry::from_pixels(w, h, dpi, dpi);
            let capped = natural.capped_to_a4();
            assert!(
                capped.width_pt <= natural.width_pt + 1e-3
                    && capped.height_pt <= natural.height_pt + 1e-3,
                "{w}x{h} at {dpi} grew from {natural:?} to {capped:?}"
            );
        }
    }

    #[test]
    fn a_very_long_thin_page_is_capped_by_its_long_edge() {
        // 297 mm is the binding limit here, not 210.
        let natural = PageGeometry::from_pixels(400, 8000, 200.0, 200.0);
        let capped = natural.capped_to_a4();
        // 20:1, so the long edge binds and the short edge ends up far inside
        // A4's: the sheet is a bound, not a target.
        assert!(close(mm(capped), (14.85, 297.0), 0.1), "{:?}", mm(capped));
        assert!(
            (capped.height_pt - PageGeometry::A4_LONG_PT).abs() < 0.01,
            "height was {}",
            capped.height_pt
        );
        assert!(capped.width_pt < PageGeometry::A4_SHORT_PT);
    }

    #[test]
    fn the_sizing_mode_selects_between_them() {
        let natural = PageGeometry::from_pixels(4032, 3024, 200.0, 200.0);
        assert_eq!(natural.sized(PageSizing::Native), natural);
        assert_eq!(natural.sized(PageSizing::CapToA4), natural.capped_to_a4());
        assert_eq!(PageSizing::default(), PageSizing::CapToA4);
    }

    #[test]
    fn a_degenerate_page_is_left_alone_rather_than_scaled_by_a_nonsense_factor() {
        for geometry in [
            PageGeometry {
                width_pt: 0.0,
                height_pt: 100.0,
            },
            PageGeometry {
                width_pt: f32::NAN,
                height_pt: 100.0,
            },
            PageGeometry {
                width_pt: f32::INFINITY,
                height_pt: 100.0,
            },
        ] {
            let capped = geometry.capped_to_a4();
            assert_eq!(
                capped.width_pt.is_nan(),
                geometry.width_pt.is_nan(),
                "{geometry:?}"
            );
            if !geometry.width_pt.is_nan() {
                assert_eq!(capped, geometry, "{geometry:?}");
            }
        }
    }

    #[test]
    fn the_print_resolution_rises_when_the_page_is_capped() {
        let natural = PageGeometry::from_pixels(4032, 3024, 200.0, 200.0);
        let requested = natural.print_dpi(4032).expect("valid page");
        assert!((requested - 200.0).abs() < 0.01, "{requested}");
        let capped = natural.capped_to_a4().print_dpi(4032).expect("valid page");
        assert!(
            (capped - 365.8).abs() < 0.5,
            "capped page prints at {capped} dpi, expected about 366"
        );
    }

    #[test]
    fn the_print_resolution_of_a_degenerate_page_is_nothing() {
        assert_eq!(
            PageGeometry {
                width_pt: 0.0,
                height_pt: 10.0
            }
            .print_dpi(100),
            None
        );
        assert_eq!(
            PageGeometry::from_pixels(10, 10, 72.0, 72.0).print_dpi(0),
            None
        );
    }
}
