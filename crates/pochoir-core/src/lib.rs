//! Core engine for `pochoir`: local-only document watermarking.
//!
//! Everything in this crate must compile for `wasm32-unknown-unknown`, because
//! the browser build is the primary target and the native CLI is the same code
//! reused. That rules out `std::fs`, `std::time`, threads and sockets here:
//! I/O belongs to the callers (`pochoir-cli`, the wasm bindings).
//!
//! This crate has zero third-party dependencies. Keep it that way.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use core::fmt;

/// Anything that can go wrong inside the core engine.
///
/// Hand-rolled on purpose: no `thiserror`, no `anyhow`. The variants carry only
/// `Copy` data so the type stays cheap to pass across the wasm boundary.
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
        }
    }
}

impl core::error::Error for Error {}

/// Shorthand for a result carrying this crate's [`Error`].
pub type Result<T> = core::result::Result<T, Error>;

/// Version of the core engine, as declared in its `Cargo.toml`.
///
/// The CLI and the wasm bindings both report this so that a user can tell
/// exactly which engine produced a given output file.
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
    use super::{Error, version};

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
    }

    #[test]
    fn error_is_a_std_error() {
        fn takes_std_error(_: &dyn core::error::Error) {}
        takes_std_error(&Error::UnsupportedFormat);
    }
}
