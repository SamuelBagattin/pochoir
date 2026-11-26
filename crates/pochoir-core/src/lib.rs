//! Core engine for `pochoir`: local-only document watermarking.
//!
//! Everything in this crate must compile for `wasm32-unknown-unknown`, because
//! the browser build is the primary target and the native CLI is the same code
//! reused. That rules out `std::fs`, `std::time`, threads and sockets here:
//! I/O belongs to the callers (`pochoir-cli`, the wasm bindings).
//!
//! # What is here today
//!
//! An image goes in, a watermarked PDF comes out.
//!
//! ```text
//! bytes -> image::decode -> ImagePage (a PageSource) -> mark::compose -> jpeg::encode -> pdf::PdfWriter
//!            |                  |                           |                |               |
//!         zune-jpeg / png    EXIF orientation,          ours: font,        ours,           ours,
//!         (the only two       no resampling             raster, blur,      baseline        /DCTDecode
//!          dependencies)                                rotation           4:4:4           verbatim
//! ```
//!
//! The page an image lands on keeps its aspect ratio and is sized at the
//! effective resolution, then capped to an A4 sheet — shrunk to fit, never
//! enlarged to fill. That cap is ADR-0006, which supersedes ADR-0002 on this
//! point for image inputs only; [`PageSizing::Native`] opts out of it.
//!
//! The mark is [`mark`], specified by
//! `docs/adr/0002-watermark-algorithm-and-quality-modes.md` and corrected in
//! four respects by `docs/adr/0007-watermark-specification-corrections.md`. It
//! is composited onto the [`Pixmap`] that [`PageSource::rasterise`] produces,
//! between those stages and nowhere else, so the compositor cannot know what
//! kind of document the pixels came from. Not yet here: PDF input
//! ([ADR-0003]), the exclusion zone, `overlay` mode and the PNG encoder.
//!
//! [ADR-0003]: ../../../docs/adr/0003-pdf-input-rasterisation.md
//!
//! # The two dependencies
//!
//! `zune-jpeg` and `png`, for decode only, justified in
//! `docs/adr/0001-stack-and-dependency-policy.md`: they parse hostile input,
//! which is the single case its rule — *hand-roll everything that writes;
//! depend deliberately only for parsers of hostile input* — says to depend for.
//! The JPEG encoder, the PDF writer, the EXIF read and the content fingerprint
//! are all ours.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use core::fmt;

pub mod exif;
pub mod fingerprint;
pub mod font;
pub mod geom;
pub mod image;
pub mod jpeg;
pub mod mark;
pub mod page;
pub mod pdf;
pub mod pixmap;
pub mod raster;

#[cfg(test)]
pub(crate) mod test_support;

pub use image::{Image, ImagePage, decode as decode_image, sniff};
pub use mark::{Mark, TileDensity};
pub use page::{PageGeometry, PageSizing, PageSource, RasterError};
pub use pixmap::Pixmap;

