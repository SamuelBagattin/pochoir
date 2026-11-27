//! What the page under the mark actually looks like.
//!
//! [ADR-0002](../../../../docs/adr/0002-watermark-algorithm-and-quality-modes.md)
//! Decision 3 fixes the mark's colour at `#1A1A1A` and warns only when the
//! user's chosen colour is too *light*. That test is one-sided, and the page it
//! is implicitly about is white paper. On a dark-mode screenshot — a bank
//! statement, an insurance portal, an identity document held in an app — the
//! default mark is black on black, the output is visually identical to the
//! input, and nothing is said.
//! [ADR-0010](../../../../docs/adr/0010-mark-contrast-against-the-page.md)
//! replaces that with a measurement of the page itself, which is what this
//! module is.
//!
//! # Why a histogram and not a mean
//!
//! A mean, and for that matter a median, answers a question about one number
//! when the thing that matters is a distribution. Half a white scan beside half
//! a black photograph has a mid-grey mean and a mid-grey median, and mid-grey
//! is the one tone that contrasts with neither half. Counting pixels answers
//! the question we actually have — *over how much of this page would the mark
//! be visible?* — and it answers it the same way for the choice and for the
//! warning, so the two can never disagree.
//!
//! # Determinism
//!
//! Everything here is integer counting plus `+ - * /` on `f32`, all of which
//! IEEE-754 specifies exactly and which therefore agree between
//! `wasm32-unknown-unknown` and the host. In particular there is **no
//! linearisation**: the proper sRGB-to-linear transfer function needs `powf`,
//! which comes from the platform's libm and is permitted to differ between
//! targets, and one such call here would break the byte-identical outputs that
//! ADR-0002 Decision 6 requires and the golden suite asserts. The Rec. 709
//! coefficients are applied to the sRGB values directly, exactly as
//! `Mark::colour_is_too_pale` used to do it. That is a crude photometric
//! measure; it is ample for choosing between two colours a long way apart and
//! for raising a warning, and it is the only one we can compute identically on
//! both targets.
//!
//! The decision between the two candidates is settled on **integer pixel
//! counts**, not on a ratio, so no rounding can tip it.

use crate::pixmap::Pixmap;

/// Bins in the luminance histogram: one per 8-bit level.
///
/// The inputs are 8-bit, so a finer histogram would measure nothing more.
pub const TONE_BINS: usize = 256;

/// Rec. 709 relative luminance of an sRGB colour, **without linearising**.
///
/// See the module documentation for why the transfer function is left out. The
/// result is in `0.0..=1.0`.
#[must_use]
pub fn relative_luminance(colour: [u8; 3]) -> f32 {
    let [r, g, b] = colour;
    let l = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
    l.clamp(0.0, 1.0)
}

/// The luminance of one RGBA pixel **as it will be seen**.
///
/// A pixel that is not opaque is composited over white first, because
/// [`crate::jpeg::encode`] does exactly that: JPEG has no alpha channel and
/// white is the right assumption for a document. Measuring the raw samples
/// instead would read a PNG's transparent background as whatever colour
/// happens to sit under the transparency — black, in most encoders — and would
/// then pick a light mark for a page that comes out white.
fn pixel_luminance(pixel: &[u8]) -> f32 {
    let alpha = f32::from(pixel[3]) / 255.0;
    let over_white = |sample: u8| f32::from(sample) * alpha + 255.0 * (1.0 - alpha);
    let l = (0.2126 * over_white(pixel[0])
        + 0.7152 * over_white(pixel[1])
        + 0.0722 * over_white(pixel[2]))
        / 255.0;
    l.clamp(0.0, 1.0)
}

/// Which histogram bin a luminance falls in.
fn bin_of(luminance: f32) -> usize {
    let scaled = (luminance * 255.0 + 0.5).floor();
    (scaled.clamp(0.0, 255.0) as usize).min(TONE_BINS - 1)
}

/// The luminance a bin stands for.
fn luminance_of(bin: usize) -> f32 {
    bin as f32 / 255.0
}

/// The distribution of luminance over one page, before the mark is drawn.
///
/// Cheap to carry: one `u32` per 8-bit level, so a kilobyte regardless of the
/// page's size. [`crate::mark::Composition`] hands it back to the caller, which
/// is how a user interface can say something about the page without measuring
/// it a second time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTone {
    bins: [u32; TONE_BINS],
    pixels: u64,
}

