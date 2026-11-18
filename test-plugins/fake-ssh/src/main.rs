//! Test only: stands in for `ssh` in the desktop shell's tests
//! (`KUMIKIT_SSH=<path to fake-ssh>`). It takes the arguments the shell gives `ssh`
//! (`-T`, `-o <option>` pairs, `--`, the destination, the remote command), ignores the
//! destination and runs the remote command here, with stdin and stdout passed through.
//!
//! The destination `denied` fails as ssh does when no key is accepted.

use std::process::{Command, ExitCode};

#[allow(clippy::print_stderr)]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() && args[i] != "--" {
        i += if args[i] == "-o" { 2 } else { 1 };
    }
    let Some(destination) = args.get(i + 1) else {
        eprintln!("usage: fake-ssh [-T] [-o option]... -- destination command");
        return ExitCode::from(255);
    };
    if destination == "denied" {
        eprintln!("{destination}: Permission denied (publickey).");
        return ExitCode::from(255);
    }
    // ssh joins the command words with spaces for the remote shell; split them again.
    let command = args[i + 2..].join(" ");
    let mut words = command.split_whitespace();
    let Some(program) = words.next() else {
        eprintln!("fake-ssh: no remote command");
        return ExitCode::from(255);
    };
    match Command::new(program).args(words).status() {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(e) => {
            eprintln!("bash: {program}: {e}");
            ExitCode::from(127)
        }
    }
}
