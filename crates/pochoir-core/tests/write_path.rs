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
use pochoir_core::{OutputOptions, image_to_pdf};

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
    // prints at the chosen DPI". Never A4, never stretched on one axis.
    for (width, height) in [(600u32, 400u32), (400, 600), (512, 512), (1000, 250)] {
        for dpi in [150.0f32, 200.0, 300.0] {
            let bytes = jpeg(&gradient(width, height), 75);
            let options = OutputOptions { quality: 75, dpi };
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
            let pdf = image_to_pdf(&tagged, &OutputOptions { quality, dpi }).expect("renders");
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
