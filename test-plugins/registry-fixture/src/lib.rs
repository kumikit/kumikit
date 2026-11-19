//! Fixture plugin registries for tests (test only).
//!
//! A [`Fixture`] is a registry checkout (`repo/`) and the directory its packages and
//! `index.json` are served from (`www/`), served on loopback by [`serve`], which counts
//! requests so tests can prove nothing was fetched behind the user's back. Publisher keys
//! are made at test time; none is ever written into the source tree.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kumikit_package::SigningKey;
use kumikit_registry::index::{PluginFile, PublisherFile};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A registry checkout and its served files.
#[derive(Debug)]
pub struct Fixture {
    /// The registry repository (`publishers/`, `plugins/`, `starter.toml`).
    pub repo: PathBuf,
    /// What is served: packages and `index.json`.
    pub www: PathBuf,
    /// The URL `www` is served at, without a trailing slash.
    pub base_url: String,
}

impl Fixture {
    /// A new, empty registry under `root`, whose files will be served at `base_url`.
    pub fn new(root: &Path, base_url: &str) -> Fixture {
        let repo = root.join("repo");
        let www = root.join("www");
        std::fs::create_dir_all(repo.join("publishers")).unwrap();
        std::fs::create_dir_all(repo.join("plugins")).unwrap();
        std::fs::create_dir_all(&www).unwrap();
        Fixture {
            repo,
            www,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// The index URL.
    pub fn index_url(&self) -> String {
        format!("{}/index.json", self.base_url)
    }

    /// Writes `publishers/<name>.toml` with the key's public half.
    pub fn publisher(&self, name: &str, key: &SigningKey, github: &[&str]) {
        let file = PublisherFile {
            name: name.to_uppercase(),
            key: key.public().to_string(),
            github: github.iter().map(|s| (*s).to_owned()).collect(),
            early: true,
        };
        std::fs::write(
            self.repo.join("publishers").join(format!("{name}.toml")),
            toml::to_string(&file).unwrap(),
        )
        .unwrap();
    }

    /// Packs `src` signed with `key` (or unsigned), serves it, and adds the version to the
    /// plugin's file, with the fields the registry copies from the manifest. Returns the
    /// package path.
    pub fn publish(&self, src: &Path, key: Option<&SigningKey>) -> PathBuf {
        let staged = self.www.join("staging.kkp");
        kumikit_package::pack_signed(src, &staged, key).unwrap();
        let bytes = std::fs::read(&staged).unwrap();
        let facts = kumikit_registry_tool::facts_of_package(&bytes).unwrap();
        let name = format!("{}-{}.kkp", facts.id, facts.version);
        let file = self.www.join(&name);
        std::fs::rename(&staged, &file).unwrap();
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let entry = kumikit_registry_tool::version_entry(
            &facts,
            &format!("{}/{name}", self.base_url),
            &sha,
        );
        let (publisher, plugin) = facts.id.split_once('.').unwrap();
        let dir = self.repo.join("plugins").join(publisher);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{plugin}.toml"));
        let mut f: PluginFile = std::fs::read_to_string(&path)
            .ok()
            .map(|t| toml::from_str(&t).unwrap())
            .unwrap_or(PluginFile {
                id: facts.id.clone(),
                name: plugin.to_owned(),
                description: Some(format!("The {plugin} test plugin.")),
                repository: Some(format!("https://example.com/{publisher}/{plugin}")),
                license: Some("MIT".into()),
                kind: facts.kind.clone(),
                versions: vec![],
            });
        f.versions.retain(|v| v.version != entry.version);
        f.versions.push(entry);
        std::fs::write(&path, toml::to_string(&f).unwrap()).unwrap();
        file
    }

    /// Edits a plugin's file (for tests that make it lie).
    pub fn edit_plugin(&self, id: &str, edit: impl FnOnce(&mut PluginFile)) {
        let (publisher, plugin) = id.split_once('.').unwrap();
        let path = self
            .repo
            .join("plugins")
            .join(publisher)
            .join(format!("{plugin}.toml"));
        let mut f: PluginFile = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        edit(&mut f);
        std::fs::write(&path, toml::to_string(&f).unwrap()).unwrap();
    }

    /// Writes `starter.toml`.
    pub fn starter(&self, ids: &[&str]) {
        let list: Vec<String> = ids.iter().map(|i| format!("{i:?}")).collect();
        std::fs::write(
            self.repo.join("starter.toml"),
            format!("ids = [{}]\n", list.join(", ")),
        )
        .unwrap();
    }

    /// Builds `www/index.json` from the checkout.
    pub fn build_index(&self) {
        let index =
            kumikit_registry_tool::build_index(&self.repo, "Fixture", "2026-11-18T00:00:00Z")
                .unwrap_or_else(|e| panic!("cannot build the fixture index: {e:?}"));
        std::fs::write(
            self.www.join("index.json"),
            serde_json::to_vec_pretty(&index).unwrap(),
        )
        .unwrap();
    }
}

/// A loopback HTTP server for a directory.
#[derive(Debug)]
pub struct Server {
    /// `http://127.0.0.1:<port>`.
    pub url: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    /// How many requests it answered.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves `dir` on a free loopback port: `GET /<file>`, nothing outside `dir`.
pub async fn serve(dir: PathBuf) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            let (dir, seen) = (dir.clone(), seen.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if s.read(&mut b).await.unwrap_or(0) == 0 || head.len() > 16 * 1024 {
                        return;
                    }
                    head.push(b[0]);
                }
                seen.fetch_add(1, Ordering::SeqCst);
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let body = (!path.is_empty() && !path.contains("..") && !path.contains('\\'))
                    .then(|| std::fs::read(dir.join(path)).ok())
                    .flatten();
                let reply = match body {
                    Some(bytes) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&bytes);
                        r
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = s.write_all(&reply).await;
                let _ = s.shutdown().await;
            });
        }
    });
    Server { url, hits, task }
}
