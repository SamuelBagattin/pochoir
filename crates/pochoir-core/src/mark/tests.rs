//! Tests for the watermark compositor.
//!
//! The page here is A4 portrait at 200 dpi — 1654 × 2339 px — which is what
//! the default path actually produces, so the tile counts and font sizes these
//! tests assert are the real ones rather than a scaled stand-in.

use super::{
    DARK_COLOUR, DEFAULT_ANGLE_DEGREES, DEFAULT_BLUR_RATIO, DEFAULT_FONT_SIZE_RATIO,
    DEFAULT_OPACITY, FAINT_SHRINK_RATIO, Frame, JITTER_FRACTION, LIGHT_COLOUR, Lattice,
    MAX_BLUR_RATIO, MAX_FONT_SIZE_RATIO, MAX_MARK_CHARACTERS, MAX_OPACITY, MIN_FONT_SIZE_RATIO,
    MIN_OPACITY, Mark, MarkColour, TileDensity, compose, derive_seed, layout, nfc, normalise,
    placements, unsupported_characters,
};
use crate::Error;
use crate::font::Font;
use crate::pixmap::Pixmap;

/// A4 portrait at 200 dpi.
const WIDTH: u32 = 1654;
const HEIGHT: u32 = 2339;

/// The mark text ADR-0002 Decision 3 argues for: recipient, purpose, date.
const REALISTIC: &str = "Document destiné exclusivement à une demande de location — 26/11/2026";

/// A blank white page, which is the worst case for a light mark and the
/// easiest to assert about.
fn page() -> Pixmap {
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).expect("A4 at 200 dpi");
    pixmap.data_mut().fill(0xFF);
    pixmap
}

/// An A4 page of one flat colour, for the contrast tests.
fn filled(rgba: [u8; 4]) -> Pixmap {
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).expect("A4 at 200 dpi");
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&rgba);
    }
    pixmap
}

/// A smaller white page, for tests that do not need the real geometry.
fn small_page(width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width, height).expect("small");
    pixmap.data_mut().fill(0xFF);
    pixmap
}

/// How many pixels differ from white.
fn inked(pixmap: &Pixmap) -> usize {
    pixmap
        .data()
        .chunks_exact(4)
        .filter(|p| p[0] != 0xFF || p[1] != 0xFF || p[2] != 0xFF)
        .count()
}

#[test]
fn the_defaults_are_adr_0002_decision_3() {
    let mark = Mark::new(REALISTIC);
    assert_eq!(mark.font_size_ratio, 0.035, "3.5% of the shorter edge");
    assert_eq!(mark.angle_degrees, -30.0);
    assert_eq!(mark.opacity, 0.30);
    assert_eq!(
        mark.colour,
        MarkColour::Automatic,
        "ADR-0010: the colour is measured from the page unless the user fixes it"
    );
    assert_eq!(mark.density, TileDensity::Normal);
    assert_eq!(mark.blur_ratio, DEFAULT_BLUR_RATIO);
    assert_eq!(DEFAULT_FONT_SIZE_RATIO, 0.035);
    assert_eq!(DEFAULT_ANGLE_DEGREES, -30.0);
    assert_eq!(DEFAULT_OPACITY, 0.30);
    assert_eq!(DARK_COLOUR, [0x1A, 0x1A, 0x1A]);
    mark.validate().expect("the defaults must be valid");
    // An empty mark is not a mark: ADR-0002's closed open question decides that
    // nothing is pre-filled, so the default text is empty and invalid.
    assert!(Mark::default().validate().is_err());
}

#[test]
fn the_mark_is_drawn_on_the_page() {
    let mut pixmap = page();
    let before = inked(&pixmap);
    assert_eq!(before, 0, "the test page should start blank");
    let composition = compose(&mut pixmap, &Mark::new(REALISTIC), 1, true).expect("composes");
    let after = inked(&pixmap);
    assert!(after > 100_000, "only {after} pixels were marked");
    assert!(composition.tiles >= 4, "only {} tiles", composition.tiles);
    assert!(!composition.layout.shrunk);
}

#[test]
fn adr_0002s_six_to_ten_repetitions_land_on_a_portrait_page() {
    // Decision 3 aims at "6-10 visible repetitions on a portrait page", and
    // ADR-0007 ruling 1 establishes that for a realistic mark those are rows.
    // This is the measurement behind it: the mark is page-wide, so each row
    // holds one or two repetitions and the row count carries the density.
    let mut pixmap = page();
    let composition = compose(&mut pixmap, &Mark::new(REALISTIC), 7, true).expect("composes");
    assert!(
        (6..=24).contains(&composition.tiles),
        "{} tiles placed, which is far from the range ADR-0002 aims at",
        composition.tiles
    );
    // The mark really is about one page wide, which is the premise of all of
    // this; if a font change made it half a page the tiling would want
    // revisiting rather than the test relaxing.
    let ratio = composition.layout.block_width_px / WIDTH as f32;
    assert!(
        (0.7..1.3).contains(&ratio),
        "the mark is {ratio:.2} pages wide, so ADR-0007 ruling 1 no longer applies"
    );
}

