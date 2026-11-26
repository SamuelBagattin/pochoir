//! Huffman tables and the bit writer that uses them.
//!
//! The four tables are the typical tables of ITU-T T.81 Annex K.3 — the ones
//! every baseline encoder ships, specified there as `BITS` (how many codes
//! exist of each length from 1 to 16) and `HUFFVAL` (the symbols, in order of
//! increasing code length). Codes are derived from those two lists by the
//! canonical procedure of T.81 Annex C, Figures C.1 and C.2, so the table in
//! the `DHT` segment and the codes we emit cannot disagree: both come from the
//! same two arrays.
//!
//! We do not compute optimal tables for the image at hand. A custom table would
//! shave a few percent, at the cost of a second pass over every block and of
//! output that depends on the image's statistics in a way that is harder to
//! reason about. The typical tables are what "baseline JPEG" means in practice.

use crate::Result;

/// A Huffman table: the two lists from `DHT`, plus the derived codes.
#[derive(Debug, Clone)]
pub struct Table {
    /// Number of codes of each length 1..=16.
    counts: [u8; 16],
    /// Symbols, in order of increasing code length.
    symbols: &'static [u8],
    /// `codes[symbol] = (code, length)`, zero length meaning "not in this
    /// table".
    codes: [(u16, u8); 256],
}

impl Table {
    /// Derive the codes for a `BITS` / `HUFFVAL` pair.
    ///
    /// The procedure is T.81 Annex C: walk the lengths in order, assigning
    /// consecutive codes within a length and shifting left on each step up.
    #[must_use]
    fn new(counts: [u8; 16], symbols: &'static [u8]) -> Self {
        let mut codes = [(0u16, 0u8); 256];
        let mut code = 0u16;
        let mut next = 0usize;
        for (index, &count) in counts.iter().enumerate() {
            let length = index as u8 + 1;
            for _ in 0..count {
                codes[usize::from(symbols[next])] = (code, length);
                next += 1;
                code = code.wrapping_add(1);
            }
            code <<= 1;
        }
        Self {
            counts,
            symbols,
            codes,
        }
    }

    /// The code and its length for `symbol`.
    #[must_use]
    pub fn code(&self, symbol: u8) -> (u16, u8) {
        self.codes[usize::from(symbol)]
    }

    /// The `BITS` list, as a `DHT` segment carries it.
    #[must_use]
    pub const fn counts(&self) -> &[u8; 16] {
        &self.counts
    }

    /// The `HUFFVAL` list, as a `DHT` segment carries it.
    #[must_use]
    pub const fn symbols(&self) -> &'static [u8] {
        self.symbols
    }
}

/// `BITS` for the typical luminance DC table, T.81 Table K.3.
const DC_LUMA_COUNTS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
/// `HUFFVAL` for the typical luminance DC table.
const DC_LUMA_SYMBOLS: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// `BITS` for the typical chrominance DC table, T.81 Table K.4.
const DC_CHROMA_COUNTS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
/// `HUFFVAL` for the typical chrominance DC table.
const DC_CHROMA_SYMBOLS: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// `BITS` for the typical luminance AC table, T.81 Table K.5.
const AC_LUMA_COUNTS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
/// `HUFFVAL` for the typical luminance AC table.
const AC_LUMA_SYMBOLS: &[u8] = &[
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// `BITS` for the typical chrominance AC table, T.81 Table K.6.
const AC_CHROMA_COUNTS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
/// `HUFFVAL` for the typical chrominance AC table.
const AC_CHROMA_SYMBOLS: &[u8] = &[
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// The four typical tables, derived once.
#[derive(Debug, Clone)]
pub struct Tables {
    /// DC table for the luminance component.
    pub dc_luma: Table,
    /// AC table for the luminance component.
    pub ac_luma: Table,
    /// DC table for both chrominance components.
    pub dc_chroma: Table,
    /// AC table for both chrominance components.
    pub ac_chroma: Table,
}

impl Tables {
    /// Derive all four.
    #[must_use]
    pub fn typical() -> Self {
        Self {
            dc_luma: Table::new(DC_LUMA_COUNTS, DC_LUMA_SYMBOLS),
            ac_luma: Table::new(AC_LUMA_COUNTS, AC_LUMA_SYMBOLS),
            dc_chroma: Table::new(DC_CHROMA_COUNTS, DC_CHROMA_SYMBOLS),
            ac_chroma: Table::new(AC_CHROMA_COUNTS, AC_CHROMA_SYMBOLS),
        }
    }
}

/// An MSB-first bit writer that performs JPEG's `0xFF` byte stuffing.
///
/// Inside entropy-coded data a `0xFF` octet would be read as the start of a
/// marker, so T.81 §B.1.1.5 requires a zero byte after every `0xFF` the coder
/// produces. Doing it here, in the one place bytes leave the coder, is what
/// makes it impossible to forget.
pub struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    accumulator: u32,
    held: u32,
}

impl<'a> BitWriter<'a> {
    /// Start writing entropy-coded bytes onto the end of `out`.
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            accumulator: 0,
            held: 0,
        }
    }

    /// Append the low `length` bits of `value`, most significant first.
    ///
    /// `length` is at most 16 for a Huffman code and at most 11 for a
    /// coefficient's magnitude bits, so the 32-bit accumulator can always take
    /// one more write before it is drained.
    pub fn write(&mut self, value: u32, length: u32) {
        debug_assert!(length <= 16, "a JPEG field is never wider than 16 bits");
        if length == 0 {
            return;
        }
        let masked = value & ((1u32 << length) - 1);
        self.accumulator = (self.accumulator << length) | masked;
        self.held += length;
        while self.held >= 8 {
            self.held -= 8;
            let byte = ((self.accumulator >> self.held) & 0xff) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
        }
    }

    /// Append a Huffman code.
    pub fn write_code(&mut self, (code, length): (u16, u8)) {
        debug_assert!(length > 0, "symbol is not present in this Huffman table");
        self.write(u32::from(code), u32::from(length));
    }

    /// Pad to a byte boundary with one bits and finish.
    ///
    /// T.81 §B.1.1.5 specifies one bits for the pad, so that the padding cannot
    /// be mistaken for a valid short code.
    pub fn finish(mut self) {
        if self.held > 0 {
            let pad = 8 - self.held;
            self.write(u32::MAX, pad);
        }
    }
}

