//! The watermark: ADR-0002's mark, burned into the pixels.
//!
//! # Path independence is structural, not maintained
//!
//! [`compose`] takes a `&mut Pixmap` and the mark's parameters. **That is all
//! it takes.** It does not see a [`PageSource`], a decoder, a page geometry or
//! an input format, and it is called from exactly one place —
//! [`crate::render_to_pdf`]'s loop, between `rasterise` and `jpeg::encode`.
//! At that point the code physically cannot know whether the pixels came from
//! a JPEG, a PNG or a rasterised PDF page.
//!
//! This is ADR-0002 Decision 1's "one compositor, operating on an RGBA pixmap",
//! and the reason it is worth being pedantic about is in that ADR: the tempting
//! alternative is to draw the mark through a PDF renderer's own scene API for
//! PDF inputs, which gives you two watermark implementations that must be kept
//! visually identical, two golden suites, and a bug class that does not
//! otherwise exist. Keeping the call site where it is makes that impossible
//! rather than merely discouraged.
//!
//! # Alpha, and why the compositor must do its own blending
//!
//! [`Pixmap`] is **straight** alpha, not premultiplied, and
//! [`crate::jpeg::encode`] composites what it is given over opaque **white**,
//! because JPEG has no alpha channel and white is the right assumption for a
//! document. So a semi-transparent mark left in the pixmap would be flattened
//! onto white rather than onto the page underneath — visibly wrong over a dark
//! photograph. This module therefore does its own source-over and hands the
//! encoder fully opaque pixels, leaving `a = 255` on everything it touches.
//!
//! # Determinism
//!
//! Everything here is `+ - * /`, `min`, `max`, `floor`, `ceil`, `sqrt` and
//! integer arithmetic, all of which IEEE-754 and Rust specify exactly, plus
//! [`trig`]'s own hand-rolled sine and cosine. No `sin`, no `cos`, no `powf`,
//! no `f32::round`. ADR-0002 Decision 6 explains what one `f32::sin` would
//! cost, and nothing in CI would catch it.
//!
//! [`PageSource`]: crate::page::PageSource

mod blur;
mod layout;
mod nfc;
mod prng;
mod trig;

pub use layout::{LINE_HEIGHT, Layout, MAX_LINES, MIN_FONT_SIZE_RATIO, measure};
pub use nfc::normalise;

use crate::font::Font;
use crate::geom::Point;
use crate::pixmap::Pixmap;
use crate::{Error, Result};

/// Font size as a fraction of the page's shorter edge. ADR-0002 Decision 3.
pub const DEFAULT_FONT_SIZE_RATIO: f32 = 0.035;

/// Largest font size the mark may be asked for, as a fraction of the shorter
/// edge. ADR-0002 Decision 3's stated range is 1–10%.
pub const MAX_FONT_SIZE_RATIO: f32 = 0.10;

/// Rotation in degrees. Negative is anticlockwise on the page, so the mark
/// runs from the bottom left to the top right.
///
/// ADR-0002 Decision 3: a horizontal mark competes with the document's own
/// text lines and crops out easily, while a diagonal one crosses them, so
/// removing it means reconstructing text across the stroke. Steeper than about
/// 40° starts to hurt the mark's own legibility.
pub const DEFAULT_ANGLE_DEGREES: f32 = -30.0;

/// Opacity of the mark. ADR-0002 Decision 3.
///
/// The mark must not stop the recipient reading the document — that defeats
/// its purpose. Coverage comes from tiling, not from making one mark heavy.
pub const DEFAULT_OPACITY: f32 = 0.30;

/// Smallest opacity the mark may be set to.
pub const MIN_OPACITY: f32 = 0.05;

/// Largest opacity the mark may be set to.
pub const MAX_OPACITY: f32 = 0.80;

/// Neutral dark grey, `#1A1A1A`. ADR-0002 Decision 3, and load-bearing.
///
/// The attacker's next move is to print and rescan, or photocopy in black and
/// white. A dark grey mark survives greyscale reproduction and aggressive JPEG
/// re-compression; a pale or pastel one can vanish entirely.
pub const DEFAULT_COLOUR: [u8; 3] = [0x1A, 0x1A, 0x1A];

