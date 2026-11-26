//! Command-line front end for the `pochoir` engine.
//!
//! Arguments are read by hand from [`std::env::args`]. There is deliberately no
//! argument-parsing crate: the surface is small enough to own outright, and
//! every dependency here is a dependency we would have to audit.
//!
//! **All file I/O lives here.** `pochoir-core` must keep compiling for
//! `wasm32-unknown-unknown`, so it never opens a file; this binary reads the
//! input, hands the bytes to the engine, and writes the result.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pochoir_core::mark::{
    MAX_BLUR_RATIO, MAX_FONT_SIZE_RATIO, MAX_OPACITY, MIN_FONT_SIZE_RATIO, MIN_OPACITY,
};
use pochoir_core::{
    DEFAULT_DPI, DEFAULT_QUALITY, Mark, OutputOptions, PageSizing, TileDensity,
    mark::unsupported_characters,
};

const USAGE: &str = "\
pochoir - watermark documents locally, without uploading anything.

Usage:
  pochoir <input> -o <output.pdf> [options]

Arguments:
  <input>              A JPEG or PNG file. HEIC is not supported.

Options:
  -o, --output FILE    Where to write the PDF. Required.
  -q, --quality N      JPEG quality, 1-100 (default 85).
      --dpi N          Output resolution in dots per inch (default 200).
                       Bitmap inputs are never resampled: this only decides
                       what physical page size their pixels represent, and a
                       resolution the input itself states is honoured when it
                       is higher.
      --page-size S    a4-max (default) or native.

                       a4-max keeps the image's aspect ratio and shrinks the
                       page to fit an A4 sheet if it would be larger. It never
                       enlarges: a 600 dpi scan of an identity card stays 86 by
                       54 mm. Without it a 12-megapixel photo makes a page
                       bigger than A2, which upload portals reject and printers
                       tile across four sheets.

                       native gives the true size at the resolution above,
                       however large. No pixel differs between the two; only
                       the page does.

Watermark:
  -m, --mark TEXT      The text to burn into the page, up to 200 characters.
                       Without it no watermark is drawn.

                       Name the recipient and the purpose: it is what makes the
                       mark attributable, and attributability is most of the
                       deterrent. For instance:

                         -m 'Document destine exclusivement a une demande de
                             location - 26/11/2026'

      --mark-size P    Font size as a percent of the page's shorter edge,
                       1 to 10 (default 3.5). Relative to the page, so the mark
                       looks the same at every resolution and on every sheet.
      --mark-angle D   Rotation in degrees, -90 to 90 (default -30). Negative
                       runs from the bottom left to the top right. A diagonal
                       mark crosses the document's own text lines, so removing
                       it means reconstructing text across the stroke.
      --mark-opacity O 0.05 to 0.80 (default 0.30). The mark must not stop the
                       recipient reading the document. Coverage comes from
                       tiling, not from one heavy mark.
      --mark-colour H  Six hex digits, e.g. 1a1a1a (the default). A dark grey
                       survives a black-and-white photocopy and heavy JPEG
                       re-compression; a pale one can vanish. A colour too
                       light to survive is warned about, not refused.
      --mark-density D low, normal (default) or high.
      --mark-blur P    Blur as a percent of the font size, 0 to 6
                       (default 1.5). Softens the glyph edges so the mark reads
                       as part of the image. This is antialiasing: it has not
                       been measured against any watermark-removal tool and is
                       not claimed as a defence.
      --seed N         Seed the position jitter, so a run is reproducible
                       across documents. Without it the seed is derived from
                       the document and the parameters, which is already
                       reproducible for a given file.
      --no-jitter      Put every tile on an unshifted grid. Jitter is on by
                       default because two runs then place the mark
                       differently, so somebody holding two marked copies
                       cannot difference them to recover the clean original.

  -V, --version        Print the core engine version and exit.
  -h, --help           Print this help and exit.

The document never leaves this machine. The output carries no EXIF, no GPS,
no dates and no build identifier.";