/// The size category of a coefficient, and the bits that encode it.
///
/// T.81 §F.1.2.1: a non-zero value of magnitude needing `s` bits is coded as
/// the category `s` followed by `s` bits — the value itself when positive, and
/// `value + 2^s - 1` when negative, which is what makes the two halves of each
/// category contiguous.
#[must_use]
pub fn category(value: i32) -> (u8, u32) {
    if value == 0 {
        return (0, 0);
    }
    let magnitude = value.unsigned_abs();
    let size = 32 - magnitude.leading_zeros();
    let bits = if value < 0 {
        (value + (1i32 << size) - 1) as u32
    } else {
        value as u32
    };
    (size as u8, bits & ((1u32 << size) - 1))
}

/// Write a `DHT` segment for one table.
///
/// `class_and_id` is the `Tc`/`Th` byte: the high nibble is 0 for a DC table
/// and 1 for an AC table, the low nibble the table number.
///
/// # Errors
///
/// Never fails; the signature matches the rest of the writer for consistency.
pub fn write_dht(out: &mut Vec<u8>, class_and_id: u8, table: &Table) -> Result<()> {
    let symbols = table.symbols();
    out.extend_from_slice(&[0xFF, 0xC4]);
    // Length covers itself (2), Tc/Th (1), the 16 counts, and the symbols.
    let length = 2 + 1 + 16 + symbols.len();
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.push(class_and_id);
    out.extend_from_slice(table.counts());
    out.extend_from_slice(symbols);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BitWriter, Tables, category};

    #[test]
    fn every_table_declares_as_many_symbols_as_it_has_codes() {
        let tables = Tables::typical();
        for (name, table) in [
            ("dc_luma", &tables.dc_luma),
            ("ac_luma", &tables.ac_luma),
            ("dc_chroma", &tables.dc_chroma),
            ("ac_chroma", &tables.ac_chroma),
        ] {
            let declared: usize = table.counts().iter().map(|&c| usize::from(c)).sum();
            assert_eq!(
                declared,
                table.symbols().len(),
                "{name}: BITS sums to {declared} but HUFFVAL has {}",
                table.symbols().len()
            );
        }
        assert_eq!(tables.ac_luma.symbols().len(), 162);
        assert_eq!(tables.ac_chroma.symbols().len(), 162);
    }

    #[test]
    fn the_derived_codes_are_prefix_free() {
        let tables = Tables::typical();
        for (name, table) in [
            ("dc_luma", &tables.dc_luma),
            ("ac_luma", &tables.ac_luma),
            ("dc_chroma", &tables.dc_chroma),
            ("ac_chroma", &tables.ac_chroma),
        ] {
            let mut assigned: Vec<(u16, u8)> = table
                .symbols()
                .iter()
                .map(|&symbol| table.code(symbol))
                .collect();
            assert!(
                assigned.iter().all(|&(_, length)| length > 0),
                "{name}: a symbol got no code"
            );
            assigned.sort_unstable();
            for (i, &(code_a, len_a)) in assigned.iter().enumerate() {
                for &(code_b, len_b) in &assigned[i + 1..] {
                    // Is the shorter code a prefix of the longer one?
                    let (short, short_len, long) = if len_a <= len_b {
                        (code_a, len_a, code_b >> (len_b - len_a))
                    } else {
                        (code_b, len_b, code_a >> (len_a - len_b))
                    };
                    assert_ne!(
                        short, long,
                        "{name}: code {short:b} of length {short_len} is a prefix of another"
                    );
                }
            }
        }
    }

    #[test]
    fn symbols_not_in_a_table_report_zero_length() {
        let tables = Tables::typical();
        // The DC tables carry only categories 0..=11.
        assert_eq!(tables.dc_luma.code(12).1, 0);
        assert_eq!(tables.dc_luma.code(0xf0).1, 0);
        // Every AC symbol that can occur is present.
        for run in 0..16u8 {
            for size in 1..11u8 {
                let symbol = (run << 4) | size;
                assert_ne!(
                    tables.ac_luma.code(symbol).1,
                    0,
                    "AC symbol {symbol:#04x} is missing"
                );
            }
        }
        // End-of-block and the zero-run-length escape.
        assert_ne!(tables.ac_luma.code(0x00).1, 0);
        assert_ne!(tables.ac_luma.code(0xf0).1, 0);
    }

    #[test]
    fn categories_follow_t81_f_1_2_1() {
        assert_eq!(category(0), (0, 0));
        assert_eq!(category(1), (1, 1));
        assert_eq!(category(-1), (1, 0));
        assert_eq!(category(2), (2, 2));
        assert_eq!(category(3), (2, 3));
        assert_eq!(category(-2), (2, 1));
        assert_eq!(category(-3), (2, 0));
        assert_eq!(category(1023), (10, 1023));
        assert_eq!(category(-1023), (10, 0));
        // The widest DC difference the clamp in `dct` can produce.
        assert_eq!(category(2046).0, 11);
        assert_eq!(category(-2046).0, 11);
    }

    #[test]
    fn each_category_is_a_contiguous_block_of_codes() {
        // Every value of size s maps onto a distinct s-bit pattern, with the
        // negatives below the positives.
        for size in 1..=11u8 {
            let low = 1i32 << (size - 1);
            let high = (1i32 << size) - 1;
            let mut seen = Vec::new();
            for magnitude in low..=high {
                for value in [-magnitude, magnitude] {
                    let (s, bits) = category(value);
                    assert_eq!(s, size, "value {value}");
                    seen.push(bits);
                }
            }
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), 2 * (high - low + 1) as usize);
        }
    }

    #[test]
    fn bits_are_written_most_significant_first() {
        let mut out = Vec::new();
        let mut writer = BitWriter::new(&mut out);
        writer.write(0b101, 3);
        writer.write(0b01010, 5);
        writer.finish();
        assert_eq!(out, vec![0b1010_1010]);
    }

    #[test]
    fn the_pad_at_the_end_is_one_bits() {
        let mut out = Vec::new();
        let mut writer = BitWriter::new(&mut out);
        writer.write(0, 1);
        writer.finish();
        assert_eq!(out, vec![0b0111_1111]);
    }

    #[test]
    fn an_ff_byte_is_stuffed_with_a_zero() {
        let mut out = Vec::new();
        let mut writer = BitWriter::new(&mut out);
        writer.write(0xFF, 8);
        writer.write(0xFF, 8);
        writer.finish();
        assert_eq!(out, vec![0xFF, 0x00, 0xFF, 0x00]);
    }

    #[test]
    fn no_ff_byte_can_escape_unstuffed() {
        // Write every bit pattern that could straddle a byte boundary.
        for offset in 0..8u32 {
            let mut out = Vec::new();
            let mut writer = BitWriter::new(&mut out);
            writer.write(u32::MAX, offset);
            for _ in 0..4 {
                writer.write(0xFFFF, 16);
            }
            writer.finish();
            let mut index = 0;
            while index < out.len() {
                if out[index] == 0xFF {
                    assert_eq!(
                        out.get(index + 1),
                        Some(&0x00),
                        "offset {offset}: unstuffed 0xFF at {index} in {out:02x?}"
                    );
                    index += 2;
                } else {
                    index += 1;
                }
            }
        }
    }
}
