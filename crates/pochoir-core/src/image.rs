//! Decoding a bitmap input, and presenting it as a [`PageSource`].
//!
//! Decoding is the one capability ADR-0001 says to *depend* for rather than
//! hand-roll, and the reasoning is worth repeating where the dependency is
//! actually used: a decoder eats attacker-controlled bytes, `zune-jpeg` and
//! `png` have been fuzzed over millions of hostile files, and our first draft
//! would not have been. "We wrote it ourselves" is not a security win here.
//!
//! What this module adds around them is ours: the EXIF read, the orientation
//! transform, the choice of effective resolution, and the refusal to resample.

use crate::exif::{self, Metadata, Orientation};
use crate::page::{PageGeometry, PageSizing, PageSource, RasterError};
use crate::pixmap::{MAX_PIXELS, Pixmap};
use crate::{Error, Result};

/// The bitmap formats the common path can read.
///
/// HEIC is deliberately absent: ADR-0001 ships no HEVC decoder, so the browser
/// handles it where it can and the CLI does not handle it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// JPEG, decoded by `zune-jpeg`.
    Jpeg,
    /// PNG, decoded by `png`.
    Png,
}

impl Format {
    /// The format's usual name, for error messages and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
        }
    }
}

/// Identify an input from its leading bytes.
///
/// Content sniffing rather than a file extension: the extension is the user's
/// guess and the magic number is the file's own claim.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else {
        None
    }
}

/// A decoded bitmap, upright, with whatever the input said about its own
/// resolution.
#[derive(Debug, Clone)]
pub struct Image {
    pixmap: Pixmap,
    format: Format,
    metadata: Metadata,
}

impl Image {
    /// The pixels, already rotated or flipped to be upright.
    #[must_use]
    pub const fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Consume the image and return its pixels.
    #[must_use]
    pub fn into_pixmap(self) -> Pixmap {
        self.pixmap
    }

    /// Which decoder read it.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// The orientation tag the input carried, before it was applied.
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.metadata.orientation
    }

    /// The resolution the input claimed, in dots per inch, if any.
    ///
    /// Axes are swapped along with the pixels when the orientation transposes
    /// them, so this is the resolution of the **upright** image.
    #[must_use]
    pub const fn source_dpi(&self) -> (Option<f32>, Option<f32>) {
        (self.metadata.x_dpi, self.metadata.y_dpi)
    }
}

/// Decode a JPEG or PNG into an upright RGBA8 pixmap.
///
/// # Errors
///
/// - [`Error::UnsupportedFormat`] if the bytes are neither a JPEG nor a PNG.
/// - [`Error::Decode`] if the decoder refused the file.
/// - [`Error::ImageTooLarge`] if the image exceeds [`MAX_PIXELS`].
pub fn decode(bytes: &[u8]) -> Result<Image> {
    match sniff(bytes).ok_or(Error::UnsupportedFormat)? {
        Format::Jpeg => decode_jpeg(bytes),
        Format::Png => decode_png(bytes),
    }
}

/// Apply an orientation to a decoded pixmap and its metadata together, so the
/// two can never disagree about which way round the image is.
fn upright(pixmap: Pixmap, format: Format, mut metadata: Metadata) -> Result<Image> {
    let pixmap = exif::apply(&pixmap, metadata.orientation)?;
    if metadata.orientation.swaps_axes() {
        core::mem::swap(&mut metadata.x_dpi, &mut metadata.y_dpi);
    }
    Ok(Image {
        pixmap,
        format,
        metadata,
    })
}

fn decode_jpeg(bytes: &[u8]) -> Result<Image> {
    use zune_jpeg::JpegDecoder;
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    // Our own EXIF read, over the raw bytes, rather than the decoder's: it is
    // the behaviour ADR-0001 asked us to own, and it does not depend on the
    // decoder continuing to expose metadata.
    let metadata = exif::find_in_jpeg(bytes)
        .map(exif::parse)
        .unwrap_or_default();

    // RGB out, whatever came in. zune's own documentation states that every
    // input colorspace maps to RGB, which is what makes this one branch
    // sufficient for greyscale, YCbCr, CMYK and YCCK sources alike. The
    // dimension caps turn a hostile frame header into an error before anything
    // is allocated.
    let side = MAX_PIXELS as usize;
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(side)
        .set_max_height(side);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|error| decode_error(Format::Jpeg, jpeg_reason(&error)))?;
    let info = decoder
        .info()
        .ok_or_else(|| decode_error(Format::Jpeg, "no frame header"))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    // Allocate through Pixmap so the pixel budget is enforced in one place.
    let mut pixmap = Pixmap::new(width, height)?;
    let rgb = decoder
        .decode()
        .map_err(|error| decode_error(Format::Jpeg, jpeg_reason(&error)))?;
    expand_to_rgba(&rgb, 3, false, pixmap.data_mut())?;
    upright(pixmap, Format::Jpeg, metadata)
}