/// Blur σ as a fraction of the **font size**, not of a pixel count.
///
/// ADR-0002 says "σ ≈ 0.6 px at render resolution, scaled with DPI", which
/// pulls both ways at once and, at its own default, rounds to a no-op.
/// [ADR-0007](../../../../docs/adr/0007-watermark-specification-corrections.md)
/// re-expresses it as a fraction of the font size, which is
/// resolution-independent in the same way the glyph size already is. 0.015 is
/// about 0.87 px at the default font size on A4 at 200 dpi — a blur that
/// demonstrably changes pixels, which the test suite asserts.
pub const DEFAULT_BLUR_RATIO: f32 = 0.015;

/// Largest blur the mark may be given, as a fraction of the font size.
///
/// ADR-0002's "0–3 px" range re-expressed in these units: 0.06 is about 3.5 px
/// at the default font size on A4 at 200 dpi.
pub const MAX_BLUR_RATIO: f32 = 0.06;

/// Longest mark text, in characters. ADR-0002 Decision 3.
pub const MAX_MARK_CHARACTERS: usize = 200;

/// How far a tile may be displaced, as a fraction of the lattice step.
///
/// 0.15 of the step is roughly nine tenths of a line at the default density:
/// plainly visible when two runs are differenced, which is the point.
///
/// It is further **capped at half the free gap** between neighbouring tiles —
/// see [`Lattice::new`]. Without that cap two adjacent tiles can meet, and two
/// marks composited over each other are both twice as dark as the chosen
/// opacity and illegible where they cross. The cap is why the compositor can
/// blend each tile straight into the pixmap instead of accumulating coverage
/// across the whole page first.
pub const JITTER_FRACTION: f32 = 0.15;

/// Below this fraction of the font size the caller asked for, a mark is
/// reported as having been shrunk.
///
/// A quarter smaller than requested is the point at which somebody who chose a
/// size deserves to be told they did not get it. Like
/// [`Mark::colour_is_too_pale`], this is a **warning and never an error**: the
/// mark is still drawn, and the alternative — refusing — is worse on every
/// input that is merely long. See ADR-0007 Decision 2.
///
/// It is deliberately a statement about the size the caller asked for and not
/// about legibility, which we have not measured and would not know how to
/// state honestly.
pub const FAINT_SHRINK_RATIO: f32 = 0.75;

/// Luminance above which a colour is called too pale to survive a photocopier.
///
/// Rec. 709 relative luminance on the sRGB values directly, which is a rough
/// measure and good enough for a warning. ADR-0002 Decision 3 says we "allow a
/// colour choice and warn when the chosen colour's luminance is too high"; this
/// is that threshold, and it warns rather than refusing because the choice is
/// the user's.
pub const PALE_LUMINANCE: f32 = 0.55;

/// Most tiles the lattice may hold, as a guard rather than a real limit.
///
/// The default page and density use about twenty. A pathological combination of
/// a huge page and a tiny mark is refused rather than left to churn.
const MAX_TILES: usize = 65_536;

/// How thickly the mark is tiled across the page.
///
/// ADR-0002 Decision 3 specifies `normal` as "gaps of 1.6× text width, 4× line
/// height", aiming at six to ten visible repetitions on a portrait page;
/// `low` and `high` double and halve those gaps.
///
/// **Both gaps here are multiples of the line height**, and the horizontal one
/// is the place this departs from the ADR's words. A gap proportional to the
/// *text width* is unimplementable for a realistic mark: at the default font
/// size the text is about as wide as the page, so a gap of 1.6× its width is
/// wider than the page, and — because the mark is rotated — each row of tiles
/// has to span the page's **diagonal**, 2601 px on A4 at 200 dpi rather than
/// 1654. One column of 1634 px therefore leaves a hole of roughly 1000 px in
/// every row, and the page comes out with four visible marks and large blank
/// regions instead of the six to ten even ones the same table asks for. An
/// unmarked region of the page is an unmarked crop of the document, which is
/// the failure this product exists to prevent.
///
/// Keyed to the line height, the horizontal gap is a modest absolute distance
/// that behaves sensibly at both extremes: a page-wide mark repeats with a
/// small gap and fills each row, and a short mark like `CONFIDENTIEL` repeats
/// several times across each row, which is what a tiled watermark is supposed
/// to look like. ADR-0007 records this as the fifth correction, and that it was
/// found by measuring the page rather than by re-reading the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileDensity {
    /// Half as many repetitions as `Normal`.
    Low,
    /// ADR-0002's default, as corrected: gaps of 1.6 and 4 line heights.
    #[default]
    Normal,
    /// Twice as many repetitions as `Normal`.
    High,
}