/// Anything that can go wrong inside the core engine.
///
/// Hand-rolled on purpose: no `thiserror`, no `anyhow`. The variants carry only
/// `Copy` data so the type stays cheap to pass across the wasm boundary, which
/// is also why a decoder's own formatted message is reduced to a `&'static str`
/// reason rather than kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The input was not recognised as a document format we support.
    UnsupportedFormat,
    /// The input ended before a complete document could be read.
    TruncatedInput {
        /// Byte offset at which the input ran out.
        offset: usize,
    },
    /// A caller-supplied parameter was outside its accepted range.
    InvalidParameter {
        /// Name of the offending parameter.
        name: &'static str,
    },
    /// A decoder refused the input.
    Decode {
        /// The format we believed we were reading, e.g. `JPEG`.
        format: &'static str,
        /// Why the decoder gave up, in a few words.
        reason: &'static str,
    },
    /// An internal buffer was not the size the operation needed.
    BufferSize {
        /// Bytes the operation required.
        expected: usize,
        /// Bytes it was given.
        got: usize,
    },
    /// The image exceeds what this build will allocate for.
    ImageTooLarge {
        /// Width that was asked for.
        width: u32,
        /// Height that was asked for.
        height: u32,
    },
    /// The mark text contains a character the bundled font cannot draw.
    ///
    /// Only the first one is carried, because every [`Error`] variant holds
    /// `Copy` data only; [`mark::unsupported_characters`] lists them all.
    UnsupportedCharacter {
        /// The first character that has no glyph.
        character: char,
    },
    /// The mark text will not fit the page at any permitted font size.
    ///
    /// Raised rather than clipping the text, because a truncated mark asserts
    /// something different from the one the user wrote. See ADR-0007.
    MarkDoesNotFit {
        /// How many characters the mark text has.
        characters: usize,
    },
    /// A page was requested that the source does not have.
    PageOutOfRange {
        /// Index that was asked for.
        index: usize,
        /// Number of pages the source has.
        count: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnsupportedFormat => f.write_str("unsupported document format"),
            Self::TruncatedInput { offset } => {
                write!(f, "input truncated at byte {offset}")
            }
            Self::InvalidParameter { name } => {
                write!(f, "invalid value for parameter `{name}`")
            }
            Self::Decode { format, reason } => {
                write!(f, "could not decode this {format}: {reason}")
            }
            Self::BufferSize { expected, got } => {
                write!(f, "expected a buffer of {expected} bytes, got {got}")
            }
            Self::ImageTooLarge { width, height } => {
                write!(f, "image is too large to process: {width}x{height}")
            }
            Self::UnsupportedCharacter { character } => write!(
                f,
                "the mark text contains a character the bundled font cannot \
                 draw: {character:?} (U+{:04X})",
                u32::from(character)
            ),
            Self::MarkDoesNotFit { characters } => write!(
                f,
                "the mark text does not fit the page: {characters} characters \
                 will not fit in {} lines even at the smallest font size; \
                 shorten it",
                mark::MAX_LINES
            ),
            Self::PageOutOfRange { index, count } => {
                write!(f, "no page {index}: this document has {count}")
            }
        }
    }
}

impl core::error::Error for Error {}

/// Shorthand for a result carrying this crate's [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

/// JPEG quality of ADR-0002's default `standard` mode.
pub const DEFAULT_QUALITY: u8 = 85;

/// Resolution of ADR-0002's default `standard` mode, in dots per inch.
///
/// For a bitmap input this only decides what physical page size the pixels
/// represent — the pixels themselves are never resampled. It is the
/// rasterisation resolution that will matter for PDF inputs.
pub const DEFAULT_DPI: f32 = 200.0;

/// Highest resolution we accept, as a sanity bound rather than a real limit.
pub const MAX_DPI: f32 = 10_000.0;

/// What the caller chooses about the output.
///
/// The encoding knobs are ADR-0002's `standard` mode; the remaining
/// modes — `compact`, `high`, `lossless` — are combinations of them plus a PNG
/// encoder, and are not here yet.
///
/// Not `Copy`, because [`mark`] carries the mark text. Every field is cheap to
/// clone and the struct is passed by reference everywhere.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputOptions {
    /// JPEG quality, `1..=100`.
    pub quality: u8,
    /// Output resolution in dots per inch.
    pub dpi: f32,
    /// Whether a bitmap input's page may exceed an A4 sheet. See ADR-0006.
    ///
    /// Applies to image inputs only. For a PDF input the DPI is a real
    /// rasterisation resolution and the page is the input's own `MediaBox`, so
    /// there is nothing here to bound.
    pub page_sizing: PageSizing,
    /// The watermark to burn in, or `None` for the bare write path.
    ///
    /// `None` by default: the engine does not invent a mark, because ADR-0002's
    /// closed open question decides that nothing is pre-filled. A mark naming
    /// no recipient and no purpose is exactly the mark that fails to deter
    /// reuse, so an empty one is not drawn at all.
    pub mark: Option<mark::Mark>,
    /// Seed for the mark's position jitter, or `None` to derive one.
    ///
    /// ADR-0002 Decision 6: derived from the document and the parameter set, so
    /// two runs over the same input agree and two different inputs do not.
    /// Setting it makes a run reproducible across documents, which is what the
    /// golden tests use.
    pub seed: Option<u64>,
    /// Whether the mark's tiles are displaced at all.
    ///
    /// On by default. ADR-0002 Decision 3 keeps the jitter because two runs
    /// over the same document then put the mark in different places, so an
    /// attacker holding two watermarked copies cannot difference them to
    /// recover the clean original. Turning it off is for reproducing a figure
    /// or comparing two parameter sets, not for normal use.
    pub jitter: bool,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            quality: DEFAULT_QUALITY,
            dpi: DEFAULT_DPI,
            page_sizing: PageSizing::CapToA4,
            mark: None,
            seed: None,
            jitter: true,
        }
    }
}

