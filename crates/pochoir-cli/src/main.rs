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

use pochoir_core::{DEFAULT_DPI, DEFAULT_QUALITY, OutputOptions, PageSizing};

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
  -V, --version        Print the core engine version and exit.
  -h, --help           Print this help and exit.

The document never leaves this machine. The output carries no EXIF, no GPS,
no dates and no build identifier.

No watermark is drawn yet: this is the write path only.";

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
    let pdf = pochoir_core::image_to_pdf(&bytes, &invocation.options)
        .map_err(|error| format!("{}: {error}", show(&invocation.input)))?;
    std::fs::write(&invocation.output, &pdf)
        .map_err(|error| format!("cannot write {}: {error}", show(&invocation.output)))?;
    Ok(format!(
        "wrote {} ({} bytes)",
        show(&invocation.output),
        pdf.len()
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
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(Abort::Usage(format!("unrecognised option `{other}`")));
            }
            path => set_input(&mut input, path)?,
        }
    }

    let input = input.ok_or_else(|| Abort::Usage("no input file given".into()))?;
    let output = output
        .ok_or_else(|| Abort::Usage("no output file given; pass `-o <output.pdf>`".into()))?;
    let options = OutputOptions {
        quality,
        dpi,
        page_sizing,
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

    fn usage_message(words: &[&str]) -> String {
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