/// What the command line asked for.
struct Invocation {
    input: PathBuf,
    output: PathBuf,
    options: OutputOptions,
}

/// Why we are not going to do any work.
#[derive(Debug)]
enum Abort {
    /// Print the help on stdout and succeed.
    Help,
    /// Print the version on stdout and succeed.
    Version,
    /// Print a message on stderr, with the usage, and fail.
    Usage(String),
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match parse(&arguments) {
        Ok(invocation) => invocation,
        Err(Abort::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(Abort::Version) => {
            println!("pochoir-core {}", pochoir_core::version());
            return ExitCode::SUCCESS;
        }
        Err(Abort::Usage(message)) => {
            eprintln!("pochoir: {message}\n");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    match run(&invocation) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("pochoir: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Read the input, render it, write the output.
fn run(invocation: &Invocation) -> Result<String, String> {
    let bytes = std::fs::read(&invocation.input)
        .map_err(|error| format!("cannot read {}: {error}", show(&invocation.input)))?;
    // The compositor knows things the pixels do not show — chiefly that the
    // text would not fit at the size that was asked for. ADR-0007 Decision 2
    // keeps drawing in that case rather than refusing, which makes saying so
    // the whole of the user's protection against a mark quietly coming out
    // much smaller than they chose.
    let mut shrunk: Option<(f32, f32, f32)> = None;
    let pdf =
        pochoir_core::image_to_pdf_observed(&bytes, &invocation.options, |_, _, composition| {
            if let Some(composition) = composition
                && composition.was_shrunk_noticeably()
                && shrunk.is_none()
            {
                shrunk = Some((
                    composition.shrink_ratio(),
                    composition.requested_font_size_px,
                    composition.layout.font_size_px,
                ));
            }
        })
        .map_err(|error| format!("{}: {error}", show(&invocation.input)))?;
    if let Some((ratio, asked, drawn)) = shrunk {
        eprintln!(
            "pochoir: warning: the mark text did not fit at the size you asked \
             for, so it was drawn at {:.0}% of it ({drawn:.0} px instead of \
             {asked:.0}). Shorten the text, or raise --mark-size and let it \
             wrap over more lines.",
            ratio * 100.0
        );
    }
    std::fs::write(&invocation.output, &pdf)
        .map_err(|error| format!("cannot write {}: {error}", show(&invocation.output)))?;
    Ok(format!(
        "wrote {} ({} bytes{})",
        show(&invocation.output),
        pdf.len(),
        if invocation.options.mark.is_some() {
            ", watermarked"
        } else {
            ", no watermark"
        }
    ))
}

/// A path as the user would recognise it, with no attempt to canonicalise.
fn show(path: &Path) -> String {
    path.display().to_string()
}

/// Read the command line.
///
/// Long options accept both `--dpi 300` and `--dpi=300`; `--` ends option
/// processing so that a file named `-o` is still reachable.
fn parse(arguments: &[String]) -> Result<Invocation, Abort> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut quality = DEFAULT_QUALITY;
    let mut dpi = DEFAULT_DPI;
    let mut page_sizing = PageSizing::default();
    let mut text: Option<String> = None;
    let mut mark = Mark::default();
    let mut seed: Option<u64> = None;
    let mut jitter = true;
    let mut positional_only = false;

    if arguments.is_empty() {
        return Err(Abort::Help);
    }

    let mut index = 0usize;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        index += 1;

        if positional_only {
            set_input(&mut input, argument)?;
            continue;
        }

        // A long option may carry its value after an `=`.
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (argument, None),
        };

        match flag {
            "--" => positional_only = true,
            "-h" | "--help" => return Err(Abort::Help),
            "-V" | "--version" => return Err(Abort::Version),
            "-o" | "--output" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                if output.is_some() {
                    return Err(Abort::Usage("`--output` given more than once".into()));
                }
                output = Some(PathBuf::from(value));
            }
            "-q" | "--quality" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                quality = value
                    .parse::<u8>()
                    .ok()
                    .filter(|q| (1..=100).contains(q))
                    .ok_or_else(|| {
                        Abort::Usage(format!(
                            "`--quality` wants a whole number from 1 to 100, not `{value}`"
                        ))
                    })?;
            }
            "--dpi" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                dpi = value
                    .parse::<f32>()
                    .ok()
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .ok_or_else(|| {
                        Abort::Usage(format!("`--dpi` wants a positive number, not `{value}`"))
                    })?;
            }
            "--page-size" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                page_sizing = match value {
                    "a4-max" => PageSizing::CapToA4,
                    "native" => PageSizing::Native,
                    other => {
                        return Err(Abort::Usage(format!(
                            "`--page-size` wants `a4-max` or `native`, not `{other}`"
                        )));
                    }
                };
            }
            "-m" | "--mark" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                if text.is_some() {
                    return Err(Abort::Usage("`--mark` given more than once".into()));
                }
                text = Some(value.to_string());
            }
            "--mark-size" => {
                // Shown to the user as a percent, because "3.5" reads better
                // than "0.035" on a command line; the engine wants a fraction.
                let percent = number(flag, inline, arguments, &mut index)?;
                mark.font_size_ratio = percent / 100.0;
                if !(MIN_FONT_SIZE_RATIO..=MAX_FONT_SIZE_RATIO).contains(&mark.font_size_ratio) {
                    return Err(Abort::Usage(format!(
                        "`--mark-size` wants a percent from {:.0} to {:.0}, not `{percent}`",
                        MIN_FONT_SIZE_RATIO * 100.0,
                        MAX_FONT_SIZE_RATIO * 100.0
                    )));
                }
            }
            "--mark-angle" => {
                mark.angle_degrees = number(flag, inline, arguments, &mut index)?;
                if !(-90.0..=90.0).contains(&mark.angle_degrees) {
                    return Err(Abort::Usage(
                        "`--mark-angle` wants degrees from -90 to 90".into(),
                    ));
                }
            }
            "--mark-opacity" => {
                mark.opacity = number(flag, inline, arguments, &mut index)?;
                if !(MIN_OPACITY..=MAX_OPACITY).contains(&mark.opacity) {
                    return Err(Abort::Usage(format!(
                        "`--mark-opacity` wants a number from {MIN_OPACITY} to {MAX_OPACITY}"
                    )));
                }
            }
            "--mark-colour" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                mark.colour = hex_colour(value).ok_or_else(|| {
                    Abort::Usage(format!(
                        "`--mark-colour` wants six hexadecimal digits such as `1a1a1a`, not `{value}`"
                    ))
                })?;
            }
            "--mark-density" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                mark.density = match value {
                    "low" => TileDensity::Low,
                    "normal" => TileDensity::Normal,
                    "high" => TileDensity::High,
                    other => {
                        return Err(Abort::Usage(format!(
                            "`--mark-density` wants `low`, `normal` or `high`, not `{other}`"
                        )));
                    }
                };
            }
            "--mark-blur" => {
                let percent = number(flag, inline, arguments, &mut index)?;
                mark.blur_ratio = percent / 100.0;
                if !(0.0..=MAX_BLUR_RATIO).contains(&mark.blur_ratio) {
                    return Err(Abort::Usage(format!(
                        "`--mark-blur` wants a percent from 0 to {:.0}, not `{percent}`",
                        MAX_BLUR_RATIO * 100.0
                    )));
                }
            }
            "--seed" => {
                let value = take_value(flag, inline, arguments, &mut index)?;
                seed = Some(value.parse::<u64>().map_err(|_| {
                    Abort::Usage(format!(
                        "`--seed` wants a whole number from 0 to {}, not `{value}`",
                        u64::MAX
                    ))
                })?);
            }
            "--no-jitter" => jitter = false,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(Abort::Usage(format!("unrecognised option `{other}`")));
            }
            path => set_input(&mut input, path)?,
        }
    }

    let input = input.ok_or_else(|| Abort::Usage("no input file given".into()))?;
    let output = output
        .ok_or_else(|| Abort::Usage("no output file given; pass `-o <output.pdf>`".into()))?;
    // Every appearance flag is ignored without `--mark`, and saying so is
    // better than silently producing an unmarked PDF for somebody who meant to
    // watermark it and mistyped the one flag that matters.
    let mark = match text {
        Some(text) => Some(Mark { text, ..mark }),
        None if mark != Mark::default() || seed.is_some() || !jitter => {
            return Err(Abort::Usage(
                "watermark options were given but no `--mark TEXT`, so there would be \
                 no watermark to apply them to"
                    .into(),
            ));
        }
        None => None,
    };
    if let Some(mark) = &mark {
        // Name every character the font cannot draw, rather than the one the
        // engine's error can carry.
        let missing = unsupported_characters(&mark.text).unwrap_or_default();
        if !missing.is_empty() {
            let named: Vec<String> = missing
                .iter()
                .map(|c| format!("{c:?} (U+{:04X})", u32::from(*c)))
                .collect();
            return Err(Abort::Usage(format!(
                "the mark text contains {} the bundled font cannot draw: {}",
                if named.len() == 1 {
                    "a character"
                } else {
                    "characters"
                },
                named.join(", ")
            )));
        }
        if mark.colour_is_too_pale() {
            eprintln!(
                "pochoir: warning: this mark colour is light enough that a \
                 black-and-white photocopy may lose it"
            );
        }
    }

    let options = OutputOptions {
        quality,
        dpi,
        page_sizing,
        mark,
        seed,
        jitter,
    };
    // The engine validates too, but failing here means failing before the file
    // is read, with a message about the command line rather than the document.
    options
        .validate()
        .map_err(|error| Abort::Usage(error.to_string()))?;

    Ok(Invocation {
        input,
        output,
        options,
    })
}