fn decode_png(bytes: &[u8]) -> Result<Image> {
    use png::{ColorType, Transformations, Unit};

    // `png` wants a seekable reader, so the slice is wrapped in an
    // in-memory cursor. Nothing here touches a file: ADR-0001 keeps all I/O in
    // the callers so that this crate stays wasm32-clean.
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Expand palettes, sub-byte greyscale and tRNS transparency, and reduce
    // 16-bit samples to 8, so the match below has four cases instead of
    // sixteen.
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|error| decode_error(Format::Png, png_reason(&error)))?;

    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    // A PNG states its resolution in pixels per metre, in a pHYs chunk, and may
    // carry an orientation in an eXIf chunk. Both are read for exactly the same
    // two purposes as a JPEG's EXIF, and nothing else from the chunk list is
    // looked at.
    let mut metadata = reader
        .info()
        .exif_metadata
        .as_deref()
        .map(exif::parse)
        .unwrap_or_default();
    if let Some(dims) = reader.info().pixel_dims.filter(|d| d.unit == Unit::Meter) {
        // 1 inch is 0.0254 m.
        let to_dpi = |ppu: u32| (ppu > 0).then_some(ppu as f32 * 0.0254);
        metadata.x_dpi = metadata.x_dpi.or_else(|| to_dpi(dims.xppu));
        metadata.y_dpi = metadata.y_dpi.or_else(|| to_dpi(dims.yppu));
    }

    let mut pixmap = Pixmap::new(width, height)?;
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        // `normalize_to_color8` should have made this unreachable; refusing is
        // better than writing wrong samples if it ever is not.
        return Err(decode_error(Format::Png, "unsupported bit depth"));
    }
    let (channels, has_alpha) = match color {
        ColorType::Grayscale => (1usize, false),
        ColorType::GrayscaleAlpha => (2, true),
        ColorType::Rgb => (3, false),
        ColorType::Rgba => (4, true),
        ColorType::Indexed => {
            return Err(decode_error(Format::Png, "palette was not expanded"));
        }
    };

    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| decode_error(Format::Png, "image too large for this platform"))?;
    let mut raw = vec![0u8; buffer_size];
    let frame = reader
        .next_frame(&mut raw)
        .map_err(|error| decode_error(Format::Png, png_reason(&error)))?;
    raw.truncate(frame.buffer_size());
    expand_to_rgba(&raw, channels, has_alpha, pixmap.data_mut())?;
    upright(pixmap, Format::Png, metadata)
}

/// Widen interleaved 1-, 2-, 3- or 4-channel 8-bit samples into RGBA8.
///
/// Greyscale channels are replicated across R, G and B; a missing alpha channel
/// becomes opaque.
fn expand_to_rgba(source: &[u8], channels: usize, has_alpha: bool, out: &mut [u8]) -> Result<()> {
    let pixels = out.len() / 4;
    let expected = pixels * channels;
    if source.len() < expected {
        return Err(Error::BufferSize {
            expected,
            got: source.len(),
        });
    }
    let grey = channels <= 2;
    for index in 0..pixels {
        let from = &source[index * channels..index * channels + channels];
        let to = &mut out[index * 4..index * 4 + 4];
        if grey {
            to[0] = from[0];
            to[1] = from[0];
            to[2] = from[0];
        } else {
            to[0] = from[0];
            to[1] = from[1];
            to[2] = from[2];
        }
        to[3] = if has_alpha { from[channels - 1] } else { 0xFF };
    }
    Ok(())
}

fn decode_error(format: Format, reason: &'static str) -> Error {
    Error::Decode {
        format: format.name(),
        reason,
    }
}

/// Map a decoder error onto a static reason.
///
/// [`Error`] carries only `Copy` data so that it stays cheap across the wasm
/// boundary, which rules out keeping the decoder's own formatted string. The
/// variant is enough to tell a user whether their file is truncated, corrupt or
/// simply a kind of JPEG we do not accept.
fn jpeg_reason(error: &zune_jpeg::errors::DecodeErrors) -> &'static str {
    use zune_jpeg::errors::DecodeErrors as E;
    match error {
        E::IllegalMagicBytes(_) => "not a JPEG",
        E::ExhaustedData => "truncated",
        E::ZeroError => "zero width or height",
        E::LargeDimensions(_) => "dimensions too large",
        E::Unsupported(_) => "unsupported JPEG variant",
        E::HuffmanDecode(_) | E::MCUError(_) => "corrupt scan data",
        E::DqtError(_) | E::SosError(_) | E::SofError(_) => "corrupt header",
        E::TooSmallOutput(_, _) => "internal buffer too small",
        E::IoErrors(_) => "could not read the input",
        E::Format(_) | E::FormatStatic(_) => "malformed JPEG",
    }
}

