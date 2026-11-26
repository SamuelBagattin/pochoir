//! A hand-rolled TrueType reader, scoped to the one font we ship.
//!
//! [ADR-0001](../../../../docs/adr/0001-stack-and-dependency-policy.md) decides
//! that we ship **one font we choose, in-repo**, and parse it ourselves. The
//! justification is worth restating because it is what makes this module small
//! enough to own: our rule is *depend deliberately only for parsers of hostile
//! input*, and the bundled subset is not hostile input. It is a build artefact
//! we produced from a known upstream with `scripts/subset-font.py`, its
//! `sha256` is recorded beside it, and it cannot change without a commit. So
//! this reader does not have to survive a fuzzer the way [`crate::image`]'s
//! decoders do.
//!
//! It is still written defensively — every read is bounds-checked and returns
//! [`Error::Decode`] rather than panicking — because "cannot be hostile" is a
//! property of how the file arrives, not a licence to index a slice blind, and
//! because [`Font::parse`] is public: a caller may hand it anything.
//!
//! # What is read, and what is ignored
//!
//! | Table | Used for |
//! | --- | --- |
//! | `head` | units per em, and which width `loca` entries have |
//! | `hhea` | ascender and descender, and how many `hmtx` entries are full ones |
//! | `maxp` | glyph count, to bound `loca` |
//! | `cmap` | character to glyph, format 4 only |
//! | `hmtx` | advance widths |
//! | `loca` | where each glyph's outline is |
//! | `glyf` | the outlines themselves, in [`outline`] |
//!
//! Nothing else. There is no `GPOS`, `GSUB` or `kern` read, so there is no
//! kerning and no shaping — ADR-0001 scopes the renderer that way, and
//! ADR-0002's mark is Latin text in one font at one size, which is the case
//! that needs neither. There is no hinting: `fpgm`, `prep` and the per-glyph
//! instruction streams are skipped, and the subsetter already dropped them.

pub mod outline;

use crate::{Error, Result};

pub use outline::{Contour, Outline, Segment};

/// The font subset Pochoir draws every mark with.
///
/// Roboto Regular, Apache-2.0, cut down to the Latin ranges a French mark
/// needs. See `assets/roboto/README.md` for provenance and the upstream hash.
pub const ROBOTO_SUBSET: &[u8] = include_bytes!("../../assets/roboto/Roboto-Regular-subset.ttf");

/// Deepest a composite glyph may reference another composite glyph.
///
/// TrueType permits arbitrary nesting and nothing in the format prevents a
/// cycle. Latin accented glyphs are one level deep — `é` is `e` plus `acute` —
/// so four is already generous, and a bound is what turns a malformed font
/// from a stack overflow into an error.
pub const MAX_COMPOSITE_DEPTH: u8 = 4;

/// Why a font was refused, as the `reason` of [`Error::Decode`].
fn bad(reason: &'static str) -> Error {
    Error::Decode {
        format: "TrueType font",
        reason,
    }
}

