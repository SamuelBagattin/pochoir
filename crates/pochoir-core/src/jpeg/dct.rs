//! The forward discrete cosine transform, and the quantisation that follows it.
//!
//! The transform is the eight-point scaled DCT of Arai, Agui and Nakajima
//! ("A fast DCT-SQ scheme for images", Trans. IEICE E-71(11):1095, 1988),
//! applied separably to rows and then to columns. It is the standard fast
//! forward DCT: five multiplications and twenty-nine additions per
//! eight-point pass, against the sixty-four multiplications of the direct
//! form, which matters when a 12-megapixel photograph is 560 000 blocks.
//!
//! The output it produces is **scaled**: coefficient `(row, col)` comes out
//! larger than the true DCT coefficient by `8 * S(row) * S(col)` where
//! `S(0) = 1` and `S(k) = cos(k * pi / 16) * sqrt(2)`. Rather than undo that
//! with sixty-four multiplications per block, the scale factors are folded into
//! the quantisation divisors once, in [`Quantiser`], which is why this module
//! owns both halves of the operation.
//!
//! # Determinism
//!
//! ADR-0002 requires the wasm32 and native builds to produce byte-identical
//! output, and notes that `sin` and `cos` may differ between them because they
//! come from the platform's libm. Nothing here calls a transcendental function:
//! every constant is a literal, and the arithmetic is IEEE-754 `+`, `-`, `*`,
//! `/` and `floor`, all of which are exactly specified on both targets.

use super::quant::ZIGZAG;

/// `S(k) = cos(k * pi / 16) * sqrt(2)`, with `S(0)` taken as 1.
///
/// Literals rather than a call to `cos`, deliberately: see the determinism note
/// above.
const SCALE: [f32; 8] = [
    1.0,
    1.387_039_8,
    1.306_563,
    1.175_875_6,
    1.0,
    0.785_694_96,
    0.541_196_1,
    0.275_899_38,
];

/// `cos(pi / 4)`, i.e. `1 / sqrt(2)`.
const C4: f32 = 0.707_106_77;
/// `cos(3 * pi / 8) * sqrt(2)`.
const C6: f32 = 0.541_196_1;
/// `cos(pi / 8) * sqrt(2)`.
const C2: f32 = 1.306_563;
/// `sin(3 * pi / 16) - cos(3 * pi / 16)`, the shared term of the odd butterfly.
const C26: f32 = 0.382_683_43;

/// One eight-by-eight block of samples, natural (row-major) order.
pub type Block = [f32; 64];

/// Transform `block` in place, leaving scaled DCT coefficients in it.
pub fn forward(block: &mut Block) {
    // Pass 1: each row independently.
    for row in 0..8 {
        let base = row * 8;
        butterfly(block, base, 1);
    }
    // Pass 2: each column independently, by striding instead of transposing.
    for col in 0..8 {
        butterfly(block, col, 8);
    }
}

/// One eight-point pass over `data[start]`, `data[start + stride]`, ...
fn butterfly(data: &mut Block, start: usize, stride: usize) {
    let at = |i: usize| start + i * stride;

    let s0 = data[at(0)];
    let s1 = data[at(1)];
    let s2 = data[at(2)];
    let s3 = data[at(3)];
    let s4 = data[at(4)];
    let s5 = data[at(5)];
    let s6 = data[at(6)];
    let s7 = data[at(7)];

    let t0 = s0 + s7;
    let t7 = s0 - s7;
    let t1 = s1 + s6;
    let t6 = s1 - s6;
    let t2 = s2 + s5;
    let t5 = s2 - s5;
    let t3 = s3 + s4;
    let t4 = s3 - s4;

    // Even half: a two-level butterfly plus one rotation.
    let e0 = t0 + t3;
    let e3 = t0 - t3;
    let e1 = t1 + t2;
    let e2 = t1 - t2;

    data[at(0)] = e0 + e1;
    data[at(4)] = e0 - e1;

    let rotated = (e2 + e3) * C4;
    data[at(2)] = e3 + rotated;
    data[at(6)] = e3 - rotated;

    // Odd half.
    let o0 = t4 + t5;
    let o1 = t5 + t6;
    let o2 = t6 + t7;

    let shared = (o0 - o2) * C26;
    let z2 = C6 * o0 + shared;
    let z4 = C2 * o2 + shared;
    let z3 = o1 * C4;

    let upper = t7 + z3;
    let lower = t7 - z3;

    data[at(5)] = lower + z2;
    data[at(3)] = lower - z2;
    data[at(1)] = upper + z4;
    data[at(7)] = upper - z4;
}

