//! A PDF writer, hand-rolled, covering exactly the subset we emit.
//!
//! Catalog, pages tree, one image XObject per page, a content stream that
//! paints it over the whole page, a cross-reference table, a trailer. That is
//! the whole of it. ADR-0001 calls this the closest call in its decision table
//! — `pdf-writer` measures 15.7 KiB brotli and five crates, which is cheap —
//! and sends it to hand-roll anyway for one reason: the output PDF is the
//! artefact we most need total control over, and ADR-0002's metadata hygiene is
//! easier to guarantee in code we wrote than to police through someone else's
//! API.
//!
//! # Why the image bytes are never recompressed
//!
//! The JPEG datastream is embedded verbatim under `/DCTDecode`. ISO 32000-1
//! §7.4.8 is explicit that the parameters controlling the encoding, "which
//! include the dimensions of the image and the number of components per
//! sample, are entirely under the control of the encoder and shall be stored in
//! the encoded data", and that "DCTDecode may obtain the parameter values it
//! requires directly from the encoded data". So the stream *is* the JPEG, and a
//! conforming reader decodes it with the encoder's own quantisation tables.
//! That is what makes ADR-0002's quality modes mean anything: the bytes a
//! viewer decodes are the bytes our encoder produced, with no second
//! generation of loss.
//!
//! Two consequences of the same section bound what may be embedded. §7.4.8
//! admits "the JPEG baseline format" and, since PDF 1.3, "the progressive JPEG
//! extension" — and nothing else, so [`crate::jpeg::parse_info`] refuses
//! arithmetic-coded, lossless and hierarchical datastreams. And Table 89 in
//! §8.9.5.1 states that "a RunLengthDecode or DCTDecode filter shall always
//! deliver 8-bit samples", which is why `/BitsPerComponent` is 8 and a
//! 12-bit-precision frame is refused rather than mislabelled.
//!
//! # Metadata
//!
//! Per ADR-0002: `/Producer` and `/Creator` are the constant string `Pochoir`
//! with no version number, there is no `/CreationDate` or `/ModDate`, no
//! `/Author`, `/Title`, `/Subject` or `/Keywords`, no XMP packet, and the
//! trailer's `/ID` is derived from the bytes of the file rather than from a
//! clock or a random number. The result is a file that says nothing about the
//! user, the machine or the build — and that is byte-identical across runs.

use crate::fingerprint::{fingerprint, to_hex};
use crate::jpeg::{self, Info};
use crate::page::PageGeometry;
use crate::{Error, Result};

/// The PDF version the output declares.
///
/// `/DCTDecode` has existed since PDF 1.0 and the progressive extension since
/// 1.3, so nothing we write needs 1.7; declaring it costs nothing and avoids a
/// reader applying 1.3-era restrictions we never relied on.
const VERSION: &str = "1.7";

/// Object number of the document catalog.
const CATALOG: u32 = 1;
/// Object number of the root of the pages tree.
const PAGES: u32 = 2;
/// Object number of the document information dictionary.
const INFO: u32 = 3;
/// First object number available to page content.
const FIRST_PAGE_OBJECT: u32 = 4;

/// The name every page gives its single image XObject.
const IMAGE_NAME: &str = "Im0";

/// Builds a PDF one page at a time.
///
/// Page objects are written as they arrive, so an image's bytes are copied into
/// the output once and the encoded page is never held twice. Only the three
/// small dictionaries that need to know the whole document — catalog, pages
/// tree, information — are deferred to [`finish`], and their object numbers are
/// reserved up front so that nothing has to be patched afterwards.
///
/// [`finish`]: PdfWriter::finish
pub struct PdfWriter {
    out: Vec<u8>,
    /// Byte offset of each object, indexed by object number. Slot 0 is the free
    /// head and is never written.
    offsets: Vec<Option<usize>>,
    /// Object number of each page, in page order.
    pages: Vec<u32>,
}

