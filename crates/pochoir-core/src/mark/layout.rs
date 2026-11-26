//! Fitting the mark text onto the page: wrap, then shrink, then refuse.
//!
//! # The rule, and why it is not "clip"
//!
//! ADR-0002 allows "up to 200 characters, up to 3 lines" at a font size of
//! 3.5% of the page's shorter edge, and does not say what happens when those
//! two do not fit together — which, worked through, they frequently do not: a
//! realistic 54-character French mark is already about one page wide, and 200
//! characters on one line is several times a page.
//!
//! [ADR-0007](../../../../docs/adr/0007-watermark-specification-corrections.md)
//! settles it, in this order:
//!
//! 1. **Wrap** on word boundaries, up to the three lines ADR-0002 already
//!    allows. Words are never broken.
//! 2. **Shrink** the font if three lines still overflow, down to
//!    [`MIN_FONT_SIZE_RATIO`] — which is not an invented number but ADR-0002
//!    Decision 3's own stated minimum of 1% of the shorter edge.
//! 3. **Refuse**, with an error naming the problem, if even that overflows.
//!
//! **Never clip.** Truncating "Document destiné exclusivement à une demande de
//! location" to "Document destiné exclusiv" silently changes what the document
//! asserts about itself. For a tool whose entire purpose is making a document's
//! intended use legible on its face, that is a correctness bug, not a cosmetic
//! one — so the mark that cannot be drawn honestly is not drawn at all.
//!
//! **Step 3 is a corner, not the ordinary fate of long text.** An unbreakable
//! 200-character string is usually drawn, by shrinking a long way, because the
//! floor is a fraction of the page's *shorter edge* while the width budget
//! follows the *rotated extent* — so widening a page lowers the floor faster
//! than it lowers the budget, and the same text is refused on one page shape
//! and drawn on another. That makes a silent shrink the common outcome and the
//! thing actually worth reporting, which
//! [`Composition::was_shrunk_noticeably`] exists for. ADR-0007 Decision 2 has
//! the measurements.
//!
//! [`Composition::was_shrunk_noticeably`]: super::Composition::was_shrunk_noticeably
//!
//! # What "fits" means
//!
//! Not the text block, but its **rotated** bounding box. A block `W` wide and
//! `H` tall, turned by θ, occupies
//!
//! ```text
//! W·|cos θ| + H·|sin θ|   across
//! W·|sin θ| + H·|cos θ|   down
//! ```
//!
//! and both have to fit the page, because a mark the page cannot hold once
//! cannot be tiled either.

use crate::font::Font;
use crate::geom::Transform;
use crate::raster::{Coverage, Mask};
use crate::{Error, Result};

use super::blur;
use super::trig::sin_cos_degrees;

/// Most lines the mark may wrap to, from ADR-0002 Decision 3.
pub const MAX_LINES: usize = 3;

/// Baseline-to-baseline distance, as a multiple of the font size.
///
/// 1.2 is the conventional default for Latin text and is what a browser would
/// use for `line-height: normal` on this font. The font's own `hhea` line gap
/// is not consulted: one constant here is easier to reason about than a value
/// that changes if the asset is regenerated.
pub const LINE_HEIGHT: f32 = 1.2;

/// How far the font may shrink before the mark is refused, as a fraction of
/// the page's shorter edge.
///
/// Deliberately ADR-0002 Decision 3's own documented minimum for the font-size
/// parameter, so the floor is a number the specification already blesses as
/// legible rather than one chosen to make a hard case pass.
pub const MIN_FONT_SIZE_RATIO: f32 = 0.01;

/// Each shrink attempt multiplies the size by this.
///
/// 4% a step: fine enough that the accepted size is close to the largest that
/// would have fitted, coarse enough that the search is over in a few dozen
/// multiplications.
const SHRINK_FACTOR: f32 = 0.96;

/// Most shrink attempts before giving up.
///
/// `0.96^80` is about 0.04, so from the 3.5% default this reaches well past the
/// 1% floor; the floor is what actually stops the loop and this only bounds it.
const MAX_SHRINK_ATTEMPTS: u32 = 80;