impl TileDensity {
    /// The gaps between tiles, as multiples of the line height:
    /// `(across, down)`.
    #[must_use]
    pub const fn gaps(self) -> (f32, f32) {
        match self {
            Self::Low => (3.2, 8.0),
            Self::Normal => (1.6, 4.0),
            Self::High => (0.8, 2.0),
        }
    }
}

/// The watermark to draw, as the user chose it.
///
/// [`Default`] is ADR-0002 Decision 3's recommended set, with empty text —
/// which is not a valid mark. The field starts empty on purpose: ADR-0002's
/// closed open question decides that nothing is pre-filled, because a default
/// nobody edits produces a mark naming no recipient and no purpose, which is
/// exactly the mark that fails to deter reuse.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// What the mark says. Normalised to NFC before layout.
    pub text: String,
    /// Font size as a fraction of the page's shorter edge.
    pub font_size_ratio: f32,
    /// Rotation in degrees; negative runs bottom-left to top-right.
    pub angle_degrees: f32,
    /// Opacity of the mark, `0.05..=0.80`.
    pub opacity: f32,
    /// Mark colour, sRGB.
    pub colour: [u8; 3],
    /// How thickly to tile.
    pub density: TileDensity,
    /// Blur σ as a fraction of the font size.
    pub blur_ratio: f32,
}

impl Default for Mark {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size_ratio: DEFAULT_FONT_SIZE_RATIO,
            angle_degrees: DEFAULT_ANGLE_DEGREES,
            opacity: DEFAULT_OPACITY,
            colour: DEFAULT_COLOUR,
            density: TileDensity::default(),
            blur_ratio: DEFAULT_BLUR_RATIO,
        }
    }
}

impl Mark {
    /// A mark with ADR-0002's defaults and this text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Check every parameter, and that the text can be drawn.
    ///
    /// Called before any pixels are touched, so a bad parameter or an
    /// undrawable character costs nothing.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidParameter`] naming the first offending field, or
    /// [`Error::UnsupportedCharacter`] naming the first character the bundled
    /// font cannot draw. Use [`unsupported_characters`] to list them all.
    pub fn validate(&self) -> Result<()> {
        if !(MIN_FONT_SIZE_RATIO..=MAX_FONT_SIZE_RATIO).contains(&self.font_size_ratio) {
            return Err(Error::InvalidParameter {
                name: "mark font size",
            });
        }
        if !(-90.0..=90.0).contains(&self.angle_degrees) {
            return Err(Error::InvalidParameter { name: "mark angle" });
        }
        if !(MIN_OPACITY..=MAX_OPACITY).contains(&self.opacity) {
            return Err(Error::InvalidParameter {
                name: "mark opacity",
            });
        }
        if !(0.0..=MAX_BLUR_RATIO).contains(&self.blur_ratio) {
            return Err(Error::InvalidParameter { name: "mark blur" });
        }
        self.prepared().map(|_| ())
    }

