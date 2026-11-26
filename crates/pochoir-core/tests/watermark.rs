//! End-to-end tests for the watermark: ADR-0002's mark, burned into pixels.
//!
//! ADR-0002 Decision 6's test strategy, point by point:
//!
//! - **Inputs generated procedurally**, never committed as binary blobs — see
//!   `fixtures/mod.rs`.
//! - **Golden hash on the composited raster, before encoding**, through
//!   [`render_to_pdf_observed`]. Determinism makes an exact comparison
//!   legitimate, so there is no perceptual tolerance anywhere here.
//! - **Structural assertions on the PDF**, not a byte comparison of it.
//!
//! Pixel counts are deliberately small. Page geometry is a function of
//! pixels ÷ dpi, so a sheet can be reproduced exactly at a sixteenth of the
//! encoding cost by scaling both — 413 px at 50 dpi stands in for 1654 px at
//! 200 dpi and lands on the same A4 page. **Do not "fix" these to realistic
//! pixel counts**: the suite was 58 s before the same trick was applied to the
//! A4 goldens and 10 s after. The full-size numbers are asserted in
//! `mark`'s own unit tests, where they cost nothing.

mod fixtures;

use fixtures::{close, decode_with_zune, exif_block, gradient, jpeg, quadrant_colours, with_exif};

use pochoir_core::exif::{EXIF_HEADER, Orientation};
use pochoir_core::fingerprint::{fingerprint, to_hex};
use pochoir_core::mark::{Mark, TileDensity};
use pochoir_core::page::{PageGeometry, PageSource, RasterError};
use pochoir_core::{OutputOptions, Pixmap, image_to_pdf, render_to_pdf, render_to_pdf_observed};

/// A4 portrait at 50 dpi, standing in for 1654 × 2339 at 200 dpi.
const PAGE_WIDTH: u32 = 413;
const PAGE_HEIGHT: u32 = 584;

/// The mark ADR-0002 Decision 3 argues for: recipient, purpose, date.
const REALISTIC: &str = "Document destiné exclusivement à une demande de location — 26/11/2026";

/// A mark with the defaults and a fixed appearance, for the goldens.
fn mark() -> Mark {
    Mark::new(REALISTIC)
}

/// Options that draw `mark()` with a fixed seed, so a golden is reproducible
/// across documents rather than only across runs.
fn marked(seed: u64) -> OutputOptions {
    OutputOptions {
        dpi: 50.0,
        mark: Some(mark()),
        seed: Some(seed),
        ..OutputOptions::default()
    }
}

/// The bytes of the first `/DCTDecode` stream in a PDF.
fn first_image_stream(pdf: &[u8]) -> &[u8] {
    let needle = b"/Filter /DCTDecode";
    let at = pdf
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("there is a DCTDecode image");
    let stream_at = pdf[at..]
        .windows(7)
        .position(|w| w == b"stream\n")
        .expect("the stream follows")
        + at
        + 7;
    let end = pdf[stream_at..]
        .windows(10)
        .position(|w| w == b"\nendstream")
        .expect("the stream ends")
        + stream_at;
    &pdf[stream_at..end]
}

/// The `/MediaBox` of the first page, in points.
fn media_box(pdf: &[u8]) -> (f64, f64) {
    let text = String::from_utf8_lossy(pdf);
    let at = text.find("/MediaBox [").expect("a MediaBox");
    let rest = &text[at + "/MediaBox [".len()..];
    let end = rest.find(']').expect("a closing bracket");
    let numbers: Vec<f64> = rest[..end]
        .split_whitespace()
        .map(|n| n.parse().expect("a number"))
        .collect();
    assert_eq!(numbers.len(), 4, "a MediaBox has four numbers");
    (numbers[2], numbers[3])
}

/// How many pixels of `pixmap` are not the background colour.
fn marked_pixels(pixmap: &Pixmap, background: [u8; 3]) -> usize {
    pixmap
        .data()
        .chunks_exact(4)
        .filter(|p| !close([p[0], p[1], p[2]], background, 3))
        .count()
}