/// Below this, a sine or cosine counts as zero for the fit arithmetic, so that
/// a mark at 0° or 90° divides by nothing.
const AXIS_EPSILON: f32 = 1e-4;

/// The text, laid out and measured, ready to be drawn into a stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The wrapped lines, in order. Between one and [`MAX_LINES`] of them.
    pub lines: Vec<String>,
    /// The font size this fitted at, in pixels.
    pub font_size_px: f32,
    /// Baseline-to-baseline distance, in pixels.
    pub line_height_px: f32,
    /// Width of the widest line, in pixels.
    pub block_width_px: f32,
    /// Height of the whole block from the first line's ascender to the last
    /// line's descender, in pixels.
    pub block_height_px: f32,
    /// Distance from the top of the block to the first baseline, in pixels.
    pub ascender_px: f32,
    /// `true` if the font had to be made smaller than the caller asked for.
    ///
    /// Surfaced so a UI can say so; nothing in the engine branches on it.
    pub shrunk: bool,
}

/// Width of `text` at `font_size_px`, in pixels.
///
/// Advance widths only: no kerning and no shaping, which ADR-0001 scopes out.
/// A character the font lacks contributes nothing, because
/// [`super::unsupported_characters`] has already refused such text before
/// anything reaches here.
#[must_use]
pub fn measure(font: &Font<'_>, text: &str, font_size_px: f32) -> f32 {
    let scale = font_size_px / f32::from(font.units_per_em());
    let units: u32 = text
        .chars()
        .filter_map(|character| font.glyph_for(character))
        .filter_map(|glyph| font.advance_width(glyph))
        .map(u32::from)
        .sum();
    units as f32 * scale
}

