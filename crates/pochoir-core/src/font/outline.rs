//! `glyf` outlines: simple glyphs, composite glyphs, and the point
//! representation TrueType leaves half-implicit.
//!
//! A TrueType contour is a list of points, each flagged on-curve or off-curve,
//! with two conventions that have to be undone before anything can draw it:
//!
//! - **The contour is closed implicitly.** The last point joins the first and
//!   the file does not say so.
//! - **On-curve points between two off-curve points are implicit.** Two
//!   consecutive control points mean there is a knot at their midpoint. A
//!   contour may even have *no* on-curve point at all — a circle drawn as four
//!   controls — in which case the start is the midpoint of the last and first.
//!
//! Both are resolved here, so that [`Outline`] is an explicit list of lines and
//! quadratic Béziers and [`crate::raster`] never has to know the format.
//!
//! Reference: *The TrueType Reference Manual*, `glyf` table —
//! <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6glyf.html>
//! and the OpenType specification's `glyf` chapter —
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/glyf>

use crate::geom::{Point, Transform};
use crate::{Error, Result};

use super::Font;

fn bad(reason: &'static str) -> Error {
    Error::Decode {
        format: "TrueType font",
        reason,
    }
}

/// One curve segment, named by where it ends.
///
/// There is no "move to": a [`Contour`] carries its own start point, and the
/// closing segment back to that start is implicit — [`crate::raster`] always
/// draws it. A contour that did not close would leak its winding across the
/// rest of the buffer, so closure is structural rather than something each
/// producer has to remember.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// A straight line to this point.
    Line(Point),
    /// A quadratic Bézier through this control point, ending at this point.
    Quad(Point, Point),
}

impl Segment {
    /// Where this segment ends, whatever its shape.
    #[must_use]
    pub const fn end(&self) -> Point {
        match *self {
            Self::Line(end) | Self::Quad(_, end) => end,
        }
    }
}

/// One closed contour.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contour {
    /// The point the contour starts at, and implicitly returns to.
    pub start: Point,
    /// The segments, in order.
    pub segments: Vec<Segment>,
}

/// A glyph's whole outline: every contour, in font units, y-up.
///
/// Winding direction is as the font stored it — outer contours one way, holes
/// the other — and [`crate::raster`] relies on that, resolving overlap by
/// non-zero winding. Nothing here normalises or reorders contours.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outline {
    /// The contours.
    pub contours: Vec<Contour>,
}

impl Outline {
    /// `true` when there is nothing to draw, as for a space.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.contours.iter().all(|c| c.segments.is_empty())
    }

    /// The outline's bounding box in its own units, as `(min, max)`.
    ///
    /// Computed from the control points, so for a curve that bulges outward the
    /// box is conservative — larger than the painted ink, never smaller. That
    /// is the direction a buffer-sizing caller needs to be wrong in.
    #[must_use]
    pub fn control_bounds(&self) -> Option<(Point, Point)> {
        let mut bounds: Option<(Point, Point)> = None;
        let mut include = |p: Point| {
            bounds = Some(match bounds {
                None => (p, p),
                Some((min, max)) => (
                    Point::new(min.x.min(p.x), min.y.min(p.y)),
                    Point::new(max.x.max(p.x), max.y.max(p.y)),
                ),
            });
        };
        for contour in &self.contours {
            include(contour.start);
            for segment in &contour.segments {
                match *segment {
                    Segment::Line(end) => include(end),
                    Segment::Quad(control, end) => {
                        include(control);
                        include(end);
                    }
                }
            }
        }
        bounds
    }
}

/// Component-glyph flags, from the OpenType `glyf` chapter.
mod component {
    /// Both arguments are `i16` rather than `i8`.
    pub const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
    /// The arguments are an offset, not a pair of point indices.
    pub const ARGS_ARE_XY_VALUES: u16 = 0x0002;
    /// One `F2Dot14` scale for both axes.
    pub const WE_HAVE_A_SCALE: u16 = 0x0008;
    /// Another component follows this one.
    pub const MORE_COMPONENTS: u16 = 0x0020;
    /// Separate `F2Dot14` scales for x and y.
    pub const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
    /// A full `F2Dot14` 2×2 matrix.
    pub const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
}