impl PageTone {
    /// Measure a pixmap, one pass, no allocation.
    ///
    /// The pixmap must be the page **before** the mark is composited onto it:
    /// measuring afterwards would measure the mark.
    #[must_use]
    pub fn measure(pixmap: &Pixmap) -> Self {
        let mut bins = [0u32; TONE_BINS];
        let mut pixels = 0u64;
        for pixel in pixmap.data().chunks_exact(4) {
            // `MAX_PIXELS` is 256 Mpx, well inside `u32`, so no bin can wrap.
            bins[bin_of(pixel_luminance(pixel))] += 1;
            pixels += 1;
        }
        Self { bins, pixels }
    }

    /// An empty measurement, for a page with no pixels.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            bins: [0; TONE_BINS],
            pixels: 0,
        }
    }

    /// How many pixels were measured.
    #[must_use]
    pub const fn pixels(&self) -> u64 {
        self.pixels
    }

    /// `true` when nothing was measured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pixels == 0
    }

    /// The raw histogram: `bins[i]` counts the pixels at luminance `i / 255`.
    #[must_use]
    pub const fn bins(&self) -> &[u32; TONE_BINS] {
        &self.bins
    }

    /// The luminance below which `fraction` of the page's pixels sit.
    ///
    /// `fraction` is clamped to `0.0..=1.0`. Returns 0.0 for an empty page.
    #[must_use]
    pub fn quantile(&self, fraction: f32) -> f32 {
        if self.pixels == 0 {
            return 0.0;
        }
        let target = {
            let f = f64::from(fraction.clamp(0.0, 1.0));
            let raw = (self.pixels as f64 * f).ceil() as u64;
            raw.clamp(1, self.pixels)
        };
        let mut seen = 0u64;
        for (bin, count) in self.bins.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= target {
                return luminance_of(bin);
            }
        }
        1.0
    }

    /// The page's median luminance.
    #[must_use]
    pub fn median(&self) -> f32 {
        self.quantile(0.5)
    }

    /// Pixels whose luminance is at least `distance` away from `luminance`.
    #[must_use]
    pub fn pixels_beyond(&self, luminance: f32, distance: f32) -> u64 {
        let mut count = 0u64;
        for (bin, pixels) in self.bins.iter().enumerate() {
            if (luminance_of(bin) - luminance).abs() >= distance {
                count += u64::from(*pixels);
            }
        }
        count
    }

    /// Pixels over which a mark of this colour and opacity would be visible.
    ///
    /// "Visible" is [`MIN_VISIBLE_DELTA`]: at full glyph coverage the mark
    /// changes a pixel's luminance by `opacity * |mark - page|`, so the page
    /// has to be at least `MIN_VISIBLE_DELTA / opacity` away from the mark's
    /// colour. The opacity is in the test because it is in the physics — the
    /// same colour at 0.05 is a fifth of the mark it is at 0.30.
    #[must_use]
    pub fn visible_pixels(&self, colour: [u8; 3], opacity: f32) -> u64 {
        if opacity <= 0.0 {
            return 0;
        }
        self.pixels_beyond(relative_luminance(colour), MIN_VISIBLE_DELTA / opacity)
    }

    /// [`visible_pixels`] as a fraction of the page, `0.0..=1.0`.
    ///
    /// 0.0 for an empty page, which reports as the worst case rather than as a
    /// division by zero.
    ///
    /// [`visible_pixels`]: PageTone::visible_pixels
    #[must_use]
    pub fn visible_fraction(&self, colour: [u8; 3], opacity: f32) -> f32 {
        if self.pixels == 0 {
            return 0.0;
        }
        self.visible_pixels(colour, opacity) as f32 / self.pixels as f32
    }

    /// The better of the two candidate colours for this page.
    ///
    /// The criterion is [`visible_pixels`]: whichever of [`DARK_COLOUR`] and
    /// [`LIGHT_COLOUR`] is visible over more of the page wins, compared as
    /// integer counts so that no rounding decides it. A tie — including the
    /// tie where neither is visible anywhere, which a very low opacity can
    /// produce — goes to whichever is further from the page's median, and
    /// failing that to [`DARK_COLOUR`], whose photocopy argument is ADR-0002's
    /// and still holds on a light page.
    ///
    /// Two fixed candidates rather than a colour derived from the image: the
    /// mark is text that has to stay legible, and the average of a page is the
    /// one colour guaranteed to contrast with nothing on it.
    /// [ADR-0010](../../../../docs/adr/0010-mark-contrast-against-the-page.md)
    /// records the argument.
    ///
    /// [`visible_pixels`]: PageTone::visible_pixels
    #[must_use]
    pub fn choose_colour(&self, opacity: f32) -> [u8; 3] {
        let dark = self.visible_pixels(DARK_COLOUR, opacity);
        let light = self.visible_pixels(LIGHT_COLOUR, opacity);
        if light > dark {
            return LIGHT_COLOUR;
        }
        if dark > light {
            return DARK_COLOUR;
        }
        let median = self.median();
        let from_dark = (median - relative_luminance(DARK_COLOUR)).abs();
        let from_light = (median - relative_luminance(LIGHT_COLOUR)).abs();
        if from_light > from_dark {
            LIGHT_COLOUR
        } else {
            DARK_COLOUR
        }
    }
}

