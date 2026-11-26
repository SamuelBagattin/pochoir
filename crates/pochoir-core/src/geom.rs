//! Points and affine transforms, in the two coordinate systems that matter.
//!
//! There are exactly two, and keeping them straight is most of what this
//! module is for:
//!
//! - **Font units**, y-**up**, origin on the baseline at the glyph's left
//!   sidebearing point, one em being [`crate::font::Font::units_per_em`] units.
//!   This is what [`crate::font`] hands out.
//! - **Device pixels**, y-**down**, origin at the top-left of a [`Pixmap`] or a
//!   coverage buffer. This is what [`crate::raster`] fills in.
//!
//! The y-flip between them happens in exactly one place: the [`Transform`] that
//! a caller hands to the rasteriser, built by [`Transform::scale_flip_y`]. If
//! you find yourself negating a y anywhere else, one of the two flips is
//! wrong.
//!
//! [`Pixmap`]: crate::pixmap::Pixmap

/// A point in whichever space the surrounding code is working in.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate: upwards in font units, downwards in device pixels.
    pub y: f32,
}

impl Point {
    /// A point from its two coordinates.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The midpoint of `self` and `other`.
    ///
    /// Used for the on-curve points TrueType leaves implicit between two
    /// consecutive off-curve points.
    #[must_use]
    pub fn midpoint(self, other: Self) -> Self {
        Self {
            x: (self.x + other.x) * 0.5,
            y: (self.y + other.y) * 0.5,
        }
    }
}

/// A 2-D affine transform, applied as `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
///
/// The field names follow the PDF and PostScript convention — `[a b c d e f]`
/// is the matrix a `cm` operator takes — because [`crate::pdf`] already speaks
/// that language and two conventions in one crate is one too many.
///
/// Every operation here is `+`, `-` or `*` on `f32`, which IEEE-754 specifies
/// exactly, so a transform composed on wasm32 and the same transform composed
/// natively are bit-identical. That is a requirement, not an observation: see
/// ADR-0002 Decision 6.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Row 1, column 1: x's contribution to x'.
    pub a: f32,
    /// Row 1, column 2: x's contribution to y'.
    pub b: f32,
    /// Row 2, column 1: y's contribution to x'.
    pub c: f32,
    /// Row 2, column 2: y's contribution to y'.
    pub d: f32,
    /// Horizontal translation.
    pub e: f32,
    /// Vertical translation.
    pub f: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A pure translation.
    #[must_use]
    pub const fn translate(dx: f32, dy: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: dx,
            f: dy,
        }
    }

    /// A uniform scale about the origin.
    #[must_use]
    pub const fn scale(factor: f32) -> Self {
        Self {
            a: factor,
            b: 0.0,
            c: 0.0,
            d: factor,
            e: 0.0,
            f: 0.0,
        }
    }

    /// **The one y-flip in the codebase.**
    ///
    /// Scales font units to pixels by `factor` and negates y, so that a point
    /// `units` above the baseline lands `units * factor` pixels *above* the
    /// device-space origin — which is to say at a smaller `y`. Compose a
    /// translation after it to put the baseline somewhere useful.
    #[must_use]
    pub const fn scale_flip_y(factor: f32) -> Self {
        Self {
            a: factor,
            b: 0.0,
            c: 0.0,
            d: -factor,
            e: 0.0,
            f: 0.0,
        }
    }

    /// A 2×2 linear part with no translation, as a composite glyph component
    /// carries it.
    #[must_use]
    pub const fn linear(a: f32, b: f32, c: f32, d: f32) -> Self {
        Self {
            a,
            b,
            c,
            d,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Apply this transform to a point.
    #[must_use]
    pub fn apply(&self, point: Point) -> Point {
        Point {
            x: self.a * point.x + self.c * point.y + self.e,
            y: self.b * point.x + self.d * point.y + self.f,
        }
    }

    /// `self` after `first`: the transform that applies `first`, then `self`.
    #[must_use]
    pub fn then(&self, outer: &Self) -> Self {
        Self {
            a: self.a * outer.a + self.b * outer.c,
            b: self.a * outer.b + self.b * outer.d,
            c: self.c * outer.a + self.d * outer.c,
            d: self.c * outer.b + self.d * outer.d,
            e: self.e * outer.a + self.f * outer.c + outer.e,
            f: self.e * outer.b + self.f * outer.d + outer.f,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Point, Transform};

    #[test]
    fn identity_leaves_a_point_alone() {
        let p = Point::new(3.0, -4.0);
        assert_eq!(Transform::IDENTITY.apply(p), p);
    }

    #[test]
    fn the_y_flip_sends_up_to_a_smaller_y() {
        // 100 font units above the baseline, at half a pixel per unit.
        let t = Transform::scale_flip_y(0.5);
        assert_eq!(t.apply(Point::new(0.0, 100.0)), Point::new(0.0, -50.0));
        assert_eq!(t.apply(Point::new(100.0, 0.0)), Point::new(50.0, 0.0));
    }

    #[test]
    fn composition_applies_the_inner_transform_first() {
        // Scale by 2, then translate by 10: (1,0) -> (2,0) -> (12,0).
        let composed = Transform::scale(2.0).then(&Transform::translate(10.0, 0.0));
        assert_eq!(composed.apply(Point::new(1.0, 0.0)), Point::new(12.0, 0.0));

        // Translate by 10, then scale by 2: (1,0) -> (11,0) -> (22,0).
        let other = Transform::translate(10.0, 0.0).then(&Transform::scale(2.0));
        assert_eq!(other.apply(Point::new(1.0, 0.0)), Point::new(22.0, 0.0));
    }

    #[test]
    fn composition_matches_applying_the_two_in_turn() {
        let inner = Transform::linear(0.5, 0.25, -0.75, 1.5);
        let outer = Transform {
            a: 2.0,
            b: -1.0,
            c: 0.5,
            d: 3.0,
            e: 7.0,
            f: -2.0,
        };
        let composed = inner.then(&outer);
        for p in [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(-3.5, 9.25),
        ] {
            let stepwise = outer.apply(inner.apply(p));
            let fused = composed.apply(p);
            assert!(
                (stepwise.x - fused.x).abs() < 1e-5 && (stepwise.y - fused.y).abs() < 1e-5,
                "{p:?}: stepwise {stepwise:?} but fused {fused:?}"
            );
        }
    }

    #[test]
    fn a_midpoint_is_halfway() {
        assert_eq!(
            Point::new(0.0, 0.0).midpoint(Point::new(4.0, -2.0)),
            Point::new(2.0, -1.0)
        );
    }
}