impl OutputOptions {
    /// Check both values before any work is done.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidParameter`] naming the first offending field.
    pub fn validate(&self) -> Result<()> {
        if !(1..=100).contains(&self.quality) {
            return Err(Error::InvalidParameter { name: "quality" });
        }
        if !self.dpi.is_finite() || self.dpi <= 0.0 || self.dpi > MAX_DPI {
            return Err(Error::InvalidParameter { name: "dpi" });
        }
        if let Some(mark) = &self.mark {
            mark.validate()?;
        }
        Ok(())
    }
}

/// Render every page of a [`PageSource`] into one PDF.
///
/// One pixmap is allocated and reused for every page, which is the memory model
/// `docs/adr/0003-pdf-input-rasterisation.md` requires: rasterise into one
/// buffer, encode, append the encoded page to the output, move on. Peak live
/// memory is one page's pixels plus the output, not the whole document's.
///
/// # Errors
///
/// Returns whatever the source, the encoder or the writer rejected; in
/// particular [`Error::InvalidParameter`] for bad options and
/// [`Error::UnsupportedFormat`] for a source with no pages.
pub fn render_to_pdf<S>(source: &S, options: &OutputOptions) -> Result<Vec<u8>>
where
    S: PageSource + ?Sized,
{
    render_to_pdf_observed(source, options, |_, _| {})
}

/// [`render_to_pdf`], with a look at each page's pixels after the mark and
/// before the encoder.
///
/// This is ADR-0002 Decision 6's **golden-hash hook**. Its test strategy asks
/// for a "golden hash on the composited raster, before encoding", on the
/// grounds that the raster is the layer the watermark algorithm owns and that
/// determinism makes an exact comparison legitimate there — whereas a byte
/// comparison of the finished PDF would break whenever the JPEG encoder
/// changed, which tells you nothing about the watermark.
///
/// `observe` is handed the page index and the composited pixmap. It cannot
/// change them: a hook that could would turn the pure function this crate
/// promises into something else. A caller wanting the bytes themselves can
/// clone inside the closure; the golden tests only fingerprint.
///
/// The same hook is what a live preview will use, which is the other reason it
/// is a closure rather than a debug flag.
///
/// # Errors
///
/// As [`render_to_pdf`].
pub fn render_to_pdf_observed<S, F>(
    source: &S,
    options: &OutputOptions,
    mut observe: F,
) -> Result<Vec<u8>>
where
    S: PageSource + ?Sized,
    F: FnMut(usize, &Pixmap),
{
    options.validate()?;
    let mut writer = pdf::PdfWriter::new();
    let mut pixmap = Pixmap::empty();
    for index in 0..source.page_count() {
        let geometry = source.page_geometry(index);
        source.rasterise(index, options.dpi, &mut pixmap)?;
        // **The mark goes here**, between rasterising and encoding, and
        // nowhere else. At this point the code cannot tell whether the pixels
        // came from a JPEG, a PNG or a rasterised PDF page, which is how
        // ADR-0002 Decision 1's "one compositor, operating on an RGBA pixmap"
        // is made structural rather than a thing somebody has to maintain.
        if let Some(mark) = &options.mark {
            let seed = options
                .seed
                .unwrap_or_else(|| mark::derive_seed(&pixmap, mark));
            mark::compose(&mut pixmap, mark, seed, options.jitter)?;
        }
        observe(index, &pixmap);
        // The density written into the JPEG is the one the page actually
        // prints at, not the one the caller asked for. Capping a page to A4
        // leaves the pixels alone and shrinks the sheet, so the two differ, and
        // writing the requested figure would be stating something untrue about
        // the datastream. Falls back to the request only for a degenerate page,
        // which the writer rejects a moment later anyway.
        let density = geometry.print_dpi(pixmap.width()).unwrap_or(options.dpi);
        let encoded = jpeg::encode(&pixmap, options.quality, density)?;
        writer.add_jpeg_page(geometry, &encoded)?;
    }
    writer.finish()
}

/// Decode a JPEG or PNG and write it out as a one-page PDF.
///
/// The whole write path, end to end. Output is a pure function of
/// `(bytes, options)`: no clock is read, no random number is drawn, and the
/// input's metadata is dropped structurally because the pixels are re-encoded.
///
/// # Errors
///
/// Returns [`Error::UnsupportedFormat`] for input that is neither a JPEG nor a
/// PNG, [`Error::Decode`] if the decoder refused it, and
/// [`Error::InvalidParameter`] for bad options.
pub fn image_to_pdf(bytes: &[u8], options: &OutputOptions) -> Result<Vec<u8>> {
    options.validate()?;
    let image = image::decode(bytes)?;
    let page = ImagePage::new(image, options.dpi, options.page_sizing)?;
    render_to_pdf(&page, options)
}

