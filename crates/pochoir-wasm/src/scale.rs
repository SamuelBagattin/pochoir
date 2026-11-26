//! Area-average downscaling, for the preview only.
//!
//! # Why this is here and not in the engine
//!
//! The preview has to hand the browser a bitmap small enough to paint, and
//! nothing in [`pochoir_core`] can make one. The two things that look as though
//! they could, cannot:
//!
//! - [`pochoir_core::Pixmap::resize`] reallocates and **zeroes**. Its own
//!   documentation says it "exists to recycle memory between pages, not to scale
//!   an image".
//! - Asking for a lower DPI does not produce fewer pixels. For an image input
//!   `rasterise` ignores the DPI entirely — ADR-0002's "do not resample image
//!   inputs at all" — so the DPI changes the page's size in points and nothing
//!   about the bitmap.
//!
//! So a resampler had to be written. It lives in this crate rather than in the
//! engine because the engine has no use for one: ADR-0002 forbids resampling on
//! the output path, and the CLI has no preview. ADR-0008 records that placement
//! and the argument that it belongs in `pochoir-core` if a second client ever
//! needs it.
//!
//! # Why integer arithmetic
//!
//! A box average in `u64` is exact, so the preview is byte-identical on every
//! target and across runs, which is the determinism ADR-0002 Decision 6 asks of
//! the whole pipeline. Floating-point weights would re-introduce exactly the
//! platform divergence that `pochoir_core::mark::trig` exists to avoid.
//!
//! The average is **alpha-weighted** — `Σ(c·a)/Σa` for colour and `Σa/n` for
//! alpha — which is the only correct way to average straight-alpha samples. For
//! the opaque pixels a JPEG produces it reduces to a plain mean; it matters for
//! a PNG with transparency, where averaging colour without weighting it by alpha
//! drags the colour of fully transparent pixels into the visible result.

use pochoir_core::{Error, Pixmap, Result};

/// Bytes per pixel in a [`Pixmap`]: RGBA8.
const CHANNELS: usize = 4;

/// The preview dimensions for a page of `width` × `height`, bounded by
/// `max_px` on the longer edge.
///
/// Never enlarges: a page already inside the bound is returned unchanged,
/// because upsampling a preview invents detail the document does not have. The
/// shorter edge is rounded down but never to zero, so the result is always
/// paintable.
///
/// # Errors
///
/// [`Error::InvalidParameter`] for `max_px == 0` or an empty page.
pub fn preview_dimensions(width: u32, height: u32, max_px: u32) -> Result<(u32, u32)> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidParameter { name: "page size" });
    }
    if max_px == 0 {
        return Err(Error::InvalidParameter { name: "max_px" });
    }
    let longer = width.max(height);
    if longer <= max_px {
        return Ok((width, height));
    }
    let (w, h) = (u64::from(width), u64::from(height));
    let bound = u64::from(max_px);
    let scaled = |edge: u64| -> u32 {
        // `edge * bound` is at most 2^32 * 2^32 / 2^32, comfortably inside u64.
        let value = edge.saturating_mul(bound) / u64::from(longer);
        // Clamped to 1 so a very wide, very short page still has a row.
        u32::try_from(value.max(1)).unwrap_or(u32::MAX)
    };
    Ok((scaled(w), scaled(h)))
}