#[test]
fn no_region_of_the_page_is_left_unmarked() {
    // ADR-0007 ruling 1 as corrected, and the reason it had to be corrected: a
    // horizontal gap of 1.6x the *text width* leaves roughly a thousand pixels
    // of every row unmarked, because a rotated row spans the page's diagonal
    // rather than its width. An unmarked region is an unmarked crop of the
    // document.
    let mut pixmap = page();
    compose(&mut pixmap, &Mark::new(REALISTIC), 21, true).expect("composes");
    // A 3x4 grid, whose cells are taller than the 424 px row pitch the default
    // density uses. A finer grid would fail honestly: ADR-0002 asks for a gap
    // of four line heights between rows, so a small enough cell can land
    // inside that gap and carry no ink by design. What must not happen is a
    // *region* going unmarked.
    let (across, down) = (3_u32, 4_u32);
    let mut empty: Vec<(u32, u32)> = Vec::new();
    for row in 0..down {
        for column in 0..across {
            let x0 = column * pixmap.width() / across;
            let x1 = (column + 1) * pixmap.width() / across;
            let y0 = row * pixmap.height() / down;
            let y1 = (row + 1) * pixmap.height() / down;
            let mut ink = 0usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    if pixmap.pixel(x, y).is_some_and(|p| p[0] < 250) {
                        ink += 1;
                    }
                }
            }
            if ink < 2_000 {
                empty.push((column, row));
            }
        }
    }
    assert!(
        empty.is_empty(),
        "these cells of a 3x4 grid carry no mark: {empty:?}"
    );
}

