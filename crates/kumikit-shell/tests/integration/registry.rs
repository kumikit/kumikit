//! UI plugins and themes from a registry (06-client, 10-distribution): the shell's client
//! registry against an index served on loopback, with keys made here. The index is built
//! directly from `kumikit-registry`'s types (the shell stays free of the plugin host).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kumikit_package::SigningKey;
use kumikit_registry::index::{FORMAT, Index, Plugin, Publisher, UiNeeds, Version};
use kumikit_shell::client_registry::{ClientRef, ClientRegistry};
use kumikit_shell::store::Store;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PANEL: &str = r#"
id = "acme.graph"
name = "Graph"
version = "VERSION"
kind = ["ui"]
api = "^0.1"
[ui]
entry = "index.html"
isolation = "iframe"
NETWORK
[contributes]
slots = [{ slot = "inspector.tab", id = "graph", title = "Graph" }]
"#;

struct Reg {
    _dir: tempfile::TempDir,
    root: PathBuf,
    www: PathBuf,
    url: String,
    hits: Arc<AtomicUsize>,
    pem: String,
    index: Index,
}

async fn serve(dir: PathBuf) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let (dir, seen) = (dir.clone(), seen.clone());
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if s.read(&mut b).await.unwrap_or(0) == 0 {
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
                let reply = match std::fs::read(dir.join(path)) {
                    Ok(bytes) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&bytes);
                        r
                    }
                    Err(_) => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = s.write_all(&reply).await;
            });
        }
    });
    (url, hits)
}

impl Reg {
    async fn new() -> Reg {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let www = root.join("www");
        std::fs::create_dir_all(&www).unwrap();
        let (url, hits) = serve(www.clone()).await;
        let (key, pem) = SigningKey::generate().unwrap();
        let index = Index {
            format: FORMAT,
            name: "Fixture".into(),
            generated: "2026-11-18T00:00:00Z".into(),
            starter: vec!["acme.graph".into()],
            publishers: BTreeMap::from([(
                "acme".into(),
                Publisher {
                    name: "Acme".into(),
                    key: key.public().to_string(),
                },
            )]),
            plugins: vec![Plugin {
                id: "acme.graph".into(),
                name: "Graph".into(),
                description: Some("A panel.".into()),
                repository: None,
                license: None,
                kind: vec!["ui".into()],
                versions: vec![],
            }],
        };
        Reg {
            _dir: dir,
            root,
            www,
            url,
            hits,
            pem,
            index,
        }
    }

    /// The publisher's key.
    fn key(&self) -> SigningKey {
        SigningKey::from_pem(&self.pem).unwrap()
    }

    fn index_url(&self) -> String {
        format!("{}/index.json", self.url)
    }

    /// Packs the panel at `version` (with `network` hosts), signed by `key`, and lists it.
    fn publish(&mut self, version: &str, network: &[&str], key: Option<&SigningKey>) {
        let src = self.root.join(format!("src-{version}-{}", network.len()));
        std::fs::create_dir_all(&src).unwrap();
        let hosts: Vec<String> = network.iter().map(|h| format!("{h:?}")).collect();
        let net = if hosts.is_empty() {
            String::new()
        } else {
            format!("network = [{}]", hosts.join(", "))
        };
        std::fs::write(
            src.join("plugin.toml"),
            PANEL.replace("VERSION", version).replace("NETWORK", &net),
        )
        .unwrap();
        std::fs::write(src.join("index.html"), "<!doctype html>").unwrap();
        let file = self.www.join(format!("acme.graph-{version}.kkp"));
        kumikit_package::pack_signed(&src, &file, key).unwrap();
        let bytes = std::fs::read(&file).unwrap();
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let versions = &mut self.index.plugins[0].versions;
        versions.retain(|v| v.version != version);
        versions.push(Version {
            version: version.into(),
            url: format!("{}/acme.graph-{version}.kkp", self.url),
            sha256: sha,
            api: "^0.1".into(),
            requested: None,
            ui: Some(UiNeeds {
                isolation: Some("iframe".into()),
                network: network.iter().map(|s| (*s).to_owned()).collect(),
                requires_core: vec![],
            }),
            yanked: false,
        });
        self.write();
    }