impl Default for PdfWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfWriter {
    /// Start a new document.
    #[must_use]
    pub fn new() -> Self {
        let mut out = Vec::with_capacity(4096);
        out.extend_from_slice(format!("%PDF-{VERSION}\n").as_bytes());
        // A comment of four bytes above 127, which tells every tool that has
        // ever guessed at a file's type that this one is binary.
        out.extend_from_slice(b"%\xE2\xE3\xCF\xD3\n");
        Self {
            out,
            // Slots for the free-list head and the three reserved objects.
            offsets: vec![None; FIRST_PAGE_OBJECT as usize],
            pages: Vec::new(),
        }
    }

    /// How many pages have been written so far.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Append one page whose entire content is `jpeg`, scaled to fill it.
    ///
    /// The image dictionary is derived from the datastream's own frame header,
    /// never from the caller, which is what makes `/ColorSpace`,
    /// `/BitsPerComponent` and `/Decode` agree with it by construction.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidParameter`] if `geometry` is not a finite, positive
    ///   page box.
    /// - [`Error::UnsupportedFormat`] if the datastream is not a baseline or
    ///   progressive 8-bit JPEG, or has a component count with no PDF colour
    ///   space.
    pub fn add_jpeg_page(&mut self, geometry: PageGeometry, jpeg: &[u8]) -> Result<()> {
        if !geometry.is_valid() {
            return Err(Error::InvalidParameter { name: "geometry" });
        }
        let info = jpeg::parse_info(jpeg)?;
        let colour_space = colour_space(&info)?;

        let content = page_content(geometry);
        let content_object = self.next_object();
        self.write_stream(content_object, "", content.as_bytes());

        let image_object = self.next_object();
        let mut image_dict = format!(
            "/Type /XObject /Subtype /Image /Width {} /Height {} \
             /ColorSpace /{colour_space} /BitsPerComponent 8 /Filter /DCTDecode",
            info.width, info.height
        );
        if info.needs_inverted_decode() {
            // A four-component JPEG carrying Adobe's APP14 marker stores
            // inverted CMYK, which renders as a photographic negative unless
            // the image dictionary undoes it. ADR-0001 names this as a trap to
            // handle up front rather than discover.
            image_dict.push_str(" /Decode [1 0 1 0 1 0 1 0]");
        }
        self.write_stream(image_object, &image_dict, jpeg);

        let page_object = self.next_object();
        self.write_object(
            page_object,
            &format!(
                "<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 {} {}] \
                 /Resources << /XObject << /{IMAGE_NAME} {image_object} 0 R >> >> \
                 /Contents {content_object} 0 R >>",
                number(geometry.width_pt),
                number(geometry.height_pt)
            ),
        );
        self.pages.push(page_object);
        Ok(())
    }

