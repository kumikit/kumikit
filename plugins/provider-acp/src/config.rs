//! The `acp` table of a provider entry; its schema is `acp.schema.json` in this plugin.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A model offered by an ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    /// Model id, passed to the agent with `model-arg` if set.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Capability strings.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// How to run one ACP agent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    /// Program name (must be in the declaring plugin's `permissions.spawn`).
    pub command: String,
    /// Arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Plain environment variables.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Variables set from secrets: variable name -> secret name.
    #[serde(default)]
    pub secret_env: BTreeMap<String, String>,
    /// Models to offer; default one model, `default`.
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Flag that selects a model on the command line (for example `--model`); the
    /// chosen model id follows it. Without it the agent picks its model itself.
    #[serde(default)]
    pub model_arg: Option<String>,
    /// Session mode to set after opening (`session/set_mode`), for agents whose default
    /// mode never asks for approval.
    #[serde(default)]
    pub mode: Option<String>,
}

impl Config {
    /// Parses the table (as JSON, the way the host passes it).
    pub fn parse(text: &str) -> Result<Config, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid acp table: {e}"))
    }

    /// The models to offer.
    pub fn models(&self) -> Vec<ModelEntry> {
        if self.models.is_empty() {
            vec![ModelEntry {
                id: "default".into(),
                label: "Default".into(),
                capabilities: Vec::new(),
            }]
        } else {
            self.models.clone()
        }
    }

    /// The command-line arguments for a session with `model`.
    pub fn args_for(&self, model: &str) -> Vec<String> {
        let mut args = self.args.clone();
        if let Some(flag) = &self.model_arg
            && model != "default"
        {
            args.push(flag.clone());
            args.push(model.to_owned());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_and_full_tables() {
        let c = Config::parse(r#"{"command":"claude-agent-acp"}"#).unwrap();
        assert_eq!(c.models()[0].id, "default");
        assert!(c.args_for("default").is_empty());
        let c = Config::parse(
            r#"{"command":"gemini","args":["--experimental-acp"],"model-arg":"--model",
                "models":[{"id":"pro","label":"Pro"}],"mode":"default","secret-env":{"KEY":"k"}}"#,
        )
        .unwrap();
        assert_eq!(c.args_for("pro"), ["--experimental-acp", "--model", "pro"]);
        assert!(Config::parse(r#"{"command":"x","unknown":1}"#).is_err());
    }
}