/// Greedily wrap `words` so that no line exceeds `limit`, if it can.
///
/// A single word wider than `limit` goes on a line of its own and overflows it:
/// we do not hyphenate, and breaking a word inside a watermark would be worse
/// than the alternative. The caller checks the resulting width and shrinks.
fn wrap(font: &Font<'_>, words: &[&str], font_size_px: f32, limit: f32) -> Vec<String> {
    let space = measure(font, " ", font_size_px);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0_f32;
    for word in words {
        let word_width = measure(font, word, font_size_px);
        if current.is_empty() {
            current.push_str(word);
            current_width = word_width;
            continue;
        }
        if current_width + space + word_width <= limit {
            current.push(' ');
            current.push_str(word);
            current_width += space + word_width;
        } else {
            lines.push(core::mem::take(&mut current));
            current.push_str(word);
            current_width = word_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// The widest a block `height` tall may be and still fit the page once rotated.
///
/// `None` when no width works — the block is already too tall on its own.
fn width_budget(page_width: f32, page_height: f32, height: f32, angle: f32) -> Option<f32> {
    let (sine, cosine) = sin_cos_degrees(angle);
    let (sine, cosine) = (sine.abs(), cosine.abs());
    let mut budget = f32::INFINITY;
    if cosine > AXIS_EPSILON {
        budget = budget.min((page_width - height * sine) / cosine);
    } else if height * sine > page_width {
        return None; // Upright mark, taller than the page is wide.
    }
    if sine > AXIS_EPSILON {
        budget = budget.min((page_height - height * cosine) / sine);
    } else if height * cosine > page_height {
        return None; // Horizontal mark, taller than the page.
    }
    (budget.is_finite() && budget > 0.0).then_some(budget)
}

/// Lay the text out, shrinking the font as far as [`MIN_FONT_SIZE_RATIO`]
/// allows and refusing beyond that.
///
/// A successful [`Layout`] may still be much smaller than `requested_font_size_px`;
/// [`Layout::shrunk`] says that it is, and the caller is expected to pass that
/// on rather than let the mark come out quietly faint.
///
/// `text` must already be NFC-normalised and known to be drawable; see
/// [`super::prepare`].
///
/// # Errors
///
/// [`Error::MarkDoesNotFit`] when the text cannot be fitted at any permitted
/// size, and [`Error::InvalidParameter`] for a page with no pixels or text with
/// no words.
pub fn plan(
    font: &Font<'_>,
    text: &str,
    page_width: f32,
    page_height: f32,
    requested_font_size_px: f32,
    angle: f32,
) -> Result<Layout> {
    if !(page_width > 0.0 && page_height > 0.0) {
        return Err(Error::InvalidParameter { name: "page size" });
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Err(Error::InvalidParameter { name: "mark text" });
    }

    let shorter = page_width.min(page_height);
    let floor_px = MIN_FONT_SIZE_RATIO * shorter;
    let mut size = requested_font_size_px.max(floor_px);
    let mut shrunk = false;

    for _ in 0..=MAX_SHRINK_ATTEMPTS {
        if let Some(mut layout) = try_at(font, &words, page_width, page_height, size, angle) {
            layout.shrunk = shrunk;
            return Ok(layout);
        }
        if size <= floor_px {
            break;
        }
        size = (size * SHRINK_FACTOR).max(floor_px);
        shrunk = true;
    }

    Err(Error::MarkDoesNotFit {
        characters: text.chars().count(),
    })
}

/// One attempt at a given font size, over each permitted line count.
fn try_at(
    font: &Font<'_>,
    words: &[&str],
    page_width: f32,
    page_height: f32,
    font_size_px: f32,
    angle: f32,
) -> Option<Layout> {
    let scale = font_size_px / f32::from(font.units_per_em());
    // The font's own line box, which is taller than the ink and therefore the
    // conservative figure to fit — a mark whose box fits cannot have glyphs
    // sticking off the page.
    let ascender_px = f32::from(font.ascender()) * scale;
    let descender_px = -f32::from(font.descender()) * scale;
    let em_height = ascender_px + descender_px;
    let line_height_px = LINE_HEIGHT * font_size_px;

    for count in 1..=MAX_LINES {
        let height = (count - 1) as f32 * line_height_px + em_height;
        let Some(budget) = width_budget(page_width, page_height, height, angle) else {
            continue;
        };
        let lines = wrap(font, words, font_size_px, budget);
        if lines.len() > count {
            continue; // Needs more lines than this attempt allows.
        }
        let block_width_px = lines
            .iter()
            .map(|line| measure(font, line, font_size_px))
            .fold(0.0_f32, f32::max);
        if block_width_px > budget {
            continue; // An unbreakable word overflowed.
        }
        // Recompute the height for the line count we actually used, which may
        // be fewer than this attempt allowed; fewer lines only ever fits.
        let block_height_px = (lines.len() - 1) as f32 * line_height_px + em_height;
        return Some(Layout {
            lines,
            font_size_px,
            line_height_px,
            block_width_px,
            block_height_px,
            ascender_px,
            shrunk: false,
        });
    }
    None
}

impl Layout {
    /// The rotated bounding box of this block at `angle`, as `(width, height)`.
    #[must_use]
    pub fn rotated_extent(&self, angle: f32) -> (f32, f32) {
        let (sine, cosine) = sin_cos_degrees(angle);
        let (sine, cosine) = (sine.abs(), cosine.abs());
        (
            self.block_width_px * cosine + self.block_height_px * sine,
            self.block_width_px * sine + self.block_height_px * cosine,
        )
    }
}

/// Draw a laid-out mark into a coverage mask, blurred.
///
/// The mask is the block's size plus [`blur::padding_for`] on every side, so
/// the blur fades into spare pixels rather than eating the glyphs. Lines are
/// **centred** within the block, which reads better than ragged-right for a
/// two- or three-line mark and costs one subtraction.
///
/// # Errors
///
/// Returns [`Error::ImageTooLarge`] if the stamp would exceed
/// [`crate::pixmap::MAX_PIXELS`], which needs a page of absurd size.
pub fn stamp(font: &Font<'_>, layout: &Layout, sigma: f32) -> Result<Mask> {
    let padding = blur::padding_for(sigma);
    let width = layout.block_width_px.ceil().max(1.0) as u32 + padding * 2;
    let height = layout.block_height_px.ceil().max(1.0) as u32 + padding * 2;
    let mut coverage = Coverage::new(width, height)?;

    let scale = layout.font_size_px / f32::from(font.units_per_em());
    let margin = padding as f32;
    for (index, line) in layout.lines.iter().enumerate() {
        let line_width = measure(font, line, layout.font_size_px);
        let mut pen = margin + (layout.block_width_px - line_width) * 0.5;
        let baseline = margin + layout.ascender_px + index as f32 * layout.line_height_px;
        for character in line.chars() {
            let Some(glyph) = font.glyph_for(character) else {
                continue;
            };
            let outline = font.outline(glyph)?;
            if !outline.is_empty() {
                let transform =
                    Transform::scale_flip_y(scale).then(&Transform::translate(pen, baseline));
                coverage.fill(&outline, &transform);
            }
            let advance = font.advance_width(glyph).map_or(0.0, f32::from) * scale;
            pen += advance;
        }
    }

    let mut mask = coverage.resolve();
    blur::blur(&mut mask, sigma);
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::{MAX_LINES, MIN_FONT_SIZE_RATIO, measure, plan, stamp, width_budget, wrap};
    use crate::Error;
    use crate::font::Font;

    /// A4 portrait at 200 dpi, which is the default path's page.
    const A4_WIDTH: f32 = 1654.0;
    const A4_HEIGHT: f32 = 2339.0;

    /// A realistic French mark: the example ADR-0002 Decision 3 argues for.
    const REALISTIC: &str = "Document destiné exclusivement à une demande de location — 26/11/2026";

    fn font() -> Font<'static> {
        Font::bundled().expect("parses")
    }

    #[test]
    fn measuring_is_monotonic_and_scales_with_the_size() {
        let font = font();
        assert_eq!(measure(&font, "", 58.0), 0.0);
        let one = measure(&font, "M", 58.0);
        let two = measure(&font, "MM", 58.0);
        assert!((two - one * 2.0).abs() < 1e-3, "{one} then {two}");
        let double = measure(&font, "M", 116.0);
        assert!((double - one * 2.0).abs() < 1e-3);
    }

    #[test]
    fn the_realistic_mark_is_about_one_page_wide_at_the_default_size() {
        // The measurement behind ADR-0007's first ruling: at 3.5% of the
        // shorter edge a realistic French mark nearly fills the page, so the
        // horizontal tile gap rarely engages and the repetitions are rows.
        let font = font();
        let size = 0.035 * A4_WIDTH;
        let width = measure(&font, REALISTIC, size);
        let ratio = width / A4_WIDTH;
        assert!(
            (0.7..1.6).contains(&ratio),
            "the mark is {ratio:.2} pages wide at {size:.0} px; ADR-0007 says ~1"
        );
    }

    #[test]
    fn wrapping_respects_the_limit_and_keeps_every_word() {
        let font = font();
        let words: Vec<&str> = REALISTIC.split_whitespace().collect();
        for limit in [200.0_f32, 400.0, 800.0, 1600.0] {
            let lines = wrap(&font, &words, 58.0, limit);
            let rejoined = lines.join(" ");
            assert_eq!(rejoined, REALISTIC, "limit {limit} lost or reordered words");
            for line in &lines {
                let width = measure(&font, line, 58.0);
                let single_word = !line.contains(' ');
                assert!(
                    width <= limit || single_word,
                    "line {line:?} is {width} wide, over the {limit} limit"
                );
            }
        }
    }

    #[test]
    fn the_realistic_mark_fits_the_default_page_without_shrinking() {
        let font = font();
        let layout = plan(
            &font,
            REALISTIC,
            A4_WIDTH,
            A4_HEIGHT,
            0.035 * A4_WIDTH,
            -30.0,
        )
        .expect("a realistic mark must fit A4");
        assert!(!layout.shrunk, "it should not have needed shrinking");
        assert!(layout.lines.len() <= MAX_LINES);
        assert_eq!(layout.lines.join(" "), REALISTIC);
        let (across, down) = layout.rotated_extent(-30.0);
        assert!(
            across <= A4_WIDTH && down <= A4_HEIGHT,
            "rotated extent {across}x{down} does not fit {A4_WIDTH}x{A4_HEIGHT}"
        );
    }

    #[test]
    fn a_two_hundred_character_mark_wraps_and_shrinks_rather_than_overflowing() {
        // ADR-0002 allows up to 200 characters. On one line at the default size
        // that is several pages wide, so this is the case ADR-0007 rules on.
        let font = font();
        let text: String = core::iter::repeat_n("location", 25)
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text.chars().count(), 224);
        let layout = plan(&font, &text, A4_WIDTH, A4_HEIGHT, 0.035 * A4_WIDTH, -30.0)
            .expect("200 characters must be drawable, not refused");
        assert!(layout.shrunk, "it should have had to shrink");
        assert!(layout.lines.len() <= MAX_LINES, "{:?}", layout.lines);
        assert_eq!(layout.lines.join(" "), text, "no word may be dropped");
        assert!(
            layout.font_size_px >= MIN_FONT_SIZE_RATIO * A4_WIDTH,
            "shrank below the floor: {}",
            layout.font_size_px
        );
        let (across, down) = layout.rotated_extent(-30.0);
        assert!(across <= A4_WIDTH && down <= A4_HEIGHT);
    }

    #[test]
    fn nothing_is_ever_clipped() {
        // The property that matters: for every text that plans successfully,
        // the lines rejoin to exactly the input.
        let font = font();
        for text in [
            "A",
            "Document destiné à une banque",
            REALISTIC,
            "une phrase assez longue qui devra certainement passer à la ligne pour tenir",
        ] {
            let layout = plan(&font, text, A4_WIDTH, A4_HEIGHT, 0.035 * A4_WIDTH, -30.0)
                .unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(layout.lines.join(" "), text);
        }
    }

    #[test]
    fn an_unbreakable_word_that_cannot_fit_is_refused_not_clipped() {
        let font = font();
        // 400 characters with no space in them: no wrap can help, and at the
        // 1% floor it is still wider than the page.
        let text = "M".repeat(400);
        let error = plan(&font, &text, A4_WIDTH, A4_HEIGHT, 0.035 * A4_WIDTH, -30.0)
            .expect_err("this cannot be drawn honestly, so it must be refused");
        assert_eq!(error, Error::MarkDoesNotFit { characters: 400 });
        // And the message has to say what is wrong.
        let message = error.to_string();
        assert!(message.contains("400"), "{message}");
    }

    #[test]
    fn a_tiny_page_shrinks_rather_than_refusing() {
        // Worth stating as a property, because the arithmetic is not obvious:
        // shrinking to the floor buys a great deal of room, so text that looks
        // far too long is usually drawn rather than refused. Illegible is the
        // user's call to make, and the caller is told when the mark came out
        // much smaller than asked for; missing is ours, and is reserved for
        // text no size can fit.
        let font = font();
        let layout =
            plan(&font, REALISTIC, 40.0, 40.0, 1.4, -30.0).expect("even 40 pixels gets a mark");
        assert_eq!(layout.lines.join(" "), REALISTIC);
        assert!(layout.font_size_px >= MIN_FONT_SIZE_RATIO * 40.0);
    }

    #[test]
    fn a_degenerate_page_or_empty_text_is_a_parameter_error() {
        let font = font();
        assert_eq!(
            plan(&font, REALISTIC, 0.0, 100.0, 4.0, -30.0),
            Err(Error::InvalidParameter { name: "page size" })
        );
        for text in ["", "   ", "\t\n"] {
            assert_eq!(
                plan(&font, text, A4_WIDTH, A4_HEIGHT, 58.0, -30.0),
                Err(Error::InvalidParameter { name: "mark text" })
            );
        }
    }

    #[test]
    fn the_fit_budget_handles_the_axis_aligned_angles() {
        // 0°: only the height constraint bites on the page height.
        assert!(width_budget(1000.0, 500.0, 100.0, 0.0).is_some());
        assert_eq!(width_budget(1000.0, 50.0, 100.0, 0.0), None);
        // 90°: the block's height now runs across the page.
        assert!(width_budget(1000.0, 500.0, 100.0, 90.0).is_some());
        assert_eq!(width_budget(50.0, 500.0, 100.0, 90.0), None);
        // A block taller than the page diagonally gets no budget.
        assert_eq!(width_budget(100.0, 100.0, 1000.0, -30.0), None);
    }

    #[test]
    fn every_angle_in_adr_0002s_range_can_lay_out_a_mark() {
        let font = font();
        let mut angle = -90.0_f32;
        while angle <= 90.0 {
            plan(
                &font,
                REALISTIC,
                A4_WIDTH,
                A4_HEIGHT,
                0.035 * A4_WIDTH,
                angle,
            )
            .unwrap_or_else(|error| panic!("angle {angle}: {error}"));
            angle += 7.5;
        }
    }

    #[test]
    fn the_stamp_holds_the_text_with_room_for_the_blur() {
        let font = font();
        let layout = plan(
            &font,
            REALISTIC,
            A4_WIDTH,
            A4_HEIGHT,
            0.035 * A4_WIDTH,
            -30.0,
        )
        .expect("fits");
        let sigma = 0.015 * layout.font_size_px;
        let mask = stamp(&font, &layout, sigma).expect("small enough");
        mask.dump("stamp");

        let padding = super::blur::padding_for(sigma);
        assert!(mask.width() >= layout.block_width_px as u32);
        assert!(mask.height() >= layout.block_height_px as u32);

        // There is ink.
        let ink: u64 = mask.data().iter().map(|&v| u64::from(v)).sum();
        assert!(ink > 100_000, "the stamp is nearly empty: {ink}");

        // And the padding really is empty, so no glyph is cut off and the blur
        // has not run out of room.
        for x in 0..i64::from(mask.width()) {
            for y in 0..i64::from(padding) {
                assert_eq!(mask.at(x, y), 0, "ink in the top margin at ({x},{y})");
                let bottom = i64::from(mask.height()) - 1 - y;
                assert_eq!(mask.at(x, bottom), 0, "ink in the bottom margin");
            }
        }
    }

    #[test]
    fn a_multi_line_stamp_has_ink_on_every_line() {
        let font = font();
        let text: String = core::iter::repeat_n("Document destiné", 8)
            .collect::<Vec<_>>()
            .join(" ");
        let layout = plan(&font, &text, A4_WIDTH, A4_HEIGHT, 0.035 * A4_WIDTH, -30.0)
            .expect("fits after wrapping");
        assert!(layout.lines.len() > 1, "{:?}", layout.lines);
        let mask = stamp(&font, &layout, 0.0).expect("small");
        mask.dump("stamp-multiline");
        // Each baseline's row band must carry ink.
        for index in 0..layout.lines.len() {
            let baseline = layout.ascender_px + index as f32 * layout.line_height_px;
            let row = baseline as i64 - 2;
            let ink: u32 = (0..i64::from(mask.width()))
                .map(|x| u32::from(mask.at(x, row)))
                .sum();
            assert!(ink > 0, "line {index} has no ink near row {row}");
        }
    }

    #[test]
    fn laying_out_and_stamping_are_bit_identical_across_runs() {
        let font = font();
        let first = plan(&font, REALISTIC, A4_WIDTH, A4_HEIGHT, 57.89, -30.0).expect("fits");
        let second = plan(&font, REALISTIC, A4_WIDTH, A4_HEIGHT, 57.89, -30.0).expect("fits");
        assert_eq!(first, second);
        assert_eq!(
            stamp(&font, &first, 0.868).expect("small"),
            stamp(&font, &second, 0.868).expect("small")
        );
    }
}
