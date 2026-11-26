//! Reading back a JPEG datastream's own frame header.
//!
//! The PDF writer needs four facts about the JPEG it is about to embed —
//! dimensions, component count, sample precision and whether an Adobe `APP14`
//! marker is present — and ADR-0001 names getting two of them wrong as a trap
//! to handle up front: the image XObject's `/ColorSpace` and
//! `/BitsPerComponent` must agree with the JPEG's own `SOF` marker, and a CMYK
//! JPEG carrying `APP14` needs an inverting `/Decode` array.
//!
//! So the writer does not take the caller's word for any of it. It parses the
//! bytes it is given. That is the only way the agreement can be guaranteed for
//! a datastream the writer did not produce — which is exactly the case
//! ADR-0002 anticipates for a future passthrough path.

use crate::{Error, Result};

/// The coding process a frame header announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Process {
    /// `SOF0` — baseline sequential, Huffman coded. What we write.
    Baseline,
    /// `SOF2` — progressive, Huffman coded. Legal in a PDF since 1.3.
    Progressive,
}

/// What a JPEG's frame header says about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    /// Samples per line.
    pub width: u16,
    /// Number of lines.
    pub height: u16,
    /// Components per sample: 1, 3 or 4 for anything we will embed.
    pub components: u8,
    /// Bits per sample. `/DCTDecode` delivers 8-bit samples, so anything else
    /// is refused.
    pub precision: u8,
    /// Which coding process the frame uses.
    pub process: Process,
    /// The transform byte of an Adobe `APP14` marker, if the datastream has one.
    pub adobe_transform: Option<u8>,
}

impl Info {
    /// `true` when this is the Adobe-marked CMYK case that renders inverted
    /// unless the image dictionary carries `/Decode [1 0 1 0 1 0 1 0]`.
    #[must_use]
    pub const fn needs_inverted_decode(&self) -> bool {
        self.components == 4 && self.adobe_transform.is_some()
    }
}

/// Parse the frame header of a JPEG datastream.
///
/// Walks marker segments from `SOI` and stops at `SOS`, which is where entropy
/// coded data begins and where a byte pair that looks like a marker is just
/// data.
///
/// # Errors
///
/// - [`Error::UnsupportedFormat`] if the bytes are not a JPEG, or use a coding
///   process `/DCTDecode` does not accept (arithmetic coding, lossless or
///   hierarchical modes), or a sample precision other than 8.
/// - [`Error::TruncatedInput`] if a segment runs past the end of the input.
pub fn parse(jpeg: &[u8]) -> Result<Info> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return Err(Error::UnsupportedFormat);
    }
    let mut at = 2usize;
    let mut adobe_transform = None;
    loop {
        while jpeg.get(at) == Some(&0xFF) && jpeg.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        let (Some(&0xFF), Some(&marker)) = (jpeg.get(at), jpeg.get(at + 1)) else {
            return Err(Error::TruncatedInput { offset: at });
        };
        at += 2;
        match marker {
            // Standalone markers: no length, no payload.
            0x01 | 0xD0..=0xD7 => continue,
            // Reached the scan without ever seeing a frame header.
            0xDA | 0xD9 => return Err(Error::UnsupportedFormat),
            _ => {}
        }
        let Some(length_bytes) = jpeg.get(at..at + 2) else {
            return Err(Error::TruncatedInput { offset: at });
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            return Err(Error::UnsupportedFormat);
        }
        let Some(payload) = jpeg.get(at + 2..at + length) else {
            return Err(Error::TruncatedInput { offset: at });
        };

        match marker {
            // The Adobe APP14 marker: "Adobe", version, two flag words, then
            // the colour transform byte.
            0xEE if payload.starts_with(b"Adobe") => {
                adobe_transform = payload.get(11).copied();
            }
            // The frame headers `/DCTDecode` accepts.
            0xC0 | 0xC2 => {
                let process = if marker == 0xC0 {
                    Process::Baseline
                } else {
                    Process::Progressive
                };
                // precision(1) height(2) width(2) components(1)
                if payload.len() < 6 {
                    return Err(Error::TruncatedInput { offset: at });
                }
                let precision = payload[0];
                if precision != 8 {
                    // ISO 32000-1 Table 89: "a RunLengthDecode or DCTDecode
                    // filter shall always deliver 8-bit samples".
                    return Err(Error::UnsupportedFormat);
                }
                let height = u16::from_be_bytes([payload[1], payload[2]]);
                let width = u16::from_be_bytes([payload[3], payload[4]]);
                let components = payload[5];
                if width == 0 || height == 0 {
                    return Err(Error::UnsupportedFormat);
                }
                return Ok(Info {
                    width,
                    height,
                    components,
                    precision,
                    process,
                    adobe_transform,
                });
            }
            // Every other SOF: extended sequential, lossless, differential and
            // the arithmetic-coded variants. ISO 32000-1 7.4.8 admits the
            // baseline format and, since PDF 1.3, the progressive extension.
            // Nothing else, so we refuse rather than write a file a conforming
            // reader may reject.
            0xC1 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::UnsupportedFormat);
            }
            _ => {}
        }
        at += length;
    }
}

