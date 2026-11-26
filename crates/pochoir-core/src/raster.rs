//! A scanline rasteriser with analytic antialiasing, over lines and quadratic
//! Béziers.
//!
//! # The technique
//!
//! There is no supersampling and no active-edge list. Instead there is one
//! `f32` accumulation buffer the size of the target, and each edge adds the
//! *derivative along x* of its own contribution to the winding number. A single
//! linear prefix sum over the whole buffer then turns those derivatives into
//! per-pixel signed winding, and `min(|w|, 1)` is the coverage. Antialiasing
//! comes out of the arithmetic rather than being approximated, because what
//! each edge deposits is the exact area it covers in each cell.
//!
//! This is the signed-area accumulation technique Raph Levien described and
//! published in **`font-rs`** (2016,
//! <https://github.com/raphlinus/font-rs>, Apache-2.0). The implementation here
//! is written from the description and from the area integral below, not copied
//! from that code; the derivation is ours and the per-cell weights are stated
//! in closed form so a reader can check them.
//!
//! # The derivation, because the weights are otherwise unreadable
//!
//! Take one edge and one scanline row `y`. Within that row the edge spans a
//! vertical extent `dy` and its x goes from `xa` to `xb`; write
//! `x0 = min(xa, xb)` and `x1 = max(xa, xb)`. Let `edge_x(Y)` be the edge's x at
//! height `Y` and define
//!
//! ```text
//! G(u) = ∫ max(0, u - edge_x(Y)) dY     over the row's vertical extent
//!
//!      = 0                              u ≤ x0
//!      = dy · (u - x0)² / (2 (x1 - x0))  x0 < u < x1
//!      = dy · (u - (x0 + x1) / 2)        u ≥ x1
//! ```
//!
//! The area of pixel column `i` lying to the right of the edge within this row
//! is `G(i+1) - 2·G(i) + G(i-1)` — the second difference, because clamping the
//! horizontal extent to one pixel's width is `max(0,t) - max(0,t-1)` and taking
//! a per-column difference costs one more. So that second difference, signed by
//! the edge's direction, is exactly what each cell accumulates. Being a second
//! difference of a piecewise-linear-then-quadratic function it is **zero** once
//! the column is clear of the edge, which is what makes the prefix sum settle
//! instead of running away.
//!
//! Only `+ - * /`, `min`, `max`, `floor` and `ceil` appear, every one of them
//! exactly specified by IEEE-754, so the same outline rasterises bit-identically
//! on wasm32 and on native. That is ADR-0002 Decision 6's requirement and it is
//! why there is no `sin`, no `powf` and no `round` anywhere in this file.
//!
//! # Two things that will cost an hour each if forgotten
//!
//! - **The buffer is wider than the target.** A cell index reaches
//!   `ceil(x1)`, one past the last column an edge touches, so each row carries
//!   [`SLACK`] spare cells. They are summed through and never read.
//! - **A contour that does not close smears the rest of the image.** The prefix
//!   sum is linear over the entire buffer, not restarted per row, which is only
//!   sound because every closed contour's accumulations cancel to zero by the
//!   end of its row. [`Coverage::fill`] therefore always draws the closing edge
//!   back to [`Contour::start`] itself rather than trusting the caller.

use crate::font::{Contour, Outline, Segment};
use crate::geom::{Point, Transform};
use crate::{Error, Result};

/// Spare cells at the end of each accumulation row.
///
/// One is needed, for the cell at `ceil(x1)`; the second is insurance against
/// a rounding pathology at the right edge and costs four bytes a row.
pub const SLACK: usize = 2;

/// How far a flattened quadratic may stray from the true curve, in pixels.
///
/// A tenth of a pixel is below what the antialiasing can express, so the
/// flattening is not what limits quality. Halving it roughly doubles the
/// segment count for no visible gain.
pub const FLATNESS: f32 = 0.1;