#[test]
fn density_changes_how_many_tiles_are_placed() {
    let mut counts = Vec::new();
    for density in [TileDensity::Low, TileDensity::Normal, TileDensity::High] {
        let mut pixmap = page();
        let mark = Mark {
            density,
            ..Mark::new(REALISTIC)
        };
        let composition = compose(&mut pixmap, &mark, 3, true).expect("composes");
        counts.push((density, composition.tiles, inked(&pixmap)));
    }
    for pair in counts.windows(2) {
        assert!(
            pair[1].1 >= pair[0].1,
            "{:?} placed {} tiles but {:?} placed {}",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
    assert!(
        counts[2].2 > counts[0].2,
        "high density should cover more of the page than low"
    );
}

#[test]
fn the_mark_never_changes_the_page_size() {
    // Trap 4 of the hand-over, and the reason it matters: the JFIF density in
    // the embedded JPEG is derived from the page geometry, so a mark that moved
    // the page would quietly make the output lie about its own resolution.
    for (width, height) in [(1654_u32, 2339_u32), (800, 400), (64, 64)] {
        let mut pixmap = small_page(width, height);
        let before = (pixmap.width(), pixmap.height(), pixmap.data().len());
        let _ = compose(&mut pixmap, &Mark::new("destiné à une agence"), 11, true);
        assert_eq!(
            (pixmap.width(), pixmap.height(), pixmap.data().len()),
            before,
            "{width}x{height} changed size"
        );
    }
}

#[test]
fn every_pixel_stays_opaque() {
    // The pixmap is straight alpha and the JPEG encoder flattens whatever it
    // is given onto *white*, not onto the page. So the compositor must do its
    // own source-over and leave a = 255; a semi-transparent mark left here
    // would be visibly wrong over a dark photograph.
    let mut pixmap = page();
    compose(&mut pixmap, &Mark::new(REALISTIC), 5, true).expect("composes");
    assert!(
        pixmap.data().chunks_exact(4).all(|p| p[3] == 0xFF),
        "the compositor left a transparent pixel behind"
    );
}

#[test]
fn the_mark_darkens_rather_than_replacing() {
    // Source-over at 0.30 of #1A1A1A over white should land near 0.7*255 +
    // 0.3*26 = 186 at full coverage, and never reach the mark colour itself.
    let mut pixmap = page();
    compose(&mut pixmap, &Mark::new(REALISTIC), 2, true).expect("composes");
    let darkest = pixmap
        .data()
        .chunks_exact(4)
        .map(|p| p[0])
        .min()
        .expect("pixels");
    assert!(
        darkest > 180 && darkest < 195,
        "the darkest pixel is {darkest}; expected about 186 for 0.30 of #1A1A1A over white"
    );
}

#[test]
fn a_dark_page_is_lightened_towards_the_mark_colour_not_flattened_to_white() {
    // The other half of the same property: over a dark photograph the mark must
    // blend with what is underneath.
    let mut pixmap = Pixmap::new(700, 900).expect("small");
    pixmap.data_mut().fill(0x10);
    for pixel in pixmap.data_mut().chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    let before = pixmap.clone();
    let composition =
        compose(&mut pixmap, &Mark::new("destiné à une banque"), 4, true).expect("composes");
    assert_ne!(pixmap, before, "nothing was drawn on the dark page");
    // ADR-0010: on a page this dark the engine now reaches for the light mark,
    // which is the point. What this test is about is unchanged — the mark must
    // blend with the 0x10 underneath rather than be flattened onto white.
    assert_eq!(composition.colour, LIGHT_COLOUR);
    let brightest = pixmap
        .data()
        .chunks_exact(4)
        .map(|p| p[0])
        .max()
        .expect("pixels");
    // 0.30 of 0xE5 over 0x10 is 80. Flattening onto white would give 255, and
    // ignoring the page underneath would give 0xE5.
    assert!(
        (0x48..=0x58).contains(&brightest),
        "the brightest pixel is {brightest}; expected about 80 for 0.30 of \
         #E5E5E5 over #101010"
    );
}

#[test]
fn opacity_controls_how_heavy_the_mark_is() {
    let mut previous = 255u8;
    for opacity in [MIN_OPACITY, 0.15, DEFAULT_OPACITY, 0.6, MAX_OPACITY] {
        let mut pixmap = page();
        let mark = Mark {
            opacity,
            ..Mark::new(REALISTIC)
        };
        compose(&mut pixmap, &mark, 1, true).expect("composes");
        let darkest = pixmap
            .data()
            .chunks_exact(4)
            .map(|p| p[0])
            .min()
            .expect("pixels");
        assert!(
            darkest < previous,
            "opacity {opacity} was no darker than the one before: {darkest} vs {previous}"
        );
        previous = darkest;
    }
}

// --- ADR-0007 ruling 3: the blur must actually do something ------------------

#[test]
fn the_default_blur_changes_pixels() {
    // The regression test ADR-0007 requires. ADR-0002's stated default of
    // σ ≈ 0.6 px gives a three-pass box radius of 0.28, which an
    // integer-radius blur rounds to nothing at all — the parameter would look
    // broken at its own default.
    let mark = Mark::new(REALISTIC);
    let unblurred = Mark {
        blur_ratio: 0.0,
        ..mark.clone()
    };
    let mut blurred_page = page();
    let mut sharp_page = page();
    let composition = compose(&mut blurred_page, &mark, 9, false).expect("composes");
    compose(&mut sharp_page, &unblurred, 9, false).expect("composes");

    assert!(
        composition.sigma_px > 0.5,
        "σ is only {} px at the default, which is not a blur",
        composition.sigma_px
    );
    assert_ne!(
        blurred_page, sharp_page,
        "the default blur left the pixels identical to no blur at all"
    );
    let differing = blurred_page
        .data()
        .iter()
        .zip(sharp_page.data())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differing > 10_000,
        "only {differing} bytes differ with the blur on"
    );
}

#[test]
fn sigma_is_keyed_to_the_font_size_not_to_pixels() {
    // Resolution independence, which is the other half of ADR-0007 ruling 3:
    // the same mark on a page twice the size gets twice the blur, so it looks
    // the same at both resolutions rather than progressively sharper.
    let mut small = small_page(827, 1170);
    let mut large = page();
    let mark = Mark::new(REALISTIC);
    let a = compose(&mut small, &mark, 1, false).expect("composes");
    let b = compose(&mut large, &mark, 1, false).expect("composes");
    let ratio = b.sigma_px / a.sigma_px;
    let size_ratio = b.layout.font_size_px / a.layout.font_size_px;
    assert!(
        (ratio - size_ratio).abs() < 1e-3,
        "σ scaled by {ratio} while the font scaled by {size_ratio}"
    );
    assert!((size_ratio - 2.0).abs() < 0.01, "the pages differ by 2x");
}

#[test]
fn blur_stays_inside_adr_0002s_re_expressed_range() {
    // ADR-0002's "0-3 px" at the default font size, in the units ADR-0007
    // moved it to. At 58 px of font, 0.06 is about 3.5 px.
    let font_size = DEFAULT_FONT_SIZE_RATIO * WIDTH as f32;
    assert!(
        (font_size - 57.9).abs() < 0.5,
        "font size is {font_size} px"
    );
    assert!(
        (DEFAULT_BLUR_RATIO * font_size - 0.87).abs() < 0.05,
        "the default σ is {} px",
        DEFAULT_BLUR_RATIO * font_size
    );
    let widest = MAX_BLUR_RATIO * font_size;
    assert!(
        (3.0..4.0).contains(&widest),
        "the maximum σ is {widest} px, which should be ADR-0002's 3 px"
    );
}

// --- ADR-0007 ruling 4: per-tile jitter -------------------------------------

#[test]
fn every_tile_is_displaced_independently() {
    // The property ADR-0007 ruling 4 requires, and the one a single global
    // grid offset would not have: each tile's offset from its own lattice
    // point differs from every other tile's. A global-offset implementation
    // would make all of these identical.
    let font = Font::bundled().expect("parses");
    let mark = Mark::new(REALISTIC);
    let layout = layout::plan(
        &font,
        REALISTIC,
        WIDTH as f32,
        HEIGHT as f32,
        mark.font_size_ratio * WIDTH as f32,
        mark.angle_degrees,
    )
    .expect("fits");
    let lattice = Lattice::new(&layout, (100.0, 40.0), &mark, 42, true);
    let frame = Frame::new(
        mark.angle_degrees,
        (WIDTH as f32, HEIGHT as f32),
        (100.0, 40.0),
    );
    let tiles = placements(
        &frame,
        &lattice,
        (WIDTH as f32, HEIGHT as f32),
        (100.0, 40.0),
        42,
        true,
    )
    .expect("a sane lattice");
    assert!(tiles.len() > 6, "only {} tiles to compare", tiles.len());

    let offsets: Vec<(i64, i64)> = tiles
        .iter()
        .map(|tile| {
            let dx = tile.origin.x - tile.column as f32 * lattice.step_x - lattice.phase_x;
            let dy = tile.origin.y - tile.row as f32 * lattice.step_y - lattice.phase_y;
            // Quantised so that the comparison is about real displacement
            // rather than float noise.
            ((dx * 100.0) as i64, (dy * 100.0) as i64)
        })
        .collect();
    let distinct: std::collections::HashSet<_> = offsets.iter().collect();
    assert_eq!(
        distinct.len(),
        offsets.len(),
        "two tiles share an offset, so the jitter is not per-tile: {offsets:?}"
    );
    assert!(
        offsets.iter().any(|&(dx, _)| dx != 0) && offsets.iter().any(|&(_, dy)| dy != 0),
        "nothing moved at all"
    );
    // And no tile moves further than the documented fraction of a step.
    for tile in &tiles {
        let dx = (tile.origin.x - tile.column as f32 * lattice.step_x - lattice.phase_x).abs();
        let dy = (tile.origin.y - tile.row as f32 * lattice.step_y - lattice.phase_y).abs();
        assert!(dx <= lattice.swing_x + 1e-3, "{dx} across");
        assert!(dy <= lattice.swing_y + 1e-3, "{dy} down");
        assert!(lattice.swing_x <= JITTER_FRACTION * lattice.step_x + 1e-3);
        assert!(lattice.swing_y <= JITTER_FRACTION * lattice.step_y + 1e-3);
    }
}

#[test]
fn turning_jitter_off_puts_every_tile_on_its_lattice_point() {
    let font = Font::bundled().expect("parses");
    let mark = Mark::new(REALISTIC);
    let layout = layout::plan(
        &font,
        REALISTIC,
        WIDTH as f32,
        HEIGHT as f32,
        mark.font_size_ratio * WIDTH as f32,
        mark.angle_degrees,
    )
    .expect("fits");
    let lattice = Lattice::new(&layout, (100.0, 40.0), &mark, 42, false);
    assert_eq!((lattice.phase_x, lattice.phase_y), (0.0, 0.0));
    for (column, row) in [(0_i64, 0_i64), (1, 2), (-3, 4)] {
        let tile = lattice.placement(42, false, column, row);
        assert_eq!(tile.origin.x, column as f32 * lattice.step_x);
        assert_eq!(tile.origin.y, row as f32 * lattice.step_y);
    }
}

#[test]
fn two_seeds_put_the_mark_in_different_places() {
    // ADR-0002 Decision 3's reason for keeping the jitter: an attacker holding
    // two watermarked copies of the same document must not be able to
    // difference them to recover the clean original.
    let mut first = page();
    let mut second = page();
    let mark = Mark::new(REALISTIC);
    compose(&mut first, &mark, 1, true).expect("composes");
    compose(&mut second, &mark, 2, true).expect("composes");
    assert_ne!(first, second, "two seeds produced the same page");
    let differing = first
        .data()
        .iter()
        .zip(second.data())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        differing > 100_000,
        "only {differing} bytes differ between two seeds"
    );
}

