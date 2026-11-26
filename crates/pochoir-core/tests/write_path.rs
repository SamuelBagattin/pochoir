//! Golden tests for the write path: an image goes in, a PDF comes out.
//!
//! These are the assertions ADR-0002's test strategy asks for. Structural
//! assertions on the PDF rather than a byte comparison of it, because "a byte
//! comparison of the final PDF would break every time the JPEG encoder changed,
//! which tells us nothing about the watermark" — with the one exception of the
//! determinism test, where byte equality between two runs of the *same* build is
//! exactly the property under test.

mod fixtures;

use fixtures::{
    close, decode_with_zune, exif_block, gps_bytes, gradient, jpeg, png, quadrant_colours,
    quadrants, with_exif,
};
use pochoir_core::exif::{EXIF_HEADER, Orientation};
use pochoir_core::{OutputOptions, PageSizing, image_to_pdf};

const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 180, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const YELLOW: [u8; 3] = [230, 210, 40];

/// One row of the orientation table: the tag, the colour expected in each
/// corner clockwise from the top left, and the upright dimensions.
type OrientationCase = (Orientation, [[u8; 3]; 4], (u32, u32));

/// `/MediaBox [0 0 w h]` of the first page, in points.
fn media_box(pdf: &[u8]) -> (f64, f64) {
    let text = String::from_utf8_lossy(pdf);
    let at = text
        .find("/MediaBox [0 0 ")
        .expect("every page has a MediaBox")
        + "/MediaBox [0 0 ".len();
    let rest = &text[at..];
    let end = rest.find(']').expect("the array closes");
    let mut parts = rest[..end].split_whitespace();
    let width = parts.next().expect("a width").parse().expect("a number");
    let height = parts.next().expect("a height").parse().expect("a number");
    (width, height)
}

/// The bytes of the first `/DCTDecode` stream in the file.
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

#[test]
fn a_jpeg_we_wrote_round_trips_through_an_independent_decoder() {
    // The encoder is ours; the decoder is `zune-jpeg`, which never saw our
    // code. ADR-0001's risk table names "hand-rolled JPEG encoder produces
    // subtly wrong colour or chroma" as a medium-likelihood risk with
    // round-trip tests against `zune-jpeg` as the mitigation. This is it.
    for quality in [75u8, 85, 95] {
        let source = gradient(64, 48);
        let encoded = jpeg(&source, quality);
        let (width, height, decoded) = decode_with_zune(&encoded);
        assert_eq!((width, height), (64, 48), "quality {quality}: wrong size");
        assert_eq!(decoded.len(), 64 * 48 * 3);

        let mut total = 0u64;
        let mut worst = 0i32;
        for index in 0..(64 * 48) {
            for channel in 0..3 {
                let original = i32::from(source.data()[index * 4 + channel]);
                let actual = i32::from(decoded[index * 3 + channel]);
                let error = (original - actual).abs();
                total += error as u64;
                worst = worst.max(error);
            }
        }
        let mean = total as f64 / (64.0 * 48.0 * 3.0);
        // Loose enough not to be brittle, tight enough that a wrong DCT
        // constant, a transposed quantisation table or a swapped chroma
        // channel could not pass: any of those put the mean error in the tens.
        let allowed = if quality >= 95 { 3.0 } else { 7.0 };
        assert!(
            mean < allowed,
            "quality {quality}: mean error {mean:.2} exceeds {allowed}"
        );
        assert!(
            worst < 90,
            "quality {quality}: worst channel error {worst} is too large"
        );
    }
}