fn png_reason(error: &png::DecodingError) -> &'static str {
    use png::DecodingError as E;
    match error {
        E::IoError(_) => "could not read the input",
        E::Format(_) => "malformed PNG",
        E::Parameter(_) => "unsupported PNG parameters",
        E::LimitsExceeded => "exceeds the decoder's limits",
    }
}

/// A single-page [`PageSource`] over a decoded bitmap.
///
/// `rasterise` hands back the decoded buffer unchanged. ADR-0002 is explicit
/// that image inputs are not resampled at all: upsampling is pure waste and
/// downsampling is pure loss, and throwing away 85% of a phone photo's pixels
/// is the specific behaviour of the tool this project exists to improve on.
#[derive(Debug, Clone)]
pub struct ImagePage {
    image: Image,
    dpi_x: f32,
    dpi_y: f32,
    sizing: PageSizing,
}

impl ImagePage {
    /// Wrap a decoded image, resolving its effective resolution.
    ///
    /// The effective DPI is the **larger** of what the input claimed and what
    /// the caller asked for, per axis. ADR-0003 describes this as "geometry from
    /// EXIF resolution or the chosen DPI"; taking the larger is how that is made
    /// safe in practice, because phone cameras routinely write a meaningless
    /// `XResolution` of 72, and honouring it literally would turn a 4 000-pixel
    /// photograph into a page five feet wide. A scanner's honest 600 dpi is
    /// still honoured, and the page is never larger than the requested DPI
    /// implies.
    ///
    /// `sizing` decides whether the resulting page may be larger than an A4
    /// sheet; see [`PageSizing`] and ADR-0006.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidParameter`] if `requested_dpi` is not a finite
    /// positive number, or [`Error::UnsupportedFormat`] if the image has no
    /// pixels.
    pub fn new(image: Image, requested_dpi: f32, sizing: PageSizing) -> Result<Self> {
        if !requested_dpi.is_finite() || requested_dpi <= 0.0 {
            return Err(Error::InvalidParameter { name: "dpi" });
        }
        if image.pixmap().is_empty() {
            return Err(Error::UnsupportedFormat);
        }
        let effective = |claimed: Option<f32>| {
            claimed
                .filter(|dpi| dpi.is_finite() && *dpi > 0.0)
                .map_or(requested_dpi, |dpi| dpi.max(requested_dpi))
        };
        let (x, y) = image.source_dpi();
        Ok(Self {
            dpi_x: effective(x),
            dpi_y: effective(y),
            sizing,
            image,
        })
    }

    /// The decoded image behind this page.
    #[must_use]
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// The resolution the **natural** page size is derived from, per axis,
    /// before any A4 cap is applied.
    ///
    /// This is not necessarily what the page prints at. Capping shrinks the
    /// sheet without touching a pixel, so a capped page is denser than this
    /// number says; [`PageGeometry::print_dpi`] on the result of
    /// [`PageSource::page_geometry`] gives the honest figure.
    #[must_use]
    pub const fn effective_dpi(&self) -> (f32, f32) {
        (self.dpi_x, self.dpi_y)
    }

    /// How this page's size is bounded.
    #[must_use]
    pub const fn sizing(&self) -> PageSizing {
        self.sizing
    }
}

impl PageSource for ImagePage {
    fn page_count(&self) -> usize {
        1
    }

    fn page_geometry(&self, _index: usize) -> PageGeometry {
        let pixmap = self.image.pixmap();
        PageGeometry::from_pixels(pixmap.width(), pixmap.height(), self.dpi_x, self.dpi_y)
            .sized(self.sizing)
    }

    fn rasterise(
        &self,
        index: usize,
        _dpi: f32,
        out: &mut Pixmap,
    ) -> core::result::Result<(), RasterError> {
        if index != 0 {
            return Err(Error::PageOutOfRange { index, count: 1 });
        }
        let source = self.image.pixmap();
        out.resize(source.width(), source.height())?;
        out.data_mut().copy_from_slice(source.data());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, ImagePage, decode, expand_to_rgba, sniff};
    use crate::page::{PageSizing, PageSource};
    use crate::pixmap::Pixmap;
    use crate::{Error, test_support};