    /// The text, NFC-normalised, if every character of it can be drawn.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidParameter`] for text that is empty or over
    /// [`MAX_MARK_CHARACTERS`], and [`Error::UnsupportedCharacter`] for the
    /// first character the font lacks.
    pub fn prepared(&self) -> Result<String> {
        let text = normalise(&self.text);
        if text.trim().is_empty() {
            return Err(Error::InvalidParameter { name: "mark text" });
        }
        if text.chars().count() > MAX_MARK_CHARACTERS {
            return Err(Error::InvalidParameter { name: "mark text" });
        }
        if let Some(character) = unsupported_characters(&self.text)?.first().copied() {
            return Err(Error::UnsupportedCharacter { character });
        }
        Ok(text)
    }

    /// `true` when the chosen colour is light enough that a photocopy might
    /// lose it. A warning for the UI, never an error.
    #[must_use]
    pub fn colour_is_too_pale(&self) -> bool {
        let [r, g, b] = self.colour;
        // Rec. 709 coefficients, applied to sRGB values without linearising:
        // crude, and ample for a threshold.
        let luminance =
            (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
        luminance > PALE_LUMINANCE
    }
}

/// Every character of `text` that the bundled font cannot draw, after NFC.
///
/// Returned in order of first appearance and without duplicates, so a UI can
/// name all of them at once while [`Mark::prepared`] names only the first —
/// [`Error`]'s variants carry `Copy` data only, by design, so the error itself
/// cannot hold a list.
///
/// A leftover combining mark is reported like any other character. That is the
/// point: it means NFC could not compose it, and drawing it would put a stray
/// accent floating on the baseline rather than over its letter.
///
/// # Errors
///
/// [`Error::Decode`] only if the bundled font asset is corrupt.
pub fn unsupported_characters(text: &str) -> Result<Vec<char>> {
    let font = Font::bundled()?;
    let mut out: Vec<char> = Vec::new();
    for character in normalise(text).chars() {
        // Spaces and newlines are layout, not glyphs: the wrapper consumes
        // them and never asks the font for one.
        if character.is_whitespace() {
            continue;
        }
        if font.glyph_for(character).is_none() && !out.contains(&character) {
            out.push(character);
        }
    }
    Ok(out)
}

/// What [`compose`] did, for the caller to report and for a future preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// How the text was laid out, including whether it had to shrink.
    pub layout: Layout,
    /// How many tiles were placed.
    pub tiles: usize,
    /// The blur σ actually used, in pixels.
    pub sigma_px: f32,
    /// The font size the mark's parameters asked for, in pixels.
    ///
    /// [`Layout::font_size_px`] is what it was actually drawn at, which is
    /// smaller whenever the text had to be shrunk to fit.
    pub requested_font_size_px: f32,
}

impl Composition {
    /// The fraction of the requested font size the mark was drawn at.
    ///
    /// 1.0 when nothing was shrunk. Never above 1.0: the layout only ever
    /// shrinks.
    #[must_use]
    pub fn shrink_ratio(&self) -> f32 {
        if self.requested_font_size_px > 0.0 {
            (self.layout.font_size_px / self.requested_font_size_px).min(1.0)
        } else {
            1.0
        }
    }

    /// `true` when the mark had to be drawn markedly smaller than asked for.
    ///
    /// A warning for the caller to pass on, never an error — see
    /// [`FAINT_SHRINK_RATIO`]. ADR-0007 Decision 2 keeps the drawing and adds
    /// the warning, because silently succeeding beats refusing on text that is
    /// merely long, and saying nothing at all is what was actually wrong.
    #[must_use]
    pub fn was_shrunk_noticeably(&self) -> bool {
        self.shrink_ratio() < FAINT_SHRINK_RATIO
    }
}