fn set_input(slot: &mut Option<PathBuf>, value: &str) -> Result<(), Abort> {
    if slot.is_some() {
        return Err(Abort::Usage(format!(
            "more than one input file given (`{value}` came second)"
        )));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

/// An option's value, parsed as a number.
fn number(
    flag: &str,
    inline: Option<&str>,
    arguments: &[String],
    index: &mut usize,
) -> Result<f32, Abort> {
    let value = take_value(flag, inline, arguments, index)?;
    value
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| Abort::Usage(format!("`{flag}` wants a number, not `{value}`")))
}

/// Six hexadecimal digits, with or without a leading `#`.
fn hex_colour(value: &str) -> Option<[u8; 3]> {
    let digits = value.strip_prefix('#').unwrap_or(value);
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 3];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// The value of an option, whether it arrived after `=` or as the next word.
fn take_value<'a>(
    flag: &str,
    inline: Option<&'a str>,
    arguments: &'a [String],
    index: &mut usize,
) -> Result<&'a str, Abort> {
    if let Some(value) = inline {
        return Ok(value);
    }
    let value = arguments
        .get(*index)
        .ok_or_else(|| Abort::Usage(format!("`{flag}` needs a value")))?;
    *index += 1;
    Ok(value.as_str())
}

#[cfg(test)]
mod tests {
    use super::{Abort, parse};
    use pochoir_core::{DEFAULT_DPI, DEFAULT_QUALITY, PageSizing};

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    pub(super) fn usage_message(words: &[&str]) -> String {
        match parse(&args(words)) {
            Err(Abort::Usage(message)) => message,
            Err(Abort::Help) => panic!("{words:?} asked for help, not a usage error"),
            Err(Abort::Version) => panic!("{words:?} asked for the version"),
            Ok(_) => panic!("{words:?} parsed successfully but should not have"),
        }
    }

