//! `kumikit-registry`: the plugin registry's checker and index builder.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kumikit_registry::fetch::{Fetcher, Limits};
use kumikit_registry_tool::{CheckOptions, build_index, check};

#[derive(Debug, Parser)]
#[command(
    name = "kumikit-registry",
    version,
    about = "Check a Kumikit plugin registry and build its index"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check a registry checkout (a pull request's), against the default branch's.
    Check {
        /// The registry checkout to check.
        dir: PathBuf,
        /// The default branch's checkout, for key continuity and immutable versions.
        #[arg(long)]
        base: Option<PathBuf>,
        /// The pull request's author (GitHub login).
        #[arg(long)]
        author: Option<String>,
        /// Accept only publishers marked `early` (the official registry before 1.0).
        #[arg(long)]
        require_early: bool,
    },
    /// Build index.json from a registry checkout.
    BuildIndex {
        /// The registry checkout.
        dir: PathBuf,
        /// The registry's name, shown to users.
        #[arg(long)]
        name: String,
        /// Where to write the index.
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[allow(clippy::print_stdout, clippy::print_stderr)]
fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    match cli.command {
        Command::Check {
            dir,
            base,
            author,
            require_early,
        } => {
            let options = CheckOptions {
                author,
                require_early,
            };
            let errors = runtime.block_on(check(
                &dir,
                base.as_deref(),
                &options,
                &Fetcher::new(Limits::default()),
            ));
            if errors.is_empty() {
                println!("registry ok");
                return Ok(ExitCode::SUCCESS);
            }
            for e in &errors {
                eprintln!("error: {e}");
            }
            Ok(ExitCode::FAILURE)
        }
        Command::BuildIndex { dir, name, output } => {
            let generated = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .context("cannot format the time")?;
            match build_index(&dir, &name, &generated) {
                Ok(index) => {
                    let json = serde_json::to_string_pretty(&index)?;
                    std::fs::write(&output, json)
                        .with_context(|| format!("cannot write {}", output.display()))?;
                    println!(
                        "wrote {} ({} plugins)",
                        output.display(),
                        index.plugins.len()
                    );
                    Ok(ExitCode::SUCCESS)
                }
                Err(errors) => {
                    for e in &errors {
                        eprintln!("error: {e}");
                    }
                    Ok(ExitCode::FAILURE)
                }
            }
        }
    }
}
