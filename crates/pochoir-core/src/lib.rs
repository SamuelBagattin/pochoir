//! Core engine for `pochoir`: local-only document watermarking.
//!
//! Everything in this crate must compile for `wasm32-unknown-unknown`, because
//! the browser build is the primary target and the native CLI is the same code
//! reused. That rules out `std::fs`, `std::time`, threads and sockets here:
//! I/O belongs to the callers (`pochoir-cli`, the wasm bindings).
//!
//! # What is here today
//!
//! The **write path**: an image goes in, a PDF comes out.
//!
//! ```text
//! bytes -> image::decode -> ImagePage (a PageSource) -> jpeg::encode -> pdf::PdfWriter
//!            |                  |                          |                |
//!         zune-jpeg / png    EXIF orientation,          ours, baseline    ours, /DCTDecode
//!         (the only two       no resampling             4:4:4             verbatim
//!          dependencies)
//! ```
//!
//! There is **no watermark yet**: the mark of
//! `docs/adr/0002-watermark-algorithm-and-quality-modes.md` composites onto the
//! [`Pixmap`] that [`PageSource::rasterise`] produces, between those last two
//! stages, and lands separately so that this path stays reviewable on its own.
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

pub mod fingerprint;
pub mod jpeg;
pub mod page;
pub mod pixmap;

#[cfg(test)]
pub(crate) mod test_support;

pub use page::{PageGeometry, PageSource, RasterError};
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
/// These are the two knobs of ADR-0002's `standard` mode. The remaining
/// modes — `compact`, `high`, `lossless` — are combinations of them plus a PNG
/// encoder, and arrive with the watermark.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputOptions {
    /// JPEG quality, `1..=100`.
    pub quality: u8,
    /// Output resolution in dots per inch.
    pub dpi: f32,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            quality: DEFAULT_QUALITY,
            dpi: DEFAULT_DPI,
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
        Ok(())
    }
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
    use super::{DEFAULT_DPI, DEFAULT_QUALITY, Error, OutputOptions, version};

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
}
