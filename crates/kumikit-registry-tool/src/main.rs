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
    /// Build (and optionally Ed25519-sign) release.json from staged release assets.
    ReleaseManifest {
        /// Git release tag (for example `v0.1.0-m8`).
        #[arg(long)]
        tag: String,
        /// Directory containing staged release archives and installers.
        #[arg(long)]
        assets_dir: PathBuf,
        /// Base download URL for assets (for example `https://github.com/kumikit/kumikit/releases/download/v0.1.0-m8`).
        #[arg(long)]
        base_url: String,
        /// Release notes URL (defaults to `https://github.com/kumikit/kumikit/releases/tag/<tag>`).
        #[arg(long)]
        notes_url: Option<String>,
        /// Optional short release summary.
        #[arg(long)]
        notes: Option<String>,
        /// Directory where `release.json` (and `release.json.sig` when signed) are written.
        #[arg(long)]
        out_dir: PathBuf,
        /// Path to a PKCS#8 Ed25519 private key PEM file (falls back to `KUMIKIT_SIGNING_KEY`).
        #[arg(long)]
        key: Option<PathBuf>,
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
        Command::ReleaseManifest {
            tag,
            assets_dir,
            base_url,
            notes_url,
            notes,
            out_dir,
            key,
        } => {
            let published_at = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .context("cannot format the time")?;
            let notes_url = notes_url.unwrap_or_else(|| {
                format!("https://github.com/kumikit/kumikit/releases/tag/{tag}")
            });
            let manifest = kumikit_registry::release::build_manifest_from_dir(
                &tag,
                &assets_dir,
                &base_url,
                &published_at,
                &notes_url,
                notes,
            )?;
            std::fs::create_dir_all(&out_dir)
                .with_context(|| format!("cannot create {}", out_dir.display()))?;
            let key_pem = match key {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .with_context(|| format!("cannot read key {}", path.display()))?,
                ),
                None => std::env::var("KUMIKIT_SIGNING_KEY")
                    .ok()
                    .filter(|s| !s.trim().is_empty()),
            };
            let manifest_path = out_dir.join(kumikit_registry::release::RELEASE_MANIFEST_NAME);
            if let Some(pem) = key_pem {
                let signing_key = kumikit_package::signature::SigningKey::from_pem(&pem)
                    .context("invalid Ed25519 signing key")?;
                let (json, sig) =
                    kumikit_registry::release::sign_manifest(&manifest, &signing_key)?;
                kumikit_registry::release::verify_release_bytes(&json, &sig, signing_key.public())?;
                std::fs::write(&manifest_path, &json)?;
                let sig_path = out_dir.join("release.json.sig");
                std::fs::write(&sig_path, &sig)?;
                println!(
                    "wrote {} and {} (signed by {}; {} core, {} desktop assets)",
                    manifest_path.display(),
                    sig_path.display(),
                    signing_key.public(),
                    manifest.core.len(),
                    manifest.desktop.len()
                );
            } else {
                let mut json = serde_json::to_vec_pretty(&manifest)?;
                json.push(b'\n');
                std::fs::write(&manifest_path, &json)?;
                println!(
                    "wrote {} (unsigned; {} core, {} desktop assets)",
                    manifest_path.display(),
                    manifest.core.len(),
                    manifest.desktop.len()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