    /// Finish the document and return its bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedFormat`] if no page was ever added: a PDF
    /// with an empty pages tree is not a valid document.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        if self.pages.is_empty() {
            return Err(Error::UnsupportedFormat);
        }

        let kids: Vec<String> = self.pages.iter().map(|id| format!("{id} 0 R")).collect();
        let pages_dict = format!(
            "<< /Type /Pages /Count {} /Kids [{}] >>",
            self.pages.len(),
            kids.join(" ")
        );
        self.write_object(PAGES, &pages_dict);
        self.write_object(CATALOG, &format!("<< /Type /Catalog /Pages {PAGES} 0 R >>"));
        // ADR-0002's table in full: two constant strings and nothing else. No
        // dates, no version, no XMP stream anywhere in the file.
        self.write_object(INFO, "<< /Producer (Pochoir) /Creator (Pochoir) >>");

        // The identifier is a function of every byte written so far, so it is
        // reproducible without being a timestamp.
        let id = to_hex(&fingerprint(&self.out));

        let xref_offset = self.out.len();
        let size = self.offsets.len();
        self.out
            .extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
        // Every entry is exactly twenty bytes: ten digits of offset, a space,
        // five digits of generation, a space, the keyword, then a two-character
        // end-of-line.
        self.out.extend_from_slice(b"0000000000 65535 f \n");
        for slot in 1..size {
            let offset = self.offsets[slot].unwrap_or(0);
            self.out
                .extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        self.out.extend_from_slice(
            format!(
                "trailer\n<< /Size {size} /Root {CATALOG} 0 R /Info {INFO} 0 R \
                 /ID [<{id}> <{id}>] >>\nstartxref\n{xref_offset}\n%%EOF\n"
            )
            .as_bytes(),
        );
        Ok(self.out)
    }

    /// Reserve the next object number.
    fn next_object(&mut self) -> u32 {
        self.offsets.push(None);
        (self.offsets.len() - 1) as u32
    }

    /// Write a complete indirect object whose body is `body`.
    fn write_object(&mut self, id: u32, body: &str) {
        self.record(id);
        self.out
            .extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
    }

    /// Write an indirect object holding a stream.
    ///
    /// `extra` is spliced into the stream dictionary before `/Length`, which is
    /// always exact: we know the byte count before we write it, so no reader
    /// ever has to scan for `endstream`.
    fn write_stream(&mut self, id: u32, extra: &str, data: &[u8]) {
        self.record(id);
        let separator = if extra.is_empty() { "" } else { " " };
        self.out.extend_from_slice(
            format!(
                "{id} 0 obj\n<<{separator}{extra}{separator}/Length {} >>\nstream\n",
                data.len()
            )
            .as_bytes(),
        );
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    fn record(&mut self, id: u32) {
        let offset = self.out.len();
        if let Some(slot) = self.offsets.get_mut(id as usize) {
            *slot = Some(offset);
        }
    }
}

/// The content stream that paints the image over the whole page.
///
/// `cm` scales the unit square the image occupies up to the page box, so the
/// image fills it exactly; the save/restore pair around it keeps the graphics
/// state clean for anything appended later — which is what the watermark's
/// overlay mode would do.
fn page_content(geometry: PageGeometry) -> String {
    format!(
        "q\n{} 0 0 {} 0 0 cm\n/{IMAGE_NAME} Do\nQ\n",
        number(geometry.width_pt),
        number(geometry.height_pt)
    )
}

/// The PDF colour space for a JPEG's component count.
fn colour_space(info: &Info) -> Result<&'static str> {
    match info.components {
        1 => Ok("DeviceGray"),
        3 => Ok("DeviceRGB"),
        4 => Ok("DeviceCMYK"),
        // ISO 32000-1 7.4.8 allows a two-component datastream, but PDF has no
        // two-channel device colour space to pair it with, so there is nothing
        // honest to write in the dictionary.
        _ => Err(Error::UnsupportedFormat),
    }
}

