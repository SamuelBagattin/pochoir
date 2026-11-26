//! The status codes the ABI's result header carries, the detail payload that
//! travels with a failure, and the one place [`pochoir_core::Error`] is mapped
//! onto them.

use pochoir_core::{Error, mark};

/// What a render attempt came to.
///
/// The numbers are the wire contract and are matched by the browser client;
/// they are not an implementation detail and must not be renumbered. Anything
/// that is not one of these seven is [`Status::Internal`].
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
    /// The mark text will not fit the page at any permitted font size.
    ///
    /// ADR-0007 Decision 2's refusal: wrap, then shrink, then refuse, never
    /// clip. Separate from [`Self::BadArgument`] because the user's remedy is
    /// different — shorten the text — and because telling someone their
    /// parameters are wrong when their sentence is merely too long sends them
    /// to the wrong fix.
    TextDoesNotFit = 6,
    /// The mark text contains characters the bundled font subset cannot draw.
    ///
    /// Carries every offending character in the detail payload. ADR-0007
    /// Decision 6 asks the engine to name them all rather than the first
    /// precisely so a UI can say which; until this status existed the ABI had
    /// no way to carry the answer.
    UnsupportedCharacter = 7,
}

impl Status {
    /// The wire value.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// Longest detail payload this ABI will emit, in bytes.
///
/// The browser's reader caps its own read at 4096 bytes and yields no detail
/// past that, so exceeding it would silently lose the whole payload rather than
/// truncate it. Nothing here comes close: the longest possible payload is the
/// unsupported characters of a mark, and the mark is capped at 200 characters,
/// so at most 800 bytes.
pub const MAX_DETAIL_BYTES: usize = 4096;

/// A failed render: a machine-readable status, and data for the caller to
/// interpolate into its own wording.
///
/// # The detail payload is data, not a sentence
///
/// This product's interface is French. An English sentence produced here would
/// be a string the browser can neither show nor translate, so the payload
/// carries **the fact and nothing else**, and the caller builds its own
/// wording around it:
///
/// | status | payload |
/// | --- | --- |
/// | `UNSUPPORTED_CHARACTER` | every offending character, concatenated, in order of first appearance, no duplicates, no separators and no quoting |
/// | `BAD_ARGUMENT` | the bare parameter name, a stable ASCII identifier |
/// | anything else | **absent** — `ptr` and `len` are both zero |
///
/// An absent payload is the honest answer where the only fact is the status
/// itself: for `TEXT_DOES_NOT_FIT` there is nothing to name beyond "it does not
/// fit", and inventing a number for the UI to show would be inventing a
/// sentence by other means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The machine-readable kind.
    pub status: Status,
    /// The payload, or empty for none.
    pub detail: String,
}

impl Failure {
    /// A failure with no payload.
    #[must_use]
    pub fn bare(status: Status) -> Self {
        Self {
            status,
            detail: String::new(),
        }
    }