    #[test]
    fn the_shortest_useful_invocation_parses() {
        let invocation = parse(&args(&["photo.jpg", "-o", "out.pdf"])).expect("parses");
        assert_eq!(invocation.input.to_str(), Some("photo.jpg"));
        assert_eq!(invocation.output.to_str(), Some("out.pdf"));
        assert_eq!(invocation.options.quality, DEFAULT_QUALITY);
        assert_eq!(invocation.options.dpi, DEFAULT_DPI);
    }

    #[test]
    fn options_may_precede_or_follow_the_input() {
        for words in [
            &["-o", "out.pdf", "photo.jpg"][..],
            &["photo.jpg", "-o", "out.pdf"][..],
            &["--output", "out.pdf", "--", "photo.jpg"][..],
        ] {
            let invocation = parse(&args(words)).unwrap_or_else(|_| panic!("{words:?}"));
            assert_eq!(invocation.input.to_str(), Some("photo.jpg"));
            assert_eq!(invocation.output.to_str(), Some("out.pdf"));
        }
    }

    #[test]
    fn values_may_be_attached_with_an_equals_sign() {
        let invocation = parse(&args(&[
            "photo.jpg",
            "--output=out.pdf",
            "--quality=92",
            "--dpi=300",
        ]))
        .expect("parses");
        assert_eq!(invocation.output.to_str(), Some("out.pdf"));
        assert_eq!(invocation.options.quality, 92);
        assert_eq!(invocation.options.dpi, 300.0);
    }