/// Format a number for a PDF file.
///
/// Fixed to four decimal places and then trimmed, which keeps the output
/// compact without ever falling back to exponential notation — PDF has no
/// exponential number syntax. Rust's float formatting is implemented in the
/// core library rather than by the platform, so this is identical on wasm32 and
/// native, as ADR-0002 requires.
fn number(value: f32) -> String {
    let mut text = format!("{value:.4}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if text == "-0" { "0".to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::{PdfWriter, number};
    use crate::page::PageGeometry;
    use crate::{Error, test_support};

    fn a4() -> PageGeometry {
        PageGeometry {
            width_pt: 595.276,
            height_pt: 841.89,
        }
    }

    fn one_page_pdf() -> Vec<u8> {
        let jpeg =
            crate::jpeg::encode(&test_support::gradient(32, 48), 85, 200.0).expect("encodes");
        let mut writer = PdfWriter::new();
        writer.add_jpeg_page(a4(), &jpeg).expect("adds a page");
        writer.finish().expect("finishes")
    }

    #[test]
    fn the_file_has_a_header_a_trailer_and_a_startxref() {
        let pdf = one_page_pdf();
        assert!(pdf.starts_with(b"%PDF-1.7\n%"), "missing header");
        assert!(pdf.ends_with(b"%%EOF\n"), "missing end-of-file marker");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("\nxref\n"));
        assert!(text.contains("\ntrailer\n"));
        assert!(text.contains("\nstartxref\n"));
    }

    #[test]
    fn the_startxref_offset_points_at_the_xref_keyword() {
        let pdf = one_page_pdf();
        let text = String::from_utf8_lossy(&pdf);
        let marker = text.rfind("startxref\n").expect("there is a startxref");
        let offset: usize = text[marker + "startxref\n".len()..]
            .lines()
            .next()
            .expect("a number follows")
            .trim()
            .parse()
            .expect("the offset is a number");
        assert_eq!(&pdf[offset..offset + 4], b"xref");
    }

    #[test]
    fn every_xref_entry_is_twenty_bytes_and_points_at_its_object() {
        let pdf = one_page_pdf();
        let text = String::from_utf8_lossy(&pdf);
        let xref_at = text.rfind("\nxref\n").expect("there is an xref") + 1;
        let body = &text[xref_at..];
        let mut lines = body.lines();
        assert_eq!(lines.next(), Some("xref"));
        let header = lines.next().expect("subsection header");
        let size: usize = header
            .split_whitespace()
            .nth(1)
            .expect("count")
            .parse()
            .expect("number");
        // The free-list head, then one entry per object.
        let entries: Vec<&str> = lines.take(size).collect();
        assert_eq!(entries.len(), size);
        assert!(entries[0].starts_with("0000000000 65535 f"));
        for (id, entry) in entries.iter().enumerate().skip(1) {
            // 18 characters plus the single-space pad; the newline is the
            // second character of the two-byte end-of-line.
            assert_eq!(entry.len(), 19, "entry {id} is {entry:?}");
            let offset: usize = entry[..10].parse().expect("offset is numeric");
            let expected = format!("{id} 0 obj");
            assert_eq!(
                &pdf[offset..offset + expected.len()],
                expected.as_bytes(),
                "object {id} is not at offset {offset}"
            );
        }
    }

    #[test]
    fn the_image_dictionary_agrees_with_the_jpegs_frame_header() {
        let pdf = one_page_pdf();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Subtype /Image"));
        assert!(text.contains("/Width 32 /Height 48"));
        assert!(text.contains("/ColorSpace /DeviceRGB"));
        assert!(text.contains("/BitsPerComponent 8"));
        assert!(text.contains("/Filter /DCTDecode"));
        // No /Decode for a three-component image.
        assert!(!text.contains("/Decode"));
    }

    #[test]
    fn a_cmyk_jpeg_with_an_adobe_marker_gets_an_inverting_decode_array() {
        // Only the frame header is parsed, so a header-only datastream is
        // enough to exercise the dictionary the writer would emit for a real
        // CMYK scan. Producing one with our own encoder is impossible by
        // design: it only writes three-component YCbCr.
        let jpeg = test_support::cmyk_jpeg_header(16, 16, Some(2));
        let mut writer = PdfWriter::new();
        writer.add_jpeg_page(a4(), &jpeg).expect("adds a page");
        let pdf = writer.finish().expect("finishes");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/ColorSpace /DeviceCMYK"));
        assert!(
            text.contains("/Decode [1 0 1 0 1 0 1 0]"),
            "an Adobe-marked CMYK JPEG would render inverted without it"
        );
    }

    #[test]
    fn a_greyscale_jpeg_gets_devicegray() {
        let jpeg = test_support::grey_jpeg_header(8, 8);
        let mut writer = PdfWriter::new();
        writer.add_jpeg_page(a4(), &jpeg).expect("adds a page");
        let pdf = writer.finish().expect("finishes");
        assert!(String::from_utf8_lossy(&pdf).contains("/ColorSpace /DeviceGray"));
    }

    #[test]
    fn a_two_component_jpeg_has_no_colour_space_and_is_refused() {
        let jpeg = test_support::jpeg_header(0xC0, 8, 8, 8, 2, None);
        let mut writer = PdfWriter::new();
        assert_eq!(
            writer.add_jpeg_page(a4(), &jpeg),
            Err(Error::UnsupportedFormat)
        );
    }

    #[test]
    fn the_metadata_is_exactly_what_adr_0002_specifies() {
        let pdf = one_page_pdf();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Producer (Pochoir)"));
        assert!(text.contains("/Creator (Pochoir)"));
        for forbidden in [
            "/CreationDate",
            "/ModDate",
            "/Author",
            "/Title",
            "/Subject",
            "/Keywords",
            "/Metadata",
            "xpacket",
            "<x:xmpmeta",
        ] {
            assert!(!text.contains(forbidden), "output carries {forbidden}");
        }
        // No version number anywhere in the producer string.
        assert!(!text.contains("Pochoir 0."));
        assert!(!text.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn the_trailer_id_is_present_and_derived_from_the_content() {
        let jpeg_a =
            crate::jpeg::encode(&test_support::gradient(32, 48), 85, 200.0).expect("encodes");
        let jpeg_b =
            crate::jpeg::encode(&test_support::gradient(48, 32), 85, 200.0).expect("encodes");
        let id_of = |jpeg: &[u8]| {
            let mut writer = PdfWriter::new();
            writer.add_jpeg_page(a4(), jpeg).expect("adds");
            let pdf = writer.finish().expect("finishes");
            let text = String::from_utf8_lossy(&pdf);
            let at = text.rfind("/ID [<").expect("there is an /ID") + 6;
            text[at..at + 32].to_string()
        };
        let a = id_of(&jpeg_a);
        assert_eq!(a.len(), 32, "an /ID string is sixteen bytes");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(a, id_of(&jpeg_a), "the same input must give the same /ID");
        assert_ne!(a, id_of(&jpeg_b), "different content, different /ID");
    }

    #[test]
    fn the_whole_file_is_byte_identical_across_runs() {
        assert_eq!(one_page_pdf(), one_page_pdf());
    }

    #[test]
    fn several_pages_all_appear_in_the_pages_tree() {
        let mut writer = PdfWriter::new();
        for size in [16u32, 24, 32] {
            let jpeg = crate::jpeg::encode(&test_support::gradient(size, size), 85, 200.0)
                .expect("encodes");
            writer.add_jpeg_page(a4(), &jpeg).expect("adds a page");
        }
        assert_eq!(writer.page_count(), 3);
        let pdf = writer.finish().expect("finishes");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Type /Pages /Count 3 /Kids ["));
        assert_eq!(text.matches("/Type /Page ").count(), 3);
        assert_eq!(text.matches("/Filter /DCTDecode").count(), 3);
    }

    #[test]
    fn a_document_with_no_pages_is_refused() {
        assert_eq!(PdfWriter::new().finish(), Err(Error::UnsupportedFormat));
    }

    #[test]
    fn an_invalid_page_box_is_refused() {
        let jpeg = crate::jpeg::encode(&test_support::gradient(8, 8), 85, 200.0).expect("encodes");
        let mut writer = PdfWriter::new();
        assert_eq!(
            writer.add_jpeg_page(
                PageGeometry {
                    width_pt: 0.0,
                    height_pt: 100.0
                },
                &jpeg
            ),
            Err(Error::InvalidParameter { name: "geometry" })
        );
    }

    #[test]
    fn numbers_are_written_without_exponents_or_trailing_zeros() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(1.0), "1");
        assert_eq!(number(144.0), "144");
        assert_eq!(number(595.276), "595.276");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(1.00001), "1");
        // A tiny value would be 1e-7 under the default Display impl.
        assert_eq!(number(0.000_000_1), "0");
        for value in [1e-9f32, 1e9, 123_456.79, 0.1] {
            assert!(
                !number(value).contains('e') && !number(value).contains('E'),
                "{value} formatted as {}",
                number(value)
            );
        }
    }
}
