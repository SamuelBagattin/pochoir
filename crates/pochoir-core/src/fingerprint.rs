//! A content-derived fingerprint, used for the PDF trailer's `/ID`.
//!
//! ADR-0002 requires the output to be a pure function of (input bytes,
//! parameters), which rules out the usual ways of filling `/ID`: a random
//! number or a timestamp. So the identifier is derived from the bytes of the
//! file itself, and two runs over the same input produce the same `/ID`.
//!
//! This is **not** a cryptographic hash and nothing may depend on it being
//! collision-resistant. It is FNV-1a, chosen because it is ten lines, has no
//! tables, and is exactly specified over integer arithmetic so it cannot drift
//! between wasm32 and native. Two passes with different offset bases give the
//! 16 bytes `/ID` wants.
//!
//! Reference: Fowler/Noll/Vo, FNV-1a, 64-bit parameters.

/// The standard 64-bit FNV-1a offset basis.
const BASIS_A: u64 = 0xcbf2_9ce4_8422_2325;
/// A second, different starting point, so the two halves of the fingerprint are
/// not the same value. The constant is the 64-bit golden-ratio odd integer used
/// widely as a mixing seed; any value distinct from `BASIS_A` would do.
const BASIS_B: u64 = 0x9e37_79b9_7f4a_7c15;
/// The 64-bit FNV prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(bytes: &[u8], basis: u64) -> u64 {
    let mut hash = basis;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// A 16-byte fingerprint of `bytes`.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&fnv1a(bytes, BASIS_A).to_be_bytes());
    out[8..].copy_from_slice(&fnv1a(bytes, BASIS_B).to_be_bytes());
    out
}

/// `bytes` rendered as lowercase hexadecimal.
#[must_use]
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, to_hex};

    #[test]
    fn the_fingerprint_is_a_pure_function_of_the_bytes() {
        assert_eq!(fingerprint(b"pochoir"), fingerprint(b"pochoir"));
        assert_ne!(fingerprint(b"pochoir"), fingerprint(b"pochoiq"));
        assert_ne!(fingerprint(b""), fingerprint(b"\0"));
    }

    #[test]
    fn the_two_halves_differ() {
        let fp = fingerprint(b"");
        assert_ne!(fp[..8], fp[8..], "both halves used the same basis");
    }

    #[test]
    fn fnv1a_matches_the_published_test_vector() {
        // FNV-1a 64 of "a" is 0xaf63dc4c8601ec8c, from the reference vectors.
        let fp = fingerprint(b"a");
        assert_eq!(to_hex(&fp[..8]), "af63dc4c8601ec8c");
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff, 0xa5]), "000fffa5");
        assert_eq!(to_hex(&[]), "");
        assert_eq!(to_hex(&fingerprint(b"x")).len(), 32);
    }
}