    #[test]
    fn short_forms_work_too() {
        let invocation = parse(&args(&["photo.jpg", "-o", "out.pdf", "-q", "70"])).expect("parses");
        assert_eq!(invocation.options.quality, 70);
    }

    #[test]
    fn help_and_version_short_circuit() {
        for words in [&["-h"][..], &["--help"][..], &["photo.jpg", "--help"][..]] {
            assert!(matches!(parse(&args(words)), Err(Abort::Help)), "{words:?}");
        }
        for words in [&["-V"][..], &["--version"][..]] {
            assert!(
                matches!(parse(&args(words)), Err(Abort::Version)),
                "{words:?}"
            );
        }
        // No arguments at all is a request for help, not an error.
        assert!(matches!(parse(&[]), Err(Abort::Help)));
    }

    #[test]
    fn a_missing_output_is_an_error_that_says_what_to_pass() {
        let message = usage_message(&["photo.jpg"]);
        assert!(message.contains("-o <output.pdf>"), "{message}");
    }

    #[test]
    fn a_missing_input_is_an_error() {
        assert!(usage_message(&["-o", "out.pdf"]).contains("no input file"));
    }

    #[test]
    fn repeated_or_surplus_arguments_are_refused() {
        assert!(
            usage_message(&["a.jpg", "b.jpg", "-o", "out.pdf"]).contains("more than one input")
        );
        assert!(
            usage_message(&["a.jpg", "-o", "one.pdf", "-o", "two.pdf"]).contains("more than once")
        );
    }

    #[test]
    fn out_of_range_values_are_refused_with_the_range_in_the_message() {
        for quality in ["0", "101", "abc", "-5", "85.5"] {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", "-q", quality]);
            assert!(message.contains("1 to 100"), "{quality}: {message}");
        }
        for dpi in ["0", "-300", "nonsense", "nan", "inf"] {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", "--dpi", dpi]);
            assert!(message.contains("positive number"), "{dpi}: {message}");
        }
        // Finite, positive, but past what the engine will accept.
        let message = usage_message(&["a.jpg", "-o", "o.pdf", "--dpi", "100000"]);
        assert!(message.contains("dpi"), "{message}");
    }

    #[test]
    fn the_page_size_defaults_to_the_a4_cap_and_can_be_opted_out_of() {
        let invocation = parse(&args(&["a.jpg", "-o", "o.pdf"])).expect("parses");
        assert_eq!(invocation.options.page_sizing, PageSizing::CapToA4);

        for words in [
            &["a.jpg", "-o", "o.pdf", "--page-size", "native"][..],
            &["a.jpg", "-o", "o.pdf", "--page-size=native"][..],
        ] {
            let invocation = parse(&args(words)).unwrap_or_else(|_| panic!("{words:?}"));
            assert_eq!(invocation.options.page_sizing, PageSizing::Native);
        }

        let invocation =
            parse(&args(&["a.jpg", "-o", "o.pdf", "--page-size=a4-max"])).expect("parses");
        assert_eq!(invocation.options.page_sizing, PageSizing::CapToA4);
    }