/// A bounds-checked big-endian reader over a byte slice.
///
/// Every accessor takes an absolute offset and returns a `Result`, so no read
/// in this module can panic or wrap. It is a struct rather than free functions
/// only so that the slice and its error format travel together.
#[derive(Debug, Clone, Copy)]
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn slice(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        self.bytes
            .get(at..at.checked_add(len).ok_or_else(|| bad("offset overflow"))?)
            .ok_or_else(|| bad("table ends early"))
    }

    fn u16_at(&self, at: usize) -> Result<u16> {
        let bytes = self.slice(at, 2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn i16_at(&self, at: usize) -> Result<i16> {
        self.u16_at(at).map(|v| v as i16)
    }

    fn u32_at(&self, at: usize) -> Result<u32> {
        let bytes = self.slice(at, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// How wide a `loca` entry is, which `head.indexToLocFormat` decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocaFormat {
    /// `u16` entries, each holding half the real offset.
    Short,
    /// `u32` entries holding the offset directly.
    Long,
}

/// A parsed TrueType font, borrowing the bytes it was built from.
///
/// Parsing resolves the table directory and the handful of header fields that
/// every later lookup needs; outlines and advance widths are read on demand.
/// The struct is therefore cheap to build and cheap to keep.
#[derive(Debug, Clone)]
pub struct Font<'a> {
    cmap: &'a [u8],
    glyf: &'a [u8],
    loca: &'a [u8],
    hmtx: &'a [u8],
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    glyph_count: u16,
    horizontal_metrics: u16,
    loca_format: LocaFormat,
}

impl Font<'static> {
    /// The bundled Roboto subset, parsed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Decode`] only if the committed asset is corrupt, which
    /// a unit test in this module rules out for every build.
    pub fn bundled() -> Result<Self> {
        Self::parse(ROBOTO_SUBSET)
    }
}

impl<'a> Font<'a> {
    /// Read a font's table directory and headers.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Decode`] if `bytes` is not an sfnt file with TrueType
    /// outlines, if a table this module needs is absent, or if any header field
    /// lies outside the file.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let reader = Reader::new(bytes);
        match reader.u32_at(0)? {
            // 'true' is the old Apple tag for the same thing; both mean glyf
            // outlines. 'OTTO' means CFF, which ADR-0001 excludes.
            0x0001_0000 | 0x7472_7565 => {}
            _ => return Err(bad("not an sfnt file with TrueType outlines")),
        }
        let table_count = usize::from(reader.u16_at(4)?);

        let find = |tag: &[u8; 4]| -> Result<&'a [u8]> {
            for index in 0..table_count {
                let entry = 12 + index * 16;
                if reader.slice(entry, 4)? == tag {
                    let offset = reader.u32_at(entry + 8)? as usize;
                    let length = reader.u32_at(entry + 12)? as usize;
                    return reader.slice(offset, length);
                }
            }
            Err(bad("font is missing a table we need"))
        };

        let head = Reader::new(find(b"head")?);
        let hhea = Reader::new(find(b"hhea")?);
        let maxp = Reader::new(find(b"maxp")?);
        let cmap = find(b"cmap")?;
        let glyf = find(b"glyf")?;
        let loca = find(b"loca")?;
        let hmtx = find(b"hmtx")?;

        let units_per_em = head.u16_at(18)?;
        if units_per_em == 0 {
            return Err(bad("head.unitsPerEm is zero"));
        }
        let loca_format = match head.i16_at(50)? {
            0 => LocaFormat::Short,
            1 => LocaFormat::Long,
            _ => return Err(bad("head.indexToLocFormat is neither 0 nor 1")),
        };
        let glyph_count = maxp.u16_at(4)?;
        let horizontal_metrics = hhea.u16_at(34)?;
        if horizontal_metrics == 0 {
            return Err(bad("hhea.numberOfHMetrics is zero"));
        }

        let font = Self {
            cmap,
            glyf,
            loca,
            hmtx,
            units_per_em,
            ascender: hhea.i16_at(4)?,
            descender: hhea.i16_at(6)?,
            glyph_count,
            horizontal_metrics,
            loca_format,
        };
        // Prove `loca` is long enough now rather than on the first glyph: the
        // table has one more entry than there are glyphs, and a short table
        // would otherwise surface as a mysterious per-glyph failure.
        let entries = usize::from(glyph_count) + 1;
        let needed = match loca_format {
            LocaFormat::Short => entries * 2,
            LocaFormat::Long => entries * 4,
        };
        if font.loca.len() < needed {
            return Err(bad("loca is shorter than the glyph count implies"));
        }
        Ok(font)
    }

    /// Font units to the em, from `head.unitsPerEm`. 2048 for Roboto.
    #[must_use]
    pub const fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// `hhea.ascender`, in font units above the baseline.
    #[must_use]
    pub const fn ascender(&self) -> i16 {
        self.ascender
    }

    /// `hhea.descender`, in font units; negative, being below the baseline.
    #[must_use]
    pub const fn descender(&self) -> i16 {
        self.descender
    }

    /// How many glyphs the font has, from `maxp.numGlyphs`.
    #[must_use]
    pub const fn glyph_count(&self) -> u16 {
        self.glyph_count
    }

    /// The glyph id for `character`, or `None` if the font has no mapping.
    ///
    /// Glyph id 0 is `.notdef` and is reported as `None`: a caller that wanted
    /// a tofu box could ask for glyph 0 deliberately, but nothing in Pochoir
    /// does — [`crate::mark`] refuses the text instead, so that an unsupported
    /// character is an error the user can read rather than a box in the middle
    /// of their watermark.
    #[must_use]
    pub fn glyph_for(&self, character: char) -> Option<u16> {
        // Format 4 is a 16-bit format; anything above the BMP cannot be in it.
        let code = u16::try_from(u32::from(character)).ok()?;
        let glyph = self.lookup_cmap(code).ok().flatten()?;
        (glyph != 0 && glyph < self.glyph_count).then_some(glyph)
    }

    /// Walk the `cmap` subtables for a Unicode format 4 one and look `code` up.
    fn lookup_cmap(&self, code: u16) -> Result<Option<u16>> {
        let cmap = Reader::new(self.cmap);
        let subtables = usize::from(cmap.u16_at(2)?);
        for index in 0..subtables {
            let record = 4 + index * 8;
            let platform = cmap.u16_at(record)?;
            let encoding = cmap.u16_at(record + 2)?;
            let at = cmap.u32_at(record + 4)? as usize;
            // Windows BMP (3,1) and the three Unicode BMP encodings (0,3),
            // (0,4), (0,6). Any of them addresses the same glyphs.
            let unicode = matches!((platform, encoding), (3, 1) | (0, 3) | (0, 4) | (0, 6));
            if !unicode || cmap.u16_at(at)? != 4 {
                continue;
            }
            return self.lookup_format4(&cmap, at, code);
        }
        Err(bad("font has no format 4 Unicode cmap subtable"))
    }

    /// The format 4 binary search, done linearly.
    ///
    /// Format 4 ships `searchRange`, `entrySelector` and `rangeShift` to drive
    /// a binary search; we ignore all three and scan. The subset has a few
    /// dozen segments and a mark is a couple of hundred characters, so the
    /// whole lookup cost is noise beside rasterising one glyph, and a linear
    /// scan has no off-by-one to get wrong.
    fn lookup_format4(&self, cmap: &Reader<'a>, at: usize, code: u16) -> Result<Option<u16>> {
        let segments = usize::from(cmap.u16_at(at + 6)?) / 2;
        if segments == 0 {
            return Err(bad("cmap format 4 has no segments"));
        }
        let ends = at + 14;
        let starts = ends + segments * 2 + 2; // +2 for reservedPad
        let deltas = starts + segments * 2;
        let ranges = deltas + segments * 2;

        for segment in 0..segments {
            let end = cmap.u16_at(ends + segment * 2)?;
            if code > end {
                continue;
            }
            let start = cmap.u16_at(starts + segment * 2)?;
            if code < start {
                return Ok(None);
            }
            let delta = cmap.i16_at(deltas + segment * 2)?;
            // The quirk that makes this table awkward: `idRangeOffset` is a
            // byte offset from **the position of that entry itself**, not from
            // the start of the subtable or of the array. Getting this wrong
            // yields plausible-looking glyph ids for the wrong characters, so
            // it is worth the extra local.
            let range_entry = ranges + segment * 2;
            let range_offset = cmap.u16_at(range_entry)?;
            if range_offset == 0 {
                // No glyph array: add the delta, modulo 2^16, as the spec says.
                return Ok(Some(code.wrapping_add(delta as u16)));
            }
            let index = usize::from(code - start);
            let glyph_at = range_entry + usize::from(range_offset) + index * 2;
            let glyph = cmap.u16_at(glyph_at)?;
            // A zero here means "no glyph", and the delta is not applied.
            return Ok((glyph != 0).then(|| glyph.wrapping_add(delta as u16)));
        }
        Ok(None)
    }

    /// The advance width of `glyph`, in font units.
    ///
    /// `hmtx` stores `numberOfHMetrics` full entries and then left-sidebearings
    /// alone; every glyph past that point repeats the last advance, which is
    /// how monospaced tails are compressed. Returns `None` for a glyph id the
    /// font does not have.
    #[must_use]
    pub fn advance_width(&self, glyph: u16) -> Option<u16> {
        if glyph >= self.glyph_count {
            return None;
        }
        let hmtx = Reader::new(self.hmtx);
        let index = glyph.min(self.horizontal_metrics - 1);
        hmtx.u16_at(usize::from(index) * 4).ok()
    }

    /// Where `glyph`'s outline lives in `glyf`, as a byte range.
    ///
    /// An empty range is legitimate and means a glyph with no outline, such as
    /// a space: `loca[n] == loca[n + 1]`.
    fn glyph_range(&self, glyph: u16) -> Result<core::ops::Range<usize>> {
        if glyph >= self.glyph_count {
            return Err(bad("glyph id past the end of the font"));
        }
        let loca = Reader::new(self.loca);
        let index = usize::from(glyph);
        let (start, end) = match self.loca_format {
            LocaFormat::Short => (
                usize::from(loca.u16_at(index * 2)?) * 2,
                usize::from(loca.u16_at(index * 2 + 2)?) * 2,
            ),
            LocaFormat::Long => (
                loca.u32_at(index * 4)? as usize,
                loca.u32_at(index * 4 + 4)? as usize,
            ),
        };
        if end < start || end > self.glyf.len() {
            return Err(bad("loca entry does not describe a range inside glyf"));
        }
        Ok(start..end)
    }

    /// The outline of `glyph`, in font units, y-up.
    ///
    /// Composite glyphs are resolved into their components' contours, so the
    /// caller never sees the distinction. A glyph with no outline — a space —
    /// yields an [`Outline`] with no contours rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Decode`] for a glyph id the font does not have, for a
    /// malformed `glyf` entry, or if composite nesting exceeds
    /// [`MAX_COMPOSITE_DEPTH`].
    pub fn outline(&self, glyph: u16) -> Result<Outline> {
        let mut out = Outline::default();
        self.append_outline(glyph, crate::geom::Transform::IDENTITY, 0, &mut out)?;
        Ok(out)
    }

    /// The recursive half of [`Font::outline`].
    fn append_outline(
        &self,
        glyph: u16,
        transform: crate::geom::Transform,
        depth: u8,
        out: &mut Outline,
    ) -> Result<()> {
        if depth > MAX_COMPOSITE_DEPTH {
            return Err(bad("composite glyph nests too deeply"));
        }
        let range = self.glyph_range(glyph)?;
        if range.is_empty() {
            return Ok(());
        }
        let data = &self.glyf[range];
        outline::append(self, data, transform, depth, out)
    }
}