/// Burn `mark` into `pixmap`.
///
/// `seed` drives the position jitter; `jitter` turns it off, in which case the
/// lattice sits on an unshifted grid centred on the page. ADR-0002 Decision 6
/// requires both to be exposed, and [`crate::OutputOptions`] carries them.
///
/// The pixmap's dimensions are the only thing about the page this function
/// knows, and it changes none of them: the mark must never move the page
/// geometry, because the density written into the embedded JPEG is derived from
/// it.
///
/// # Errors
///
/// [`Error::InvalidParameter`] for a bad parameter or a degenerate page,
/// [`Error::UnsupportedCharacter`] for text the font cannot draw,
/// [`Error::MarkDoesNotFit`] for text that will not fit at any permitted size,
/// and [`Error::ImageTooLarge`] for a stamp past [`crate::pixmap::MAX_PIXELS`].
pub fn compose(pixmap: &mut Pixmap, mark: &Mark, seed: u64, jitter: bool) -> Result<Composition> {
    let text = {
        mark.validate()?;
        mark.prepared()?
    };
    if pixmap.is_empty() {
        return Err(Error::InvalidParameter { name: "page size" });
    }
    let font = Font::bundled()?;

    let page_width = pixmap.width() as f32;
    let page_height = pixmap.height() as f32;
    let requested = mark.font_size_ratio * page_width.min(page_height);
    let layout = layout::plan(
        &font,
        &text,
        page_width,
        page_height,
        requested,
        mark.angle_degrees,
    )?;

    // σ is keyed to the size the text actually ended up at, so a mark that had
    // to shrink gets a blur in proportion to its glyphs rather than to the size
    // that did not fit.
    let sigma_px = mark.blur_ratio * layout.font_size_px;
    let stamp = layout::stamp(&font, &layout, sigma_px)?;

    let tiles = tile(pixmap, &stamp, &layout, mark, seed, jitter)?;
    Ok(Composition {
        layout,
        tiles,
        sigma_px,
        requested_font_size_px: requested,
    })
}

/// Where one tile of the mark sits, in the mark's own rotated frame.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    /// Lattice column, which keys the jitter.
    column: i64,
    /// Lattice row, which keys the jitter.
    row: i64,
    /// The stamp's top-left corner, in mark space, jitter included.
    origin: Point,
}

/// The lattice: how the mark is stepped across the page, and how far each tile
/// is displaced from its lattice point.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lattice {
    step_x: f32,
    step_y: f32,
    phase_x: f32,
    phase_y: f32,
    /// Largest displacement a tile may take, across and down.
    swing_x: f32,
    swing_y: f32,
}

impl Lattice {
    /// The lattice for a laid-out mark at a given density, stamp and seed.
    fn new(layout: &Layout, stamp: (f32, f32), mark: &Mark, seed: u64, jitter: bool) -> Self {
        let (gap_across, gap_down) = mark.density.gaps();
        // Both gaps are line heights. See `TileDensity` for why the horizontal
        // one is not a multiple of the text width, which is what ADR-0002 says
        // and what ADR-0007 corrects.
        let step_x = (layout.block_width_px + gap_across * layout.line_height_px).max(1.0);
        let step_y = (layout.block_height_px + gap_down * layout.line_height_px).max(1.0);
        // How far a tile may move before it touches its neighbour. Two tiles
        // that meet composite over each other, which is twice the chosen
        // opacity and illegible where the glyphs cross, so the jitter is
        // bounded by the gap it has to play in.
        let swing_x = (JITTER_FRACTION * step_x)
            .min((step_x - stamp.0) * 0.5)
            .max(0.0);
        let swing_y = (JITTER_FRACTION * step_y)
            .min((step_y - stamp.1) * 0.5)
            .max(0.0);
        // A global phase so the lattice itself differs between documents, with
        // the per-tile offsets on top of it. ADR-0002's letter asks only for
        // this part; ADR-0007 adds the per-tile part, which is what "a remover
        // tuned to fixed tile positions fails" actually requires.
        let mut run = prng::Rng::new(seed);
        let (phase_x, phase_y) = if jitter {
            (run.next_unit() * step_x, run.next_unit() * step_y)
        } else {
            (0.0, 0.0)
        };
        Self {
            step_x,
            step_y,
            phase_x,
            phase_y,
            swing_x,
            swing_y,
        }
    }