    #[test]
    fn an_unknown_page_size_is_refused_with_the_choices_in_the_message() {
        for value in ["a4", "A4-MAX", "letter", "", "true"] {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", "--page-size", value]);
            assert!(
                message.contains("`a4-max` or `native`"),
                "{value}: {message}"
            );
        }
    }

    #[test]
    fn an_option_without_its_value_is_refused() {
        for flag in ["-o", "--output", "-q", "--quality", "--dpi", "--page-size"] {
            let message = usage_message(&["a.jpg", flag]);
            assert!(message.contains("needs a value"), "{flag}: {message}");
        }
    }

    #[test]
    fn unknown_options_are_refused_rather_than_treated_as_filenames() {
        let message = usage_message(&["--colour", "red", "a.jpg", "-o", "o.pdf"]);
        assert!(
            message.contains("unrecognised option `--colour`"),
            "{message}"
        );
    }

    #[test]
    fn a_double_dash_lets_a_file_be_named_like_an_option() {
        let invocation = parse(&args(&["-o", "out.pdf", "--", "-o"])).expect("parses");
        assert_eq!(invocation.input.to_str(), Some("-o"));
    }

    #[test]
    fn a_lone_dash_is_a_filename_not_an_option() {
        let invocation = parse(&args(&["-", "-o", "out.pdf"])).expect("parses");
        assert_eq!(invocation.input.to_str(), Some("-"));
    }
}

#[cfg(test)]
mod mark_tests {
    use super::tests::usage_message;
    use super::{USAGE, hex_colour, parse};
    use pochoir_core::TileDensity;
    use pochoir_core::mark::{
        DEFAULT_ANGLE_DEGREES, DEFAULT_BLUR_RATIO, DEFAULT_FONT_SIZE_RATIO, DEFAULT_OPACITY,
    };

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn no_mark_flag_means_no_watermark() {
        let invocation = parse(&args(&["a.jpg", "-o", "o.pdf"])).expect("parses");
        assert!(invocation.options.mark.is_none());
        assert!(invocation.options.jitter, "jitter is on by default");
        assert_eq!(invocation.options.seed, None);
    }

    #[test]
    fn the_mark_text_is_taken_verbatim() {
        for words in [
            &["a.jpg", "-o", "o.pdf", "-m", "destiné à une agence"][..],
            &["a.jpg", "-o", "o.pdf", "--mark", "destiné à une agence"][..],
            &["a.jpg", "-o", "o.pdf", "--mark=destiné à une agence"][..],
        ] {
            let invocation = parse(&args(words)).unwrap_or_else(|_| panic!("{words:?}"));
            let mark = invocation.options.mark.expect("a mark");
            assert_eq!(mark.text, "destiné à une agence");
        }
    }

    #[test]
    fn the_appearance_flags_reach_the_engine() {
        let invocation = parse(&args(&[
            "a.jpg",
            "-o",
            "o.pdf",
            "-m",
            "destiné",
            "--mark-size=7.5",
            "--mark-angle=-45",
            "--mark-opacity=0.5",
            "--mark-colour=#204080",
            "--mark-density=high",
            "--mark-blur=3",
            "--seed=4242",
            "--no-jitter",
        ]))
        .expect("parses");
        let options = invocation.options;
        // The whole set must be acceptable to the engine, checked before the
        // mark is moved out for the field assertions.
        options.validate().expect("the engine must accept these");
        let mark = options.mark.clone().expect("a mark");
        assert!(
            (mark.font_size_ratio - 0.075).abs() < 1e-6,
            "percent to fraction"
        );
        assert_eq!(mark.angle_degrees, -45.0);
        assert_eq!(mark.opacity, 0.5);
        assert_eq!(mark.colour, [0x20, 0x40, 0x80]);
        assert_eq!(mark.density, TileDensity::High);
        assert!((mark.blur_ratio - 0.03).abs() < 1e-6);
        assert_eq!(options.seed, Some(4242));
        assert!(!options.jitter);
    }