#[test]
fn a_flat_colour_survives_encoding_almost_exactly() {
    // Flat areas are where quantisation should be invisible, so this catches a
    // systematic colour-conversion error that a gradient's average might hide.
    let source = quadrants(64, 64);
    let (_, _, decoded) = decode_with_zune(&jpeg(&source, 95));
    let sample = |x: u32, y: u32| {
        let index = (y * 64 + x) as usize * 3;
        [decoded[index], decoded[index + 1], decoded[index + 2]]
    };
    for (x, y, expected) in [
        (16u32, 16u32, RED),
        (48, 16, GREEN),
        (48, 48, YELLOW),
        (16, 48, BLUE),
    ] {
        let actual = sample(x, y);
        assert!(
            close(actual, expected, 4),
            "at ({x},{y}): expected {expected:?}, got {actual:?}"
        );
    }
}

#[test]
fn the_eight_exif_orientations_all_come_out_upright() {
    // A landscape source, so the four transposing orientations visibly swap the
    // page's axes as well as moving the pixels.
    let source = quadrants(64, 32);
    // Where each quadrant's colour belongs after the transform, clockwise from
    // the top left. Derived from TIFF 6.0 section 8 rather than from our own
    // code, so this table is an independent statement of the expected result.
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
        let image = pochoir_core::decode_image(&tagged)
            .unwrap_or_else(|e| panic!("{orientation:?} failed to decode: {e}"));
        assert_eq!(
            image.orientation(),
            orientation,
            "{orientation:?}: the tag was not read"
        );
        let pixmap = image.pixmap();
        assert_eq!(
            (pixmap.width(), pixmap.height()),
            (width, height),
            "{orientation:?}: wrong dimensions"
        );
        let actual = quadrant_colours(pixmap);
        for (index, (got, want)) in actual.iter().zip(corners.iter()).enumerate() {
            assert!(
                close(*got, *want, 12),
                "{orientation:?}: corner {index} is {got:?}, expected {want:?}"
            );
        }

        // And the page the PDF declares follows the upright pixels, not the
        // stored ones: a portrait photograph must not produce a landscape page.
        let pdf = image_to_pdf(&tagged, &OutputOptions::default()).expect("renders");
        let (page_width, page_height) = media_box(&pdf);
        assert_eq!(
            page_width > page_height,
            width > height,
            "{orientation:?}: page is {page_width}x{page_height} for a {width}x{height} image"
        );
    }
}

#[test]
fn an_absent_or_nonsensical_orientation_tag_leaves_the_image_alone() {
    let source = quadrants(32, 16);
    // No EXIF at all.
    let image = pochoir_core::decode_image(&jpeg(&source, 95)).expect("decodes");
    assert_eq!(image.orientation(), Orientation::TopLeft);
    assert_eq!((image.pixmap().width(), image.pixmap().height()), (32, 16));

    // An EXIF block whose orientation value is not one of the eight.
    let mut block = exif_block(Orientation::RightTop, 72, false);
    let orientation_value_at = 10 + 8;
    block[orientation_value_at..orientation_value_at + 2].copy_from_slice(&99u16.to_le_bytes());
    let tagged = with_exif(&jpeg(&source, 95), &block);
    let image = pochoir_core::decode_image(&tagged).expect("decodes");
    assert_eq!(image.orientation(), Orientation::TopLeft);
}

#[test]
fn the_page_is_the_images_own_aspect_ratio_at_the_chosen_dpi() {
    // ADR-0002: "the page is the image's own aspect ratio, sized so that it
    // prints at the chosen DPI". Never stretched on one axis. ADR-0006 adds a
    // cap at A4, so this checks the uncapped rule through the opt-out; the cap
    // has its own table below.
    for (width, height) in [(600u32, 400u32), (400, 600), (512, 512), (1000, 250)] {
        for dpi in [150.0f32, 200.0, 300.0] {
            let bytes = jpeg(&gradient(width, height), 75);
            let options = OutputOptions {
                quality: 75,
                dpi,
                page_sizing: PageSizing::Native,
                ..OutputOptions::default()
            };
            let pdf = image_to_pdf(&bytes, &options).expect("renders");
            let (page_width, page_height) = media_box(&pdf);
            let expected_width = f64::from(width) / f64::from(dpi) * 72.0;
            let expected_height = f64::from(height) / f64::from(dpi) * 72.0;
            assert!(
                (page_width - expected_width).abs() < 0.01,
                "{width}x{height} at {dpi} dpi: width {page_width} not {expected_width}"
            );
            assert!(
                (page_height - expected_height).abs() < 0.01,
                "{width}x{height} at {dpi} dpi: height {page_height} not {expected_height}"
            );
            // The pixel aspect ratio is preserved exactly.
            let pixel_ratio = f64::from(width) / f64::from(height);
            let page_ratio = page_width / page_height;
            assert!(
                (pixel_ratio - page_ratio).abs() < 1e-4,
                "{width}x{height}: aspect ratio changed from {pixel_ratio} to {page_ratio}"
            );
        }
    }
}

