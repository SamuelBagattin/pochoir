//! A separable three-pass box blur with a fractional radius.
//!
//! # Why not an integer radius
//!
//! Three box passes approximate a Gaussian well, which is the standard trick
//! and the one ADR-0002 assumes. The usual relation for `n` passes of a box of
//! width `w` is `σ² = n (w² - 1) / 12`, so for three passes
//!
//! ```text
//! w = sqrt(4 σ² + 1)        radius = (w - 1) / 2
//! ```
//!
//! At ADR-0002's stated default of σ ≈ 0.6 px that gives a radius of **0.28**,
//! which an integer-radius box blur rounds to zero: the blur parameter would
//! do nothing at its own default value, and would then jump to a visible
//! smear at the first radius that survives rounding. That is one of the two
//! problems [ADR-0007](../../../../docs/adr/0007-watermark-specification-corrections.md)
//! fixes — the other being that σ is now keyed to the font size rather than to
//! pixels, so it is resolution-independent the way the glyph size already is.
//!
//! So the box here has a **fractional** radius: the taps inside the radius
//! weigh 1, the two taps straddling its edge weigh the fractional part, and the
//! whole window is normalised. A radius of 0.28 then blurs by 0.28 of a pixel,
//! which is what the parameter says it does.
//!
//! # What it is for, and what it is not for
//!
//! Antialiasing. ADR-0002 is explicit that blur has **not** been measured
//! against any watermark-removal tool and must not be described as a defence;
//! its certain benefit is that a hard glyph edge over a photograph stops
//! looking like a hard glyph edge over a photograph. Nothing in this module or
//! its documentation should come to claim otherwise.
//!
//! Outside the mask, coverage reads as **zero** rather than clamping to the
//! edge value, which is correct for a stamp: ink fades out at its boundary
//! instead of being smeared along it. The caller pads the mask by
//! [`padding_for`] so that fade happens in spare pixels rather than eating the
//! glyphs.

use crate::raster::Mask;

/// How many passes the Gaussian is approximated with.
///
/// Three is the usual answer: one box is visibly square, two is triangular,
/// three is close enough to a Gaussian that a fourth pass changes nothing a
/// viewer could see.
pub const PASSES: u32 = 3;

/// The box radius that approximates a Gaussian of this σ over [`PASSES`].
///
/// `sqrt` is IEEE-754-exact, so this is identical on wasm32 and native.
#[must_use]
pub fn radius_for_sigma(sigma: f32) -> f32 {
    if !sigma.is_finite() || sigma <= 0.0 {
        return 0.0;
    }
    let width = (4.0 * sigma * sigma + 1.0).sqrt();
    (width - 1.0) * 0.5
}

/// Pixels of empty margin a mask needs so a blur of this σ has room to fade.
///
/// Three σ of a Gaussian holds 99.7% of its weight, and the three box passes
/// reach `3 · ceil(radius)`; the larger of the two, plus one, is the margin.
#[must_use]
pub fn padding_for(sigma: f32) -> u32 {
    let radius = radius_for_sigma(sigma);
    if radius <= 0.0 {
        return 0;
    }
    let boxes = PASSES as f32 * (radius + 1.0);
    let gaussian = 3.0 * sigma;
    (boxes.max(gaussian).ceil() as u32) + 1
}

