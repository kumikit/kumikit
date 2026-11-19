use std::sync::Arc;

use super::*;
use crate::KernelConfig;
use crate::clock::{SequentialIds, StepClock};

async fn kernel(dir: &tempfile::TempDir) -> Kernel {
    Kernel::open(KernelConfig {
        clock: Arc::new(StepClock::default()),
        ids: Arc::new(SequentialIds::new("ID")),
        ..KernelConfig::new(dir.path().join("core.sqlite"))
    })
    .await
    .unwrap()
}

fn record(id: &str) -> PluginRecord {
    PluginRecord {
        id: id.into(),
        version: "0.1.0".into(),
        state: PluginState::Installed,
        sha256: "ab".into(),
        granted: None,
        installed: Timestamp("2026-11-15T00:00:00Z".into()),
        publisher_key: None,
        source: PluginSource::File,
    }
}

#[tokio::test]
async fn the_publisher_key_and_source_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let mut signed = record("acme.probe");
    signed.publisher_key = Some("ed25519:abc".into());
    signed.source = PluginSource::Registry("https://r.example/index.json".into());
    k.plugin_insert(signed.clone()).await.unwrap();
    let mut bundled = record("kumikit.x");
    bundled.source = PluginSource::Bundled;
    k.plugin_insert(bundled.clone()).await.unwrap();
    k.close().await;
    let k = kernel(&dir).await;
    assert_eq!(k.plugin_get("acme.probe").await.unwrap(), Some(signed));
    assert_eq!(k.plugin_get("kumikit.x").await.unwrap(), Some(bundled));
    for s in ["file", "bundled", "registry+https://x/i.json"] {
        assert_eq!(PluginSource::from_stored(s).as_stored(), s);
    }
}

#[tokio::test]
async fn records_round_trip_and_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    k.plugin_insert(record("b.two")).await.unwrap();
    k.plugin_insert(record("a.one")).await.unwrap();
    let err = k.plugin_insert(record("a.one")).await.unwrap_err();
    assert!(matches!(err, KernelError::Conflict(_)));

    let mut grants = Grants::default();
    grants.spawn.insert("gemini".into());
    k.plugin_set_state("a.one", PluginState::Enabled, Some(grants.clone()))
        .await
        .unwrap();
    k.close().await;

    let k = kernel(&dir).await;
    let list = k.plugin_list().await.unwrap();
    assert_eq!(
        list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["a.one", "b.two"]
    );
    assert_eq!(list[0].state, PluginState::Enabled);
    assert_eq!(list[0].granted.as_ref(), Some(&grants));
    assert_eq!(list[1].granted, None);
    assert!(matches!(
        k.plugin_set_state("nope", PluginState::Enabled, None).await,
        Err(KernelError::NotFound(_))
    ));
}

#[tokio::test]
async fn kv_is_namespaced_and_listed_by_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    k.kv_set("a", "s/1", b"x".to_vec()).await.unwrap();
    k.kv_set("a", "s/2", b"y".to_vec()).await.unwrap();
    k.kv_set("a", "t", b"z".to_vec()).await.unwrap();
    k.kv_set("b", "s/1", b"other".to_vec()).await.unwrap();
    k.kv_set("a", "s/1", b"x2".to_vec()).await.unwrap();

    assert_eq!(
        k.kv_get("a", "s/1").await.unwrap().as_deref(),
        Some(&b"x2"[..])
    );
    assert_eq!(
        k.kv_get("b", "s/1").await.unwrap().as_deref(),
        Some(&b"other"[..])
    );
    assert_eq!(k.kv_list("a", "s/").await.unwrap(), ["s/1", "s/2"]);
    assert_eq!(k.kv_list("a", "").await.unwrap(), ["s/1", "s/2", "t"]);
    k.kv_delete("a", "s/1").await.unwrap();
    k.kv_delete("a", "missing").await.unwrap();
    assert_eq!(k.kv_get("a", "s/1").await.unwrap(), None);
}

#[tokio::test]
async fn kv_limits() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let long_key = "k".repeat(KV_KEY_MAX + 1);
    assert!(matches!(
        k.kv_get("a", &long_key).await,
        Err(KernelError::Limit(_))
    ));
    let big = vec![0u8; KV_VALUE_MAX + 1];
    assert!(matches!(
        k.kv_set("a", "k", big).await,
        Err(KernelError::Limit(_))
    ));
    k.kv_set("a", "k", vec![0u8; KV_VALUE_MAX]).await.unwrap();
}

#[tokio::test]
async fn deleting_a_plugin_keeps_or_deletes_its_data() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    for id in ["a.keep", "a.drop"] {
        k.plugin_insert(record(id)).await.unwrap();
        k.kv_set(id, "k", b"v".to_vec()).await.unwrap();
    }
    k.plugin_delete("a.keep", false).await.unwrap();
    k.plugin_delete("a.drop", true).await.unwrap();
    assert!(k.kv_get("a.keep", "k").await.unwrap().is_some());
    assert!(k.kv_get("a.drop", "k").await.unwrap().is_none());
    assert!(k.plugin_list().await.unwrap().is_empty());
}

#[tokio::test]
async fn an_update_replaces_the_record_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let k = kernel(&dir).await;
    let mut r = record("acme.probe");
    r.publisher_key = Some("ed25519:old".into());
    k.plugin_insert(r.clone()).await.unwrap();
    let mut next = r.clone();
    next.version = "0.2.0".into();
    next.sha256 = "cd".into();
    next.state = PluginState::Enabled;
    next.granted = Some(Grants::default());
    next.source = PluginSource::Registry("https://r.example/index.json".into());
    k.plugin_replace(next.clone()).await.unwrap();
    let got = k.plugin_get("acme.probe").await.unwrap().unwrap();
    assert_eq!(got.version, "0.2.0");
    assert_eq!(got.installed, r.installed, "the install time stays");
    assert_eq!(got.source, next.source);
    assert!(matches!(
        k.plugin_replace(record("no.such")).await,
        Err(KernelError::NotFound(_))
    ));
}