/// A multi-page source that is not an image, so the compositor is exercised
/// through something other than the one `PageSource` that exists today.
///
/// This is the path-independence assertion of ADR-0002 Decision 1 made
/// executable: the mark is applied to these pages by the same call site, with
/// no branch on what produced them. When a PDF renderer arrives it will arrive
/// behind this same trait.
struct Sheets {
    count: usize,
    shade: u8,
}

impl PageSource for Sheets {
    fn page_count(&self) -> usize {
        self.count
    }

    fn page_geometry(&self, _index: usize) -> PageGeometry {
        PageGeometry::from_pixels(PAGE_WIDTH, PAGE_HEIGHT, 50.0, 50.0)
    }

    fn rasterise(&self, index: usize, _dpi: f32, out: &mut Pixmap) -> Result<(), RasterError> {
        out.resize(PAGE_WIDTH, PAGE_HEIGHT)?;
        // A different flat shade per page, so a test can tell them apart.
        let shade = self.shade.saturating_add((index as u8).saturating_mul(17));
        for pixel in out.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[shade, shade, shade, 0xFF]);
        }
        Ok(())
    }
}

// --- the goldens ------------------------------------------------------------

/// Fingerprints of the composited raster, before encoding.
///
/// ADR-0002 Decision 6: goldens are **regenerated behind an explicit flag** and
/// the resulting diff is reviewed as part of the change, never refreshed
/// automatically. To regenerate:
///
/// ```text
/// POCHOIR_PRINT_GOLDENS=1 cargo test -p pochoir-core --test watermark -- --nocapture
/// ```
///
/// A diff here means the rendered mark moved. That is sometimes correct — a
/// deliberate change to the rasteriser, the font asset or the lattice — and is
/// otherwise a bug. It is never something to paste over without knowing which.
const GOLDENS: [(&str, &str); 6] = [
    ("flat-grey-default", "d92864d6a813cb44f25445d864c75694"),
    ("flat-grey-no-jitter", "49524701e6d61265deb7741e91471f95"),
    ("gradient-default", "2da649d741716f0836d60f8fadba6198"),
    ("gradient-high-density", "6b00f9e1c21eaab3a4884c8d12db8823"),
    ("gradient-upright", "216c7773572742396ced98be11db3149"),
    ("gradient-black-heavy", "3615d42441e619a9cb96c70ce54c2759"),
];

/// The raster fingerprint of one page rendered with these options.
fn raster_fingerprint(source: &dyn PageSource, options: &OutputOptions) -> String {
    let mut digest = None;
    render_to_pdf_observed(source, options, |index, pixmap| {
        assert_eq!(index, 0, "these goldens are single-page");
        digest = Some(to_hex(&fingerprint(pixmap.data())));
    })
    .expect("renders");
    digest.expect("the hook must have been called")
}

/// Every golden case, as `(name, source, options)`.
fn golden_cases() -> Vec<(&'static str, Box<dyn PageSource>, OutputOptions)> {
    let flat = || {
        Box::new(Sheets {
            count: 1,
            shade: 0xC8,
        }) as Box<dyn PageSource>
    };
    let ramp = || Box::new(Ramp) as Box<dyn PageSource>;
    vec![
        ("flat-grey-default", flat(), marked(0x5EED)),
        (
            "flat-grey-no-jitter",
            flat(),
            OutputOptions {
                jitter: false,
                ..marked(0x5EED)
            },
        ),
        ("gradient-default", ramp(), marked(0x1234_5678)),
        (
            "gradient-high-density",
            ramp(),
            OutputOptions {
                mark: Some(Mark {
                    density: TileDensity::High,
                    ..mark()
                }),
                ..marked(0x1234_5678)
            },
        ),
        (
            "gradient-upright",
            ramp(),
            OutputOptions {
                mark: Some(Mark {
                    angle_degrees: 0.0,
                    ..mark()
                }),
                ..marked(0x1234_5678)
            },
        ),
        (
            "gradient-black-heavy",
            ramp(),
            OutputOptions {
                mark: Some(Mark {
                    colour: [0, 0, 0],
                    opacity: 0.6,
                    blur_ratio: 0.0,
                    ..mark()
                }),
                ..marked(0x1234_5678)
            },
        ),
    ]
}

/// The gradient fixture, as a `PageSource`.
struct Ramp;

impl PageSource for Ramp {
    fn page_count(&self) -> usize {
        1
    }