/// Most line segments one quadratic is ever split into.
///
/// At [`FLATNESS`] this bound is only approached by a curve spanning hundreds
/// of pixels, which a glyph at a watermark's size never is. It exists so that a
/// pathological control point cannot make the flattener allocate without end.
pub const MAX_FLATTEN_STEPS: u32 = 128;

/// The accumulation buffer an outline is drawn into.
///
/// Create it, [`fill`] it with as many outlines as you like — they accumulate,
/// which is how a line of text becomes one mask — then [`resolve`] it.
///
/// [`fill`]: Coverage::fill
/// [`resolve`]: Coverage::resolve
#[derive(Debug, Clone)]
pub struct Coverage {
    width: u32,
    height: u32,
    stride: usize,
    cells: Vec<f32>,
}

impl Coverage {
    /// A zeroed buffer for a `width` by `height` mask.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] for a buffer past
    /// [`crate::pixmap::MAX_PIXELS`], which is the same bound the rest of the
    /// pipeline uses.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > crate::pixmap::MAX_PIXELS {
            return Err(Error::ImageTooLarge { width, height });
        }
        let stride = width as usize + SLACK;
        let cells = usize::try_from(u64::from(height) * stride as u64)
            .map_err(|_| Error::ImageTooLarge { width, height })?;
        Ok(Self {
            width,
            height,
            stride,
            cells: vec![0.0; cells],
        })
    }

    /// Width of the mask this will resolve to.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of the mask this will resolve to.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Accumulate every contour of `outline`, mapped through `transform`.
    ///
    /// Contours are closed here whether or not the caller closed them, for the
    /// reason in the module documentation.
    pub fn fill(&mut self, outline: &Outline, transform: &Transform) {
        for contour in &outline.contours {
            self.fill_contour(contour, transform);
        }
    }

    fn fill_contour(&mut self, contour: &Contour, transform: &Transform) {
        if self.height == 0 || self.width == 0 {
            return;
        }
        let start = transform.apply(contour.start);
        let mut from = start;
        for segment in &contour.segments {
            match *segment {
                Segment::Line(end) => {
                    let end = transform.apply(end);
                    self.line(from, end);
                    from = end;
                }
                Segment::Quad(control, end) => {
                    let control = transform.apply(control);
                    let end = transform.apply(end);
                    self.quad(from, control, end);
                    from = end;
                }
            }
        }
        // The closing edge. Zero length when the contour was already closed, in
        // which case `line` returns immediately.
        self.line(from, start);
    }

    /// Flatten a quadratic into lines and accumulate them.
    fn quad(&mut self, from: Point, control: Point, to: Point) {
        // The curve's greatest departure from its chord is |p0 - 2c + p2| / 4,
        // and splitting into n pieces divides it by n², so n = ceil(sqrt(d/t)).
        let dx = from.x - 2.0 * control.x + to.x;
        let dy = from.y - 2.0 * control.y + to.y;
        let deviation = (dx * dx + dy * dy).sqrt() * 0.25;
        let steps = if deviation <= FLATNESS {
            1
        } else {
            let exact = (deviation / FLATNESS).sqrt().ceil();
            // `exact` is finite and ≥ 1 here; the cast is saturating in Rust.
            (exact as u32).clamp(1, MAX_FLATTEN_STEPS)
        };
        let inverse = 1.0 / steps as f32;
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 * inverse;
            let s = 1.0 - t;
            // de Casteljau, written out: no powf.
            let point = Point::new(
                s * s * from.x + 2.0 * s * t * control.x + t * t * to.x,
                s * s * from.y + 2.0 * s * t * control.y + t * t * to.y,
            );
            self.line(previous, point);
            previous = point;
        }
    }

    /// Accumulate one straight edge. The whole algorithm is here.
    fn line(&mut self, p0: Point, p1: Point) {
        if !(p0.x.is_finite() && p0.y.is_finite() && p1.x.is_finite() && p1.y.is_finite()) {
            return;
        }
        if p0.y == p1.y {
            return; // A horizontal edge contributes no winding.
        }
        // Order the endpoints downwards and remember which way the edge ran:
        // that sign is the winding contribution.
        let (direction, top, bottom) = if p0.y < p1.y {
            (1.0_f32, p0, p1)
        } else {
            (-1.0_f32, p1, p0)
        };

        let limit = self.width as f32;
        let height = self.height as f32;
        // Clamp x into the buffer. For a mask sized to hold its own text this
        // never engages; for an outline that does run off the edge it shears
        // the offending edge rather than writing out of bounds, which is the
        // right way to be wrong about something already out of frame.
        let x_top = top.x.clamp(0.0, limit);
        let x_bottom = bottom.x.clamp(0.0, limit);
        let dx_dy = (x_bottom - x_top) / (bottom.y - top.y);

        // Both are finite: the endpoints were checked above and `height` is a
        // pixel count, so this comparison cannot see a NaN.
        let first_row = top.y.max(0.0).floor();
        let last_row = bottom.y.min(height).ceil();
        if first_row >= last_row {
            return;
        }
        let mut row = first_row as u32;
        let row_end = last_row as u32;
        while row < row_end && row < self.height {
            let row_top = (row as f32).max(top.y);
            let row_bottom = ((row + 1) as f32).min(bottom.y);
            let dy = row_bottom - row_top;
            if dy > 0.0 {
                let xa = x_top + (row_top - top.y) * dx_dy;
                let xb = x_top + (row_bottom - top.y) * dx_dy;
                self.accumulate_row(row, dy * direction, xa.min(xb), xa.max(xb));
            }
            row += 1;
        }
    }

    /// Deposit one edge's contribution across one row of cells.
    ///
    /// `signed_dy` is the vertical extent times the winding direction; `x0` and
    /// `x1` bracket the edge's horizontal travel within the row.
    fn accumulate_row(&mut self, row: u32, signed_dy: f32, x0: f32, x1: f32) {
        // G from the module documentation, for this row.
        let span = x1 - x0;
        let midpoint = (x0 + x1) * 0.5;
        let g = |u: f32| -> f32 {
            if u <= x0 {
                0.0
            } else if u >= x1 {
                u - midpoint
            } else {
                let t = u - x0;
                t * t / (2.0 * span)
            }
        };

        let lower = x0.floor().max(0.0);
        let upper = x1.ceil().min((self.stride - 1) as f32);
        if lower > upper {
            return;
        }
        let base = row as usize * self.stride;
        let mut column = lower as usize;
        let last = upper as usize;
        // Carry two values of G along rather than evaluating it three times a
        // cell.
        let mut g_minus = g(lower - 1.0);
        let mut g_here = g(lower);
        while column <= last {
            let g_plus = g(column as f32 + 1.0);
            let area = g_plus - 2.0 * g_here + g_minus;
            if let Some(cell) = self.cells.get_mut(base + column) {
                *cell += signed_dy * area;
            }
            g_minus = g_here;
            g_here = g_plus;
            column += 1;
        }
    }

    /// Prefix-sum the accumulation into an 8-bit coverage mask.
    ///
    /// Non-zero winding: a pixel inside an odd overlap and a pixel inside a
    /// doubly-wound one are both fully covered, and a counter — the inside of
    /// an `o` — is wound the other way and cancels.
    #[must_use]
    pub fn resolve(&self) -> Mask {
        let mut data = vec![0u8; self.width as usize * self.height as usize];
        let mut winding = 0.0_f32;
        for row in 0..self.height as usize {
            let base = row * self.stride;
            let out = row * self.width as usize;
            for column in 0..self.width as usize {
                winding += self.cells[base + column];
                // `+ 0.5` then `floor`, never `round`: ADR-0002 Decision 6
                // bans `f32::round`, which lowers to a libm call on some
                // targets. The same trick is in `jpeg/dct.rs` for the same
                // reason.
                let coverage = winding.abs().min(1.0);
                data[out + column] = (coverage * 255.0 + 0.5).floor() as u8;
            }
            // Sum through the slack so a closed contour's row cancels exactly.
            for column in self.width as usize..self.stride {
                winding += self.cells[base + column];
            }
        }
        Mask {
            width: self.width,
            height: self.height,
            data,
        }
    }
}