    fn write(&self) {
        std::fs::write(
            self.www.join("index.json"),
            serde_json::to_vec(&self.index).unwrap(),
        )
        .unwrap();
    }
}

fn client(reg: &Reg, dir: &Path) -> (ClientRegistry, Store) {
    let store = Store {
        bundled: None,
        installed: dir.join("ui-plugins"),
    };
    let url = reg.index_url();
    let env = move |k: &str| (k == "KUMIKIT_DEFAULT_REGISTRY").then(|| url.clone());
    (ClientRegistry::new(dir, store.clone(), &env), store)
}

fn by_id(id: &str) -> ClientRef {
    ClientRef {
        id: id.into(),
        version: None,
        sha256: None,
        registry: None,
    }
}

#[tokio::test]
async fn nothing_is_fetched_until_asked_then_install_checks_the_publisher() {
    let mut reg = Reg::new().await;
    let key = reg.key();
    reg.publish("0.1.0", &[], Some(&key));
    let dir = tempfile::tempdir().unwrap();
    let (registry, store) = client(&reg, dir.path());
    assert_eq!(registry.registries().await.unwrap(), [reg.index_url()]);
    assert_eq!(
        reg.hits.load(Ordering::SeqCst),
        0,
        "listing registries fetches nothing"
    );

    let found = registry.search("graph", false).await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].starter);
    assert_eq!(found[0].key, reg.key().public().to_string());
    assert_eq!(found[0].ui.isolation.as_deref(), Some("iframe"));

    let entry = registry.install(&by_id("acme.graph")).await.unwrap();
    assert_eq!(entry.version, "0.1.0");
    let publisher = entry.publisher.unwrap();
    assert_eq!(publisher.key, reg.key().public().to_string());
    assert_eq!(
        publisher.registry.as_deref(),
        Some(reg.index_url().as_str())
    );
    let lock = store.lock_text();
    assert!(
        lock.contains("id = \"acme.graph\"") && lock.contains(&reg.key().public().to_string()),
        "{lock}"
    );
    assert!(!lock.contains("granted"), "{lock}");
}

#[tokio::test]
async fn a_lying_registry_installs_nothing() {
    let mut reg = Reg::new().await;
    let dir = tempfile::tempdir().unwrap();
    let (registry, store) = client(&reg, dir.path());

    // Not signed.
    reg.publish("0.1.0", &[], None);
    let e = registry.install(&by_id("acme.graph")).await.unwrap_err();
    assert_eq!(e.code, "untrusted", "{e:?}");
    // Signed by someone else.
    let (other, _) = SigningKey::generate().unwrap();
    reg.publish("0.1.0", &[], Some(&other));
    let e = registry.install(&by_id("acme.graph")).await.unwrap_err();
    assert_eq!(e.code, "untrusted", "{e:?}");
    // The entry hides a network host the package declares.
    let key = reg.key();
    reg.publish("0.1.0", &["api.example.com"], Some(&key));
    reg.index.plugins[0].versions[0]
        .ui
        .as_mut()
        .unwrap()
        .network
        .clear();
    reg.write();
    let e = registry.install(&by_id("acme.graph")).await.unwrap_err();
    assert_eq!(e.code, "untrusted", "{e:?}");
    // The hash does not match.
    let key = reg.key();
    reg.publish("0.1.0", &[], Some(&key));
    reg.index.plugins[0].versions[0].sha256 = "0".repeat(64);
    reg.write();
    let e = registry.install(&by_id("acme.graph")).await.unwrap_err();
    assert_eq!(e.code, "untrusted", "{e:?}");

    assert!(store.list().is_empty(), "nothing was installed");
}

