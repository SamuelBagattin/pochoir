//! The status codes the ABI's result header carries, and the one place
//! [`pochoir_core::Error`] is mapped onto them.

use pochoir_core::Error;

/// What a render attempt came to.
///
/// The numbers are the wire contract and are matched by the browser client;
/// they are not an implementation detail and must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
    /// The result carries bytes.
    Ok = 0,
    /// A parameter was out of range, malformed, or asked for something this
    /// engine will not approximate. The user can fix it.
    BadArgument = 1,
    /// The format was recognised and the decoder refused the bytes.
    DecodeFailed = 2,
    /// The bytes are not a format this engine reads.
    UnsupportedFormat = 3,
    /// The image is larger than this build will allocate for.
    OutOfMemory = 4,
    /// A bug here.
    Internal = 5,
}

impl Status {
    /// The wire value.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// Map an engine error onto a wire status.
///
/// Deliberately one function and deliberately exhaustive-looking, so the whole
/// policy can be read at a glance. The grouping is by *what the caller should
/// do*, not by where in the engine the error came from:
///
/// - **`BadArgument`** is everything the user can correct by changing an input:
///   a parameter out of range, a mark the font cannot draw, a mark that will not
///   fit the page at any permitted size.
/// - **`DecodeFailed`** is a file that claims a format it does not honour, which
///   includes running out of bytes part way through.
/// - **`OutOfMemory`** is [`Error::ImageTooLarge`], which is the engine refusing
///   to allocate rather than failing to: it is the only memory condition that
///   can be reported instead of aborting.
/// - **`Internal`** is reserved for conditions this crate's own code should have
///   made impossible — a wrong buffer size, a page index we never pass — plus
///   the catch-all that [`Error`]'s `#[non_exhaustive]` requires, so a variant
///   added upstream surfaces as a bug here rather than as a plausible-looking
///   lie about the user's input.
#[must_use]
pub fn status_for(error: &Error) -> Status {
    match *error {
        Error::UnsupportedFormat => Status::UnsupportedFormat,
        Error::TruncatedInput { .. } | Error::Decode { .. } => Status::DecodeFailed,
        Error::InvalidParameter { .. }
        | Error::UnsupportedCharacter { .. }
        | Error::MarkDoesNotFit { .. } => Status::BadArgument,
        Error::ImageTooLarge { .. } => Status::OutOfMemory,
        Error::BufferSize { .. } | Error::PageOutOfRange { .. } => Status::Internal,
        _ => Status::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::{Status, status_for};
    use pochoir_core::Error;

    #[test]
    fn the_codes_are_the_wire_contract() {
        assert_eq!(Status::Ok.code(), 0);
        assert_eq!(Status::BadArgument.code(), 1);
        assert_eq!(Status::DecodeFailed.code(), 2);
        assert_eq!(Status::UnsupportedFormat.code(), 3);
        assert_eq!(Status::OutOfMemory.code(), 4);
        assert_eq!(Status::Internal.code(), 5);
    }

    #[test]
    fn every_error_variant_maps_the_way_the_table_says() {
        assert_eq!(
            status_for(&Error::UnsupportedFormat),
            Status::UnsupportedFormat
        );
        assert_eq!(
            status_for(&Error::TruncatedInput { offset: 9 }),
            Status::DecodeFailed
        );
        assert_eq!(
            status_for(&Error::Decode {
                format: "JPEG",
                reason: "bad"
            }),
            Status::DecodeFailed
        );
        assert_eq!(
            status_for(&Error::InvalidParameter { name: "opacity" }),
            Status::BadArgument
        );
        assert_eq!(
            status_for(&Error::UnsupportedCharacter {
                character: '\u{4e2d}'
            }),
            Status::BadArgument
        );
        assert_eq!(
            status_for(&Error::MarkDoesNotFit { characters: 200 }),
            Status::BadArgument
        );
        assert_eq!(
            status_for(&Error::ImageTooLarge {
                width: 99_999,
                height: 99_999
            }),
            Status::OutOfMemory
        );
        assert_eq!(
            status_for(&Error::BufferSize {
                expected: 4,
                got: 3
            }),
            Status::Internal
        );
        assert_eq!(
            status_for(&Error::PageOutOfRange { index: 1, count: 1 }),
            Status::Internal
        );
    }

    /// A decode failure must never surface as `BAD_ARGUMENT`: the UI words the
    /// two differently, and telling someone their parameters are wrong when
    /// their file is corrupt sends them to the wrong fix.
    #[test]
    fn decoding_and_argument_failures_stay_distinct() {
        assert_ne!(
            status_for(&Error::Decode {
                format: "PNG",
                reason: "truncated"
            }),
            status_for(&Error::InvalidParameter { name: "dpi" })
        );
    }
}
