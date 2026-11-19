//! `client-store <installed dir> <file.kkp>…` (test only): installs each package into a
//! client plugin store as the desktop app does (`scripts/check-templates.sh`), and prints
//! `installed <id> <version> signed|unsigned` per package. Fails on the first refusal.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use kumikit_shell::store::Store;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(installed) = args.next() else {
        eprintln!("usage: client-store <installed dir> <file.kkp>…");
        return ExitCode::from(2);
    };
    let store = Store {
        bundled: None,
        installed: PathBuf::from(installed),
    };
    for package in args {
        match store.install(&PathBuf::from(&package)) {
            Ok(e) => println!(
                "installed {} {} {}",
                e.id,
                e.version,
                if e.publisher.is_some() {
                    "signed"
                } else {
                    "unsigned"
                }
            ),
            Err(e) => {
                eprintln!("{package}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