/// Point flags, from the same chapter.
mod point_flag {
    /// The point is on the curve, not a control point.
    pub const ON_CURVE: u8 = 0x01;
    /// The x delta is one unsigned byte, its sign taken from `X_SAME_OR_POSITIVE`.
    pub const X_SHORT: u8 = 0x02;
    /// The y delta is one unsigned byte, its sign taken from `Y_SAME_OR_POSITIVE`.
    pub const Y_SHORT: u8 = 0x04;
    /// The next byte says how many extra times to repeat this flag.
    pub const REPEAT: u8 = 0x08;
    /// With `X_SHORT`: the byte is positive. Without it: x is unchanged.
    pub const X_SAME_OR_POSITIVE: u8 = 0x10;
    /// With `Y_SHORT`: the byte is positive. Without it: y is unchanged.
    pub const Y_SAME_OR_POSITIVE: u8 = 0x20;
}

/// `F2Dot14`: a signed 16-bit fixed-point number with 14 fractional bits.
fn f2dot14(raw: i16) -> f32 {
    f32::from(raw) / 16384.0
}

/// Parse one `glyf` entry and append its contours, transformed, to `out`.
///
/// `depth` is the composite recursion depth; [`Font::outline`] starts it at
/// zero and [`super::MAX_COMPOSITE_DEPTH`] bounds it.
pub(super) fn append(
    font: &Font<'_>,
    data: &[u8],
    transform: Transform,
    depth: u8,
    out: &mut Outline,
) -> Result<()> {
    let contour_count = read_i16(data, 0)?;
    if contour_count >= 0 {
        append_simple(
            data,
            usize::try_from(contour_count).unwrap_or(0),
            transform,
            out,
        )
    } else {
        append_composite(font, data, transform, depth, out)
    }
}

/// A glyph whose outline is in this entry.
fn append_simple(
    data: &[u8],
    contour_count: usize,
    transform: Transform,
    out: &mut Outline,
) -> Result<()> {
    if contour_count == 0 {
        return Ok(());
    }
    // Header is numberOfContours plus the four bounding-box values; the
    // bounding box is ignored, because `control_bounds` recomputes it and a
    // font's own figure is not something we need to trust.
    let mut at = 10;
    let mut contour_ends = Vec::with_capacity(contour_count);
    for _ in 0..contour_count {
        contour_ends.push(usize::from(read_u16(data, at)?));
        at += 2;
    }
    let point_count = contour_ends
        .last()
        .copied()
        .ok_or_else(|| bad("glyph claims contours but lists no endpoints"))?
        + 1;
    // Endpoints must be strictly increasing, or a contour would be empty or
    // run backwards through the point array.
    for pair in contour_ends.windows(2) {
        if pair[1] <= pair[0] {
            return Err(bad("contour endpoints are not increasing"));
        }
    }

    let instruction_length = usize::from(read_u16(data, at)?);
    at += 2 + instruction_length; // The hinting bytecode, which we never run.

    // Flags, run-length decoded.
    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = read_u8(data, at)?;
        at += 1;
        flags.push(flag);
        if flag & point_flag::REPEAT != 0 {
            let extra = usize::from(read_u8(data, at)?);
            at += 1;
            if flags.len() + extra > point_count {
                return Err(bad("a repeated point flag overruns the point count"));
            }
            for _ in 0..extra {
                flags.push(flag);
            }
        }
    }

    // Coordinates: all the x deltas, then all the y deltas, each either absent
    // (same as previous), one unsigned byte with a sign bit elsewhere, or a
    // signed word.
    let mut xs = Vec::with_capacity(point_count);
    let mut x = 0i32;
    for &flag in &flags {
        if flag & point_flag::X_SHORT != 0 {
            let magnitude = i32::from(read_u8(data, at)?);
            at += 1;
            x += if flag & point_flag::X_SAME_OR_POSITIVE != 0 {
                magnitude
            } else {
                -magnitude
            };
        } else if flag & point_flag::X_SAME_OR_POSITIVE == 0 {
            x += i32::from(read_i16(data, at)?);
            at += 2;
        }
        xs.push(x);
    }
    let mut ys = Vec::with_capacity(point_count);
    let mut y = 0i32;
    for &flag in &flags {
        if flag & point_flag::Y_SHORT != 0 {
            let magnitude = i32::from(read_u8(data, at)?);
            at += 1;
            y += if flag & point_flag::Y_SAME_OR_POSITIVE != 0 {
                magnitude
            } else {
                -magnitude
            };
        } else if flag & point_flag::Y_SAME_OR_POSITIVE == 0 {
            y += i32::from(read_i16(data, at)?);
            at += 2;
        }
        ys.push(y);
    }

    let mut first = 0usize;
    for &last in &contour_ends {
        let points: Vec<(Point, bool)> = (first..=last)
            .map(|index| {
                let point = Point::new(xs[index] as f32, ys[index] as f32);
                (
                    transform.apply(point),
                    flags[index] & point_flag::ON_CURVE != 0,
                )
            })
            .collect();
        if let Some(contour) = contour_from_points(&points) {
            out.contours.push(contour);
        }
        first = last + 1;
    }
    Ok(())
}