    fn page_geometry(&self, _index: usize) -> PageGeometry {
        PageGeometry::from_pixels(PAGE_WIDTH, PAGE_HEIGHT, 50.0, 50.0)
    }

    fn rasterise(&self, _index: usize, _dpi: f32, out: &mut Pixmap) -> Result<(), RasterError> {
        let source = gradient(PAGE_WIDTH, PAGE_HEIGHT);
        out.resize(PAGE_WIDTH, PAGE_HEIGHT)?;
        out.data_mut().copy_from_slice(source.data());
        Ok(())
    }
}

#[test]
fn the_composited_raster_matches_its_golden_hash() {
    let printing = std::env::var_os("POCHOIR_PRINT_GOLDENS").is_some();
    let mut actual: Vec<(&str, String)> = Vec::new();
    for (name, source, options) in golden_cases() {
        actual.push((name, raster_fingerprint(source.as_ref(), &options)));
    }
    if printing {
        println!("const GOLDENS: [(&str, &str); {}] = [", actual.len());
        for (name, digest) in &actual {
            println!("    (\"{name}\", \"{digest}\"),");
        }
        println!("];");
    }
    let expected: std::collections::BTreeMap<&str, &str> = GOLDENS.iter().copied().collect();
    let mut wrong: Vec<String> = Vec::new();
    for (name, digest) in &actual {
        match expected.get(name) {
            Some(&want) if want == digest => {}
            Some(&want) => wrong.push(format!("  {name}: expected {want}, got {digest}")),
            None => wrong.push(format!("  {name}: no golden recorded, got {digest}")),
        }
    }
    assert!(
        wrong.is_empty(),
        "the rendered mark has moved:\n{}\n\nIf that is deliberate, regenerate with \
         POCHOIR_PRINT_GOLDENS=1 and review the diff as part of the change.",
        wrong.join("\n")
    );
}

#[test]
fn the_golden_hashes_are_all_different() {
    // Six cases that must not collide: if two did, one of the parameters they
    // differ in is not reaching the raster and a golden would be asserting
    // nothing.
    let digests: std::collections::BTreeSet<&str> = GOLDENS.iter().map(|&(_, d)| d).collect();
    assert_eq!(digests.len(), GOLDENS.len(), "two goldens share a hash");
}

// --- determinism ------------------------------------------------------------

#[test]
fn two_runs_produce_byte_identical_pdfs() {
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    for options in [
        marked(7),
        OutputOptions {
            jitter: false,
            ..marked(7)
        },
        // And with the seed derived rather than given, which is the default
        // path and the one a user actually gets.
        OutputOptions {
            seed: None,
            ..marked(7)
        },
    ] {
        let first = image_to_pdf(&jpeg_bytes, &options).expect("renders");
        let second = image_to_pdf(&jpeg_bytes, &options).expect("renders");
        assert_eq!(first, second, "two runs differed");
        assert!(first.starts_with(b"%PDF-"));
    }
}

#[test]
fn a_derived_seed_is_stable_for_a_document_and_differs_between_documents() {
    let options = OutputOptions {
        seed: None,
        ..marked(0)
    };
    let one = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let other = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT - 1), 85);
    assert_eq!(
        image_to_pdf(&one, &options).expect("renders"),
        image_to_pdf(&one, &options).expect("renders")
    );
    assert_ne!(
        image_to_pdf(&one, &options).expect("renders"),
        image_to_pdf(&other, &options).expect("renders"),
        "two documents got the same output"
    );
}

#[test]
fn two_seeds_place_the_mark_differently() {
    // ADR-0002 Decision 3's reason for keeping the jitter: two marked copies of
    // one document must not be differenceable back to the clean original.
    let source = Sheets {
        count: 1,
        shade: 0xC8,
    };
    assert_ne!(
        raster_fingerprint(&source, &marked(1)),
        raster_fingerprint(&source, &marked(2))
    );
}

// --- the mark reaches every page --------------------------------------------

