//! A test helper program, started by tests through the core's process supervisor.
//!
//! Modes (first argument):
//! - `tree`: starts `hang` as a child that shares this process's stdout, prints
//!   `ready`, then hangs. Stdout reaches EOF only when both processes are gone, which is
//!   how tests see that a whole process tree was killed.
//! - `hang`: blocks forever.
//! - `env`: prints its environment as `NAME=value` lines, sorted, and exits.
//! - `cwd`: prints its working directory and exits.
//! - `cat`: copies stdin to stdout until EOF.
//! - `stderr-flood <bytes>`: writes that many bytes to stderr, then `done` to stdout.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::{self, Read, Write};
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("tree") => tree(),
        Some("hang") => hang(),
        Some("env") => {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort();
            for (k, v) in vars {
                println!("{k}={v}");
            }
            ExitCode::SUCCESS
        }
        Some("cwd") => match std::env::current_dir() {
            Ok(d) => {
                println!("{}", d.display());
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e.to_string()),
        },
        Some("cat") => {
            let mut buf = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut buf) {
                return fail(&e.to_string());
            }
            let _ = io::stdout().write_all(&buf);
            ExitCode::SUCCESS
        }
        Some("stderr-flood") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 64 * 1024];
            let mut left = n;
            let mut err = io::stderr().lock();
            while left > 0 {
                let k = left.min(chunk.len());
                if err.write_all(&chunk[..k]).is_err() {
                    break;
                }
                left -= k;
            }
            drop(err);
            println!("done");
            ExitCode::SUCCESS
        }
        _ => fail("usage: fake-acp-agent <tree|hang|env|cwd|cat|stderr-flood N>"),
    }
}

fn tree() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return fail(&e.to_string()),
    };
    // The child inherits stdout, so it keeps the pipe open as long as it lives.
    if let Err(e) = Command::new(exe).arg("hang").stdin(Stdio::null()).spawn() {
        return fail(&e.to_string());
    }
    println!("ready");
    let _ = io::stdout().flush();
    hang()
}

fn hang() -> ExitCode {
    loop {
        std::thread::park();
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("fake-acp-agent: {message}");
    ExitCode::FAILURE
}
