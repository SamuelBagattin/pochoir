//! Sine and cosine, computed here rather than borrowed from libm.
//!
//! # Why on earth
//!
//! ADR-0002 Decision 6 requires the output to be byte-identical on
//! `wasm32-unknown-unknown` and on native. IEEE-754 specifies `+ - * /` and
//! `sqrt` exactly, so those agree everywhere. It says **nothing** about the
//! transcendental functions: `sin` and `cos` come from the platform's libm, and
//! a browser's and a host's are permitted to differ in the last bits — and do.
//!
//! The mark needs exactly one sine and one cosine, for its rotation. If those
//! two numbers differ by one bit between targets, the rotation matrix differs,
//! every bilinearly sampled pixel differs, the JPEG differs, the PDF differs,
//! and the golden tests pass only on the machine that generated them. For one
//! library call.
//!
//! So: a Taylor polynomial, with literal coefficients, over a reduced range.
//! **Agreement matters here and accuracy barely does** — the angle is a user
//! parameter in degrees, and an error of 1e-9 radians is invisible in a
//! watermark — but the polynomials below agree with libm to about 1e-12 anyway,
//! which is far beyond what the `f32` the caller gets back can hold.
//!
//! The range reduction is a single subtraction of a multiple of π/2, which
//! loses absolute precision in proportion to the magnitude of the argument.
//! That is fine here and stated rather than hidden: the mark's angle is
//! validated to ±90°, so the argument never leaves ±π/2, and the tests cover
//! ±100 radians. Nothing in this crate needs Payne–Hanek reduction and nothing
//! should grow a use for it.
//!
//! Nothing else in the raster path needs a transcendental function, and nothing
//! in this crate may call one. There is no test that would catch a stray
//! `f32::sin`, because CI builds for wasm32 but does not run the suite there;
//! the discipline is the whole of the protection.

/// π, as the core library's `f64` constant.
///
/// A compile-time constant, not a libm call, so this is as portable as a
/// literal and more readable than one.
const PI: f64 = core::f64::consts::PI;

/// Sine and cosine of `radians`, by range reduction and a polynomial.
///
/// Reduces to the nearest multiple of π/2 and evaluates on `[-π/4, π/4]`, where
/// the truncation error of the two series below is around 1e-13.
fn sin_cos_radians(radians: f64) -> (f64, f64) {
    // Nearest integer multiple of π/2. `floor(x + 0.5)` rather than `round`,
    // which ADR-0002 Decision 6 bans for the same reason as the rest of libm;
    // `floor` is exactly specified. It works for negative inputs too.
    let quadrant = (radians * (2.0 / PI) + 0.5).floor();
    let r = radians - quadrant * (PI / 2.0);
    let (sine, cosine) = (polynomial_sin(r), polynomial_cos(r));
    // sin(r + kπ/2) and cos(r + kπ/2) for k mod 4. `& 3` on a negative i64 is
    // the two's-complement remainder, which is what we want.
    match (quadrant as i64) & 3 {
        0 => (sine, cosine),
        1 => (cosine, -sine),
        2 => (-sine, -cosine),
        _ => (-cosine, sine),
    }
}

/// `sin` on `[-π/4, π/4]`: the Taylor series to `r¹³`.
///
/// Truncating after `r¹³` leaves `r¹⁵/15!`, which at `r = π/4` is about 7e-16 —
/// below an `f64`'s own resolution there, so the series is the limit of the
/// type rather than of the approximation. Each coefficient is written as the
/// factorial it is, so a reader can check the whole thing against the series
/// without consulting a table of minimax coefficients.
fn polynomial_sin(r: f64) -> f64 {
    let r2 = r * r;
    // r - r³/3! + r⁵/5! - r⁷/7! + r⁹/9! - r¹¹/11! + r¹³/13!, by Horner.
    r * (1.0
        - r2 * (1.0 / 6.0
            - r2 * (1.0 / 120.0
                - r2 * (1.0 / 5_040.0
                    - r2 * (1.0 / 362_880.0
                        - r2 * (1.0 / 39_916_800.0 - r2 * (1.0 / 6_227_020_800.0)))))))
}