#[test]
fn a_resolution_the_input_states_is_honoured_when_it_is_higher() {
    // A 600 dpi scan carries an honest resolution, and the page should be the
    // document's real physical size: 1200 px at 600 dpi is two inches.
    let bytes = with_exif(
        &jpeg(&gradient(1200, 600), 75),
        &exif_block(Orientation::TopLeft, 600, false),
    );
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let (width, height) = media_box(&pdf);
    assert!((width - 144.0).abs() < 0.01, "width was {width}");
    assert!((height - 72.0).abs() < 0.01, "height was {height}");

    // A camera's meaningless 72 dpi does not get to make a five-foot page: the
    // requested 200 dpi wins.
    let bytes = with_exif(
        &jpeg(&gradient(1200, 600), 75),
        &exif_block(Orientation::TopLeft, 72, false),
    );
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let (width, _) = media_box(&pdf);
    assert!(
        (width - 1200.0 / 200.0 * 72.0).abs() < 0.01,
        "width was {width}, expected the 200 dpi page"
    );
}

/// One row of the A4 table: pixel width and height, the resolution the input
/// states, the page in millimetres with the cap, and the page without it.
type SizingCase = (u32, u32, f32, (f64, f64), (f64, f64));

/// Page size in millimetres, which is how every one of these cases is stated.
fn page_mm(pdf: &[u8]) -> (f64, f64) {
    let (width, height) = media_box(pdf);
    (width / 72.0 * 25.4, height / 72.0 * 25.4)
}

