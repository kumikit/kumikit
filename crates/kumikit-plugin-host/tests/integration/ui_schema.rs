//! The published JSON Schema of UI plugin and theme manifests
//! (`sdk/schema/ui-plugin-manifest.schema.json`, 07-ui-plugins) accepts the first-party and
//! test manifests and refuses what the app's store refuses by structure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

fn validator() -> jsonschema::Validator {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text =
        std::fs::read_to_string(root.join("sdk/schema/ui-plugin-manifest.schema.json")).unwrap();
    jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap()
}

fn manifest(path: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(path)).unwrap();
    serde_json::to_value(toml::from_str::<toml::Value>(&text).unwrap()).unwrap()
}

#[test]
fn first_party_and_test_manifests_validate() {
    let v = validator();
    for path in [
        "plugins/ui-chat/plugin.toml",
        "plugins/ui-settings/plugin.toml",
        "plugins/ui-usage/plugin.toml",
        "test-plugins/panel-smoke/plugins/test.panel-smoke/plugin.toml",
    ] {
        let m = manifest(path);
        let errors: Vec<String> = v.iter_errors(&m).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{path}: {errors:?}");
    }
}

#[test]
fn bundled_themes_validate_against_theme_schema() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema_text = std::fs::read_to_string(root.join("sdk/schema/theme.schema.json")).unwrap();
    let v = jsonschema::validator_for(&serde_json::from_str(&schema_text).unwrap()).unwrap();
    let mut count = 0;
    for entry in std::fs::read_dir(root.join("themes")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let errors: Vec<String> = v.iter_errors(&doc).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{}: {errors:?}", path.display());
        count += 1;
    }
    assert!(count >= 20, "expected bundled themes, found {count}");
}

#[test]
fn wrong_structures_are_refused() {
    let v = validator();
    let base = manifest("test-plugins/panel-smoke/plugins/test.panel-smoke/plugin.toml");
    type Change = fn(&mut serde_json::Value);
    let cases: [(&str, Change); 4] = [
        ("unknown field", |m| m["surprise"] = true.into()),
        ("core kind", |m| m["kind"] = serde_json::json!(["provider"])),
        ("isolation", |m| m["ui"]["isolation"] = "worker".into()),
        ("permissions", |m| {
            m["permissions"] = serde_json::json!({"spawn": ["sh"]})
        }),
    ];
    for (name, change) in cases {
        let mut m = base.clone();
        change(&mut m);
        assert!(!v.is_valid(&m), "{name} should be refused");
    }
}