/// Reciprocal divisors for one component, with the transform's scaling folded
/// in.
#[derive(Debug, Clone)]
pub struct Quantiser {
    /// `1 / (table[i] * 8 * S(row) * S(col))`, natural order.
    reciprocals: [f32; 64],
    /// The table as it will be written into the `DQT` segment, zigzag order.
    zigzag_table: [u8; 64],
}

impl Quantiser {
    /// Build a quantiser from a scaled quantisation table in natural order.
    #[must_use]
    pub fn new(table: &[u16; 64]) -> Self {
        let mut reciprocals = [0.0f32; 64];
        for (row, &row_scale) in SCALE.iter().enumerate() {
            for (col, &col_scale) in SCALE.iter().enumerate() {
                let index = row * 8 + col;
                let divisor = f32::from(table[index]) * 8.0 * row_scale * col_scale;
                reciprocals[index] = 1.0 / divisor;
            }
        }
        let mut zigzag_table = [0u8; 64];
        for (slot, &natural) in zigzag_table.iter_mut().zip(ZIGZAG.iter()) {
            // `quant::scale` clamps to 1..=255, so this never truncates.
            *slot = table[natural] as u8;
        }
        Self {
            reciprocals,
            zigzag_table,
        }
    }

    /// The table in the order a `DQT` segment wants it.
    #[must_use]
    pub const fn dqt_bytes(&self) -> &[u8; 64] {
        &self.zigzag_table
    }

    /// Quantise one transformed block into `out`, natural order.
    pub fn quantise(&self, block: &Block, out: &mut [i32; 64]) {
        for index in 0..64 {
            let value = round(block[index] * self.reciprocals[index]);
            // Baseline entropy coding can only express magnitudes up to 10 bits
            // for an AC coefficient and an 11-bit DC *difference*, because the
            // standard Huffman tables of T.81 Annex K define no longer size
            // category. Clamping here is what guarantees the entropy coder
            // never needs one. In practice no real image at any quality we
            // offer reaches this bound; it exists so that a pathological
            // synthetic input produces a valid file rather than a corrupt one.
            out[index] = value.clamp(-1023, 1023);
        }
    }
}