#[test]
fn no_jitter_is_the_same_page_whatever_the_seed() {
    let mut first = page();
    let mut second = page();
    let mark = Mark::new(REALISTIC);
    compose(&mut first, &mark, 1, false).expect("composes");
    compose(&mut second, &mark, 999_999, false).expect("composes");
    assert_eq!(first, second, "--no-jitter should ignore the seed entirely");
}

// --- ADR-0007 ruling 7: NFC output must stay inside the subset ---------------

#[test]
fn every_character_the_nfc_table_can_produce_is_in_the_font_subset() {
    // Two lists that have to agree: the composition table's outputs and the
    // bundled font's cmap. A drifting pair would put a .notdef box in the
    // middle of somebody's watermark, which is why this is a test rather than
    // a comment. See ADR-0007 ruling 7.
    let font = Font::bundled().expect("parses");
    let mut missing: Vec<char> = Vec::new();
    for character in nfc::outputs() {
        if font.glyph_for(character).is_none() {
            missing.push(character);
        }
    }
    assert!(
        missing.is_empty(),
        "the NFC table composes characters the font cannot draw: {missing:?}. \
         Regenerate both with scripts/gen-nfc-table.py and scripts/subset-font.py."
    );
    // And every base it accepts, which is the other half of the same promise.
    for character in nfc::bases() {
        assert!(
            font.glyph_for(character).is_some(),
            "the NFC table takes {character:?} as a base but the font lacks it"
        );
    }
}