/// Turn TrueType's flagged point list into explicit segments.
///
/// This is where both of the module-level conventions are undone.
fn contour_from_points(points: &[(Point, bool)]) -> Option<Contour> {
    let count = points.len();
    if count < 2 {
        return None;
    }
    // Start at the first on-curve point if there is one. If there is not, every
    // point is a control point and the curve passes through the midpoint of the
    // last and the first, so that is the start.
    let (start, from) = match points.iter().position(|&(_, on_curve)| on_curve) {
        Some(index) => (points[index].0, index + 1),
        None => (points[count - 1].0.midpoint(points[0].0), 0),
    };

    let mut segments: Vec<Segment> = Vec::with_capacity(count);
    let mut pending: Option<Point> = None;
    // `count` steps from `from` visits every point exactly once and, in the
    // on-curve case, ends on the start point — which closes the contour.
    for step in 0..count {
        let (point, on_curve) = points[(from + step) % count];
        if on_curve {
            match pending.take() {
                Some(control) => segments.push(Segment::Quad(control, point)),
                None => segments.push(Segment::Line(point)),
            }
        } else {
            if let Some(control) = pending.take() {
                // Two controls in a row: the implied knot is their midpoint.
                segments.push(Segment::Quad(control, control.midpoint(point)));
            }
            pending = Some(point);
        }
    }
    // Only reachable when the contour has no on-curve point at all, or ends on
    // a control point; either way the curve returns to where it began.
    if let Some(control) = pending {
        segments.push(Segment::Quad(control, start));
    }
    (!segments.is_empty()).then_some(Contour { start, segments })
}