/// Neutral dark grey, `#1A1A1A`: the candidate for a light page.
///
/// ADR-0002 Decision 3, and still load-bearing there. The attacker's next move
/// is to print and rescan, or photocopy in black and white; a dark grey mark
/// survives greyscale reproduction and aggressive JPEG re-compression.
pub const DARK_COLOUR: [u8; 3] = [0x1A, 0x1A, 0x1A];

/// Neutral light grey, `#E5E5E5`: the candidate for a dark page.
///
/// The photometric mirror of [`DARK_COLOUR`] — 229 is as far below white as 26
/// is above black — chosen so that neither candidate is special. On a dark page
/// the photocopy argument runs the other way round: a photocopier renders a
/// dark-mode screenshot as a dark field, and it is the light mark that is left
/// standing.
pub const LIGHT_COLOUR: [u8; 3] = [0xE5, 0xE5, 0xE5];

/// The smallest change in a pixel's luminance that counts as a visible mark.
///
/// 0.05 is about 13 levels of an 8-bit channel at full glyph coverage: well
/// clear of what JPEG at the default quality rounds away, and about a fifth of
/// what the default dark mark achieves on white paper
/// (`0.30 * (1.0 - 0.102) = 0.269`, some 69 levels), which is the case the
/// product was designed around and visibly succeeds at.
///
/// It is a threshold on a crude luminance measure, not a perceptual model, and
/// it is used only to choose between two colours and to raise a warning.
pub const MIN_VISIBLE_DELTA: f32 = 0.05;

/// Below this fraction of the page, a mark's contrast is reported as poor.
///
/// 0.90 tolerates a dark page carrying a white logo, or a scan with a bright
/// margin, and catches the page that is genuinely split between a light half
/// and a dark one — where no single colour works and the honest answer is to
/// say so rather than to pick one quietly.
pub const MIN_CONTRAST_COVERAGE: f32 = 0.90;

#[cfg(test)]
mod tests {
    use super::{DARK_COLOUR, LIGHT_COLOUR, PageTone, TONE_BINS, bin_of, relative_luminance};
    use crate::pixmap::Pixmap;

    /// A page of one flat shade.
    fn flat(shade: u8, pixels: u32) -> Pixmap {
        Pixmap::from_rgba(
            pixels,
            1,
            [shade, shade, shade, 255].repeat(pixels as usize),
        )
        .expect("fixture")
    }

    /// A page that is `dark_share` of `dark` and the rest `light`.
    fn split(dark: u8, light: u8, dark_share: u32, total: u32) -> Pixmap {
        let mut data = Vec::with_capacity(total as usize * 4);
        for index in 0..total {
            let shade = if index < dark_share { dark } else { light };
            data.extend_from_slice(&[shade, shade, shade, 255]);
        }
        Pixmap::from_rgba(total, 1, data).expect("fixture")
    }

    #[test]
    fn the_candidates_are_photometric_mirrors() {
        let dark = relative_luminance(DARK_COLOUR);
        let light = relative_luminance(LIGHT_COLOUR);
        assert!((dark + light - 1.0).abs() < 1e-6, "{dark} + {light}");
    }

