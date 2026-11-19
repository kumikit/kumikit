//! The registry checker against fixture registries on loopback: every refusal the
//! registry's CI makes (10-distribution, Registry).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use kumikit_package::SigningKey;
use kumikit_registry::fetch::Fetcher;
use kumikit_registry::index::parse_index;
use kumikit_registry_tool::{CheckOptions, build_index, check};
use kumikit_test_registry_fixture::{Fixture, Server, serve};

const CORE: &str = r#"
id = "acme.probe"
name = "Probe"
version = "0.1.0"
kind = ["provider"]
api = "^0.2"
[permissions]
spawn = ["fake-acp-agent"]
[contributes]
providers = [{ id = "probe", name = "Probe", acp = { command = "fake-acp-agent" } }]
"#;

const PANEL: &str = r#"
id = "acme.graph"
name = "Graph"
version = "0.1.0"
kind = ["ui"]
api = "^0.1.1"
[ui]
entry = "index.html"
isolation = "iframe"
network = ["api.example.com"]
[requires]
core = ["acme.probe@^0.1"]
[contributes]
slots = [{ slot = "inspector.tab", id = "graph", title = "Graph" }]
"#;

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    fixture: Fixture,
    _server: Server,
    key: SigningKey,
}

fn src(root: &Path, name: &str, manifest: &str, extra: &[(&str, &str)]) -> PathBuf {
    let dir = root.join("src").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
    for (p, text) in extra {
        std::fs::write(dir.join(p), text).unwrap();
    }
    dir
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let www = root.join("www");
    std::fs::create_dir_all(&www).unwrap();
    let server = serve(www).await;
    let fixture = Fixture::new(&root, &server.url);
    let (key, _) = SigningKey::generate().unwrap();
    fixture.publisher("acme", &key, &["acme-bot"]);
    fixture.publish(&src(&root, "probe", CORE, &[]), Some(&key));
    fixture.publish(
        &src(&root, "graph", PANEL, &[("index.html", "<!doctype html>")]),
        Some(&key),
    );
    fixture.starter(&["acme.probe"]);
    Setup {
        _dir: dir,
        root,
        fixture,
        _server: server,
        key,
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// The checkout as it is now, kept as the "base" (the default branch).
fn snapshot(s: &Setup) -> PathBuf {
    let base = s.root.join(format!("base-{}", rand_suffix()));
    copy_dir(&s.fixture.repo, &base);
    base
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

async fn errors(s: &Setup, base: Option<&Path>, options: &CheckOptions) -> Vec<String> {
    check(&s.fixture.repo, base, options, &Fetcher::default()).await
}

fn has(errors: &[String], needle: &str) -> bool {
    errors.iter().any(|e| e.contains(needle))
}

#[tokio::test]
async fn a_good_registry_passes_and_builds_an_index() {
    let s = setup().await;
    let e = errors(
        &s,
        None,
        &CheckOptions {
            author: Some("acme-bot".into()),
            require_early: true,
        },
    )
    .await;
    assert!(e.is_empty(), "{e:#?}");
    let index = build_index(&s.fixture.repo, "Fixture", "2026-11-18T00:00:00Z").unwrap();
    let index = parse_index(&serde_json::to_vec(&index).unwrap()).unwrap();
    assert_eq!(index.starter, ["acme.probe"]);
    assert_eq!(index.publishers["acme"].key, s.key.public().to_string());
    let probe = index.plugins.iter().find(|p| p.id == "acme.probe").unwrap();
    assert_eq!(
        probe.versions[0].requested.as_ref().unwrap().spawn,
        ["fake-acp-agent"]
    );
    let graph = index.plugins.iter().find(|p| p.id == "acme.graph").unwrap();
    let ui = graph.versions[0].ui.as_ref().unwrap();
    assert_eq!(ui.isolation.as_deref(), Some("iframe"));
    assert_eq!(ui.network, ["api.example.com"]);
    assert_eq!(ui.requires_core, ["acme.probe@^0.1"]);
}

#[tokio::test]
async fn a_wrong_hash_fails() {
    let s = setup().await;
    s.fixture
        .edit_plugin("acme.probe", |f| f.versions[0].sha256 = "0".repeat(64));
    assert!(has(
        &errors(&s, None, &CheckOptions::default()).await,
        "SHA-256"
    ));
}

#[tokio::test]
async fn unsigned_version_fails() {
    let s = setup().await;
    let v2 = CORE.replace("0.1.0", "0.2.0");
    s.fixture.publish(&src(&s.root, "probe2", &v2, &[]), None);
    let e = errors(&s, None, &CheckOptions::default()).await;
    assert!(
        has(&e, "acme.probe 0.2.0: the package is not signed"),
        "{e:#?}"
    );
}

#[tokio::test]
async fn another_key_fails() {
    let s = setup().await;
    let (other, _) = SigningKey::generate().unwrap();
    let v2 = CORE.replace("0.1.0", "0.2.0");
    s.fixture
        .publish(&src(&s.root, "probe2", &v2, &[]), Some(&other));
    let e = errors(&s, None, &CheckOptions::default()).await;
    assert!(has(&e, "not by the publisher's key"), "{e:#?}");
}

#[tokio::test]
async fn copied_fields_must_match() {
    let s = setup().await;
    s.fixture.edit_plugin("acme.probe", |f| {
        f.versions[0].permissions.as_mut().unwrap().spawn.clear();
    });
    s.fixture
        .edit_plugin("acme.graph", |f| f.versions[0].network.clear());
    let e = errors(&s, None, &CheckOptions::default()).await;
    assert!(
        has(
            &e,
            "acme.probe 0.1.0: `permissions` and `limits` must be copied"
        ),
        "{e:#?}"
    );
    assert!(
        has(&e, "acme.graph 0.1.0: `isolation`, `network`"),
        "{e:#?}"
    );
}

#[tokio::test]
async fn publisher_key_change_fails() {
    let s = setup().await;
    let base = snapshot(&s);
    let (other, _) = SigningKey::generate().unwrap();
    s.fixture.publisher("acme", &other, &["acme-bot"]);
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(has(&e, "the publisher key changed"), "{e:#?}");
}

#[tokio::test]
async fn published_versions_are_immutable_but_may_be_yanked() {
    let s = setup().await;
    let base = snapshot(&s);
    s.fixture
        .edit_plugin("acme.probe", |f| f.versions[0].yanked = true);
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(e.is_empty(), "yanking is allowed: {e:#?}");

    s.fixture
        .edit_plugin("acme.probe", |f| f.versions[0].url.push_str("?x"));
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(has(&e, "a published version cannot change"), "{e:#?}");

    s.fixture.edit_plugin("acme.probe", |f| f.versions.clear());
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(has(&e, "cannot be removed"), "{e:#?}");
}

#[tokio::test]
async fn confusable_ids_are_refused_unless_reviewed() {
    let s = setup().await;
    let base = snapshot(&s);
    let twin = CORE.replace("acme.probe", "acme.prob3");
    s.fixture
        .publish(&src(&s.root, "twin", &twin, &[]), Some(&s.key));
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(
        has(
            &e,
            "acme.prob3: `acme.prob3` is one letter away from `acme.probe`"
        ),
        "{e:#?}"
    );
    std::fs::write(
        s.fixture.repo.join("allow-confusable.toml"),
        "[[allow]]\nid = \"acme.prob3\"\nlike = \"acme.probe\"\n",
    )
    .unwrap();
    let e = errors(&s, Some(&base), &CheckOptions::default()).await;
    assert!(e.is_empty(), "{e:#?}");
}

#[tokio::test]
async fn only_listed_authors_may_change_a_publisher() {
    let s = setup().await;
    let base = snapshot(&s);
    let v2 = CORE.replace("0.1.0", "0.2.0");
    s.fixture
        .publish(&src(&s.root, "probe2", &v2, &[]), Some(&s.key));
    let ok = CheckOptions {
        author: Some("acme-bot".into()),
        require_early: false,
    };
    assert!(errors(&s, Some(&base), &ok).await.is_empty());
    let stranger = CheckOptions {
        author: Some("mallory".into()),
        require_early: false,
    };
    let e = errors(&s, Some(&base), &stranger).await;
    assert!(
        has(&e, "mallory may not change the files of publisher `acme`"),
        "{e:#?}"
    );
    // Adding yourself to the list in the same pull request does not help.
    s.fixture
        .publisher("acme", &s.key, &["acme-bot", "mallory"]);
    let e = errors(&s, Some(&base), &stranger).await;
    assert!(has(&e, "mallory may not change"), "{e:#?}");
}

#[tokio::test]
async fn reserved_and_unlisted_publishers_fail() {
    let s = setup().await;
    let (k, _) = SigningKey::generate().unwrap();
    s.fixture.publisher("test", &k, &[]);
    s.fixture.publisher("kumikti", &k, &[]);
    let late = s.fixture.repo.join("publishers/late.toml");
    std::fs::write(
        &late,
        format!("name = \"Late\"\nkey = \"{}\"\n", k.public()),
    )
    .unwrap();
    let e = errors(
        &s,
        None,
        &CheckOptions {
            author: None,
            require_early: true,
        },
    )
    .await;
    assert!(has(&e, "`test` is reserved"), "{e:#?}");
    assert!(
        has(&e, "publishers/late.toml: before 1.0 only early publishers"),
        "{e:#?}"
    );
}

#[tokio::test]
async fn starter_ids_must_exist() {
    let s = setup().await;
    s.fixture.starter(&["acme.missing"]);
    let e = errors(&s, None, &CheckOptions::default()).await;
    assert!(
        has(&e, "starter.toml: acme.missing is not in the registry"),
        "{e:#?}"
    );
}