#[test]
fn an_accented_character_renders_as_one_glyph_however_it_arrives() {
    // Decomposed input must produce exactly the same page as composed input.
    // Without NFC the decomposed form would draw a bare `e` followed by an
    // accent floating on the baseline.
    let composed = "destiné à Noël";
    let decomposed = "destine\u{0301} a\u{0300} Noe\u{0308}l";
    assert_eq!(normalise(decomposed), composed);

    let mut first = small_page(900, 1200);
    let mut second = small_page(900, 1200);
    compose(&mut first, &Mark::new(composed), 8, false).expect("composes");
    compose(&mut second, &Mark::new(decomposed), 8, false).expect("composes");
    assert_eq!(
        first, second,
        "NFC did not make the decomposed form render identically"
    );

    // And it is genuinely different from the unaccented text, so the test is
    // not passing because the accents were dropped.
    let mut plain = small_page(900, 1200);
    compose(&mut plain, &Mark::new("destine a Noel"), 8, false).expect("composes");
    assert_ne!(first, plain, "the accents are not being drawn at all");
}

#[test]
fn unsupported_characters_are_named_rather_than_drawn_as_boxes() {
    // ADR-0007 ruling 7: reject with a message naming them, rather than
    // silently drawing tofu or dropping characters.
    let mark = Mark::new("Document 漢字 destiné ✓ à Ж");
    let error = mark.validate().expect_err("this text cannot be drawn");
    assert_eq!(error, Error::UnsupportedCharacter { character: '漢' });
    let message = error.to_string();
    assert!(message.contains('漢'), "{message}");
    assert!(message.contains("U+6F22"), "{message}");

    // The error carries only the first, because every Error variant holds Copy
    // data; this is how a UI gets the whole list.
    assert_eq!(
        unsupported_characters("Document 漢字 destiné ✓ à Ж").expect("the font parses"),
        vec!['漢', '字', '✓', 'Ж']
    );
    assert!(
        unsupported_characters(REALISTIC)
            .expect("parses")
            .is_empty()
    );
}

#[test]
fn a_leftover_combining_mark_is_reported_too() {
    // A mark NFC could not compose would otherwise draw as a stray accent on
    // the baseline. U+0301 after a digit has no composition.
    let stray = unsupported_characters("date 26\u{0301}/11").expect("parses");
    assert_eq!(stray, vec!['\u{0301}']);
    assert!(Mark::new("date 26\u{0301}/11").validate().is_err());
}

#[test]
fn whitespace_is_layout_and_never_asked_of_the_font() {
    assert!(
        unsupported_characters("a b\tc\nd")
            .expect("parses")
            .is_empty(),
        "whitespace should not be reported as undrawable"
    );
}

// --- ADR-0007 ruling 2: wrap, shrink, refuse --------------------------------

#[test]
fn long_text_is_wrapped_and_shrunk_rather_than_clipped() {
    // ADR-0002 allows up to 200 characters. On one line at the default size
    // that is several pages wide, so this is the case ADR-0007 ruling 2 is
    // about: wrap first, then shrink, and never lose a word.
    let text: String = core::iter::repeat_n("location", 22)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.chars().count() <= MAX_MARK_CHARACTERS,
        "{}",
        text.len()
    );
    let mut pixmap = page();
    let composition = compose(&mut pixmap, &Mark::new(&text), 6, true).expect("must be drawable");
    assert!(
        composition.layout.lines.len() > 1,
        "200 characters should have wrapped: {:?}",
        composition.layout.lines
    );
    assert!(composition.layout.lines.len() <= super::MAX_LINES);
    assert_eq!(
        composition.layout.lines.join(" "),
        text,
        "a word was lost, which means the text was clipped"
    );
    assert!(inked(&pixmap) > 50_000);
}

#[test]
fn a_small_page_shrinks_the_mark_rather_than_refusing_it() {
    // The shrink step of ADR-0007 ruling 2, and the floor it stops at. A mark
    // on a thumbnail is illegible, but illegible is the user's problem and
    // missing is ours, so the engine draws it.
    // Asked for the largest font ADR-0002 allows on a thumbnail of a page, so
    // the shrink step is forced rather than incidental.
    let mut pixmap = small_page(200, 280);
    let mark = Mark {
        font_size_ratio: MAX_FONT_SIZE_RATIO,
        ..Mark::new(REALISTIC)
    };
    let composition = compose(&mut pixmap, &mark, 1, true).expect("a small page still gets one");
    assert!(composition.layout.shrunk, "it should have had to shrink");
    assert!(
        composition.layout.font_size_px >= MIN_FONT_SIZE_RATIO * 200.0,
        "shrank below the floor: {} px",
        composition.layout.font_size_px
    );
    assert_eq!(composition.layout.lines.join(" "), REALISTIC);
    assert!(inked(&pixmap) > 100);
}

#[test]
fn text_that_cannot_fit_honestly_is_refused() {
    let mut pixmap = page();
    let text = "M".repeat(200);
    let error = compose(&mut pixmap, &Mark::new(&text), 1, true)
        .expect_err("an unbreakable 200-character word cannot fit A4");
    assert_eq!(error, Error::MarkDoesNotFit { characters: 200 });
    assert!(error.to_string().contains("shorten it"), "{error}");
    assert_eq!(inked(&pixmap), 0, "a refused mark must draw nothing at all");
}

#[test]
fn text_over_the_character_limit_is_a_parameter_error() {
    let text = "a ".repeat(MAX_MARK_CHARACTERS);
    assert!(text.chars().count() > MAX_MARK_CHARACTERS);
    assert_eq!(
        Mark::new(text).validate(),
        Err(Error::InvalidParameter { name: "mark text" })
    );
}

