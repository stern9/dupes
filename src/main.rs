use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::Parser;
use dupes::{Options, find_duplicates};

/// Find duplicate files by content and report how much space they waste.
///
/// dupes only reports. It never deletes or modifies anything.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Directories (or files) to search
    #[arg(value_name = "PATH", default_value = ".")]
    paths: Vec<PathBuf>,

    /// Ignore files smaller than this many bytes (0 includes empty files)
    #[arg(long, value_name = "BYTES", default_value_t = 1)]
    min_size: u64,

    /// Print the report as JSON instead of text
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("dupes: error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let options = Options {
        min_size: cli.min_size,
    };
    let report = find_duplicates(&cli.paths, &options);

    for warning in &report.warnings {
        eprintln!("dupes: warning: {warning}");
    }

    let mut out = BufWriter::new(io::stdout().lock());
    let written = if cli.json {
        report.write_json(&mut out)
    } else {
        report.write_text(&mut out)
    }
    .and_then(|()| out.flush());

    match written {
        // The reader went away (e.g. `dupes | head`); that is not our failure.
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        other => other.context("failed to write report"),
    }
}
