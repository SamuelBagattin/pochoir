//! Quantisation tables, and the quality knob that scales them.
//!
//! ADR-0001 gives owning these tables as one of the reasons to hand-roll the
//! encoder at all: ADR-0002's determinism requirement is a requirement about
//! these 128 numbers, and a table that arrives from a dependency is a table
//! that can change under us in a patch release.
//!
//! The base tables are the example tables of ITU-T T.81 Annex K (K.1 for
//! luminance, K.2 for chrominance), in natural row-major order rather than
//! zigzag order. The scaling rule is the independent JPEG group's: a quality of
//! 50 reproduces the tables as published, 100 scales every entry to 1, and 1
//! scales them towards 255.

/// Luminance base table, T.81 Annex K.1, natural order.
pub const LUMINANCE: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, //
    12, 12, 14, 19, 26, 58, 60, 55, //
    14, 13, 16, 24, 40, 57, 69, 56, //
    14, 17, 22, 29, 51, 87, 80, 62, //
    18, 22, 37, 56, 68, 109, 103, 77, //
    24, 35, 55, 64, 81, 104, 113, 92, //
    49, 64, 78, 87, 103, 121, 120, 101, //
    72, 92, 95, 98, 112, 100, 103, 99,
];

/// Chrominance base table, T.81 Annex K.2, natural order.
pub const CHROMINANCE: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, //
    18, 21, 26, 66, 99, 99, 99, 99, //
    24, 26, 56, 99, 99, 99, 99, 99, //
    47, 66, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99,
];

/// Zigzag order: `ZIGZAG[k]` is the natural-order index of the `k`-th
/// coefficient in the zigzag sequence. T.81 Figure A.6.
pub const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, //
    17, 24, 32, 25, 18, 11, 4, 5, //
    12, 19, 26, 33, 40, 48, 41, 34, //
    27, 20, 13, 6, 7, 14, 21, 28, //
    35, 42, 49, 56, 57, 50, 43, 36, //
    29, 22, 15, 23, 30, 37, 44, 51, //
    58, 59, 52, 45, 38, 31, 39, 46, //
    53, 60, 61, 54, 47, 55, 62, 63,
];

/// Scale a base table for a quality in `1..=100`.
///
/// Quality is mapped to a percentage scale factor the way the IJG encoder does
/// it, which is the mapping every "JPEG quality 85" in the world means: below
/// 50, `5000 / q`; at or above, `200 - 2q`. The result is clamped to `1..=255`
/// because a baseline `DQT` segment stores 8-bit values and a divisor of zero
/// is not a divisor.
#[must_use]
pub fn scale(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let quality = u32::from(quality.clamp(1, 100));
    let factor = if quality < 50 {
        5000 / quality
    } else {
        200 - 2 * quality
    };
    let mut out = [0u16; 64];
    for (slot, &value) in out.iter_mut().zip(base.iter()) {
        let scaled = (u32::from(value) * factor + 50) / 100;
        *slot = scaled.clamp(1, 255) as u16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{CHROMINANCE, LUMINANCE, ZIGZAG, scale};

    #[test]
    fn quality_fifty_reproduces_the_published_tables() {
        assert_eq!(scale(&LUMINANCE, 50), LUMINANCE);
        assert_eq!(scale(&CHROMINANCE, 50), CHROMINANCE);
    }

    #[test]
    fn quality_one_hundred_quantises_by_one() {
        assert_eq!(scale(&LUMINANCE, 100), [1u16; 64]);
        assert_eq!(scale(&CHROMINANCE, 100), [1u16; 64]);
    }

    #[test]
    fn every_quality_stays_inside_the_eight_bit_range() {
        for quality in 1..=100u8 {
            for table in [&LUMINANCE, &CHROMINANCE] {
                for value in scale(table, quality) {
                    assert!(
                        (1..=255).contains(&value),
                        "quality {quality} produced divisor {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn higher_quality_never_quantises_more_coarsely() {
        for quality in 1..100u8 {
            let coarse = scale(&LUMINANCE, quality);
            let fine = scale(&LUMINANCE, quality + 1);
            for (index, (c, f)) in coarse.iter().zip(fine.iter()).enumerate() {
                assert!(
                    f <= c,
                    "coefficient {index}: quality {quality} gave {c} but {} gave {f}",
                    quality + 1
                );
            }
        }
    }

    #[test]
    fn quality_eighty_five_is_the_adr_0002_default_and_is_stable() {
        // Pinned so that a change to the scaling rule shows up as a test
        // failure rather than as silently different output bytes. ADR-0002
        // makes `standard` mean JPEG q85, and goldens depend on these numbers.
        assert_eq!(scale(&LUMINANCE, 85)[..8], [5, 3, 3, 5, 7, 12, 15, 18][..]);
        assert_eq!(
            scale(&CHROMINANCE, 85)[..8],
            [5, 5, 7, 14, 30, 30, 30, 30][..]
        );
    }

    #[test]
    fn the_zigzag_order_is_a_permutation_of_all_sixty_four_indices() {
        let mut seen = ZIGZAG;
        seen.sort_unstable();
        assert_eq!(seen, core::array::from_fn::<usize, 64, _>(|i| i));
        assert_eq!(ZIGZAG[0], 0, "the DC coefficient comes first");
    }
}