/// An 8-bit coverage mask: 0 is uncovered, 255 fully covered.
///
/// Row-major, one byte per pixel, no padding. This is the watermark's "stamp"
/// once text has been laid out and blurred into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Mask {
    /// An all-zero mask.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] past [`crate::pixmap::MAX_PIXELS`].
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > crate::pixmap::MAX_PIXELS {
            return Err(Error::ImageTooLarge { width, height });
        }
        let len = usize::try_from(pixels).map_err(|_| Error::ImageTooLarge { width, height })?;
        Ok(Self {
            width,
            height,
            data: vec![0; len],
        })
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

    /// The coverage bytes, row-major.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The coverage bytes, mutably — for the blur, which works in place.
    #[must_use]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Coverage at integer coordinates, zero outside the mask.
    #[must_use]
    pub fn at(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return 0;
        }
        self.data[y as usize * self.width as usize + x as usize]
    }

    /// Coverage at a fractional position, bilinearly interpolated, as `0.0..=1.0`.
    ///
    /// Samples outside the mask read as zero, so a rotated stamp fades off its
    /// own edge instead of wrapping or clamping into a smear.
    #[must_use]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        if !(x.is_finite() && y.is_finite()) {
            return 0.0;
        }
        // Sample positions are pixel centres, so the texel at index 0 is at 0.5.
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (xi, yi) = (x0 as i64, y0 as i64);
        let c00 = f32::from(self.at(xi, yi));
        let c10 = f32::from(self.at(xi + 1, yi));
        let c01 = f32::from(self.at(xi, yi + 1));
        let c11 = f32::from(self.at(xi + 1, yi + 1));
        let top = c00 + (c10 - c00) * tx;
        let bottom = c01 + (c11 - c01) * tx;
        (top + (bottom - top) * ty) / 255.0
    }

    /// Render an outline straight into a mask of the given size.
    ///
    /// A convenience over [`Coverage`] for the single-outline case.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ImageTooLarge`] for an oversized buffer.
    pub fn from_outline(
        width: u32,
        height: u32,
        outline: &Outline,
        transform: &Transform,
    ) -> Result<Self> {
        let mut coverage = Coverage::new(width, height)?;
        coverage.fill(outline, transform);
        Ok(coverage.resolve())
    }

    /// The mask as a binary PGM (`P5`) image.
    ///
    /// For looking at things. A grey PGM is eight lines of header and the bytes
    /// we already have, and `open`, Preview, GIMP and ImageMagick all read it.
    /// Test-only: nothing in the shipped engine writes an image format other
    /// than what [`crate::pdf`] embeds.
    #[cfg(test)]
    pub(crate) fn to_pgm(&self) -> Vec<u8> {
        let mut out = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend_from_slice(&self.data);
        out
    }

    /// Write [`to_pgm`] into `$POCHOIR_DUMP_DIR/<name>.pgm`, if that is set.
    ///
    /// The eyeball hook. `POCHOIR_DUMP_DIR=/tmp/x cargo test -p pochoir-core`
    /// and every test that calls this leaves a picture behind; unset, it does
    /// nothing, so CI stays clean.
    ///
    /// [`to_pgm`]: Mask::to_pgm
    #[cfg(test)]
    pub(crate) fn dump(&self, name: &str) {
        if let Ok(directory) = std::env::var("POCHOIR_DUMP_DIR") {
            let path = std::path::Path::new(&directory).join(format!("{name}.pgm"));
            let _ = std::fs::create_dir_all(&directory);
            if std::fs::write(&path, self.to_pgm()).is_ok() {
                println!("wrote {}", path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Coverage, Mask};
    use crate::font::{Contour, Font, Outline, Segment};
    use crate::geom::{Point, Transform};

    /// A closed rectangle, wound clockwise in device space.
    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Outline {
        Outline {
            contours: vec![Contour {
                start: Point::new(x0, y0),
                segments: vec![
                    Segment::Line(Point::new(x1, y0)),
                    Segment::Line(Point::new(x1, y1)),
                    Segment::Line(Point::new(x0, y1)),
                    Segment::Line(Point::new(x0, y0)),
                ],
            }],
        }
    }

    #[test]
    fn a_pixel_aligned_rectangle_is_exactly_covered() {
        let mask = Mask::from_outline(6, 4, &rectangle(1.0, 1.0, 4.0, 3.0), &Transform::IDENTITY)
            .expect("small");
        for y in 0..4 {
            for x in 0..6 {
                let inside = (1..4).contains(&x) && (1..3).contains(&y);
                let expected = if inside { 255 } else { 0 };
                assert_eq!(
                    mask.at(x, y),
                    expected,
                    "({x},{y}) should be {expected}, mask is\n{mask:?}"
                );
            }
        }
    }

    #[test]
    fn a_half_pixel_edge_gives_half_coverage() {
        // x from 0.5 to 2.0: column 0 half covered, column 1 full.
        let mask = Mask::from_outline(3, 1, &rectangle(0.5, 0.0, 2.0, 1.0), &Transform::IDENTITY)
            .expect("small");
        assert_eq!(mask.at(0, 0), 128, "0.5 coverage rounds to 128");
        assert_eq!(mask.at(1, 0), 255);
        assert_eq!(mask.at(2, 0), 0);
    }

    #[test]
    fn a_quarter_pixel_square_covers_a_quarter_of_one_pixel() {
        let mask = Mask::from_outline(2, 2, &rectangle(0.0, 0.0, 0.5, 0.5), &Transform::IDENTITY)
            .expect("small");
        assert_eq!(
            mask.at(0, 0),
            64,
            "a quarter of 255 is 63.75, rounding to 64"
        );
        assert_eq!(mask.at(1, 0), 0);
        assert_eq!(mask.at(0, 1), 0);
    }

    #[test]
    fn winding_direction_does_not_change_the_coverage() {
        let clockwise = rectangle(1.0, 1.0, 4.0, 3.0);
        let anticlockwise = Outline {
            contours: vec![Contour {
                start: Point::new(1.0, 1.0),
                segments: vec![
                    Segment::Line(Point::new(1.0, 3.0)),
                    Segment::Line(Point::new(4.0, 3.0)),
                    Segment::Line(Point::new(4.0, 1.0)),
                    Segment::Line(Point::new(1.0, 1.0)),
                ],
            }],
        };
        let a = Mask::from_outline(6, 4, &clockwise, &Transform::IDENTITY).expect("small");
        let b = Mask::from_outline(6, 4, &anticlockwise, &Transform::IDENTITY).expect("small");
        assert_eq!(a, b, "|winding| must not depend on the direction of travel");
    }

    #[test]
    fn an_opposed_inner_contour_is_a_hole() {
        let mut outer = rectangle(0.0, 0.0, 6.0, 6.0);
        // An inner square wound the other way: non-zero winding cancels it.
        outer.contours.push(Contour {
            start: Point::new(2.0, 2.0),
            segments: vec![
                Segment::Line(Point::new(2.0, 4.0)),
                Segment::Line(Point::new(4.0, 4.0)),
                Segment::Line(Point::new(4.0, 2.0)),
                Segment::Line(Point::new(2.0, 2.0)),
            ],
        });
        let mask = Mask::from_outline(6, 6, &outer, &Transform::IDENTITY).expect("small");
        assert_eq!(mask.at(0, 0), 255, "the frame is filled");
        assert_eq!(mask.at(3, 3), 0, "the counter is empty");
    }

    #[test]
    fn the_prefix_sum_does_not_leak_past_a_closed_contour() {
        // The bug this guards against smears every pixel after the shape.
        let mask = Mask::from_outline(32, 4, &rectangle(1.0, 1.0, 3.0, 3.0), &Transform::IDENTITY)
            .expect("small");
        for y in 0..4 {
            for x in 4..32 {
                assert_eq!(mask.at(x, y), 0, "({x},{y}) picked up stray winding");
            }
        }
    }

    #[test]
    fn an_outline_running_off_the_buffer_does_not_panic() {
        for shape in [
            rectangle(-100.0, -100.0, 200.0, 200.0),
            rectangle(-50.0, 1.0, -10.0, 3.0),
            rectangle(100.0, 1.0, 200.0, 3.0),
            rectangle(1.0, -50.0, 3.0, -10.0),
            rectangle(f32::NAN, 0.0, 1.0, 1.0),
            rectangle(0.0, 0.0, f32::INFINITY, 1.0),
        ] {
            let mask = Mask::from_outline(8, 4, &shape, &Transform::IDENTITY).expect("small");
            assert_eq!(mask.width(), 8);
        }
    }

    #[test]
    fn a_curve_is_flattened_finely_enough_to_look_round() {
        // A quarter disc of radius 8 drawn as one quadratic, as a font would.
        let outline = Outline {
            contours: vec![Contour {
                start: Point::new(0.0, 8.0),
                segments: vec![
                    Segment::Quad(Point::new(0.0, 0.0), Point::new(8.0, 0.0)),
                    Segment::Line(Point::new(0.0, 0.0)),
                    Segment::Line(Point::new(0.0, 8.0)),
                ],
            }],
        };
        let mask = Mask::from_outline(10, 10, &outline, &Transform::IDENTITY).expect("small");
        mask.dump("quarter-disc");
        // The curve is the parabola x(t) = 8t², y(t) = 8(1-t)², so the filled
        // side is √x + √y < √8: the corner at the origin is in, (3,3) is out.
        assert_eq!(mask.at(0, 0), 255);
        assert_eq!(mask.at(1, 1), 255);
        assert_eq!(mask.at(3, 3), 0);
        assert_eq!(mask.at(9, 9), 0);
        // And the boundary is antialiased rather than stepped: some pixel must
        // hold a partial value. A jaggy fill would be all 0 and 255.
        let partial = mask.data().iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(
            partial > 4,
            "only {partial} partially covered pixels: the edge is not antialiased"
        );
    }

    #[test]
    fn empty_buffers_are_legal_and_draw_nothing() {
        let mask = Mask::from_outline(0, 0, &rectangle(0.0, 0.0, 1.0, 1.0), &Transform::IDENTITY)
            .expect("zero is a legal size");
        assert_eq!(mask.data().len(), 0);
        let mut coverage = Coverage::new(4, 0).expect("zero height");
        coverage.fill(&rectangle(0.0, 0.0, 1.0, 1.0), &Transform::IDENTITY);
        assert_eq!(coverage.resolve().data().len(), 0);
    }

    #[test]
    fn sampling_is_bilinear_and_zero_outside() {
        let mut mask = Mask::new(2, 1).expect("small");
        mask.data_mut()[0] = 0;
        mask.data_mut()[1] = 255;
        // Pixel centres are at 0.5 and 1.5.
        assert!((mask.sample(0.5, 0.5) - 0.0).abs() < 1e-6);
        assert!((mask.sample(1.5, 0.5) - 1.0).abs() < 1e-6);
        assert!((mask.sample(1.0, 0.5) - 0.5).abs() < 1e-3);
        // Outside, and off the end, read as nothing.
        assert_eq!(mask.sample(-5.0, 0.5), 0.0);
        assert_eq!(mask.sample(50.0, 0.5), 0.0);
        assert_eq!(mask.sample(f32::NAN, 0.0), 0.0);
    }

    /// The day-one test: an `é` rendered from the bundled font, big enough to
    /// look at. It is a composite glyph, so this exercises the composite path,
    /// the cmap, the flattener and the rasteriser at once.
    #[test]
    fn e_acute_renders_as_a_letter_with_an_accent_above_it() {
        let font = Font::bundled().expect("parses");
        let glyph = font.glyph_for('é').expect("é is in the subset");
        let outline = font.outline(glyph).expect("an outline");

        let size = 64.0_f32;
        let scale = size / f32::from(font.units_per_em());
        // Put the baseline three quarters of the way down a 64-pixel box.
        let transform = Transform::scale_flip_y(scale).then(&Transform::translate(6.0, 48.0));
        let mask = Mask::from_outline(48, 64, &outline, &transform).expect("small");
        mask.dump("e-acute");

        // Ink exists.
        let ink: u32 = mask.data().iter().map(|&v| u32::from(v)).sum();
        assert!(ink > 10_000, "almost nothing was drawn: ink total {ink}");

        // Row profile: there must be ink in a band well above the letter (the
        // accent), then a gap, then the letter itself. That gap is the whole
        // point — it is what distinguishes a composite that worked from one
        // that drew the accent on top of the `e`.
        let row_ink: Vec<u32> = (0..mask.height() as i64)
            .map(|y| {
                (0..mask.width() as i64)
                    .map(|x| u32::from(mask.at(x, y)))
                    .sum()
            })
            .collect();
        let inked: Vec<usize> = row_ink
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v > 0)
            .map(|(y, _)| y)
            .collect();
        let first = *inked.first().expect("some row has ink");
        let last = *inked.last().expect("some row has ink");
        let gaps: Vec<usize> = (first..last).filter(|&y| row_ink[y] == 0).collect();
        assert!(
            !gaps.is_empty(),
            "no blank row between accent and letter; the accent is not detached. \
             Row ink profile: {row_ink:?}"
        );

        // The `e` has a counter, so some interior pixel must be empty while
        // surrounded by ink on the same row.
        let counter = (0..mask.height() as i64).any(|y| {
            let row: Vec<u8> = (0..mask.width() as i64).map(|x| mask.at(x, y)).collect();
            let left = row.iter().position(|&v| v > 128);
            let right = row.iter().rposition(|&v| v > 128);
            match (left, right) {
                (Some(l), Some(r)) if r > l + 2 => row[l..r].contains(&0),
                _ => false,
            }
        });
        assert!(counter, "the `e` should have a hole in it");
    }

    #[test]
    fn a_whole_alphabet_rasterises_without_panicking() {
        let font = Font::bundled().expect("parses");
        let scale = 24.0 / f32::from(font.units_per_em());
        let transform = Transform::scale_flip_y(scale).then(&Transform::translate(2.0, 26.0));
        for glyph in 0..font.glyph_count() {
            let outline = font.outline(glyph).expect("parses");
            let mask = Mask::from_outline(32, 32, &outline, &transform).expect("small");
            assert_eq!(mask.data().len(), 32 * 32);
        }
    }

    #[test]
    fn rasterising_is_bit_identical_across_runs() {
        let font = Font::bundled().expect("parses");
        let outline = font
            .outline(font.glyph_for('g').expect("g"))
            .expect("outline");
        let transform = Transform::scale_flip_y(37.0 / f32::from(font.units_per_em()))
            .then(&Transform::translate(3.5, 40.25));
        let first = Mask::from_outline(40, 48, &outline, &transform).expect("small");
        let second = Mask::from_outline(40, 48, &outline, &transform).expect("small");
        assert_eq!(first, second);
    }
}