/// Blur `mask` in place by a Gaussian of standard deviation `sigma` pixels.
///
/// A σ of zero or less is a no-op, so a caller need not special-case "no blur".
pub fn blur(mask: &mut Mask, sigma: f32) {
    let radius = radius_for_sigma(sigma);
    if radius <= 0.0 || mask.width() == 0 || mask.height() == 0 {
        return;
    }
    let width = mask.width() as usize;
    let height = mask.height() as usize;

    // Work in `f32` across all six passes and quantise once at the end.
    // Quantising to `u8` between passes loses a bit a time, and at the small
    // radii this module exists to make work there is not a bit to spare.
    let mut buffer: Vec<f32> = mask.data().iter().map(|&v| f32::from(v)).collect();
    let mut scratch = vec![0.0_f32; buffer.len()];

    let whole = radius.floor();
    let fraction = radius - whole;
    let taps = whole as i64;
    let total = 2.0 * whole + 1.0 + 2.0 * fraction;
    let normalise = 1.0 / total;

    for _ in 0..PASSES {
        // Horizontal.
        for y in 0..height {
            let row = y * width;
            for x in 0..width {
                let mut sum = 0.0;
                let mut offset = -taps;
                while offset <= taps {
                    let at = x as i64 + offset;
                    if at >= 0 && (at as usize) < width {
                        sum += buffer[row + at as usize];
                    }
                    offset += 1;
                }
                if fraction > 0.0 {
                    for edge in [-(taps + 1), taps + 1] {
                        let at = x as i64 + edge;
                        if at >= 0 && (at as usize) < width {
                            sum += buffer[row + at as usize] * fraction;
                        }
                    }
                }
                scratch[row + x] = sum * normalise;
            }
        }
        core::mem::swap(&mut buffer, &mut scratch);

        // Vertical.
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0.0;
                let mut offset = -taps;
                while offset <= taps {
                    let at = y as i64 + offset;
                    if at >= 0 && (at as usize) < height {
                        sum += buffer[at as usize * width + x];
                    }
                    offset += 1;
                }
                if fraction > 0.0 {
                    for edge in [-(taps + 1), taps + 1] {
                        let at = y as i64 + edge;
                        if at >= 0 && (at as usize) < height {
                            sum += buffer[at as usize * width + x] * fraction;
                        }
                    }
                }
                scratch[y * width + x] = sum * normalise;
            }
        }
        core::mem::swap(&mut buffer, &mut scratch);
    }

    for (out, value) in mask.data_mut().iter_mut().zip(&buffer) {
        // `+ 0.5` then `floor`: never `round`. See ADR-0002 Decision 6.
        *out = (value.clamp(0.0, 255.0) + 0.5).floor() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::{blur, padding_for, radius_for_sigma};
    use crate::raster::Mask;

    /// A mask with one fully covered pixel in the middle.
    fn impulse(size: u32) -> Mask {
        let mut mask = Mask::new(size, size).expect("small");
        let middle = (size / 2) as i64;
        let index = middle as usize * size as usize + middle as usize;
        mask.data_mut()[index] = 255;
        mask
    }

    #[test]
    fn the_radius_formula_matches_the_three_pass_relation() {
        // σ² = 3(w² - 1)/12 rearranged. Spot-check both directions.
        for sigma in [0.25_f32, 0.6, 0.87, 2.0, 5.0] {
            let radius = radius_for_sigma(sigma);
            let width = 2.0 * radius + 1.0;
            let recovered = ((width * width - 1.0) / 4.0).sqrt();
            assert!(
                (recovered - sigma).abs() < 1e-5,
                "σ {sigma} gave radius {radius}, which is σ {recovered}"
            );
        }
        assert_eq!(radius_for_sigma(0.0), 0.0);
        assert_eq!(radius_for_sigma(-1.0), 0.0);
        assert_eq!(radius_for_sigma(f32::NAN), 0.0);
    }

    #[test]
    fn adr_0002s_stated_default_rounds_to_a_sub_pixel_radius() {
        // The number that motivated ADR-0007: at σ = 0.6 px the box radius is
        // 0.28, and an integer-radius implementation would do nothing at all.
        let radius = radius_for_sigma(0.6);
        assert!(
            radius > 0.25 && radius < 0.3,
            "radius for σ=0.6 is {radius}"
        );
        assert_eq!(radius.floor(), 0.0, "the integer part is zero");
    }

    #[test]
    fn a_sub_pixel_blur_changes_pixels() {
        // The regression test ADR-0007 requires: a fractional radius must
        // actually do something, not silently round away.
        for sigma in [0.2_f32, 0.3, 0.6, 0.87] {
            let mut mask = impulse(9);
            let before = mask.clone();
            blur(&mut mask, sigma);
            assert_ne!(mask, before, "σ={sigma} left the mask untouched");
            assert!(
                mask.at(4, 4) < 255,
                "σ={sigma} did not spread the impulse at all"
            );
            assert!(
                mask.at(3, 4) > 0 || mask.at(5, 4) > 0,
                "σ={sigma} put nothing into a neighbouring pixel"
            );
        }
    }

    #[test]
    fn zero_sigma_is_a_no_op() {
        let mut mask = impulse(9);
        let before = mask.clone();
        blur(&mut mask, 0.0);
        assert_eq!(mask, before);
        blur(&mut mask, -3.0);
        assert_eq!(mask, before);
    }

    #[test]
    fn a_bigger_sigma_spreads_further() {
        let mut spreads = Vec::new();
        for sigma in [0.5_f32, 1.0, 2.0, 4.0] {
            let mut mask = impulse(65);
            blur(&mut mask, sigma);
            // Where the energy has reached, as the count of non-zero pixels.
            let touched = mask.data().iter().filter(|&&v| v > 0).count();
            spreads.push((sigma, touched, mask.at(32, 32)));
        }
        for pair in spreads.windows(2) {
            assert!(
                pair[1].1 > pair[0].1,
                "σ={} touched {} pixels, σ={} only {}",
                pair[1].0,
                pair[1].1,
                pair[0].0,
                pair[0].1
            );
            assert!(
                pair[1].2 <= pair[0].2,
                "a larger σ should not make the centre brighter"
            );
        }
    }

    #[test]
    fn a_blur_is_symmetric_about_an_impulse() {
        let mut mask = impulse(21);
        blur(&mut mask, 1.5);
        for offset in 1..10 {
            assert_eq!(
                mask.at(10 - offset, 10),
                mask.at(10 + offset, 10),
                "horizontally asymmetric at offset {offset}"
            );
            assert_eq!(
                mask.at(10, 10 - offset),
                mask.at(10, 10 + offset),
                "vertically asymmetric at offset {offset}"
            );
            assert_eq!(
                mask.at(10 - offset, 10),
                mask.at(10, 10 - offset),
                "the two axes disagree at offset {offset}"
            );
        }
    }

    #[test]
    fn a_blur_does_not_brighten_a_flat_field() {
        // A fully covered mask must stay fully covered in the middle: the
        // window is normalised, so no energy is invented.
        let mut mask = Mask::new(21, 21).expect("small");
        mask.data_mut().fill(255);
        blur(&mut mask, 2.0);
        assert_eq!(mask.at(10, 10), 255, "the interior lost or gained coverage");
        assert!(mask.at(0, 0) < 255, "the border should fade outwards");
    }

    #[test]
    fn padding_leaves_room_for_the_fade() {
        assert_eq!(padding_for(0.0), 0);
        for sigma in [0.3_f32, 0.87, 2.0, 5.0] {
            let padding = padding_for(sigma);
            assert!(padding >= 1, "σ={sigma} asked for no padding");
            // Blur an impulse placed exactly `padding` from the edge: nothing
            // should reach the border, which is what the margin is promising.
            let size = padding * 2 + 1;
            let mut mask = Mask::new(size, size).expect("small");
            let middle = i64::from(padding);
            let index = padding as usize * size as usize + padding as usize;
            mask.data_mut()[index] = 255;
            blur(&mut mask, sigma);
            let edge: u32 = (0..i64::from(size))
                .map(|x| u32::from(mask.at(x, 0)) + u32::from(mask.at(x, middle * 2)))
                .sum();
            assert_eq!(edge, 0, "σ={sigma} leaked ink to the border");
        }
    }

    #[test]
    fn blurring_is_bit_identical_across_runs() {
        let mut a = impulse(17);
        let mut b = impulse(17);
        blur(&mut a, 0.87);
        blur(&mut b, 0.87);
        assert_eq!(a, b);
    }

    #[test]
    fn an_empty_mask_is_not_a_problem() {
        let mut mask = Mask::new(0, 0).expect("zero is legal");
        blur(&mut mask, 1.0);
        assert_eq!(mask.data().len(), 0);
    }
}