#[test]
fn every_page_of_a_multi_page_document_is_marked() {
    let source = Sheets {
        count: 4,
        shade: 0x90,
    };
    let options = marked(3);
    let mut seen: Vec<(usize, usize, String)> = Vec::new();
    let pdf = render_to_pdf_observed(&source, &options, |index, pixmap| {
        let shade = 0x90u8.saturating_add((index as u8).saturating_mul(17));
        seen.push((
            index,
            marked_pixels(pixmap, [shade, shade, shade]),
            to_hex(&fingerprint(pixmap.data())),
        ));
    })
    .expect("renders");

    assert_eq!(seen.len(), 4, "the hook should see every page");
    for (index, marked_count, _) in &seen {
        assert!(
            *marked_count > 2_000,
            "page {index} has only {marked_count} marked pixels"
        );
    }
    // Each page's mark differs, because the derived-seed material includes the
    // page's own pixels — here, its shade.
    let digests: std::collections::BTreeSet<&String> = seen.iter().map(|(_, _, d)| d).collect();
    assert_eq!(digests.len(), 4, "two pages rendered identically");
    assert!(String::from_utf8_lossy(&pdf).contains("/Count 4"));
}

#[test]
fn an_unmarked_render_leaves_the_pixels_exactly_as_the_source_made_them() {
    // The control for every assertion above: with no mark, the hook sees the
    // source's own pixels untouched.
    let source = Sheets {
        count: 1,
        shade: 0x90,
    };
    let options = OutputOptions {
        dpi: 50.0,
        ..OutputOptions::default()
    };
    render_to_pdf_observed(&source, &options, |_, pixmap| {
        assert_eq!(marked_pixels(pixmap, [0x90, 0x90, 0x90]), 0);
    })
    .expect("renders");
}

// --- geometry and metadata are untouched ------------------------------------

#[test]
fn the_mark_does_not_move_the_page() {
    // The JFIF density in the embedded JPEG is derived from the page geometry,
    // so a mark that changed the page would make the output misreport its own
    // resolution.
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let bare = OutputOptions {
        dpi: 50.0,
        ..OutputOptions::default()
    };
    let unmarked = image_to_pdf(&jpeg_bytes, &bare).expect("renders");
    let watermarked = image_to_pdf(&jpeg_bytes, &marked(1)).expect("renders");
    assert_eq!(
        media_box(&unmarked),
        media_box(&watermarked),
        "the mark changed the MediaBox"
    );
    // An A4 page, within a point.
    let (width, height) = media_box(&watermarked);
    assert!(
        (width - 595.3).abs() < 1.5 && (height - 841.9).abs() < 1.5,
        "{width}x{height} is not A4"
    );
}

#[test]
fn no_exif_or_gps_survives_onto_the_marked_path() {
    // ADR-0002 Decision 5 requires this assertion on *every* mode, and the
    // watermarked path is a mode. It holds structurally — compositing forces a
    // re-encode — but the easiest privacy bug in the project to ship by
    // accident is a passthrough optimisation, so the test belongs here rather
    // than only on the bare write path.
    let tiff = exif_block(Orientation::TopLeft, 300, true);
    let input = with_exif(&jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85), &tiff);
    assert!(
        input.windows(EXIF_HEADER.len()).any(|w| w == EXIF_HEADER),
        "the fixture must really carry EXIF, or this test proves nothing"
    );
    assert!(
        input.windows(4).any(|w| w == fixtures::CAMERA_MAKE),
        "the fixture must really carry a camera make"
    );

    let pdf = image_to_pdf(&input, &marked(1)).expect("renders");
    assert!(
        !pdf.windows(EXIF_HEADER.len()).any(|w| w == EXIF_HEADER),
        "an Exif header reached the watermarked output"
    );
    assert!(
        !pdf.windows(4).any(|w| w == fixtures::CAMERA_MAKE),
        "the camera make reached the watermarked output"
    );
    let text = String::from_utf8_lossy(&pdf);
    for forbidden in ["/CreationDate", "/ModDate", "/Author", "<?xpacket"] {
        assert!(!text.contains(forbidden), "{forbidden} reached the output");
    }
}

/// Where each quadrant's colour belongs after each orientation is applied,
/// clockwise from the top left, plus the upright dimensions.
///
/// Derived from TIFF 6.0 §8 rather than from our own code, exactly as the
/// write-path suite's own table is, so this is an independent statement of the
/// expected result and not a restatement of what `exif.rs` does.
type OrientationCase = (Orientation, [[u8; 3]; 4], (u32, u32));