#[cfg(test)]
mod tests {
    use super::{Process, parse};
    use crate::Error;

    /// Assemble a minimal JPEG whose frame header says what we want to test.
    /// No scan data: the parser stops at `SOS` and never looks past it.
    fn framed(marker: u8, precision: u8, width: u16, height: u16, components: u8) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        let mut payload = vec![precision];
        payload.extend_from_slice(&height.to_be_bytes());
        payload.extend_from_slice(&width.to_be_bytes());
        payload.push(components);
        for id in 1..=components {
            payload.extend_from_slice(&[id, 0x11, 0]);
        }
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        out
    }

    fn with_adobe(mut jpeg: Vec<u8>, transform: u8) -> Vec<u8> {
        let mut payload = Vec::from(&b"Adobe"[..]);
        payload.extend_from_slice(&[0x00, 0x64, 0x00, 0x00, 0x00, 0x00, transform]);
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xEE];
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(jpeg.drain(2..).as_slice());
        out
    }

    #[test]
    fn a_baseline_frame_header_is_read() {
        let info = parse(&framed(0xC0, 8, 640, 480, 3)).expect("valid baseline frame");
        assert_eq!(info.width, 640);
        assert_eq!(info.height, 480);
        assert_eq!(info.components, 3);
        assert_eq!(info.precision, 8);
        assert_eq!(info.process, Process::Baseline);
        assert_eq!(info.adobe_transform, None);
        assert!(!info.needs_inverted_decode());
    }

    #[test]
    fn a_progressive_frame_header_is_accepted() {
        let info = parse(&framed(0xC2, 8, 10, 20, 3)).expect("progressive is legal since PDF 1.3");
        assert_eq!(info.process, Process::Progressive);
    }

    #[test]
    fn the_cmyk_plus_adobe_case_is_detected() {
        let info = parse(&with_adobe(framed(0xC0, 8, 8, 8, 4), 2)).expect("valid");
        assert_eq!(info.components, 4);
        assert_eq!(info.adobe_transform, Some(2));
        assert!(
            info.needs_inverted_decode(),
            "a four-component Adobe JPEG needs /Decode [1 0 1 0 1 0 1 0]"
        );
        // Three components with the marker is YCCK/RGB territory, not inverted.
        let info = parse(&with_adobe(framed(0xC0, 8, 8, 8, 3), 1)).expect("valid");
        assert!(!info.needs_inverted_decode());
    }

    #[test]
    fn processes_dctdecode_does_not_admit_are_refused() {
        for marker in [
            0xC1u8, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
        ] {
            assert_eq!(
                parse(&framed(marker, 8, 8, 8, 3)),
                Err(Error::UnsupportedFormat),
                "marker {marker:#04x} should be refused"
            );
        }
    }

    #[test]
    fn twelve_bit_samples_are_refused() {
        assert_eq!(
            parse(&framed(0xC0, 12, 8, 8, 3)),
            Err(Error::UnsupportedFormat)
        );
    }

    #[test]
    fn rubbish_is_refused_without_panicking() {
        assert_eq!(parse(&[]), Err(Error::UnsupportedFormat));
        assert_eq!(parse(b"not a jpeg"), Err(Error::UnsupportedFormat));
        // SOI then nothing.
        assert!(parse(&[0xFF, 0xD8, 0xFF, 0xC0]).is_err());
        // A length that overruns the buffer.
        assert!(parse(&[0xFF, 0xD8, 0xFF, 0xC0, 0xFF, 0xFF, 0x08]).is_err());
        // A zero-length segment.
        assert_eq!(
            parse(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x00]),
            Err(Error::UnsupportedFormat)
        );
        // A frame with zero dimensions.
        assert_eq!(
            parse(&framed(0xC0, 8, 0, 8, 3)),
            Err(Error::UnsupportedFormat)
        );
    }

    #[test]
    fn a_scan_with_no_frame_header_is_refused() {
        assert_eq!(
            parse(&[0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02]),
            Err(Error::UnsupportedFormat)
        );
    }
}