// --- parameters -------------------------------------------------------------

#[test]
fn out_of_range_parameters_are_refused_by_name() {
    let base = Mark::new(REALISTIC);
    let cases: [(Mark, &str); 8] = [
        (
            Mark {
                font_size_ratio: MIN_FONT_SIZE_RATIO - 0.001,
                ..base.clone()
            },
            "mark font size",
        ),
        (
            Mark {
                font_size_ratio: MAX_FONT_SIZE_RATIO + 0.001,
                ..base.clone()
            },
            "mark font size",
        ),
        (
            Mark {
                font_size_ratio: f32::NAN,
                ..base.clone()
            },
            "mark font size",
        ),
        (
            Mark {
                angle_degrees: 91.0,
                ..base.clone()
            },
            "mark angle",
        ),
        (
            Mark {
                angle_degrees: -91.0,
                ..base.clone()
            },
            "mark angle",
        ),
        (
            Mark {
                opacity: MIN_OPACITY - 0.01,
                ..base.clone()
            },
            "mark opacity",
        ),
        (
            Mark {
                opacity: MAX_OPACITY + 0.01,
                ..base.clone()
            },
            "mark opacity",
        ),
        (
            Mark {
                blur_ratio: MAX_BLUR_RATIO + 0.01,
                ..base.clone()
            },
            "mark blur",
        ),
    ];
    for (mark, name) in cases {
        assert_eq!(
            mark.validate(),
            Err(Error::InvalidParameter { name }),
            "expected {name} to be refused"
        );
    }
    // And the edges of each range are accepted.
    for mark in [
        Mark {
            font_size_ratio: MIN_FONT_SIZE_RATIO,
            ..base.clone()
        },
        Mark {
            font_size_ratio: MAX_FONT_SIZE_RATIO,
            ..base.clone()
        },
        Mark {
            angle_degrees: 0.0,
            ..base.clone()
        },
        Mark {
            angle_degrees: 90.0,
            ..base.clone()
        },
        Mark {
            opacity: MIN_OPACITY,
            ..base.clone()
        },
        Mark {
            opacity: MAX_OPACITY,
            ..base.clone()
        },
        Mark {
            blur_ratio: 0.0,
            ..base.clone()
        },
        Mark {
            blur_ratio: MAX_BLUR_RATIO,
            ..base.clone()
        },
    ] {
        mark.validate().expect("a range edge must be accepted");
    }
}

#[test]
fn every_angle_in_range_draws_something() {
    let mut angle = -90.0_f32;
    while angle <= 90.0 {
        let mut pixmap = small_page(900, 1200);
        let mark = Mark {
            angle_degrees: angle,
            ..Mark::new("destiné à une agence")
        };
        compose(&mut pixmap, &mark, 1, true)
            .unwrap_or_else(|error| panic!("angle {angle}: {error}"));
        assert!(inked(&pixmap) > 1_000, "angle {angle} drew almost nothing");
        angle += 7.5;
    }
}

#[test]
fn a_colour_close_to_the_page_is_flagged_but_not_refused() {
    // ADR-0010 replaces ADR-0002's one-sided pale test with contrast against
    // the page that is actually there. The choice stays the user's, so this is
    // a warning and not an error — both ways round.
    let pale = Mark {
        colour: MarkColour::Fixed([0xF0, 0xF0, 0xF0]),
        ..Mark::new(REALISTIC)
    };
    pale.validate().expect("a pale colour is legal");
    let mut white = page();
    let composition = compose(&mut white, &pale, 1, true).expect("composes");
    assert!(
        composition.has_poor_contrast(),
        "near-white on white: visible over {:.3}",
        composition.visible_fraction
    );
    assert!(!composition.colour_was_chosen);
    assert!(
        composition.best_visible_fraction(pale.opacity) > 0.99,
        "a dark mark would have covered this page"
    );

    // And the failure ADR-0010 exists for: the default dark mark on a page
    // that is darker than it is. One-sided tests see nothing here.
    let dark_page_mark = Mark {
        colour: MarkColour::Fixed(DARK_COLOUR),
        ..Mark::new(REALISTIC)
    };
    let mut dark = filled([0x12, 0x12, 0x12, 0xFF]);
    let composition = compose(&mut dark, &dark_page_mark, 1, true).expect("composes");
    assert_eq!(composition.visible_fraction, 0.0);
    assert!(composition.has_poor_contrast());
}

#[test]
fn the_automatic_colour_follows_the_page() {
    let mark = Mark::new(REALISTIC);

    let mut white = page();
    let on_white = compose(&mut white, &mark, 1, true).expect("composes");
    assert_eq!(on_white.colour, DARK_COLOUR);
    assert!(on_white.colour_was_chosen);
    assert!(!on_white.has_poor_contrast());

    let mut dark = filled([0x12, 0x12, 0x12, 0xFF]);
    let on_dark = compose(&mut dark, &mark, 1, true).expect("composes");
    assert_eq!(on_dark.colour, LIGHT_COLOUR);
    assert!(on_dark.colour_was_chosen);
    assert!(!on_dark.has_poor_contrast());
}