    #[test]
    fn formats_are_identified_from_their_magic_bytes() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some(Format::Png));
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"%PDF-1.7"), None);
        // A HEIC file's ftyp box: recognised as nothing, which is the decision
        // ADR-0001 records for the CLI.
        assert_eq!(sniff(b"\0\0\0\x18ftypheic"), None);
    }

    #[test]
    fn a_jpeg_we_encoded_decodes_back_to_the_same_dimensions() {
        let pixmap = test_support::gradient(29, 17);
        let jpeg = crate::jpeg::encode(&pixmap, 95, 200.0).expect("encodes");
        let image = decode(&jpeg).expect("decodes");
        assert_eq!(image.format(), Format::Jpeg);
        assert_eq!(image.pixmap().width(), 29);
        assert_eq!(image.pixmap().height(), 17);
        // Alpha is always opaque coming out of a JPEG.
        assert_eq!(image.pixmap().pixel(0, 0).expect("in range")[3], 0xFF);
    }

    #[test]
    fn unrecognised_input_is_refused() {
        assert!(matches!(
            decode(b"neither a jpeg nor a png"),
            Err(Error::UnsupportedFormat)
        ));
    }

    #[test]
    fn a_corrupt_jpeg_reports_an_error_rather_than_panicking() {
        let mut jpeg =
            crate::jpeg::encode(&test_support::gradient(16, 16), 85, 200.0).expect("encodes");
        // Truncate the scan data.
        jpeg.truncate(jpeg.len() / 2);
        assert!(decode(&jpeg).is_err());
        // Corrupt the frame header.
        let mut jpeg =
            crate::jpeg::encode(&test_support::gradient(16, 16), 85, 200.0).expect("encodes");
        for byte in jpeg.iter_mut().skip(20).take(40) {
            *byte = 0xAA;
        }
        // Either an error or a successful decode is acceptable; a panic is not.
        let _ = decode(&jpeg);
    }

    #[test]
    fn greyscale_and_alpha_channels_widen_correctly() {
        let mut out = [0u8; 8];
        expand_to_rgba(&[0x40, 0x80], 1, false, &mut out).expect("fits");
        assert_eq!(out, [0x40, 0x40, 0x40, 0xFF, 0x80, 0x80, 0x80, 0xFF]);
        expand_to_rgba(&[0x40, 0x10, 0x80, 0x20], 2, true, &mut out).expect("fits");
        assert_eq!(out, [0x40, 0x40, 0x40, 0x10, 0x80, 0x80, 0x80, 0x20]);
        expand_to_rgba(&[1, 2, 3, 4, 5, 6], 3, false, &mut out).expect("fits");
        assert_eq!(out, [1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
        expand_to_rgba(&[1, 2, 3, 4, 5, 6, 7, 8], 4, true, &mut out).expect("fits");
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn expanding_from_too_few_samples_is_an_error() {
        let mut out = [0u8; 8];
        assert_eq!(
            expand_to_rgba(&[1, 2, 3], 3, false, &mut out),
            Err(Error::BufferSize {
                expected: 6,
                got: 3
            })
        );
    }

    #[test]
    fn rasterising_hands_back_the_decoded_buffer_unchanged() {
        let pixmap = test_support::gradient(13, 7);
        let jpeg = crate::jpeg::encode(&pixmap, 95, 200.0).expect("encodes");
        let image = decode(&jpeg).expect("decodes");
        let expected = image.pixmap().clone();
        let page = ImagePage::new(image, 200.0, PageSizing::default()).expect("one page");
        assert_eq!(page.page_count(), 1);
        let mut out = Pixmap::empty();
        page.rasterise(0, 200.0, &mut out).expect("rasterises");
        assert_eq!(out, expected, "rasterise must not resample");
        // A second page does not exist.
        assert_eq!(
            page.rasterise(1, 200.0, &mut out),
            Err(Error::PageOutOfRange { index: 1, count: 1 })
        );
    }

    #[test]
    fn a_requested_dpi_wins_over_a_camera_default_of_seventy_two() {
        let pixmap = test_support::gradient(400, 200);
        let jpeg = crate::jpeg::encode(&pixmap, 85, 72.0).expect("encodes");
        // Our encoder writes JFIF density, not EXIF, so this image claims no
        // EXIF resolution at all and the requested DPI applies directly.
        let image = decode(&jpeg).expect("decodes");
        assert_eq!(image.source_dpi(), (None, None));
        let page = ImagePage::new(image, 200.0, PageSizing::default()).expect("one page");
        assert_eq!(page.effective_dpi(), (200.0, 200.0));
        let geometry = page.page_geometry(0);
        assert_eq!(geometry.width_pt, 400.0 / 200.0 * 72.0);
        assert_eq!(geometry.height_pt, 200.0 / 200.0 * 72.0);
    }

    #[test]
    fn the_dpi_parameter_is_validated() {
        let pixmap = test_support::gradient(8, 8);
        let jpeg = crate::jpeg::encode(&pixmap, 85, 200.0).expect("encodes");
        let image = decode(&jpeg).expect("decodes");
        assert!(matches!(
            ImagePage::new(image, 0.0, PageSizing::default()),
            Err(Error::InvalidParameter { name: "dpi" })
        ));
    }
}