#[test]
fn an_image_page_is_capped_to_a4_but_never_enlarged_to_it() {
    // ADR-0006. Every case is (pixels, stated dpi, expected mm with the cap,
    // expected mm without it), and the expectations are computed from ISO 216
    // rather than from our own code.
    //
    // The pixel counts are scaled down from the real-world ones, and the
    // resolutions with them: the page is a function of pixels divided by dpi,
    // so a 1008 px image at 50 dpi lands on exactly the same sheet as the
    // 4032 px one at 200 that it stands in for, for a sixteenth of the
    // encoding. `page.rs` checks the full-size numbers at the geometry level,
    // where it costs nothing, and `the_embedded_jpeg_reports_the_resolution_
    // the_page_really_prints_at` runs a real 12-megapixel frame end to end.
    let cases: [SizingCase; 5] = [
        // A 12 MP phone photo, landscape: the headline case. Larger than A2
        // before the cap. Stands in for 4032x3024 at 200 dpi.
        (1008, 756, 50.0, (280.0, 210.0), (512.1, 384.1)),
        // The same photo portrait.
        (756, 1008, 50.0, (210.0, 280.0), (384.1, 512.1)),
        // Square: bounded by A4's short edge whichever way the sheet is turned.
        (750, 750, 50.0, (210.0, 210.0), (381.0, 381.0)),
        // A scan of an identity card, already far inside A4. Must come out at
        // its true size, not blown up to fill the sheet.
        (1011, 638, 300.0, (85.6, 54.0), (85.6, 54.0)),
        // An A4 scan. 210 mm at 150 dpi is 1240.2 px, so a scanner writes 1241
        // and the page is a hair over A4; the cap takes the hair off and
        // nothing more.
        (1241, 1755, 150.0, (210.0, 297.0), (210.1, 297.2)),
    ];

    for (width, height, dpi, capped_mm, native_mm) in cases {
        // Stating the resolution in EXIF is what a scanner does, and
        // `max(stated, requested)` then makes it the effective one.
        let bytes = with_exif(
            &jpeg(&gradient(width, height), 60),
            &exif_block(Orientation::TopLeft, dpi as u32, false),
        );

        let capped = image_to_pdf(
            &bytes,
            &OutputOptions {
                quality: 60,
                dpi,
                page_sizing: PageSizing::CapToA4,
                ..OutputOptions::default()
            },
        )
        .expect("renders");
        let got = page_mm(&capped);
        assert!(
            (got.0 - capped_mm.0).abs() < 0.2 && (got.1 - capped_mm.1).abs() < 0.2,
            "{width}x{height} at {dpi} dpi capped to {got:?} mm, expected {capped_mm:?}"
        );
        // Inside an A4 sheet turned either way, with a hair of slack for the
        // rounding above.
        let (short, long) = (got.0.min(got.1), got.0.max(got.1));
        assert!(
            short <= 210.05 && long <= 297.05,
            "{width}x{height}: {got:?} mm does not fit an A4 sheet"
        );

        let native = image_to_pdf(
            &bytes,
            &OutputOptions {
                quality: 60,
                dpi,
                page_sizing: PageSizing::Native,
                ..OutputOptions::default()
            },
        )
        .expect("renders");
        let got = page_mm(&native);
        assert!(
            (got.0 - native_mm.0).abs() < 0.2 && (got.1 - native_mm.1).abs() < 0.2,
            "{width}x{height} at {dpi} dpi native was {got:?} mm, expected {native_mm:?}"
        );
    }
}

#[test]
fn the_cap_changes_the_page_and_nothing_else() {
    // The argument that settled this: there is no quality at stake, because
    // capping touches no pixel. Same input, same quality, two page sizings -
    // the embedded JPEG must differ only in the density it reports about
    // itself, and the decoded pixels must be identical.
    let bytes = jpeg(&gradient(1200, 900), 85);
    let options = OutputOptions {
        quality: 85,
        dpi: 50.0,
        page_sizing: PageSizing::CapToA4,
        ..OutputOptions::default()
    };
    let capped = image_to_pdf(&bytes, &options).expect("renders");
    let native = image_to_pdf(
        &bytes,
        &OutputOptions {
            page_sizing: PageSizing::Native,
            ..options
        },
    )
    .expect("renders");

    assert_ne!(page_mm(&capped), page_mm(&native), "the cap did nothing");

    let (cw, ch, capped_pixels) = decode_with_zune(first_image_stream(&capped));
    let (nw, nh, native_pixels) = decode_with_zune(first_image_stream(&native));
    assert_eq!((cw, ch), (nw, nh), "the cap resampled the image");
    assert_eq!((cw, ch), (1200, 900));
    assert_eq!(
        capped_pixels, native_pixels,
        "the cap changed a pixel; it is only allowed to change the page"
    );

    // The two datastreams differ only in the JFIF density field, which is the
    // one thing that honestly should change.
    let capped_stream = first_image_stream(&capped);
    let native_stream = first_image_stream(&native);
    assert_eq!(capped_stream.len(), native_stream.len());
    let differing: Vec<usize> = capped_stream
        .iter()
        .zip(native_stream.iter())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    // JFIF APP0 density: X at bytes 14..16 and Y at 16..18 of the datastream.
    assert!(
        differing.iter().all(|&i| (14..18).contains(&i)),
        "the two datastreams differ outside the JFIF density field: {differing:?}"
    );
}