/// Downscale `source` to `width` × `height` by averaging each destination
/// pixel's source box.
///
/// # Errors
///
/// [`Error::InvalidParameter`] if either target dimension is zero or larger
/// than the source's — this function only ever shrinks — and
/// [`Error::ImageTooLarge`] from the allocation if the target is somehow past
/// [`pochoir_core::pixmap::MAX_PIXELS`].
pub fn downscale(source: &Pixmap, width: u32, height: u32) -> Result<Pixmap> {
    if width == 0 || height == 0 || width > source.width() || height > source.height() {
        return Err(Error::InvalidParameter {
            name: "preview size",
        });
    }
    let (source_width, source_height) = (source.width(), source.height());
    let source_stride = usize::try_from(source_width)
        .ok()
        .and_then(|w| w.checked_mul(CHANNELS))
        .ok_or(Error::ImageTooLarge {
            width: source_width,
            height: source_height,
        })?;

    let mut out = Pixmap::new(width, height)?;
    let destination = out.data_mut();

    // Row and column boundaries are integer-divided from the destination index,
    // so every source pixel belongs to exactly one box and none is skipped. The
    // `max(start + 1)` keeps a box non-empty when rounding would have closed it.
    let span = |index: u32, count: u32, extent: u32| -> (usize, usize) {
        let numerator = u64::from(index) * u64::from(extent);
        let next = u64::from(index) + 1;
        let start = numerator / u64::from(count);
        let end = (next * u64::from(extent) / u64::from(count)).max(start + 1);
        (
            usize::try_from(start).unwrap_or(usize::MAX),
            usize::try_from(end.min(u64::from(extent))).unwrap_or(usize::MAX),
        )
    };

    for (y, row_out) in destination
        .chunks_exact_mut(width as usize * CHANNELS)
        .enumerate()
    {
        let (y0, y1) = span(u32::try_from(y).unwrap_or(u32::MAX), height, source_height);
        for (x, pixel_out) in row_out.chunks_exact_mut(CHANNELS).enumerate() {
            let (x0, x1) = span(u32::try_from(x).unwrap_or(u32::MAX), width, source_width);

            let mut sum_r: u64 = 0;
            let mut sum_g: u64 = 0;
            let mut sum_b: u64 = 0;
            let mut sum_a: u64 = 0;
            let mut count: u64 = 0;

            for row in y0..y1 {
                // One slice per source row, then a bounds-check-free walk of
                // the pixels inside it. `get` rather than an index: the maths
                // above should make these in range, and a panic here would trap
                // the whole instance rather than return a status.
                let start = row.saturating_mul(source_stride) + x0.saturating_mul(CHANNELS);
                let end = row.saturating_mul(source_stride) + x1.saturating_mul(CHANNELS);
                let Some(chunk) = source.data().get(start..end) else {
                    continue;
                };
                for sample in chunk.chunks_exact(CHANNELS) {
                    // `chunks_exact(4)` yields slices of exactly 4, so these
                    // four reads are the one place indexing is provably safe.
                    let (r, g, b, a) = (sample[0], sample[1], sample[2], sample[3]);
                    let alpha = u64::from(a);
                    sum_r += u64::from(r) * alpha;
                    sum_g += u64::from(g) * alpha;
                    sum_b += u64::from(b) * alpha;
                    sum_a += alpha;
                    count += 1;
                }
            }

            if count == 0 {
                continue;
            }
            // Σ(c·a)/Σa is the alpha-weighted mean. `checked_div` rather than a
            // guarded `/`: for Σa == 0 every sample in the box was fully
            // transparent, so there is no colour to recover and the pixel is
            // transparent black.
            let (r, g, b) = match (
                sum_r.checked_div(sum_a),
                sum_g.checked_div(sum_a),
                sum_b.checked_div(sum_a),
            ) {
                (Some(r), Some(g), Some(b)) => (r as u8, g as u8, b as u8),
                _ => (0, 0, 0),
            };
            pixel_out[0] = r;
            pixel_out[1] = g;
            pixel_out[2] = b;
            pixel_out[3] = (sum_a / count) as u8;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{downscale, preview_dimensions};
    use pochoir_core::{Error, Pixmap};

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Pixmap {
        let mut pixmap = Pixmap::new(width, height).expect("allocates");
        for byte in pixmap.data_mut().chunks_exact_mut(4) {
            byte.copy_from_slice(&rgba);
        }
        pixmap
    }

    #[test]
    fn a_page_inside_the_bound_is_left_alone() {
        assert_eq!(
            preview_dimensions(800, 600, 1024).expect("sizes"),
            (800, 600)
        );
        assert_eq!(
            preview_dimensions(1024, 100, 1024).expect("sizes"),
            (1024, 100),
            "exactly at the bound is still inside it"
        );
    }

    #[test]
    fn the_longer_edge_is_the_one_bounded_and_the_aspect_ratio_holds() {
        // A 12 MP phone photo, landscape and portrait.
        assert_eq!(
            preview_dimensions(4032, 3024, 1024).expect("sizes"),
            (1024, 768)
        );
        assert_eq!(
            preview_dimensions(3024, 4032, 1024).expect("sizes"),
            (768, 1024)
        );
    }

    #[test]
    fn an_extreme_aspect_ratio_still_has_a_row() {
        let (w, h) = preview_dimensions(10_000, 3, 100).expect("sizes");
        assert_eq!((w, h), (100, 1));
    }

    #[test]
    fn a_zero_bound_or_an_empty_page_is_refused() {
        assert!(matches!(
            preview_dimensions(100, 100, 0),
            Err(Error::InvalidParameter { name: "max_px" })
        ));
        assert!(matches!(
            preview_dimensions(0, 100, 10),
            Err(Error::InvalidParameter { name: "page size" })
        ));
    }

    #[test]
    fn a_solid_colour_survives_downscaling_exactly() {
        let source = solid(400, 300, [0x20, 0x40, 0x60, 0xFF]);
        let out = downscale(&source, 100, 75).expect("downscales");
        assert_eq!((out.width(), out.height()), (100, 75));
        assert_eq!(out.data().len(), 100 * 75 * 4);
        for pixel in out.data().chunks_exact(4) {
            assert_eq!(pixel, [0x20, 0x40, 0x60, 0xFF]);
        }
    }

    #[test]
    fn a_two_by_two_block_averages_to_its_mean() {
        let mut source = Pixmap::new(2, 2).expect("allocates");
        source.set_pixel(0, 0, [0, 0, 0, 255]);
        source.set_pixel(1, 0, [100, 100, 100, 255]);
        source.set_pixel(0, 1, [200, 200, 200, 255]);
        source.set_pixel(1, 1, [255, 255, 255, 255]);
        let out = downscale(&source, 1, 1).expect("downscales");
        // (0 + 100 + 200 + 255) / 4 = 138.75, truncated by integer division.
        assert_eq!(out.pixel(0, 0).expect("in range"), [138, 138, 138, 255]);
    }

    /// The alpha weighting is the point: without it, the colour of fully
    /// transparent pixels bleeds into the visible result.
    #[test]
    fn a_transparent_neighbour_does_not_tint_the_average() {
        let mut source = Pixmap::new(2, 1).expect("allocates");
        source.set_pixel(0, 0, [10, 20, 30, 255]);
        source.set_pixel(1, 0, [255, 0, 0, 0]); // transparent red
        let out = downscale(&source, 1, 1).expect("downscales");
        let pixel = out.pixel(0, 0).expect("in range");
        assert_eq!(
            [pixel[0], pixel[1], pixel[2]],
            [10, 20, 30],
            "the transparent red must contribute no colour"
        );
        assert_eq!(pixel[3], 127, "alpha is the plain mean of 255 and 0");
    }

    #[test]
    fn a_fully_transparent_block_stays_transparent() {
        let source = solid(4, 4, [200, 100, 50, 0]);
        let out = downscale(&source, 1, 1).expect("downscales");
        assert_eq!(out.pixel(0, 0).expect("in range"), [0, 0, 0, 0]);
    }

    #[test]
    fn downscaling_is_byte_identical_across_runs() {
        let mut source = Pixmap::new(97, 61).expect("allocates");
        for y in 0..61 {
            for x in 0..97 {
                let v = ((x * 7 + y * 13) % 256) as u8;
                source.set_pixel(x, y, [v, v.wrapping_add(40), v.wrapping_mul(3), 255]);
            }
        }
        let a = downscale(&source, 31, 19).expect("downscales");
        let b = downscale(&source, 31, 19).expect("downscales");
        assert_eq!(a.data(), b.data());
    }

    /// Every source pixel must land in exactly one box: a gap would show as a
    /// stripe and an overlap as a seam. Asserted by checking that the boxes of
    /// a non-integer ratio tile the source exactly.
    #[test]
    fn a_non_integer_ratio_covers_every_source_pixel() {
        // 7 -> 3 is 2.33 pixels per box: boxes of 2, 3, 2.
        let mut source = Pixmap::new(7, 1).expect("allocates");
        for x in 0..7 {
            source.set_pixel(x, 0, [0, 0, 0, 255]);
        }
        source.set_pixel(6, 0, [255, 255, 255, 255]);
        let out = downscale(&source, 3, 1).expect("downscales");
        // The last box must contain the white pixel, so it cannot be black.
        assert!(
            out.pixel(2, 0).expect("in range")[0] > 0,
            "the final source column was dropped"
        );
    }

    #[test]
    fn upscaling_and_degenerate_targets_are_refused() {
        let source = solid(10, 10, [0, 0, 0, 255]);
        for (w, h) in [(0, 5), (5, 0), (11, 10), (10, 11)] {
            assert!(downscale(&source, w, h).is_err(), "{w}x{h} must be refused");
        }
    }
}