#[cfg(test)]
mod tests {
    use super::{Font, ROBOTO_SUBSET};

    #[test]
    fn the_bundled_asset_parses() {
        let font = Font::bundled().expect("the committed subset must parse");
        assert_eq!(font.units_per_em(), 2048, "Roboto is a 2048-unit font");
        assert!(font.ascender() > 0 && font.descender() < 0);
        // The README records 347 glyphs; a mismatch means the asset changed
        // without its documentation, which is worth failing over.
        assert_eq!(font.glyph_count(), 347);
    }

    #[test]
    fn ascii_and_french_characters_all_have_glyphs() {
        let font = Font::bundled().expect("parses");
        for character in "The quick brown fox 0123456789 .,;:!?-()".chars() {
            assert!(
                font.glyph_for(character).is_some(),
                "no glyph for {character:?}"
            );
        }
        for character in "éèêëàâäîïôöùûüçÉÈÊÀÇÙ«»€—’…".chars() {
            assert!(
                font.glyph_for(character).is_some(),
                "no glyph for {character:?}"
            );
        }
    }

    #[test]
    fn characters_outside_the_subset_report_no_glyph() {
        let font = Font::bundled().expect("parses");
        // Cyrillic, CJK, an emoji and an astral-plane codepoint: the subsetter
        // dropped all of these, and `glyph_for` must say so rather than hand
        // back glyph 0.
        for character in ['Ж', '漢', '✓', '🙂', '\u{10FFFF}'] {
            assert_eq!(
                font.glyph_for(character),
                None,
                "{character:?} should not be in the subset"
            );
        }
    }