#[test]
fn the_embedded_jpeg_reports_the_resolution_the_page_really_prints_at() {
    // A capped page is denser than the request: the pixels did not move and
    // the sheet got smaller. Writing 200 into the datastream would be false.
    let bytes = jpeg(&gradient(4032, 3024), 60);
    let pdf = image_to_pdf(
        &bytes,
        &OutputOptions {
            quality: 60,
            dpi: 200.0,
            page_sizing: PageSizing::CapToA4,
            ..OutputOptions::default()
        },
    )
    .expect("renders");
    let stream = first_image_stream(&pdf);
    assert_eq!(&stream[6..11], b"JFIF\0");
    assert_eq!(stream[13], 0x01, "density should be in dots per inch");
    let density = u16::from_be_bytes([stream[14], stream[15]]);
    assert!(
        (365..=367).contains(&density),
        "datastream claims {density} dpi, but the page prints at about 366"
    );

    // Without the cap it is the requested figure, because that is then true.
    let pdf = image_to_pdf(
        &bytes,
        &OutputOptions {
            quality: 60,
            dpi: 200.0,
            page_sizing: PageSizing::Native,
            ..OutputOptions::default()
        },
    )
    .expect("renders");
    let stream = first_image_stream(&pdf);
    assert_eq!(u16::from_be_bytes([stream[14], stream[15]]), 200);
}

#[test]
fn the_capped_page_is_still_byte_identical_across_runs() {
    let bytes = jpeg(&gradient(1008, 756), 60);
    let options = OutputOptions::default();
    assert_eq!(
        image_to_pdf(&bytes, &options).expect("renders"),
        image_to_pdf(&bytes, &options).expect("renders")
    );
}

#[test]
fn the_default_caps() {
    // The default is the cap, so somebody who passes nothing gets an A4 page.
    let bytes = jpeg(&gradient(1008, 756), 60);
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let (width, height) = page_mm(&pdf);
    // 1008x756 at the default 200 dpi is 128 by 96 mm, already inside A4, so
    // the default must leave it alone rather than inflate it.
    assert!(
        (width - 128.0).abs() < 0.2 && (height - 96.0).abs() < 0.2,
        "the default produced a {width}x{height} mm page"
    );

    // And at a resolution that does overflow the sheet, the default caps.
    let pdf = image_to_pdf(
        &bytes,
        &OutputOptions {
            dpi: 50.0,
            ..OutputOptions::default()
        },
    )
    .expect("renders");
    let (width, height) = page_mm(&pdf);
    assert!(
        (width - 280.0).abs() < 0.2 && (height - 210.0).abs() < 0.2,
        "the default produced a {width}x{height} mm page"
    );
}

#[test]
fn no_exif_and_no_gps_reaches_the_output() {
    // ADR-0002 Decision 5 makes this a named requirement rather than a
    // footnote: "A test must assert that a GPS-tagged input produces an output
    // PDF containing no `Exif\x00\x00` byte sequence, on every mode."
    let source = gradient(96, 64);
    let tagged = with_exif(
        &jpeg(&source, 85),
        &exif_block(Orientation::RightTop, 72, true),
    );
    // First, prove the fixture really is tagged. A hygiene test over an input
    // with no metadata would pass for the wrong reason.
    let coordinates = gps_bytes();
    assert!(
        contains(&tagged, EXIF_HEADER),
        "the fixture carries no Exif header, so this test would prove nothing"
    );
    assert!(
        contains(&tagged, &coordinates),
        "the fixture carries no GPS coordinates"
    );
    assert!(contains(&tagged, b"CAM\0"), "the fixture carries no Make");

    for quality in [75u8, 85, 95] {
        for dpi in [150.0f32, 300.0] {
            let options = OutputOptions {
                quality,
                dpi,
                ..OutputOptions::default()
            };
            let pdf = image_to_pdf(&tagged, &options).expect("renders");
            assert!(
                !contains(&pdf, EXIF_HEADER),
                "q{quality} {dpi}dpi: the output contains an Exif header"
            );
            assert!(
                !contains(&pdf, &coordinates),
                "q{quality} {dpi}dpi: the output contains the GPS coordinates"
            );
            assert!(
                !contains(&pdf, b"CAM\0"),
                "q{quality} {dpi}dpi: the output contains the camera make"
            );
            // The TIFF magic that every Exif block starts with, in either byte
            // order, and the APP1 marker that would carry it.
            assert!(
                !contains(&pdf, b"II\x2a\x00"),
                "q{quality}: TIFF header leaked"
            );
            assert!(
                !contains(&pdf, b"MM\x00\x2a"),
                "q{quality}: TIFF header leaked"
            );
            let image = first_image_stream(&pdf);
            assert!(
                !contains(image, &[0xFF, 0xE1]),
                "q{quality}: the embedded JPEG has an APP1 segment"
            );
        }
    }
}