/// Version of the core engine, as declared in its `Cargo.toml`.
///
/// Reported by the CLI and the wasm bindings so a user can tell which engine
/// they are running. It is deliberately **not** written into any output file:
/// ADR-0002 keeps `/Producer` a constant with no version number, because a
/// version string tells a recipient which build made the file and therefore
/// which known bugs it has.
///
/// ```
/// let v = pochoir_core::version();
/// assert_eq!(v.split('.').count(), 3, "expected a semver triple, got {v:?}");
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_DPI, DEFAULT_QUALITY, Error, OutputOptions, image_to_pdf, test_support, version,
    };

    #[test]
    fn version_is_a_semver_triple() {
        let parts: Vec<&str> = version().split('.').collect();
        assert_eq!(parts.len(), 3, "version() should be `major.minor.patch`");
        for part in parts {
            assert!(
                part.parse::<u32>().is_ok(),
                "version component {part:?} is not a number"
            );
        }
    }

    #[test]
    fn errors_render_a_useful_message() {
        assert_eq!(
            Error::UnsupportedFormat.to_string(),
            "unsupported document format"
        );
        assert_eq!(
            Error::TruncatedInput { offset: 42 }.to_string(),
            "input truncated at byte 42"
        );
        assert_eq!(
            Error::InvalidParameter { name: "opacity" }.to_string(),
            "invalid value for parameter `opacity`"
        );
        assert_eq!(
            Error::Decode {
                format: "JPEG",
                reason: "truncated"
            }
            .to_string(),
            "could not decode this JPEG: truncated"
        );
        assert_eq!(
            Error::ImageTooLarge {
                width: 9,
                height: 8
            }
            .to_string(),
            "image is too large to process: 9x8"
        );
        assert_eq!(
            Error::PageOutOfRange { index: 3, count: 1 }.to_string(),
            "no page 3: this document has 1"
        );
        assert_eq!(
            Error::BufferSize {
                expected: 4,
                got: 3
            }
            .to_string(),
            "expected a buffer of 4 bytes, got 3"
        );
    }

    #[test]
    fn error_is_a_std_error() {
        fn takes_std_error(_: &dyn core::error::Error) {}
        takes_std_error(&Error::UnsupportedFormat);
    }

    #[test]
    fn the_defaults_are_adr_0002s_standard_mode() {
        let options = OutputOptions::default();
        assert_eq!(options.quality, 85);
        assert_eq!(options.dpi, 200.0);
        assert_eq!(DEFAULT_QUALITY, 85);
        assert_eq!(DEFAULT_DPI, 200.0);
        options.validate().expect("the defaults must be valid");
    }

    #[test]
    fn options_are_validated() {
        for quality in [0u8, 101, 255] {
            let options = OutputOptions {
                quality,
                ..Default::default()
            };
            assert_eq!(
                options.validate(),
                Err(Error::InvalidParameter { name: "quality" })
            );
        }
        for dpi in [0.0f32, -1.0, f32::NAN, f32::INFINITY, 10_001.0] {
            let options = OutputOptions {
                dpi,
                ..Default::default()
            };
            assert_eq!(
                options.validate(),
                Err(Error::InvalidParameter { name: "dpi" })
            );
        }
    }

    #[test]
    fn the_write_path_runs_end_to_end() {
        let jpeg = test_support::jpeg_of(test_support::gradient(40, 60));
        let pdf = image_to_pdf(&jpeg, &OutputOptions::default()).expect("renders");
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Filter /DCTDecode"));
        assert!(text.contains("/Count 1"));
    }

    #[test]
    fn the_write_path_is_byte_identical_across_runs() {
        let jpeg = test_support::jpeg_of(test_support::gradient(40, 60));
        let options = OutputOptions::default();
        assert_eq!(
            image_to_pdf(&jpeg, &options).expect("renders"),
            image_to_pdf(&jpeg, &options).expect("renders")
        );
    }

    #[test]
    fn bad_options_are_refused_before_any_decoding() {
        let options = OutputOptions {
            quality: 0,
            ..OutputOptions::default()
        };
        assert_eq!(
            image_to_pdf(b"not an image", &options),
            Err(Error::InvalidParameter { name: "quality" })
        );
    }
}