/// A glyph assembled from other glyphs — `é` from `e` and `acute`.
fn append_composite(
    font: &Font<'_>,
    data: &[u8],
    transform: Transform,
    depth: u8,
    out: &mut Outline,
) -> Result<()> {
    let mut at = 10;
    loop {
        let flags = read_u16(data, at)?;
        let glyph = read_u16(data, at + 2)?;
        at += 4;

        let (dx, dy) = if flags & component::ARG_1_AND_2_ARE_WORDS != 0 {
            let pair = (read_i16(data, at)?, read_i16(data, at + 2)?);
            at += 4;
            (f32::from(pair.0), f32::from(pair.1))
        } else {
            let pair = (read_i8(data, at)?, read_i8(data, at + 1)?);
            at += 2;
            (f32::from(pair.0), f32::from(pair.1))
        };

        if flags & component::ARGS_ARE_XY_VALUES == 0 {
            // The alternative is point matching: the arguments are indices of a
            // point in this glyph and a point in the component, to be made
            // coincident. No font in Latin use does this for accents, the
            // bundled subset does not, and implementing it would mean keeping
            // every component's point array alive through the recursion.
            return Err(bad("composite glyph uses point matching, which we do not"));
        }

        let linear = if flags & component::WE_HAVE_A_SCALE != 0 {
            let scale = f2dot14(read_i16(data, at)?);
            at += 2;
            Transform::linear(scale, 0.0, 0.0, scale)
        } else if flags & component::WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let x = f2dot14(read_i16(data, at)?);
            let y = f2dot14(read_i16(data, at + 2)?);
            at += 4;
            Transform::linear(x, 0.0, 0.0, y)
        } else if flags & component::WE_HAVE_A_TWO_BY_TWO != 0 {
            let m = Transform::linear(
                f2dot14(read_i16(data, at)?),
                f2dot14(read_i16(data, at + 2)?),
                f2dot14(read_i16(data, at + 4)?),
                f2dot14(read_i16(data, at + 6)?),
            );
            at += 8;
            m
        } else {
            Transform::IDENTITY
        };

        // The component's points are scaled, then offset, then put through
        // whatever transform brought us here.
        let component = linear.then(&Transform::translate(dx, dy)).then(&transform);
        font.append_outline(glyph, component, depth + 1, out)?;

        if flags & component::MORE_COMPONENTS == 0 {
            return Ok(());
        }
    }
}

fn read_u8(data: &[u8], at: usize) -> Result<u8> {
    data.get(at).copied().ok_or_else(|| bad("glyph ends early"))
}

fn read_i8(data: &[u8], at: usize) -> Result<i8> {
    read_u8(data, at).map(|v| v as i8)
}

fn read_u16(data: &[u8], at: usize) -> Result<u16> {
    let hi = read_u8(data, at)?;
    let lo = read_u8(data, at + 1)?;
    Ok(u16::from_be_bytes([hi, lo]))
}

fn read_i16(data: &[u8], at: usize) -> Result<i16> {
    read_u16(data, at).map(|v| v as i16)
}

#[cfg(test)]
mod tests {
    use super::{Contour, Outline, Point, Segment, contour_from_points, f2dot14};
    use crate::font::Font;

    #[test]
    fn f2dot14_reads_the_documented_values() {
        // The OpenType spec's own worked examples for F2Dot14.
        assert_eq!(f2dot14(0x7000), 1.75);
        assert_eq!(f2dot14(0x0001), 1.0 / 16384.0);
        assert_eq!(f2dot14(0x0000), 0.0);
        assert_eq!(f2dot14(-16384), -1.0);
        assert_eq!(f2dot14(16384), 1.0);
    }

    fn on(x: f32, y: f32) -> (Point, bool) {
        (Point::new(x, y), true)
    }

    fn off(x: f32, y: f32) -> (Point, bool) {
        (Point::new(x, y), false)
    }

    #[test]
    fn a_triangle_of_on_curve_points_becomes_three_lines() {
        let contour = contour_from_points(&[on(0.0, 0.0), on(10.0, 0.0), on(0.0, 10.0)])
            .expect("three points are a contour");
        assert_eq!(contour.start, Point::new(0.0, 0.0));
        assert_eq!(
            contour.segments,
            vec![
                Segment::Line(Point::new(10.0, 0.0)),
                Segment::Line(Point::new(0.0, 10.0)),
                Segment::Line(Point::new(0.0, 0.0)),
            ],
            "the last segment must return to the start"
        );
    }