const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 180, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const YELLOW: [u8; 3] = [230, 210, 40];

/// A one-page source from encoded image bytes, at its native page size.
fn image_page(bytes: &[u8]) -> pochoir_core::ImagePage {
    pochoir_core::ImagePage::new(
        pochoir_core::decode_image(bytes).expect("decodes"),
        50.0,
        pochoir_core::PageSizing::Native,
    )
    .expect("a page")
}

#[test]
fn the_eight_exif_orientations_still_come_out_upright_with_a_mark_on() {
    // Orientation is applied at decode, in `image.rs::upright`, before the
    // compositor ever sees the pixels. So this asserts that the mark did not
    // disturb a transform that has already happened — which is the regression
    // worth guarding, because the mark is now the only thing writing into
    // those pixels afterwards, and a coordinate bug in the tiling would show
    // up here as a page that is marked but no longer upright.
    let source = fixtures::quadrants(64, 32);
    let expected: [OrientationCase; 8] = [
        (Orientation::TopLeft, [RED, GREEN, YELLOW, BLUE], (64, 32)),
        (Orientation::TopRight, [GREEN, RED, BLUE, YELLOW], (64, 32)),
        (
            Orientation::BottomRight,
            [YELLOW, BLUE, RED, GREEN],
            (64, 32),
        ),
        (
            Orientation::BottomLeft,
            [BLUE, YELLOW, GREEN, RED],
            (64, 32),
        ),
        (Orientation::LeftTop, [RED, BLUE, YELLOW, GREEN], (32, 64)),
        (Orientation::RightTop, [BLUE, RED, GREEN, YELLOW], (32, 64)),
        (
            Orientation::RightBottom,
            [YELLOW, GREEN, RED, BLUE],
            (32, 64),
        ),
        (
            Orientation::LeftBottom,
            [GREEN, YELLOW, BLUE, RED],
            (32, 64),
        ),
    ];

    for (orientation, corners, (width, height)) in expected {
        let tagged = with_exif(&jpeg(&source, 95), &exif_block(orientation, 72, false));
        let options = OutputOptions {
            // A short mark, so that at 64 px the text still fits without being
            // shrunk to the floor.
            mark: Some(Mark::new("destiné à une agence")),
            seed: Some(11),
            dpi: 50.0,
            page_sizing: pochoir_core::PageSizing::Native,
            ..OutputOptions::default()
        };

        let mut observed: Option<(u32, u32, [[u8; 3]; 4], usize)> = None;
        render_to_pdf_observed(&image_page(&tagged), &options, |_, pixmap| {
            observed = Some((
                pixmap.width(),
                pixmap.height(),
                quadrant_colours(pixmap),
                pixmap
                    .data()
                    .chunks_exact(4)
                    .filter(|p| p[3] == 0xFF)
                    .count(),
            ));
        })
        .expect("renders");
        let (got_width, got_height, quadrants, opaque) = observed.expect("the hook ran");

        assert_eq!(
            (got_width, got_height),
            (width, height),
            "{orientation:?}: the mark changed the upright dimensions"
        );
        assert_eq!(
            opaque,
            (width * height) as usize,
            "{orientation:?}: the compositor left a transparent pixel"
        );
        for (index, (actual, want)) in quadrants.iter().zip(&corners).enumerate() {
            // A wide tolerance, because the mark darkens whatever it crosses by
            // up to 0.3 of the distance to #1A1A1A. The assertion is that red
            // is still in the corner where red belongs, not that it is
            // untouched.
            assert!(
                close(*actual, *want, 80),
                "{orientation:?} quadrant {index}: {actual:?} is not {want:?} darkened"
            );
        }

        // And something was actually drawn, so the assertion above is not
        // passing on an unmarked page.
        let marked = raster_fingerprint(&image_page(&tagged), &options);
        let bare = raster_fingerprint(
            &image_page(&tagged),
            &OutputOptions {
                mark: None,
                ..options.clone()
            },
        );
        assert_ne!(marked, bare, "{orientation:?}: no mark was drawn");
    }
}

// --- the mark survives the encoder ------------------------------------------