    /// Where tile `(column, row)` goes.
    fn placement(&self, seed: u64, jitter: bool, column: i64, row: i64) -> Placement {
        let (offset_x, offset_y) = if jitter {
            let mut rng = prng::for_tile(seed, column, row);
            (
                rng.next_signed() * self.swing_x,
                rng.next_signed() * self.swing_y,
            )
        } else {
            (0.0, 0.0)
        };
        Placement {
            column,
            row,
            origin: Point::new(
                column as f32 * self.step_x + self.phase_x + offset_x,
                row as f32 * self.step_y + self.phase_y + offset_y,
            ),
        }
    }
}

/// The rotation, both ways: the mark's frame onto the page and back.
///
/// This is the only place in the crate that turns anything, which is also where
/// the two hand-rolled trigonometric values live.
#[derive(Debug, Clone, Copy)]
struct Frame {
    sine: f32,
    cosine: f32,
    page_centre: Point,
    stamp_centre: Point,
}

impl Frame {
    fn new(angle_degrees: f32, page: (f32, f32), stamp: (f32, f32)) -> Self {
        let (sine, cosine) = trig::sin_cos_degrees(angle_degrees);
        Self {
            sine,
            cosine,
            page_centre: Point::new(page.0 * 0.5, page.1 * 0.5),
            stamp_centre: Point::new(stamp.0 * 0.5, stamp.1 * 0.5),
        }
    }

    fn to_device(self, m: Point) -> Point {
        let x = m.x - self.stamp_centre.x;
        let y = m.y - self.stamp_centre.y;
        Point::new(
            self.page_centre.x + x * self.cosine - y * self.sine,
            self.page_centre.y + x * self.sine + y * self.cosine,
        )
    }

    fn to_mark(self, d: Point) -> Point {
        let x = d.x - self.page_centre.x;
        let y = d.y - self.page_centre.y;
        Point::new(
            self.stamp_centre.x + x * self.cosine + y * self.sine,
            self.stamp_centre.y - x * self.sine + y * self.cosine,
        )
    }
}

/// Every tile that could touch the page.
///
/// Computed up front rather than inside the compositing loop so that the
/// lattice is testable on its own — the per-tile jitter of ADR-0007 is a
/// property of these numbers, not of the pixels.
fn placements(
    frame: &Frame,
    lattice: &Lattice,
    page: (f32, f32),
    stamp: (f32, f32),
    seed: u64,
    jitter: bool,
) -> Result<Vec<Placement>> {
    // How far the lattice has to reach: the page's four corners, in mark space.
    let mut min = Point::new(f32::INFINITY, f32::INFINITY);
    let mut max = Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for corner in [
        Point::new(0.0, 0.0),
        Point::new(page.0, 0.0),
        Point::new(0.0, page.1),
        Point::new(page.0, page.1),
    ] {
        let m = frame.to_mark(corner);
        min = Point::new(min.x.min(m.x), min.y.min(m.y));
        max = Point::new(max.x.max(m.x), max.y.max(m.y));
    }

    // One tile of margin each way, which covers the phase and the jitter.
    let first_column = (((min.x - stamp.0) / lattice.step_x).floor() as i64) - 1;
    let last_column = ((max.x / lattice.step_x).ceil() as i64) + 1;
    let first_row = (((min.y - stamp.1) / lattice.step_y).floor() as i64) - 1;
    let last_row = ((max.y / lattice.step_y).ceil() as i64) + 1;
    let count = (last_column - first_column + 1).max(0) * (last_row - first_row + 1).max(0);
    if count > MAX_TILES as i64 {
        return Err(Error::InvalidParameter {
            name: "mark tile density",
        });
    }

    let mut out = Vec::with_capacity(count.max(0) as usize);
    for row in first_row..=last_row {
        for column in first_column..=last_column {
            out.push(lattice.placement(seed, jitter, column, row));
        }
    }
    Ok(out)
}

