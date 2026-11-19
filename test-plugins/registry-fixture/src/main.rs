//! `registry-fixture <root>` (test only): a fixture registry for the browser and desktop
//! suites. It makes the publisher "acme" a key, serves `<root>/www` on loopback, prints
//! `ready <index URL>` and then takes one command per line on stdin, answering `ok …` or
//! `error …`:
//!
//! - `publish <dir>`: pack `<dir>` signed by acme's key and list it;
//! - `publish-unsigned <dir>`, `publish-other-key <dir>`: the same, unsigned or signed by
//!   someone else (to test refusals);
//! - `probe <out dir> <version> [<permissions line>…]`: write the probe test plugin as
//!   `acme.probe` (`spawn = ["fake-acp-agent"]` plus the lines given) into `<out dir>`;
//! - `starter <id>…`, `build` (writes `index.json`), `hits` (requests served so far).
//!
//! Closing stdin stops it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::path::{Path, PathBuf};

use kumikit_package::SigningKey;
use kumikit_test_registry_fixture::{Fixture, serve};
use tokio::io::{AsyncBufReadExt, BufReader};

fn probe(out: &Path, version: &str, extra: &[&str]) {
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(
        out.join("plugin.toml"),
        format!(
            "id = \"acme.probe\"\nname = \"Acme Probe\"\nversion = \"{version}\"\n\
             description = \"A third-party provider for tests.\"\nlicense = \"MIT\"\n\
             repository = \"https://example.com/acme/probe\"\nkind = [\"provider\"]\napi = \"^0.1\"\n\
             [runtime]\nwasm = \"plugin.wasm\"\nworlds = [\"kumikit:provider/plugin@0.1.0\"]\n\
             [permissions]\nspawn = [\"fake-acp-agent\"]\n{}\n\
             [contributes]\nproviders = [{{ id = \"probe\", name = \"Acme Probe\" }}]\n",
            extra.join("\n")
        ),
    )
    .unwrap();
    std::fs::copy(kumikit_test_plugins::PROBE_WASM, out.join("plugin.wasm")).unwrap();
}

#[tokio::main]
async fn main() {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: registry-fixture <root>"),
    );
    let www = root.join("www");
    std::fs::create_dir_all(&www).unwrap();
    let server = serve(www).await;
    let fixture = Fixture::new(&root, &server.url);
    let (key, _) = SigningKey::generate().unwrap();
    let (other, _) = SigningKey::generate().unwrap();
    fixture.publisher("acme", &key, &["acme"]);
    fixture.build_index();
    println!("ready {}", fixture.index_url());
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let words: Vec<&str> = line.split_whitespace().collect();
        let answer =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match words.as_slice() {
                ["publish", dir] => {
                    fixture.publish(Path::new(dir), Some(&key));
                    "ok".to_owned()
                }
                ["publish-unsigned", dir] => {
                    fixture.publish(Path::new(dir), None);
                    "ok".to_owned()
                }
                ["publish-other-key", dir] => {
                    fixture.publish(Path::new(dir), Some(&other));
                    "ok".to_owned()
                }
                ["probe", out, version, extra @ ..] => {
                    let extra = extra.join(" ");
                    let lines: Vec<&str> =
                        extra.split(';').filter(|l| !l.trim().is_empty()).collect();
                    probe(Path::new(out), version, &lines);
                    "ok".to_owned()
                }
                ["starter", ids @ ..] => {
                    fixture.starter(ids);
                    "ok".to_owned()
                }
                ["build"] => {
                    fixture.build_index();
                    "ok".to_owned()
                }
                ["hits"] => format!("ok {}", server.hits()),
                _ => format!("error unknown command: {line}"),
            }));
        println!(
            "{}",
            answer.unwrap_or_else(|_| "error the command failed".to_owned())
        );
    }
}