/// Round half up, using only exactly specified floating-point operations.
///
/// `f32::floor` lowers to a single instruction on both targets (`f32.floor` in
/// WebAssembly), unlike `f32::round`, which on some platforms is a libm call
/// and therefore a determinism hazard under ADR-0002.
fn round(value: f32) -> i32 {
    (value + 0.5).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::{Block, Quantiser, SCALE, forward};
    use crate::jpeg::quant;

    /// The direct definition of the two-dimensional DCT-II, for cross-checking
    /// the fast transform. Deliberately written from the formula and allowed to
    /// use `cos`, because it is test-only code and never ships.
    fn reference_dct(samples: &Block) -> Block {
        let mut out = [0.0f32; 64];
        for u in 0..8usize {
            for v in 0..8usize {
                let mut sum = 0.0f64;
                for y in 0..8usize {
                    for x in 0..8usize {
                        let cx = (2.0 * x as f64 + 1.0) * v as f64 * core::f64::consts::PI / 16.0;
                        let cy = (2.0 * y as f64 + 1.0) * u as f64 * core::f64::consts::PI / 16.0;
                        sum += f64::from(samples[y * 8 + x]) * cx.cos() * cy.cos();
                    }
                }
                let cu = if u == 0 { 1.0 / 2.0_f64.sqrt() } else { 1.0 };
                let cv = if v == 0 { 1.0 / 2.0_f64.sqrt() } else { 1.0 };
                out[u * 8 + v] = (sum * cu * cv / 4.0) as f32;
            }
        }
        out
    }

    #[test]
    fn the_fast_transform_agrees_with_the_direct_definition() {
        // A deterministic pseudo-random block, plus the two degenerate ones.
        let mut blocks: Vec<Block> = vec![[0.0; 64], [127.0; 64]];
        let mut state = 0x1234_5678u32;
        let mut noisy = [0.0f32; 64];
        for slot in &mut noisy {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *slot = f32::from((state >> 24) as u8) - 128.0;
        }
        blocks.push(noisy);
        // A vertical edge, the hardest case for a DCT to get subtly wrong.
        let mut edge = [0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                edge[y * 8 + x] = if x < 4 { -128.0 } else { 127.0 };
            }
        }
        blocks.push(edge);

        for samples in blocks {
            let expected = reference_dct(&samples);
            let mut actual = samples;
            forward(&mut actual);
            for (row, &row_scale) in SCALE.iter().enumerate() {
                for (col, &col_scale) in SCALE.iter().enumerate() {
                    let index = row * 8 + col;
                    // Undo the transform's own scaling before comparing.
                    let unscaled = actual[index] / (8.0 * row_scale * col_scale);
                    assert!(
                        (unscaled - expected[index]).abs() < 0.02,
                        "coefficient ({row},{col}): fast {unscaled} vs direct {}",
                        expected[index]
                    );
                }
            }
        }
    }

    #[test]
    fn a_flat_block_has_only_a_dc_coefficient() {
        let mut block: Block = [42.0; 64];
        forward(&mut block);
        assert!(
            (block[0] - 42.0 * 64.0).abs() < 0.001,
            "DC was {}",
            block[0]
        );
        for (index, &value) in block.iter().enumerate().skip(1) {
            assert!(value.abs() < 0.001, "AC coefficient {index} was {value}");
        }
    }

    #[test]
    fn quantising_a_flat_block_recovers_the_level() {
        // With the scaling folded in, a constant block of v quantises to a DC
        // of 8v / table[0], which is the standard relationship.
        let table = quant::scale(&quant::LUMINANCE, 50);
        let quantiser = Quantiser::new(&table);
        let mut block: Block = [16.0; 64];
        forward(&mut block);
        let mut out = [0i32; 64];
        quantiser.quantise(&block, &mut out);
        assert_eq!(out[0], 8 * 16 / i32::from(table[0]));
        assert!(out[1..].iter().all(|&c| c == 0));
    }

    #[test]
    fn the_dqt_bytes_are_the_table_in_zigzag_order() {
        let table = quant::scale(&quant::LUMINANCE, 50);
        let quantiser = Quantiser::new(&table);
        let bytes = quantiser.dqt_bytes();
        for (k, &natural) in quant::ZIGZAG.iter().enumerate() {
            assert_eq!(u16::from(bytes[k]), table[natural], "zigzag slot {k}");
        }
    }

    #[test]
    fn coefficients_are_clamped_into_the_baseline_range() {
        // Quality 100 with an extreme block is the worst case for magnitude.
        let table = quant::scale(&quant::LUMINANCE, 100);
        let quantiser = Quantiser::new(&table);
        let mut block = [0.0f32; 64];
        for (index, slot) in block.iter_mut().enumerate() {
            *slot = if index % 2 == 0 { 1e9 } else { -1e9 };
        }
        forward(&mut block);
        let mut out = [0i32; 64];
        quantiser.quantise(&block, &mut out);
        assert!(out.iter().all(|&c| (-1023..=1023).contains(&c)));
    }
}