    #[test]
    fn appearance_flags_without_a_mark_are_an_error_rather_than_silently_ignored() {
        for flag in [
            "--mark-size=5",
            "--mark-angle=0",
            "--mark-opacity=0.5",
            "--mark-colour=000000",
            "--mark-density=high",
            "--mark-blur=2",
            "--seed=1",
            "--no-jitter",
        ] {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", flag]);
            assert!(message.contains("no `--mark TEXT`"), "{flag}: {message}");
        }
    }

    #[test]
    fn out_of_range_mark_values_are_refused_with_the_range_in_the_message() {
        let cases: [(&str, &str); 8] = [
            ("--mark-size=0.5", "1 to 10"),
            ("--mark-size=11", "1 to 10"),
            ("--mark-angle=91", "-90 to 90"),
            ("--mark-opacity=0.01", "0.05 to 0.8"),
            ("--mark-opacity=0.9", "0.05 to 0.8"),
            ("--mark-blur=7", "0 to 6"),
            ("--mark-colour=xyz", "hexadecimal"),
            ("--mark-density=medium", "`low`, `normal` or `high`"),
        ];
        for (flag, expected) in cases {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", "-m", "x", flag]);
            assert!(message.contains(expected), "{flag}: {message}");
        }
    }

    #[test]
    fn text_the_font_cannot_draw_is_refused_with_every_character_named() {
        let message = usage_message(&["a.jpg", "-o", "o.pdf", "-m", "Document 漢 et ✓"]);
        assert!(message.contains('漢'), "{message}");
        assert!(message.contains('✓'), "{message}");
        assert!(message.contains("U+6F22"), "{message}");
        assert!(
            message.contains("characters"),
            "both should be named: {message}"
        );
        // One bad character reads as singular.
        let single = usage_message(&["a.jpg", "-o", "o.pdf", "-m", "Document 漢"]);
        assert!(single.contains("a character"), "{single}");
    }

    #[test]
    fn a_repeated_mark_flag_is_refused() {
        let message = usage_message(&["a.jpg", "-o", "o.pdf", "-m", "one", "-m", "two"]);
        assert!(message.contains("more than once"), "{message}");
    }

    #[test]
    fn mark_options_need_their_values() {
        for flag in [
            "-m",
            "--mark",
            "--mark-size",
            "--mark-angle",
            "--mark-opacity",
            "--mark-colour",
            "--mark-density",
            "--mark-blur",
            "--seed",
        ] {
            let message = usage_message(&["a.jpg", "-o", "o.pdf", flag]);
            assert!(message.contains("needs a value"), "{flag}: {message}");
        }
    }

    #[test]
    fn hex_colours_parse_with_or_without_a_hash() {
        assert_eq!(hex_colour("1a1a1a"), Some([0x1A, 0x1A, 0x1A]));
        assert_eq!(hex_colour("#1A1A1A"), Some([0x1A, 0x1A, 0x1A]));
        assert_eq!(hex_colour("ffffff"), Some([0xFF, 0xFF, 0xFF]));
        for bad in ["", "#", "1a1a1", "1a1a1a1", "zzzzzz", "1a 1a1a", "#xyzxyz"] {
            assert_eq!(hex_colour(bad), None, "{bad:?} should not parse");
        }
    }

    #[test]
    fn the_help_text_quotes_the_engines_own_defaults() {
        // Two numbers that have to agree, so the help cannot drift away from
        // what the engine actually does.
        for expected in [
            format!("default {})", DEFAULT_FONT_SIZE_RATIO * 100.0),
            format!("default {})", DEFAULT_ANGLE_DEGREES),
            format!("default {:.2})", DEFAULT_OPACITY),
            format!("default {})", DEFAULT_BLUR_RATIO * 100.0),
        ] {
            assert!(
                USAGE.contains(&expected),
                "the help text does not say {expected:?}"
            );
        }
    }
}