    #[test]
    fn distinct_characters_get_distinct_glyphs() {
        let font = Font::bundled().expect("parses");
        let e = font.glyph_for('e').expect("e");
        let acute = font.glyph_for('é').expect("é");
        assert_ne!(e, acute, "`e` and `é` must be different glyphs");
    }

    #[test]
    fn advance_widths_are_plausible() {
        let font = Font::bundled().expect("parses");
        let space = font.glyph_for(' ').expect("space");
        let m = font.glyph_for('M').expect("M");
        let i = font.glyph_for('i').expect("i");
        let em = u32::from(font.units_per_em());
        for glyph in [space, m, i] {
            let advance = u32::from(font.advance_width(glyph).expect("advance"));
            assert!(advance > 0 && advance < em, "advance {advance} of {glyph}");
        }
        assert!(
            font.advance_width(m) > font.advance_width(i),
            "`M` should be wider than `i` in a proportional font"
        );
        assert_eq!(font.advance_width(font.glyph_count()), None);
    }

    #[test]
    fn a_space_has_no_outline_but_is_not_an_error() {
        let font = Font::bundled().expect("parses");
        let space = font.glyph_for(' ').expect("space");
        let outline = font.outline(space).expect("a space is a legal glyph");
        assert!(outline.contours.is_empty());
    }

    #[test]
    fn garbage_is_refused_rather_than_panicking() {
        assert!(Font::parse(b"").is_err());
        assert!(Font::parse(b"OTTO").is_err());
        assert!(Font::parse(&[0; 64]).is_err());
        // A plausible header with nothing behind it.
        let mut bytes = vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x09];
        bytes.extend_from_slice(&[0; 32]);
        assert!(Font::parse(&bytes).is_err());
        // Every truncation of the real font must be refused or parse cleanly,
        // never panic.
        for length in (0..ROBOTO_SUBSET.len()).step_by(1021) {
            let _ = Font::parse(&ROBOTO_SUBSET[..length]);
        }
    }
}
