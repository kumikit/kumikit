//! Plugin settings and secret names for clients (`docs/spec/03-kernel.md#plugin-settings`).
//!
//! A plugin's settings schema is a JSON Schema file in its package. Clients read it, the
//! values (stored by the kernel, defaults filled in) and set them, validated here. A
//! property with `"x-secret": "<name>"` is a secret field: its value lives in the secret
//! store under that name, never with the settings.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::manager::HostError;
use crate::manifest::{Manifest, schema_problems};

/// The secret names a schema's top-level properties map to (`x-secret`), by property.
pub fn secret_fields(schema: &Value) -> BTreeMap<String, String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .filter_map(|(k, v)| {
                    let name = v.get("x-secret")?.as_str()?;
                    Some((k.clone(), name.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Problems with a settings schema file's text, for the manifest check: it must be a
/// JSON Schema for an object, and each `x-secret` must name one of `secrets`.
pub fn schema_file_problems(text: &str, secrets: &[String]) -> Vec<String> {
    let schema: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return vec![format!("settings schema is not JSON: {e}")],
    };
    let mut p = Vec::new();
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        p.push("settings schema must describe an object (`\"type\": \"object\"`)".into());
    }
    if let Err(e) = jsonschema::validator_for(&schema) {
        p.push(format!("settings schema is not a JSON Schema: {e}"));
    }
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (key, prop) in props {
            match prop.get("x-secret") {
                None => {}
                Some(Value::String(name)) if secrets.contains(name) => {}
                Some(Value::String(name)) => p.push(format!(
                    "settings field `{key}` is the secret `{name}`, which is not in permissions.secrets"
                )),
                Some(_) => p.push(format!("settings field `{key}`: `x-secret` must be a secret name")),
            }
        }
    }
    p
}

/// The schema without its secret fields: what settings values are validated against.
fn without_secrets(schema: &Value) -> Value {
    let secret = secret_fields(schema);
    let mut s = schema.clone();
    if let Some(props) = s.get_mut("properties").and_then(Value::as_object_mut) {
        props.retain(|k, _| !secret.contains_key(k));
    }
    if let Some(req) = s.get_mut("required").and_then(Value::as_array_mut) {
        req.retain(|k| k.as_str().is_none_or(|k| !secret.contains_key(k)));
    }
    s
}

/// `stored` (or nothing) with the schema's top-level defaults filled in, secret fields
/// left out.
pub fn with_defaults(schema: &Value, stored: Option<Value>) -> Value {
    let secret = secret_fields(schema);
    let mut values = match stored {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    values.retain(|k, _| !secret.contains_key(k));
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (k, prop) in props {
            if secret.contains_key(k) || values.contains_key(k) {
                continue;
            }
            if let Some(d) = prop.get("default") {
                values.insert(k.clone(), d.clone());
            }
        }
    }
    Value::Object(values)
}

/// Checks values a client wants to store.
pub fn check_values(schema: &Value, values: &Value) -> Result<(), HostError> {
    let Some(obj) = values.as_object() else {
        return Err(HostError::Invalid(
            "settings values must be an object".into(),
        ));
    };
    let secret = secret_fields(schema);
    if let Some(k) = obj.keys().find(|k| secret.contains_key(*k)) {
        return Err(HostError::Invalid(format!(
            "settings field `{k}` is a secret; set it with secret.set"
        )));
    }
    let validator = jsonschema::validator_for(&without_secrets(schema))
        .map_err(|e| HostError::Invalid(format!("the plugin's settings schema is broken: {e}")))?;
    let problems = schema_problems(&validator, values);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(HostError::Invalid(problems.join("; ")))
    }
}

/// The secret names installed plugins declare, each with the plugins that declare it.
pub fn declared_secrets<'a>(
    manifests: impl IntoIterator<Item = &'a Manifest>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in manifests {
        for s in &m.permissions.secrets {
            out.entry(s.clone()).or_default().insert(m.id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "token": { "type": "string", "x-secret": "bot-token" },
                "limit": { "type": "integer", "default": 10 },
                "name": { "type": "string" }
            },
            "required": ["token", "name"]
        })
    }

    #[test]
    fn schema_files_are_checked() {
        let ok = schema().to_string();
        assert!(schema_file_problems(&ok, &["bot-token".into()]).is_empty());
        let p = schema_file_problems(&ok, &[]);
        assert!(p[0].contains("not in permissions.secrets"), "{p:?}");
        assert!(schema_file_problems("[]", &[])[0].contains("object"));
        assert!(schema_file_problems("{", &[])[0].contains("not JSON"));
    }

    #[test]
    fn defaults_fill_in_and_secrets_stay_out() {
        let v = with_defaults(&schema(), Some(json!({ "name": "x", "token": "leak" })));
        assert_eq!(v, json!({ "name": "x", "limit": 10 }));
    }

    #[test]
    fn values_are_validated_without_secret_fields() {
        check_values(&schema(), &json!({ "name": "x" })).unwrap();
        let e = check_values(&schema(), &json!({ "name": "x", "limit": "many" })).unwrap_err();
        assert!(e.to_string().contains("/limit"), "{e}");
        let e = check_values(&schema(), &json!({ "name": "x", "token": "t" })).unwrap_err();
        assert!(e.to_string().contains("secret.set"), "{e}");
        assert!(check_values(&schema(), &json!([])).is_err());
    }
}