    #[test]
    fn two_consecutive_controls_imply_a_knot_at_their_midpoint() {
        // on, off, off, on: one quad to the implied midpoint, then one more.
        let contour =
            contour_from_points(&[on(0.0, 0.0), off(10.0, 0.0), off(10.0, 10.0), on(0.0, 10.0)])
                .expect("a contour");
        assert_eq!(
            contour.segments,
            vec![
                Segment::Quad(Point::new(10.0, 0.0), Point::new(10.0, 5.0)),
                Segment::Quad(Point::new(10.0, 10.0), Point::new(0.0, 10.0)),
                Segment::Line(Point::new(0.0, 0.0)),
            ]
        );
    }

    #[test]
    fn a_contour_with_no_on_curve_point_starts_at_a_midpoint() {
        // Four controls, as a font may draw a circle.
        let contour =
            contour_from_points(&[off(-1.0, 0.0), off(0.0, 1.0), off(1.0, 0.0), off(0.0, -1.0)])
                .expect("a contour");
        // Start is the midpoint of the last and first points.
        assert_eq!(contour.start, Point::new(-0.5, -0.5));
        assert_eq!(contour.segments.len(), 4);
        assert_eq!(
            contour.segments.last().expect("segments").end(),
            contour.start,
            "the curve must come back to where it began"
        );
    }

    #[test]
    fn every_contour_is_closed() {
        let font = Font::bundled().expect("parses");
        for character in "aeiouAEIOU0123456789éèçàÉÀ@&#%".chars() {
            let glyph = font.glyph_for(character).expect("in the subset");
            let outline = font.outline(glyph).expect("an outline");
            for (index, contour) in outline.contours.iter().enumerate() {
                let end = contour
                    .segments
                    .last()
                    .expect("a contour has segments")
                    .end();
                assert_eq!(
                    end, contour.start,
                    "{character:?} contour {index} ends at {end:?}, not at its start"
                );
            }
        }
    }

    #[test]
    fn an_accented_letter_is_a_composite_of_its_parts() {
        let font = Font::bundled().expect("parses");
        let e = font
            .outline(font.glyph_for('e').expect("e"))
            .expect("outline");
        let acute = font
            .outline(font.glyph_for('\u{00B4}').expect("acute accent"))
            .expect("outline");
        let e_acute = font
            .outline(font.glyph_for('é').expect("é"))
            .expect("outline");

        // `e` is two contours (the bowl and its counter), the accent is one, so
        // the composite must carry all three. This is the assertion that
        // composite parsing is actually happening rather than silently
        // producing an empty glyph.
        assert_eq!(e.contours.len(), 2, "`e` should be a bowl and a counter");
        assert_eq!(
            e_acute.contours.len(),
            e.contours.len() + acute.contours.len(),
            "`é` should be `e` plus the accent"
        );

        // And the accent must sit above the `e`: the composite is taller.
        let (_, e_max) = e.control_bounds().expect("bounds");
        let (_, ea_max) = e_acute.control_bounds().expect("bounds");
        assert!(
            ea_max.y > e_max.y,
            "`é` tops out at {} but `e` already reaches {}",
            ea_max.y,
            e_max.y
        );
    }

    #[test]
    fn every_glyph_in_the_subset_parses() {
        let font = Font::bundled().expect("parses");
        for glyph in 0..font.glyph_count() {
            font.outline(glyph)
                .unwrap_or_else(|error| panic!("glyph {glyph} failed to parse: {error}"));
        }
    }

    #[test]
    fn control_bounds_of_nothing_is_nothing() {
        assert_eq!(Outline::default().control_bounds(), None);
        assert!(Outline::default().is_empty());
        let outline = Outline {
            contours: vec![Contour {
                start: Point::new(1.0, 2.0),
                segments: vec![Segment::Line(Point::new(-3.0, 9.0))],
            }],
        };
        assert_eq!(
            outline.control_bounds(),
            Some((Point::new(-3.0, 2.0), Point::new(1.0, 9.0)))
        );
    }
}