#[test]
fn an_automatic_mark_is_actually_drawn_on_a_dark_page() {
    // The whole point, measured in pixels: with ADR-0002's fixed colour this
    // page comes out byte-identical to the one that went in.
    let before = filled([0x12, 0x12, 0x12, 0xFF]);

    /// The largest change the mark made to any channel of any pixel.
    fn deepest(after: &Pixmap, before: &Pixmap) -> u8 {
        after
            .data()
            .chunks_exact(4)
            .zip(before.data().chunks_exact(4))
            .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
            .max()
            .expect("pixels")
    }

    let mut fixed_dark = before.clone();
    compose(
        &mut fixed_dark,
        &Mark {
            colour: MarkColour::Fixed(DARK_COLOUR),
            ..Mark::new(REALISTIC)
        },
        1,
        true,
    )
    .expect("composes");
    assert!(
        deepest(&fixed_dark, &before) <= 3,
        "ADR-0002's fixed dark mark changes this page by at most {} of 255, \
         which is the bug: the user sees their own document back",
        deepest(&fixed_dark, &before)
    );

    let mut automatic = before.clone();
    compose(&mut automatic, &Mark::new(REALISTIC), 1, true).expect("composes");
    assert!(
        deepest(&automatic, &before) >= 60,
        "the automatic mark only reached {} of 255",
        deepest(&automatic, &before)
    );
}

#[test]
fn a_degenerate_page_is_refused() {
    let mut empty = Pixmap::empty();
    assert_eq!(
        compose(&mut empty, &Mark::new(REALISTIC), 1, true),
        Err(Error::InvalidParameter { name: "page size" })
    );
}

// --- determinism ------------------------------------------------------------

#[test]
fn compositing_is_byte_identical_across_runs() {
    let mark = Mark::new(REALISTIC);
    let mut first = page();
    let mut second = page();
    compose(&mut first, &mark, 0xABCD, true).expect("composes");
    compose(&mut second, &mark, 0xABCD, true).expect("composes");
    assert_eq!(first, second);
}

#[test]
fn the_derived_seed_is_a_function_of_the_document_and_the_parameters() {
    let mark = Mark::new(REALISTIC);
    let white = page();
    let mut grey = page();
    grey.data_mut().fill(0x80);

    assert_eq!(derive_seed(&white, &mark), derive_seed(&white, &mark));
    assert_ne!(
        derive_seed(&white, &mark),
        derive_seed(&grey, &mark),
        "two different documents must get different jitter"
    );
    // Every parameter has to reach the seed, or two parameter sets would share
    // a jitter pattern.
    let variants = [
        Mark {
            text: "something else".into(),
            ..mark.clone()
        },
        Mark {
            font_size_ratio: 0.04,
            ..mark.clone()
        },
        Mark {
            angle_degrees: -25.0,
            ..mark.clone()
        },
        Mark {
            opacity: 0.4,
            ..mark.clone()
        },
        Mark {
            blur_ratio: 0.02,
            ..mark.clone()
        },
        Mark {
            colour: MarkColour::Fixed([0, 0, 0]),
            ..mark.clone()
        },
        // The tag alone must reach the seed: an automatic mark and a fixed one
        // that happens to resolve to the same colour are different requests.
        Mark {
            colour: MarkColour::Fixed(DARK_COLOUR),
            ..mark.clone()
        },
        Mark {
            density: TileDensity::High,
            ..mark.clone()
        },
    ];
    for variant in variants {
        assert_ne!(
            derive_seed(&white, &mark),
            derive_seed(&white, &variant),
            "this parameter does not reach the seed: {variant:?}"
        );
    }
}

#[test]
fn the_rotation_round_trips() {
    let frame = Frame::new(-30.0, (1654.0, 2339.0), (900.0, 70.0));
    for point in [
        crate::geom::Point::new(0.0, 0.0),
        crate::geom::Point::new(1654.0, 2339.0),
        crate::geom::Point::new(827.0, 1169.5),
        crate::geom::Point::new(-50.0, 3000.0),
    ] {
        let back = frame.to_mark(frame.to_device(point));
        assert!(
            (back.x - point.x).abs() < 0.01 && (back.y - point.y).abs() < 0.01,
            "{point:?} round-tripped to {back:?}"
        );
    }
}

#[test]
fn a_negative_angle_runs_the_mark_from_the_bottom_left_upwards() {
    // The sign convention, asserted once so a later "fix" cannot quietly flip
    // the mark to run downhill. In a y-down raster, -30° sends a point to the
    // right of the stamp centre to a *smaller* y.
    let frame = Frame::new(-30.0, (1000.0, 1000.0), (100.0, 100.0));
    let right_of_centre = frame.to_device(crate::geom::Point::new(100.0, 50.0));
    let centre = frame.to_device(crate::geom::Point::new(50.0, 50.0));
    assert!(right_of_centre.x > centre.x);
    assert!(
        right_of_centre.y < centre.y,
        "the mark runs downhill: {right_of_centre:?} vs {centre:?}"
    );
}