#[test]
fn the_mark_is_still_visible_after_jpeg_encoding() {
    // The mark is burned in before encoding, so an independent decoder reading
    // the embedded datastream must see it. This is the assertion that the whole
    // pipeline, not just the compositor, produces a watermarked document.
    let white = {
        let mut pixmap = Pixmap::new(PAGE_WIDTH, PAGE_HEIGHT).expect("small");
        pixmap.data_mut().fill(0xFF);
        pixmap
    };
    let input = jpeg(&white, 95);
    let options = OutputOptions {
        quality: 95,
        ..marked(1)
    };
    let pdf = image_to_pdf(&input, &options).expect("renders");

    // Pull the embedded JPEG back out and decode it with zune, which did not
    // write it. Byte offsets, not string ones: the stream is binary and a
    // lossy UTF-8 view of it does not index the same positions.
    let (_, _, pixels) = decode_with_zune(first_image_stream(&pdf));
    let dark = pixels.chunks_exact(3).filter(|p| p[0] < 230).count();
    assert!(
        dark > 2_000,
        "only {dark} pixels came back dark; the mark did not survive encoding"
    );
}

// --- text handling end to end -----------------------------------------------

#[test]
fn decomposed_and_composed_text_render_identically() {
    // NFC, end to end. Without it the decomposed form would draw a bare `e`
    // followed by an accent floating on the baseline.
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let composed = "destiné à Noël";
    let decomposed = "destine\u{0301} a\u{0300} Noe\u{0308}l";
    let render = |text: &str| {
        image_to_pdf(
            &jpeg_bytes,
            &OutputOptions {
                mark: Some(Mark::new(text)),
                ..marked(1)
            },
        )
        .expect("renders")
    };
    assert_eq!(render(composed), render(decomposed));
    assert_ne!(
        render(composed),
        render("destine a Noel"),
        "the accents are not being drawn"
    );
}

#[test]
fn undrawable_text_is_refused_before_anything_is_written() {
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let options = OutputOptions {
        mark: Some(Mark::new("Document 漢字")),
        ..marked(1)
    };
    let error = image_to_pdf(&jpeg_bytes, &options).expect_err("this cannot be drawn");
    assert_eq!(
        error,
        pochoir_core::Error::UnsupportedCharacter { character: '漢' }
    );
}

#[test]
fn text_that_cannot_fit_is_refused_rather_than_clipped() {
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let options = OutputOptions {
        mark: Some(Mark::new("M".repeat(200))),
        ..marked(1)
    };
    let error = image_to_pdf(&jpeg_bytes, &options).expect_err("200 unbreakable Ms cannot fit");
    assert_eq!(
        error,
        pochoir_core::Error::MarkDoesNotFit { characters: 200 }
    );
}

#[test]
fn the_pdf_structure_is_unchanged_by_the_mark() {
    // ADR-0002's structural assertions, re-checked on the marked path.
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let pdf = image_to_pdf(&jpeg_bytes, &marked(1)).expect("renders");
    let text = String::from_utf8_lossy(&pdf);
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(pdf.ends_with(b"%%EOF\n"));
    assert!(text.contains("/Producer (Pochoir)"));
    assert!(text.contains("/Creator (Pochoir)"));
    assert!(text.contains("/Filter /DCTDecode"));
    assert!(text.contains("/Count 1"));
    assert!(text.contains("/ID ["));
    // No version number in /Producer, per Decision 5.
    assert!(
        !text.contains(&format!("Pochoir {}", pochoir_core::version())),
        "a version string reached the output"
    );
}

#[test]
fn the_bare_write_path_still_works_and_differs_from_the_marked_one() {
    let jpeg_bytes = jpeg(&gradient(PAGE_WIDTH, PAGE_HEIGHT), 85);
    let bare = OutputOptions {
        dpi: 50.0,
        ..OutputOptions::default()
    };
    let unmarked = render_to_pdf(
        &pochoir_core::ImagePage::new(
            pochoir_core::decode_image(&jpeg_bytes).expect("decodes"),
            50.0,
            pochoir_core::PageSizing::CapToA4,
        )
        .expect("a page"),
        &bare,
    )
    .expect("renders");
    let watermarked = image_to_pdf(&jpeg_bytes, &marked(1)).expect("renders");
    assert_ne!(unmarked, watermarked);
    assert!(
        watermarked.len() > unmarked.len() / 2,
        "the marked output is implausibly small"
    );
}