    #[test]
    fn a_flat_page_lands_in_one_bin() {
        let tone = PageTone::measure(&flat(0x80, 16));
        assert_eq!(tone.pixels(), 16);
        let occupied: Vec<usize> = (0..TONE_BINS).filter(|&b| tone.bins()[b] > 0).collect();
        assert_eq!(occupied, vec![bin_of(relative_luminance([0x80; 3]))]);
    }

    #[test]
    fn transparency_is_measured_over_white() {
        // A fully transparent pixel whose samples are black: the encoder will
        // composite it onto white, so that is what it must measure as.
        let pixmap = Pixmap::from_rgba(1, 1, vec![0, 0, 0, 0]).expect("fixture");
        assert_eq!(PageTone::measure(&pixmap).median(), 1.0);
    }

    #[test]
    fn a_dark_page_chooses_the_light_mark() {
        let tone = PageTone::measure(&flat(0x14, 64));
        assert_eq!(tone.choose_colour(0.30), LIGHT_COLOUR);
    }

    #[test]
    fn a_white_page_keeps_the_dark_mark() {
        let tone = PageTone::measure(&flat(0xFF, 64));
        assert_eq!(tone.choose_colour(0.30), DARK_COLOUR);
    }

    #[test]
    fn every_single_shade_is_covered_by_one_of_the_two() {
        // The two candidates' blind spots must not overlap, or there would be
        // a flat page no colour can mark.
        for shade in 0..=255u8 {
            let tone = PageTone::measure(&flat(shade, 8));
            let chosen = tone.choose_colour(0.30);
            assert_eq!(
                tone.visible_pixels(chosen, 0.30),
                8,
                "shade {shade:#04x} is invisible to {chosen:?}"
            );
        }
    }

    #[test]
    fn a_split_page_leaves_half_of_it_unmarked() {
        let tone = PageTone::measure(&split(0x00, 0xFF, 50, 100));
        let chosen = tone.choose_colour(0.30);
        let fraction = tone.visible_fraction(chosen, 0.30);
        assert!(
            (fraction - 0.5).abs() < 0.01,
            "half a black page and half a white one: got {fraction}"
        );
    }

    #[test]
    fn opacity_moves_the_visibility_threshold() {
        // A page 0.20 away from the dark mark: visible at 0.30 opacity
        // (0.060 >= 0.05), not at 0.20 (0.040 < 0.05).
        let shade = ((relative_luminance(DARK_COLOUR) + 0.20) * 255.0).round() as u8;
        let tone = PageTone::measure(&flat(shade, 32));
        assert_eq!(tone.visible_pixels(DARK_COLOUR, 0.30), 32);
        assert_eq!(tone.visible_pixels(DARK_COLOUR, 0.20), 0);
    }

    #[test]
    fn a_vanishing_opacity_still_chooses_by_the_median() {
        // Nothing is visible at either candidate, so the tie-break decides,
        // and on a black page it must still pick the light mark.
        let tone = PageTone::measure(&flat(0x00, 8));
        assert_eq!(tone.visible_pixels(DARK_COLOUR, 0.05), 0);
        assert_eq!(tone.visible_pixels(LIGHT_COLOUR, 0.05), 0);
        assert_eq!(tone.choose_colour(0.05), LIGHT_COLOUR);
    }

    #[test]
    fn quantiles_read_off_a_known_split() {
        let tone = PageTone::measure(&split(0x00, 0xFF, 25, 100));
        assert_eq!(tone.quantile(0.10), 0.0);
        assert_eq!(tone.quantile(0.25), 0.0);
        assert_eq!(tone.median(), 1.0);
    }

    #[test]
    fn an_empty_page_measures_as_nothing() {
        let tone = PageTone::measure(&Pixmap::empty());
        assert!(tone.is_empty());
        assert_eq!(tone.median(), 0.0);
        assert_eq!(tone.visible_fraction(DARK_COLOUR, 0.30), 0.0);
    }

    #[test]
    fn the_measurement_is_identical_across_runs() {
        let page = split(0x10, 0xF0, 37, 101);
        assert_eq!(PageTone::measure(&page), PageTone::measure(&page));
    }
}