#[test]
fn a_png_text_chunk_does_not_reach_the_output_either() {
    let bytes = png(&gradient(48, 48), None, Some("secret note"));
    assert!(contains(&bytes, b"secret note"), "the fixture has no tEXt");
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    assert!(!contains(&pdf, b"secret note"));
    assert!(!contains(&pdf, b"Comment"));
}

#[test]
fn the_output_pdf_has_the_structure_adr_0002_specifies() {
    let bytes = jpeg(&gradient(80, 120), 85);
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let text = String::from_utf8_lossy(&pdf);

    assert!(pdf.starts_with(b"%PDF-1.7"), "no version header");
    assert!(pdf.ends_with(b"%%EOF\n"), "no end-of-file marker");
    assert!(text.contains("/Type /Catalog"));
    assert!(text.contains("/Type /Pages /Count 1"));
    assert!(text.contains("/Type /Page "));
    assert!(
        text.contains("/Filter /DCTDecode"),
        "image is not passed through"
    );
    assert!(!text.contains("/FlateDecode"), "nothing should be deflated");
    assert!(text.contains("/ColorSpace /DeviceRGB"));
    assert!(text.contains("/BitsPerComponent 8"));
    assert!(text.contains("/Width 80 /Height 120"));
    assert!(text.contains("/Producer (Pochoir)"));
    assert!(text.contains("/Creator (Pochoir)"));
    assert!(text.contains("/ID [<"));
    for forbidden in [
        "/CreationDate",
        "/ModDate",
        "/Author",
        "/Title",
        "/Subject",
        "/Keywords",
        "/Metadata",
        "xpacket",
        "x:xmpmeta",
    ] {
        assert!(!text.contains(forbidden), "the output carries {forbidden}");
    }

    // The embedded stream is exactly what our encoder produced, with no
    // further processing by the writer: ISO 32000-1 7.4.8 lets the JPEG
    // datastream be the stream, so `/DCTDecode` means the viewer decodes our
    // bytes with our own quantisation tables and loses nothing more.
    //
    // Note what this does *not* claim. The input JPEG is not passed through:
    // the burn-in path decodes to pixels and re-encodes, which is forced by
    // compositing and is also what makes ADR-0002's metadata hygiene
    // structural. The verbatim guarantee is between the encoder and the
    // writer, which is where a writer that recompressed would do its damage.
    let stream = first_image_stream(&pdf);
    let image = pochoir_core::decode_image(&bytes).expect("decodes");
    let options = OutputOptions::default();
    let re_encoded =
        pochoir_core::jpeg::encode(image.pixmap(), options.quality, options.dpi).expect("encodes");
    assert_eq!(
        stream,
        &re_encoded[..],
        "the writer did not embed the encoder's output unchanged"
    );
    // And it really is a JPEG datastream, agreeing with the dictionary above.
    let info = pochoir_core::jpeg::parse_info(stream).expect("the stream is a JPEG");
    assert_eq!((info.width, info.height), (80, 120));
    assert_eq!(info.components, 3);
    assert_eq!(info.precision, 8);
}

