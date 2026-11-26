//! Command-line front end for the `pochoir` engine.
//!
//! Arguments are read by hand from [`std::env::args`]. There is deliberately no
//! argument-parsing crate: the surface is small enough to own outright, and
//! every dependency here is a dependency we would have to audit.

#![forbid(unsafe_code)]

use std::process::ExitCode;

const USAGE: &str = "\
pochoir - watermark documents locally, without uploading anything.

Usage:
  pochoir [options]

Options:
  -V, --version    Print the core engine version and exit.
  -h, --help       Print this help and exit.

No watermarking is implemented yet; this binary only reports the engine
version so that the build skeleton has something real to run.";

fn main() -> ExitCode {
    let mut show_help = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => show_help = true,
            // Printing the version is the default behaviour, so this flag only
            // has to be accepted, not acted on.
            "-V" | "--version" => {}
            other => {
                eprintln!("pochoir: unrecognised argument `{other}`\n");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    if show_help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    println!("pochoir-core {}", pochoir_core::version());
    ExitCode::SUCCESS
}