#[tokio::test]
async fn updates_are_proposed_wider_ones_flagged_and_keys_pinned() {
    let mut reg = Reg::new().await;
    let key = reg.key();
    reg.publish("0.1.0", &[], Some(&key));
    let dir = tempfile::tempdir().unwrap();
    let (registry, store) = client(&reg, dir.path());
    registry.install(&by_id("acme.graph")).await.unwrap();

    reg.publish("0.2.0", &["api.example.com"], Some(&key));
    let updates = registry.updates(true).await.unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        (updates[0].from.as_str(), updates[0].to.as_str()),
        ("0.1.0", "0.2.0")
    );
    assert!(updates[0].widens, "a new network host widens");
    assert_eq!(
        store.meta("acme.graph").unwrap().version,
        "0.1.0",
        "nothing applied"
    );
    let entry = registry.update("acme.graph", "0.2.0").await.unwrap();
    assert_eq!(entry.version, "0.2.0");
    assert!(!dir.path().join("ui-plugins/acme.graph/0.1.0").exists());

    // Another key cannot take over, even through the same registry.
    let (other, _) = SigningKey::generate().unwrap();
    reg.index.publishers.get_mut("acme").unwrap().key = other.public().to_string();
    reg.publish("0.3.0", &["api.example.com"], Some(&other));
    let e = registry.update("acme.graph", "0.3.0").await.unwrap_err();
    assert_eq!(e.code, "untrusted", "{e:?}");
    assert!(
        e.message.contains("remove the plugin and install it again"),
        "{e:?}"
    );
    assert_eq!(store.meta("acme.graph").unwrap().version, "0.2.0");

    // Never a lower version.
    let e = registry.update("acme.graph", "0.1.0").await.unwrap_err();
    assert_eq!(e.code, "conflict", "{e:?}");
}

#[test]
fn a_signed_file_install_pins_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store {
        bundled: None,
        installed: dir.path().join("ui-plugins"),
    };
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("plugin.toml"),
        PANEL.replace("VERSION", "0.1.0").replace("NETWORK", ""),
    )
    .unwrap();
    std::fs::write(src.join("index.html"), "<!doctype html>").unwrap();
    let (key, _) = SigningKey::generate().unwrap();
    let signed = dir.path().join("a.kkp");
    kumikit_package::pack_signed(&src, &signed, Some(&key)).unwrap();
    let entry = store.install(&signed).unwrap();
    assert_eq!(entry.publisher.unwrap().registry, None);

    // An unsigned 0.2.0 cannot replace it.
    std::fs::write(
        src.join("plugin.toml"),
        PANEL.replace("VERSION", "0.2.0").replace("NETWORK", ""),
    )
    .unwrap();
    let unsigned = dir.path().join("b.kkp");
    kumikit_package::pack(&src, &unsigned).unwrap();
    let e = store
        .replace("acme.graph", &unsigned, "file", None)
        .unwrap_err();
    assert!(e.to_string().contains("untrusted"), "{e}");
    // A tampered package is refused at install too.
    let tampered = dir.path().join("t");
    std::fs::create_dir_all(&tampered).unwrap();
    std::fs::write(
        tampered.join("plugin.toml"),
        PANEL
            .replace("VERSION", "0.1.0")
            .replace("NETWORK", "")
            .replace("acme.graph", "acme.other"),
    )
    .unwrap();
    std::fs::write(tampered.join("index.html"), "<!doctype html>").unwrap();
    std::fs::write(
        tampered.join("SIGNATURE"),
        kumikit_package::signature::sign_dir(&tampered, &key).unwrap(),
    )
    .unwrap();
    std::fs::write(tampered.join("index.html"), "<script>evil()</script>").unwrap();
    let t = dir.path().join("t.kkp");
    kumikit_package::pack(&tampered, &t).unwrap();
    assert!(
        store
            .install(&t)
            .unwrap_err()
            .to_string()
            .contains("untrusted")
    );
}
