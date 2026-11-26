//! The one source of randomness in the pipeline, seeded and reproducible.
//!
//! ADR-0002 Decision 6: "All of it comes from one small seeded PRNG that we
//! write ourselves (xorshift or PCG, ~20 lines, no dependency). The seed is a
//! hash of the input bytes, the parameter set and the mark text."
//!
//! This is xorshift64 with a multiplicative finaliser — Marsaglia's shift
//! triple `(12, 25, 27)` followed by a 64-bit odd multiply, the variant usually
//! written `xorshift64*`. It is integer arithmetic throughout, so it cannot
//! drift between wasm32 and native, and it is not and must not be used for
//! anything that needs cryptographic strength: its only job is to move
//! watermark tiles around by a plausible-looking amount.
//!
//! Reference: George Marsaglia, *Xorshift RNGs*, Journal of Statistical
//! Software 8(14), 2003 — <https://doi.org/10.18637/jss.v008.i14>

/// The multiplier of `xorshift64*`.
const FINALISER: u64 = 0x2545_F491_4F6C_DD1D;

/// The 64-bit golden-ratio odd constant, used as the fallback seed and as the
/// mixing increment. Any odd constant with well-spread bits would do.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// A seeded generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator from `seed`.
    ///
    /// Zero is replaced, because xorshift has zero as a fixed point and would
    /// otherwise return nothing but zero for ever. A caller passing `--seed 0`
    /// gets a usable stream rather than no jitter at all.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { GOLDEN } else { seed },
        }
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(FINALISER)
    }

    /// The next value in `0.0..1.0`.
    ///
    /// Built from 24 bits over 2²⁴, both exactly representable, so the division
    /// is exact and the result never reaches 1.0.
    pub fn next_unit(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32;
        bits as f32 / 16_777_216.0
    }

    /// The next value in `-1.0..1.0`.
    pub fn next_signed(&mut self) -> f32 {
        self.next_unit() * 2.0 - 1.0
    }
}

/// SplitMix64's finaliser, used here purely as a bit mixer.
///
/// Reference: Steele, Lea and Flood, *Fast splittable pseudorandom number
/// generators*, OOPSLA 2014 — <https://doi.org/10.1145/2660193.2660195>
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A generator keyed to one tile of the watermark lattice.
///
/// Per-tile rather than per-run, so that every tile is displaced
/// independently — see ADR-0007, which supersedes ADR-0002 on this point. Being
/// keyed by coordinate rather than drawn from a running stream, a tile's offset
/// does not depend on how many tiles were visited before it, so the lattice can
/// be iterated in any order and a page that happens to need one more row does
/// not move every other row.
#[must_use]
pub fn for_tile(seed: u64, column: i64, row: i64) -> Rng {
    let mut hash = mix(seed);
    hash = mix(hash ^ (column as u64));
    hash = mix(hash ^ (row as u64).rotate_left(32));
    Rng::new(hash)
}

#[cfg(test)]
mod tests {
    use super::{Rng, for_tile, mix};

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        let mut a = Rng::new(12_345);
        let mut b = Rng::new(12_345);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_give_different_streams() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let left: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let right: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_ne!(left, right);
    }

    #[test]
    fn a_zero_seed_still_produces_a_stream() {
        let mut rng = Rng::new(0);
        let values: Vec<u64> = (0..8).map(|_| rng.next_u64()).collect();
        assert!(
            values.iter().any(|&v| v != 0),
            "a zero seed collapsed xorshift to zero"
        );
    }

    #[test]
    fn unit_values_stay_in_range_and_spread_out() {
        let mut rng = Rng::new(0xDEAD_BEEF);
        let mut sum = 0.0_f64;
        let mut buckets = [0u32; 10];
        const DRAWS: u32 = 20_000;
        for _ in 0..DRAWS {
            let value = rng.next_unit();
            assert!((0.0..1.0).contains(&value), "{value} is out of range");
            sum += f64::from(value);
            buckets[(value * 10.0) as usize] += 1;
        }
        let mean = sum / f64::from(DRAWS);
        assert!((mean - 0.5).abs() < 0.02, "mean {mean} is not near a half");
        // Not a statistical test, just a check that it is not stuck in a corner.
        for (index, &count) in buckets.iter().enumerate() {
            assert!(count > DRAWS / 20, "bucket {index} holds only {count}");
        }
    }

    #[test]
    fn signed_values_straddle_zero() {
        let mut rng = Rng::new(7);
        let mut negative = 0;
        let mut positive = 0;
        for _ in 0..1000 {
            let value = rng.next_signed();
            assert!((-1.0..1.0).contains(&value));
            if value < 0.0 {
                negative += 1;
            } else {
                positive += 1;
            }
        }
        assert!(negative > 300 && positive > 300, "{negative} / {positive}");
    }

    #[test]
    fn a_tile_key_depends_on_both_coordinates_and_not_on_order() {
        let seed = 0x1234_5678_9ABC_DEF0;
        let at = |column, row| for_tile(seed, column, row).next_u64();
        // Distinct tiles, including ones that would collide under a sloppier
        // combination such as xor or addition of the raw coordinates.
        let keys = [
            at(0, 0),
            at(1, 0),
            at(0, 1),
            at(-1, 0),
            at(0, -1),
            at(2, 3),
            at(3, 2),
            at(-2, -3),
        ];
        for (index, key) in keys.iter().enumerate() {
            for (other, compared) in keys.iter().enumerate() {
                assert!(
                    index == other || key != compared,
                    "tile keys {index} and {other} collide"
                );
            }
        }
        // And a tile's value does not depend on when it was asked for.
        assert_eq!(at(5, 9), at(5, 9));
    }

    #[test]
    fn a_different_run_seed_moves_every_tile() {
        let a: Vec<u64> = (0..8).map(|i| for_tile(1, i, 0).next_u64()).collect();
        let b: Vec<u64> = (0..8).map(|i| for_tile(2, i, 0).next_u64()).collect();
        for (left, right) in a.iter().zip(&b) {
            assert_ne!(left, right);
        }
    }

    #[test]
    fn the_mixer_is_a_bijection_on_the_values_we_feed_it() {
        // Not a proof — SplitMix64's finaliser is known invertible — just a
        // guard against a typo in one of the constants making it collapse.
        let mut seen = std::collections::HashSet::new();
        for value in 0..10_000u64 {
            assert!(seen.insert(mix(value)), "mix collides at {value}");
        }
    }
}