#[test]
fn the_mark_reaches_every_corner_of_the_page() {
    // A tiled mark that left a clean corner would hand an attacker an
    // unmarked crop of the document.
    let mut pixmap = page();
    let mark = Mark {
        density: TileDensity::High,
        ..Mark::new(REALISTIC)
    };
    compose(&mut pixmap, &mark, 13, true).expect("composes");
    let width = i64::from(pixmap.width());
    let height = i64::from(pixmap.height());
    // Each quadrant must carry ink.
    for (x0, y0, x1, y1) in [
        (0, 0, width / 2, height / 2),
        (width / 2, 0, width, height / 2),
        (0, height / 2, width / 2, height),
        (width / 2, height / 2, width, height),
    ] {
        let mut found = false;
        for y in y0..y1 {
            for x in x0..x1 {
                if pixmap
                    .pixel(x as u32, y as u32)
                    .is_some_and(|p| p[0] != 0xFF)
                {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "the quadrant at ({x0},{y0}) has no mark on it");
    }
}

// --- ADR-0007 Decision 2, as corrected: the shrink is reported ---------------

#[test]
fn a_mark_drawn_at_the_requested_size_reports_no_shrink() {
    let mut pixmap = page();
    let composition = compose(&mut pixmap, &Mark::new(REALISTIC), 1, true).expect("composes");
    assert!(!composition.layout.shrunk);
    assert_eq!(composition.shrink_ratio(), 1.0);
    assert!(!composition.was_shrunk_noticeably());
    assert_eq!(
        composition.requested_font_size_px,
        DEFAULT_FONT_SIZE_RATIO * WIDTH as f32
    );
}

#[test]
fn a_mark_that_had_to_shrink_a_long_way_says_so() {
    // The gap ADR-0007's first draft left: this input is drawn, not refused,
    // and before the warning existed nothing told the user it had come out at
    // a fraction of the size they chose.
    let text = "Documentdestineexclusivementaunedemandedelocation".repeat(5)[..200].to_string();
    assert_eq!(text.chars().count(), MAX_MARK_CHARACTERS);
    let mut pixmap = page();
    let composition = compose(&mut pixmap, &Mark::new(&text), 1, true)
        .expect("an unbreakable 200-character string is usually drawn, not refused");
    assert!(composition.layout.shrunk);
    let ratio = composition.shrink_ratio();
    assert!(
        ratio < FAINT_SHRINK_RATIO,
        "shrink ratio {ratio} should have tripped the warning"
    );
    assert!(composition.was_shrunk_noticeably());
    assert!(ratio > 0.0 && ratio < 1.0);
}

#[test]
fn the_shrink_ratio_is_the_two_sizes_divided() {
    let text = "Documentdestineexclusivementaunedemandedelocation".repeat(5)[..200].to_string();
    let mut pixmap = page();
    let composition = compose(&mut pixmap, &Mark::new(&text), 1, true).expect("composes");
    let expected = composition.layout.font_size_px / composition.requested_font_size_px;
    assert!((composition.shrink_ratio() - expected).abs() < 1e-6);
    assert!(composition.layout.font_size_px < composition.requested_font_size_px);
}

#[test]
fn refusal_needs_text_that_is_unbreakable_and_wide_for_the_page() {
    // ADR-0007 Decision 2 names the conditions; this is them, measured. The
    // floor is keyed to the page's **shorter** edge while the width budget
    // follows the rotated extent, so widening the page lowers the floor faster
    // than it lowers the budget — and the same text that is refused on one
    // page shape is drawn on another.
    let wide = "M".repeat(200);
    let narrow = "a".repeat(200);

    // A4 portrait: the widest characters do not fit even at the floor.
    let mut portrait = page();
    assert_eq!(
        compose(&mut portrait, &Mark::new(&wide), 1, true),
        Err(Error::MarkDoesNotFit { characters: 200 })
    );
    // The same count of narrow characters does fit, by shrinking.
    let mut portrait = page();
    assert!(compose(&mut portrait, &Mark::new(&narrow), 1, true).is_ok());

    // And on a wider page even the widest characters fit, because the floor
    // fell with the shorter edge.
    let mut landscape = small_page(2339, 1316);
    assert!(
        compose(&mut landscape, &Mark::new(&wide), 1, true).is_ok(),
        "a wider page should lower the floor enough to fit"
    );
}

#[test]
fn a_refused_mark_is_the_rare_case_and_an_ordinary_long_one_is_not() {
    // Guards the claim ADR-0007 Decision 2 now makes, so that a future change
    // to the floor or the budget cannot quietly turn ordinary long text into
    // an error.
    let mut drawn = 0;
    let mut refused = 0;
    for text in [
        "location ".repeat(22).trim().to_string(),
        "Document destiné exclusivement à une demande de location — 26/11/2026".to_string(),
        "a".repeat(200),
        "Documentdestineexclusivementaunedemandedelocation".repeat(4),
        "M".repeat(200),
    ] {
        let mut pixmap = page();
        match compose(&mut pixmap, &Mark::new(&text), 1, true) {
            Ok(_) => drawn += 1,
            Err(Error::MarkDoesNotFit { .. }) => refused += 1,
            Err(other) => panic!("{text:?}: unexpected {other}"),
        }
    }
    assert_eq!((drawn, refused), (4, 1), "the balance of outcomes moved");
}