    /// A failure naming one thing — a parameter, or a set of characters.
    ///
    /// Truncated to [`MAX_DETAIL_BYTES`] on a character boundary, so the
    /// payload is always valid UTF-8 and always within what the caller will
    /// read.
    #[must_use]
    pub fn naming(status: Status, detail: &str) -> Self {
        let mut end = detail.len().min(MAX_DETAIL_BYTES);
        while end > 0 && !detail.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            status,
            detail: detail.get(..end).unwrap_or_default().to_owned(),
        }
    }

    /// Map an engine error onto a status and a payload.
    ///
    /// `text` is the mark text the call was made with, needed because
    /// [`Error::UnsupportedCharacter`] carries only the *first* offending
    /// character — every variant of [`Error`] holds `Copy` data by design — and
    /// the caller wants all of them. [`mark::unsupported_characters`] returns
    /// the full list, in order and without duplicates, which is exactly the
    /// payload.
    ///
    /// The grouping is by *what the caller should do*, not by where in the
    /// engine the error arose:
    ///
    /// - **`BadArgument`** is a value the user can correct, and names it.
    /// - **`TextDoesNotFit`** and **`UnsupportedCharacter`** are ADR-0007's two
    ///   named refusals, split out because each has its own remedy.
    /// - **`DecodeFailed`** is a file that claims a format it does not honour,
    ///   including running out of bytes part way through.
    /// - **`OutOfMemory`** is [`Error::ImageTooLarge`], the engine refusing to
    ///   allocate rather than failing to: the only memory condition that can be
    ///   reported instead of aborting.
    /// - **`Internal`** is for conditions this crate's own code should have
    ///   made impossible, plus the catch-all that [`Error`]'s
    ///   `#[non_exhaustive]` requires — so a variant added upstream surfaces as
    ///   a bug to report rather than as a plausible-looking lie about the
    ///   user's input.
    #[must_use]
    pub fn for_error(error: &Error, text: &str) -> Self {
        match *error {
            Error::UnsupportedFormat => Self::bare(Status::UnsupportedFormat),
            Error::TruncatedInput { .. } | Error::Decode { .. } => Self::bare(Status::DecodeFailed),
            Error::InvalidParameter { name } => Self::naming(Status::BadArgument, name),
            Error::MarkDoesNotFit { .. } => Self::bare(Status::TextDoesNotFit),
            Error::UnsupportedCharacter { character } => {
                // The full list if the font can be read, and the one character
                // the error carried if it cannot. Never a panic, and never an
                // empty payload for this status: the caller's whole message
                // depends on naming at least one character.
                let all: String = mark::unsupported_characters(text)
                    .map(|chars| chars.into_iter().collect())
                    .unwrap_or_default();
                let detail = if all.is_empty() {
                    character.to_string()
                } else {
                    all
                };
                Self::naming(Status::UnsupportedCharacter, &detail)
            }
            Error::ImageTooLarge { .. } => Self::bare(Status::OutOfMemory),
            Error::BufferSize { .. } | Error::PageOutOfRange { .. } => Self::bare(Status::Internal),
            _ => Self::bare(Status::Internal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Failure, MAX_DETAIL_BYTES, Status};
    use pochoir_core::Error;

    #[test]
    fn the_codes_are_the_wire_contract() {
        assert_eq!(Status::Ok.code(), 0);
        assert_eq!(Status::BadArgument.code(), 1);
        assert_eq!(Status::DecodeFailed.code(), 2);
        assert_eq!(Status::UnsupportedFormat.code(), 3);
        assert_eq!(Status::OutOfMemory.code(), 4);
        assert_eq!(Status::Internal.code(), 5);
        assert_eq!(Status::TextDoesNotFit.code(), 6);
        assert_eq!(Status::UnsupportedCharacter.code(), 7);
    }

    #[test]
    fn every_error_variant_maps_the_way_the_table_says() {
        let cases = [
            (Error::UnsupportedFormat, Status::UnsupportedFormat),
            (Error::TruncatedInput { offset: 9 }, Status::DecodeFailed),
            (
                Error::Decode {
                    format: "JPEG",
                    reason: "bad",
                },
                Status::DecodeFailed,
            ),
            (
                Error::InvalidParameter { name: "opacity" },
                Status::BadArgument,
            ),
            (
                Error::MarkDoesNotFit { characters: 200 },
                Status::TextDoesNotFit,
            ),
            (
                Error::ImageTooLarge {
                    width: 99_999,
                    height: 99_999,
                },
                Status::OutOfMemory,
            ),
            (
                Error::BufferSize {
                    expected: 4,
                    got: 3,
                },
                Status::Internal,
            ),
            (
                Error::PageOutOfRange { index: 1, count: 1 },
                Status::Internal,
            ),
        ];
        for (error, want) in cases {
            assert_eq!(
                Failure::for_error(&error, "X").status,
                want,
                "{error:?} must map to {want:?}"
            );
        }
    }

    /// The payload is data the UI interpolates, never prose. If an English
    /// sentence ever appears here, the browser has a string it cannot show a
    /// French user and cannot translate.
    #[test]
    fn a_parameter_failure_carries_the_bare_name_and_nothing_else() {
        let failure = Failure::for_error(&Error::InvalidParameter { name: "mark blur" }, "X");
        assert_eq!(failure.status, Status::BadArgument);
        assert_eq!(failure.detail, "mark blur");
        assert!(
            failure.detail.is_ascii(),
            "the name must be a stable ASCII identifier"
        );
    }

    /// ADR-0007 Decision 6 asks for every unsupported character, not the first.
    /// The error variant can only carry one, so the payload is rebuilt from the
    /// text.
    #[test]
    fn an_unsupported_character_failure_carries_all_of_them_concatenated() {
        let text = "Pour \u{4e2d}\u{6587} et \u{65e5} encore \u{4e2d}";
        let failure = Failure::for_error(
            &Error::UnsupportedCharacter {
                character: '\u{4e2d}',
            },
            text,
        );
        assert_eq!(failure.status, Status::UnsupportedCharacter);
        assert_eq!(
            failure.detail, "\u{4e2d}\u{6587}\u{65e5}",
            "in order of first appearance, no duplicates, no separators, no quoting"
        );
    }

    /// Where the only fact is the status, there is no payload: `ptr` and `len`
    /// stay zero and the browser treats that as absent.
    #[test]
    fn the_refusals_with_nothing_to_name_carry_no_payload() {
        for error in [
            Error::MarkDoesNotFit { characters: 200 },
            Error::UnsupportedFormat,
            Error::TruncatedInput { offset: 3 },
            Error::ImageTooLarge {
                width: 9,
                height: 9,
            },
        ] {
            assert!(
                Failure::for_error(&error, "X").detail.is_empty(),
                "{error:?} has nothing useful to name"
            );
        }
    }

    #[test]
    fn a_payload_is_capped_on_a_character_boundary() {
        let long = "\u{4e2d}".repeat(4000);
        let failure = Failure::naming(Status::UnsupportedCharacter, &long);
        assert!(failure.detail.len() <= MAX_DETAIL_BYTES);
        // Valid UTF-8 by construction: `String` cannot hold anything else, and
        // the truncation walks back to a boundary.
        assert_eq!(failure.detail.len() % 3, 0, "cut mid-character");
    }

    /// The two refusals ADR-0007 names must not collapse back into one bucket:
    /// a missing glyph and a sentence that is too long have different remedies.
    #[test]
    fn the_two_named_refusals_stay_distinct_from_each_other_and_from_bad_argument() {
        let fit = Failure::for_error(&Error::MarkDoesNotFit { characters: 200 }, "X").status;
        let glyph = Failure::for_error(
            &Error::UnsupportedCharacter {
                character: '\u{4e2d}',
            },
            "\u{4e2d}",
        )
        .status;
        let argument = Failure::for_error(&Error::InvalidParameter { name: "dpi" }, "X").status;
        assert_ne!(fit, glyph);
        assert_ne!(fit, argument);
        assert_ne!(glyph, argument);
    }
}