/// Lay the stamp out across the page and composite every placement.
fn tile(
    pixmap: &mut Pixmap,
    stamp: &crate::raster::Mask,
    layout: &Layout,
    mark: &Mark,
    seed: u64,
    jitter: bool,
) -> Result<usize> {
    let page = (pixmap.width() as f32, pixmap.height() as f32);
    let stamp_size = (stamp.width() as f32, stamp.height() as f32);
    let frame = Frame::new(mark.angle_degrees, page, stamp_size);
    let lattice = Lattice::new(layout, stamp_size, mark, seed, jitter);
    let tiles = placements(&frame, &lattice, page, stamp_size, seed, jitter)?;

    let width = pixmap.width() as i64;
    let height = pixmap.height() as i64;
    let stride = pixmap.width() as usize * 4;
    let mut placed = 0usize;

    for tile in tiles {
        // Where this tile lands on the page, as a pixel rectangle.
        let mut low = Point::new(f32::INFINITY, f32::INFINITY);
        let mut high = Point::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for (u, v) in [
            (0.0, 0.0),
            (stamp_size.0, 0.0),
            (0.0, stamp_size.1),
            (stamp_size.0, stamp_size.1),
        ] {
            let d = frame.to_device(Point::new(tile.origin.x + u, tile.origin.y + v));
            low = Point::new(low.x.min(d.x), low.y.min(d.y));
            high = Point::new(high.x.max(d.x), high.y.max(d.y));
        }
        let x0 = (low.x.floor() as i64).clamp(0, width);
        let y0 = (low.y.floor() as i64).clamp(0, height);
        let x1 = (high.x.ceil() as i64).clamp(0, width);
        let y1 = (high.y.ceil() as i64).clamp(0, height);
        if x0 >= x1 || y0 >= y1 {
            continue; // Entirely off the page.
        }
        placed += 1;

        let data = pixmap.data_mut();
        for y in y0..y1 {
            let line = y as usize * stride;
            for x in x0..x1 {
                // Pixel centres, so that a half-pixel offset cannot creep in
                // between here and `Mask::sample`.
                let m = frame.to_mark(Point::new(x as f32 + 0.5, y as f32 + 0.5));
                let coverage = stamp.sample(m.x - tile.origin.x, m.y - tile.origin.y);
                if coverage <= 0.0 {
                    continue;
                }
                let alpha = mark.opacity * coverage;
                if alpha <= 0.0 {
                    continue;
                }
                let at = line + x as usize * 4;
                for channel in 0..3 {
                    let destination = f32::from(data[at + channel]);
                    let source = f32::from(mark.colour[channel]);
                    let blended = source * alpha + destination * (1.0 - alpha);
                    // `+ 0.5` then `floor`: never `round`.
                    data[at + channel] = (blended + 0.5).floor() as u8;
                }
                // Straight alpha, and the encoder wants opacity.
                data[at + 3] = 255;
            }
        }
    }
    Ok(placed)
}

/// The jitter seed for a page, when the caller has not chosen one.
///
/// ADR-0002 Decision 6 wants the seed derived from "the input bytes, the
/// parameter set and the mark text". The *input bytes* reach this function as
/// the page's own pre-mark pixels, which are a pure function of them and of the
/// decode — so no plumbing is needed to carry the file down here, and two
/// different documents get different jitter while two runs over the same one
/// agree. It costs one integer pass over the pixmap, which is a small fraction
/// of the JPEG encode that follows.
#[must_use]
pub fn derive_seed(pixmap: &Pixmap, mark: &Mark) -> u64 {
    let mut material: Vec<u8> = Vec::with_capacity(mark.text.len() + 48);
    material.extend_from_slice(&crate::fingerprint::fingerprint(pixmap.data()));
    material.extend_from_slice(mark.text.as_bytes());
    material.extend_from_slice(&mark.font_size_ratio.to_bits().to_be_bytes());
    material.extend_from_slice(&mark.angle_degrees.to_bits().to_be_bytes());
    material.extend_from_slice(&mark.opacity.to_bits().to_be_bytes());
    material.extend_from_slice(&mark.blur_ratio.to_bits().to_be_bytes());
    material.extend_from_slice(&mark.colour);
    material.push(match mark.density {
        TileDensity::Low => 0,
        TileDensity::Normal => 1,
        TileDensity::High => 2,
    });
    let digest = crate::fingerprint::fingerprint(&material);
    u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

#[cfg(test)]
mod tests;