/// `cos` on `[-π/4, π/4]`: the Taylor series to `r¹²`.
fn polynomial_cos(r: f64) -> f64 {
    let r2 = r * r;
    // 1 - r²/2! + r⁴/4! - r⁶/6! + r⁸/8! - r¹⁰/10! + r¹²/12!, by Horner.
    1.0 - r2
        * (1.0 / 2.0
            - r2 * (1.0 / 24.0
                - r2 * (1.0 / 720.0
                    - r2 * (1.0 / 40_320.0
                        - r2 * (1.0 / 3_628_800.0 - r2 * (1.0 / 479_001_600.0))))))
}

/// Sine and cosine of an angle in **degrees**, as `f32`.
///
/// Degrees because that is what ADR-0002 Decision 3 exposes to the user and
/// what the CLI accepts, and converting once here means the conversion factor
/// exists in one place.
#[must_use]
pub fn sin_cos_degrees(degrees: f32) -> (f32, f32) {
    let (sine, cosine) = sin_cos_radians(f64::from(degrees) * (PI / 180.0));
    (sine as f32, cosine as f32)
}

#[cfg(test)]
mod tests {
    use super::{sin_cos_degrees, sin_cos_radians};

    #[test]
    fn it_agrees_with_the_platform_library() {
        // The point of this module is *not* to be more accurate than libm; it
        // is to be the same everywhere. Agreeing with libm to 1e-9 is how we
        // know the polynomial is right, and the fixed coefficients are how we
        // know it will not drift. ±100 radians is far past the ±π/2 the mark's
        // validated angle range can produce; see the note on range reduction
        // in the module documentation.
        let mut angle = -100.0_f64;
        while angle <= 100.0 {
            let (sine, cosine) = sin_cos_radians(angle);
            assert!(
                (sine - angle.sin()).abs() < 1e-9,
                "sin({angle}): {sine} vs {}",
                angle.sin()
            );
            assert!(
                (cosine - angle.cos()).abs() < 1e-9,
                "cos({angle}): {cosine} vs {}",
                angle.cos()
            );
            angle += 0.013;
        }
    }

    #[test]
    fn the_exact_angles_come_out_right() {
        let cases: [(f32, f32, f32); 7] = [
            (0.0, 0.0, 1.0),
            (90.0, 1.0, 0.0),
            (180.0, 0.0, -1.0),
            (270.0, -1.0, 0.0),
            (-90.0, -1.0, 0.0),
            (30.0, 0.5, 0.866_025_4),
            (-30.0, -0.5, 0.866_025_4),
        ];
        for (degrees, expected_sine, expected_cosine) in cases {
            let (sine, cosine) = sin_cos_degrees(degrees);
            assert!(
                (sine - expected_sine).abs() < 1e-6,
                "sin({degrees}°) = {sine}, expected {expected_sine}"
            );
            assert!(
                (cosine - expected_cosine).abs() < 1e-6,
                "cos({degrees}°) = {cosine}, expected {expected_cosine}"
            );
        }
    }

    #[test]
    fn the_default_mark_angle_is_the_one_adr_0002_asks_for() {
        // -30°, the value the whole rotation path is built around.
        let (sine, cosine) = sin_cos_degrees(-30.0);
        assert!((sine + 0.5).abs() < 1e-6, "sin(-30°) should be -0.5");
        assert!((cosine - 0.866_025_4).abs() < 1e-6);
        // The sign is load-bearing: in a y-down raster a negative sine is what
        // makes the mark run from the bottom-left to the top-right.
        assert!(sine < 0.0 && cosine > 0.0);
    }

    #[test]
    fn it_stays_on_the_unit_circle() {
        for degrees in [-90.0_f32, -47.5, -30.0, -0.25, 0.0, 12.5, 45.0, 89.0, 90.0] {
            let (sine, cosine) = sin_cos_degrees(degrees);
            let magnitude = sine * sine + cosine * cosine;
            assert!(
                (magnitude - 1.0).abs() < 1e-5,
                "sin²+cos² at {degrees}° is {magnitude}"
            );
        }
    }

    #[test]
    fn the_same_angle_always_gives_the_same_numbers() {
        // Trivially true of a pure function, and the assertion the determinism
        // requirement actually rests on.
        for degrees in [-30.0_f32, 0.0, 17.125] {
            assert_eq!(sin_cos_degrees(degrees), sin_cos_degrees(degrees));
        }
    }
}