#[test]
fn the_same_input_and_parameters_give_byte_identical_output() {
    // ADR-0002 Decision 6: output must be a pure function of (input bytes,
    // parameters). Nothing reads a clock or a random number, so two runs of
    // the same build must agree exactly.
    let tagged = with_exif(
        &jpeg(&gradient(120, 90), 85),
        &exif_block(Orientation::RightTop, 72, true),
    );
    let options = OutputOptions::default();
    let first = image_to_pdf(&tagged, &options).expect("renders");
    let second = image_to_pdf(&tagged, &options).expect("renders");
    assert_eq!(first, second, "two runs produced different bytes");

    // And different parameters really do produce different output, so the test
    // above is not passing because something is being cached.
    let third = image_to_pdf(
        &tagged,
        &OutputOptions {
            quality: 95,
            ..options
        },
    )
    .expect("renders");
    assert_ne!(first, third);
}

#[test]
fn a_png_input_takes_the_same_path_as_a_jpeg() {
    let source = quadrants(64, 48);
    let bytes = png(&source, None, None);
    let image = pochoir_core::decode_image(&bytes).expect("decodes");
    assert_eq!(image.format(), pochoir_core::image::Format::Png);
    // PNG is lossless, so the decode is exact.
    assert_eq!(image.pixmap(), &source);

    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let text = String::from_utf8_lossy(&pdf);
    // The page is still a JPEG image XObject: `standard` mode is JPEG q85
    // whatever came in. The `lossless` mode of ADR-0002 is what will keep PNG.
    assert!(text.contains("/Filter /DCTDecode"));
    assert_eq!(media_box(&pdf).0, 64.0 / 200.0 * 72.0);
}

#[test]
fn a_pngs_physical_resolution_is_read() {
    // 300 dpi is 11811 pixels per metre, which is what a scanner writes.
    let bytes = png(&gradient(600, 300), Some(11_811), None);
    let pdf = image_to_pdf(&bytes, &OutputOptions::default()).expect("renders");
    let (width, height) = media_box(&pdf);
    // 600 px at ~300 dpi is two inches, i.e. 144 points.
    assert!((width - 144.0).abs() < 0.2, "width was {width}");
    assert!((height - 72.0).abs() < 0.2, "height was {height}");
}

#[test]
fn transparency_is_composited_over_white() {
    // JPEG has no alpha. A transparent region must become white, not black and
    // not whatever happened to be in the RGB channels underneath.
    let mut source = quadrants(32, 32);
    for y in 0..32u32 {
        for x in 0..32u32 {
            source.set_pixel(x, y, [10, 10, 10, 0]);
        }
    }
    let bytes = png(&source, None, None);
    let encoded = {
        let image = pochoir_core::decode_image(&bytes).expect("decodes");
        pochoir_core::jpeg::encode(image.pixmap(), 95, 200.0).expect("encodes")
    };
    let (_, _, decoded) = decode_with_zune(&encoded);
    assert!(
        close([decoded[0], decoded[1], decoded[2]], [255, 255, 255], 4),
        "a fully transparent pixel became {:?}",
        &decoded[..3]
    );
}

#[test]
fn garbage_input_is_refused_without_panicking() {
    let options = OutputOptions::default();
    for bytes in [
        &b""[..],
        &b"not an image"[..],
        &b"%PDF-1.7\n"[..],
        // A JPEG magic number and nothing else.
        &[0xFF, 0xD8, 0xFF][..],
        // A PNG signature and nothing else.
        &b"\x89PNG\r\n\x1a\n"[..],
    ] {
        assert!(
            image_to_pdf(bytes, &options).is_err(),
            "{bytes:?} should not have produced a PDF"
        );
    }
    // A truncated but well-formed-looking JPEG.
    let mut truncated = jpeg(&gradient(64, 64), 85);
    truncated.truncate(60);
    assert!(image_to_pdf(&truncated, &options).is_err());
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|w| w == needle)
}
