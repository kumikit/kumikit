//! The plugin manifest, `plugin.toml` (`docs/spec/04-plugin-host.md#manifest`).
//!
//! Parsing validates the manifest against the JSON Schema published with the SDK
//! (`sdk/schema/plugin-manifest.schema.json`), then reads it into strict types (unknown
//! fields are refused, so a typo is an error, not a silently ignored setting), and
//! [`Manifest::check`] adds the rules a schema cannot express. Every problem found is
//! reported, not only the first.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

use kumikit_kernel::grants::{FilesGrant, Grants};
use serde::Deserialize;

use crate::engine::{CALL_TIMEOUT_CAP, MEMORY_CAP};

/// The kernel plugin API version; a manifest's `api` range must include it.
pub const PLUGIN_API: semver::Version = semver::Version::new(0, 1, 0);

/// The base world.
pub const WORLD_PLUGIN: &str = "kumikit:plugin/plugin@0.1.0";
/// The provider world.
pub const WORLD_PROVIDER: &str = "kumikit:provider/plugin@0.1.0";
/// Worlds this host can run.
pub const WORLDS: &[&str] = &[WORLD_PLUGIN, WORLD_PROVIDER];

/// Manifest file name inside a package.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// The manifest schema, as published in `sdk/schema/`.
pub const SCHEMA: &str = include_str!("../../../sdk/schema/plugin-manifest.schema.json");

fn schema_validator() -> Result<&'static jsonschema::Validator, ManifestError> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: serde_json::Value =
                serde_json::from_str(SCHEMA).map_err(|e| e.to_string())?;
            jsonschema::validator_for(&schema).map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| ManifestError::one(format!("the manifest schema is broken: {e}")))
}

/// Validates `instance` against a JSON Schema; returns one message per problem, each
/// prefixed with where it is (`/permissions/spawn/0: …`).
pub fn schema_problems(
    validator: &jsonschema::Validator,
    instance: &serde_json::Value,
) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|e| {
            let at = e.instance_path().to_string();
            if at.is_empty() {
                e.to_string()
            } else {
                format!("{at}: {e}")
            }
        })
        .collect()
}

/// Checks a delegated provider entry's table (`acp = { … }`) against the schema its
/// backend publishes. `schema` is the schema file's JSON text.
pub fn check_backend_entry(schema: &str, entry: &serde_json::Value) -> Result<(), ManifestError> {
    let schema: serde_json::Value = serde_json::from_str(schema)
        .map_err(|e| ManifestError::one(format!("backend schema is not JSON: {e}")))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|e| ManifestError::one(format!("backend schema is not a JSON Schema: {e}")))?;
    let problems = schema_problems(&validator, entry);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ManifestError { problems })
    }
}

/// A manifest that could not be read, with every problem found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid plugin manifest: {}", .problems.join("; "))]
pub struct ManifestError {
    /// What is wrong, one entry per problem.
    pub problems: Vec<String>,
}

impl ManifestError {
    fn one(problem: impl Into<String>) -> Self {
        ManifestError {
            problems: vec![problem.into()],
        }
    }
}

/// What a plugin is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Drives an agent.
    Provider,
    /// Contributes tools.
    Tool,
    /// A bot.
    Bot,
    /// An integration.
    Integration,
    /// A UI plugin (installed on clients, not cores).
    Ui,
    /// A theme (installed on clients, not cores).
    Theme,
}

/// A parsed `plugin.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Manifest {
    /// `<publisher>.<name>`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Semver version.
    pub version: String,
    /// One-line description.
    #[serde(default)]
    pub description: Option<String>,
    /// SPDX licence expression.
    #[serde(default)]
    pub license: Option<String>,
    /// Source repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// What the plugin is.
    pub kind: Vec<Kind>,
    /// The kernel plugin API range it targets.
    pub api: String,
    /// The component, if the plugin has code.
    #[serde(default)]
    pub runtime: Option<Runtime>,
    /// Extra activation triggers.
    #[serde(default)]
    pub activation: Activation,
    /// Requested permissions.
    #[serde(default)]
    pub permissions: Permissions,
    /// Requested limits.
    #[serde(default)]
    pub limits: Limits,
    /// Contributions.
    #[serde(default)]
    pub contributes: Contributes,
}

/// `[runtime]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Runtime {
    /// Component file, relative to the package root.
    #[serde(default)]
    pub wasm: Option<String>,
    /// A sidecar command (not supported yet; refused by [`Manifest::check`]).
    #[serde(default)]
    pub sidecar: Option<toml::Value>,
    /// Worlds the component implements.
    #[serde(default)]
    pub worlds: Vec<String>,
}

/// `[activation]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    /// Triggers: `startup`, `workspace`, `rpc`, `provider:<id>`, `command:<id>`,
    /// `schedule:<cron>`.
    #[serde(default)]
    pub on: Vec<String>,
}

/// Workspace file access requested.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Files {
    /// None.
    #[default]
    None,
    /// Read.
    ReadWorkspace,
    /// Read and write.
    Workspace,
}

/// `[permissions]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Permissions {
    /// Programs it may start (bare names).
    #[serde(default)]
    pub spawn: Vec<String>,
    /// Hosts it may connect to.
    #[serde(default)]
    pub network: Vec<String>,
    /// Workspace file access.
    #[serde(default)]
    pub files: Files,
    /// Secret names it may read.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Whether it may call other plugins' tools.
    #[serde(default)]
    pub tools: bool,
}

impl Permissions {
    /// The grants these permissions ask for.
    pub fn to_grants(&self) -> Grants {
        Grants {
            spawn: self.spawn.iter().cloned().collect(),
            network: self.network.iter().cloned().collect(),
            files: match self.files {
                Files::None => FilesGrant::None,
                Files::ReadWorkspace => FilesGrant::ReadWorkspace,
                Files::Workspace => FilesGrant::Workspace,
            },
            secrets: self.secrets.iter().cloned().collect(),
            tools: self.tools,
        }
    }
}

/// `[limits]`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Limits {
    /// Linear memory per instance, in MiB.
    #[serde(default)]
    pub memory_mb: Option<u32>,
    /// Guest time per call, in seconds.
    #[serde(default)]
    pub call_timeout_s: Option<u32>,
}

/// `[contributes]`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Contributes {
    /// Providers.
    #[serde(default)]
    pub providers: Vec<ProviderEntry>,
    /// Commands.
    #[serde(default)]
    pub commands: Vec<CommandEntry>,
    /// Settings schema file.
    #[serde(default)]
    pub settings: Option<String>,
    /// Backend keys this plugin serves for other plugins' provider entries.
    #[serde(default)]
    pub provider_backends: Vec<BackendEntry>,
}

/// A provider entry. Keys other than `id` and `name` name a provider backend.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderEntry {
    /// Local id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// At most one backend key and its table.
    #[serde(flatten)]
    pub backend: BTreeMap<String, serde_json::Value>,
}

impl ProviderEntry {
    /// The backend key and its table, if the entry is delegated.
    pub fn backend(&self) -> Option<(&str, &serde_json::Value)> {
        self.backend.iter().next().map(|(k, v)| (k.as_str(), v))
    }
}

/// A command entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandEntry {
    /// Command id.
    pub id: String,
    /// Title shown to users.
    pub title: String,
}

/// A provider backend this plugin serves.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendEntry {
    /// The key provider entries use.
    pub key: String,
    /// JSON Schema file for the entries' tables.
    #[serde(default)]
    pub schema: Option<String>,
}

fn is_word(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// `<publisher>.<name>`, each `[a-z0-9-]+`.
pub fn is_plugin_id(id: &str) -> bool {
    match id.split_once('.') {
        Some((publisher, name)) => is_word(publisher) && is_word(name),
        None => false,
    }
}

fn is_secret_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A bare program name: no path separators or drive prefix.
fn is_program_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', ':', '\0'])
}

/// A path inside the package: relative, no `..`.
fn is_package_path(s: &str) -> bool {
    let p = Path::new(s);
    !s.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

impl Manifest {
    /// Parses `plugin.toml` text: TOML syntax, then the JSON Schema, then the types.
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let value: toml::Value =
            toml::from_str(text).map_err(|e| ManifestError::one(e.message().to_owned()))?;
        let json = serde_json::to_value(&value).map_err(|e| ManifestError::one(e.to_string()))?;
        let problems = schema_problems(schema_validator()?, &json);
        if !problems.is_empty() {
            return Err(ManifestError { problems });
        }
        serde_json::from_value(json).map_err(|e| ManifestError::one(e.to_string()))
    }

    /// Reads an installed plugin's manifest: strict types and the semantic rules, but not
    /// the JSON Schema, which [`Manifest::load`] already applied at install. Starting the
    /// schema validator costs about 3 MB, which a core that only lists its plugins should
    /// not pay.
    pub fn load_installed(package: &Path) -> Result<Manifest, ManifestError> {
        let text = std::fs::read_to_string(package.join(MANIFEST_FILE))
            .map_err(|e| ManifestError::one(format!("cannot read {MANIFEST_FILE}: {e}")))?;
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| ManifestError::one(e.message().to_owned()))?;
        manifest.check(Some(package))?;
        Ok(manifest)
    }

    /// Parses and checks a manifest inside an unpacked package at `package`.
    pub fn load(package: &Path) -> Result<Manifest, ManifestError> {
        let text = std::fs::read_to_string(package.join(MANIFEST_FILE))
            .map_err(|e| ManifestError::one(format!("cannot read {MANIFEST_FILE}: {e}")))?;
        let manifest = Manifest::parse(&text)?;
        manifest.check(Some(package))?;
        Ok(manifest)
    }

    /// The parsed version.
    pub fn semver(&self) -> Option<semver::Version> {
        semver::Version::parse(&self.version).ok()
    }

    /// The world to link the component as: the provider world if it is declared,
    /// otherwise the base world. `None` for a plugin without a component.
    pub fn world(&self) -> Option<&'static str> {
        let rt = self.runtime.as_ref()?;
        rt.wasm.as_ref()?;
        Some(if rt.worlds.iter().any(|w| w == WORLD_PROVIDER) {
            WORLD_PROVIDER
        } else {
            WORLD_PLUGIN
        })
    }

    /// What it asks for, in protocol form: permissions plus limits raised above the
    /// defaults (what install shows and the registry copies).
    pub fn requested(&self) -> kumikit_protocol::methods::Permissions {
        use kumikit_protocol::methods::{FilesAccess, Limits as PLimits, Permissions as P};
        let sorted = |v: &[String]| {
            let mut v = v.to_vec();
            v.sort();
            v.dedup();
            v
        };
        let l = self.limits;
        P {
            spawn: sorted(&self.permissions.spawn),
            network: sorted(&self.permissions.network),
            files: match self.permissions.files {
                Files::None => FilesAccess::None,
                Files::ReadWorkspace => FilesAccess::ReadWorkspace,
                Files::Workspace => FilesAccess::Workspace,
            },
            secrets: sorted(&self.permissions.secrets),
            tools: self.permissions.tools,
            limits: (l.memory_mb.is_some() || l.call_timeout_s.is_some()).then_some(PLimits {
                memory_mb: l.memory_mb,
                call_timeout_s: l.call_timeout_s,
            }),
        }
    }

    /// Whether the plugin has a component.
    pub fn wasm(&self) -> Option<&str> {
        self.runtime.as_ref().and_then(|r| r.wasm.as_deref())
    }

    /// Requested linear memory per instance, in bytes.
    pub fn memory_limit(&self) -> usize {
        self.limits
            .memory_mb
            .map_or(crate::engine::DEFAULT_MEMORY, |mb| (mb as usize) << 20)
    }

    /// Requested guest time per call.
    pub fn call_timeout(&self) -> std::time::Duration {
        self.limits
            .call_timeout_s
            .map_or(crate::engine::DEFAULT_CALL_TIMEOUT, |s| {
                std::time::Duration::from_secs(u64::from(s))
            })
    }

    /// Checks every rule beyond the field types. With `package`, also checks that the
    /// files the manifest names exist in it.
    pub fn check(&self, package: Option<&Path>) -> Result<(), ManifestError> {
        let mut p = Vec::new();

        if !is_plugin_id(&self.id) {
            p.push(format!(
                "id `{}` must be `<publisher>.<name>`, lowercase letters, digits and '-'",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            p.push("name is empty".into());
        }
        if self.semver().is_none() {
            p.push(format!(
                "version `{}` is not a semver version",
                self.version
            ));
        }
        match semver::VersionReq::parse(&self.api) {
            Ok(req) if req.matches(&PLUGIN_API) => {}
            Ok(_) => p.push(format!(
                "api `{}` does not include this core's plugin API {PLUGIN_API}",
                self.api
            )),
            Err(_) => p.push(format!("api `{}` is not a version range", self.api)),
        }
        if self.kind.is_empty() {
            p.push("kind is empty".into());
        }
        if self
            .kind
            .iter()
            .any(|k| matches!(k, Kind::Ui | Kind::Theme))
        {
            p.push("UI plugins and themes are installed on a client, not on a core".into());
        }

        let mut worlds = BTreeSet::new();
        match &self.runtime {
            None => {}
            Some(rt) => {
                if rt.sidecar.is_some() {
                    p.push("sidecar plugins are not supported yet".into());
                }
                match &rt.wasm {
                    None if rt.sidecar.is_none() => p.push("runtime.wasm is missing".into()),
                    None => {}
                    Some(w) if !is_package_path(w) => p.push(format!(
                        "runtime.wasm `{w}` must be a path inside the package"
                    )),
                    Some(w) => {
                        if let Some(dir) = package
                            && !dir.join(w).is_file()
                        {
                            p.push(format!("runtime.wasm `{w}` is not in the package"));
                        }
                    }
                }
                if rt.worlds.is_empty() {
                    p.push("runtime.worlds is empty".into());
                }
                for w in &rt.worlds {
                    if WORLDS.contains(&w.as_str()) {
                        worlds.insert(w.as_str());
                    } else {
                        p.push(format!(
                            "world `{w}` is not supported (supported: {})",
                            WORLDS.join(", ")
                        ));
                    }
                }
            }
        }

        let mut provider_ids = BTreeSet::new();
        for entry in &self.contributes.providers {
            if !is_word(&entry.id) {
                p.push(format!(
                    "provider id `{}` must be lowercase letters, digits and '-'",
                    entry.id
                ));
            }
            if !provider_ids.insert(entry.id.as_str()) {
                p.push(format!("provider id `{}` appears twice", entry.id));
            }
            match entry.backend.len() {
                0 => {
                    if !worlds.contains(WORLD_PROVIDER) {
                        p.push(format!(
                            "provider `{}` has no backend key, so the plugin must implement {WORLD_PROVIDER}",
                            entry.id
                        ));
                    }
                }
                1 => {
                    let (key, table) = entry.backend().unwrap_or(("", &serde_json::Value::Null));
                    if !is_word(key) {
                        p.push(format!(
                            "backend key `{key}` in provider `{}` is not valid",
                            entry.id
                        ));
                    }
                    if !table.is_object() {
                        p.push(format!(
                            "`{key}` in provider `{}` must be a table",
                            entry.id
                        ));
                    }
                }
                _ => p.push(format!(
                    "provider `{}` has several backend keys ({}); it may have one",
                    entry.id,
                    entry.backend.keys().cloned().collect::<Vec<_>>().join(", ")
                )),
            }
        }
        let mut keys = BTreeSet::new();
        for b in &self.contributes.provider_backends {
            if !is_word(&b.key) || b.key == "id" || b.key == "name" {
                p.push(format!("provider backend key `{}` is not valid", b.key));
            }
            if !keys.insert(b.key.as_str()) {
                p.push(format!("provider backend key `{}` appears twice", b.key));
            }
            if !worlds.contains(WORLD_PROVIDER) {
                p.push(format!(
                    "a plugin that serves provider backends must implement {WORLD_PROVIDER}"
                ));
            }
            if let Some(schema) = &b.schema {
                check_file(&mut p, package, "provider backend schema", schema);
            }
        }
        if let Some(settings) = &self.contributes.settings {
            check_file(&mut p, package, "settings schema", settings);
        }
        if self.runtime.is_none() && !self.contributes.provider_backends.is_empty() {
            p.push("provider backends need a runtime".into());
        }

        for trigger in &self.activation.on {
            let ok = match trigger.split_once(':') {
                None => matches!(trigger.as_str(), "startup" | "workspace" | "rpc"),
                Some(("provider", id)) => provider_ids.contains(id),
                Some(("command", id)) => !id.is_empty(),
                Some(("schedule", cron)) => !cron.trim().is_empty(),
                Some(_) => false,
            };
            if !ok {
                p.push(format!("activation trigger `{trigger}` is not valid"));
            }
        }
        if self.runtime.is_none() && !self.activation.on.is_empty() {
            p.push("activation triggers need a runtime".into());
        }

        for prog in &self.permissions.spawn {
            if !is_program_name(prog) {
                p.push(format!(
                    "permissions.spawn entry `{prog}` must be a bare program name, not a path"
                ));
            }
        }
        for s in &self.permissions.secrets {
            if !is_secret_name(s) {
                p.push(format!("secret name `{s}` is not valid"));
            }
        }
        if let Some(mb) = self.limits.memory_mb {
            let cap = MEMORY_CAP >> 20;
            if mb == 0 || mb as usize > cap {
                p.push(format!("limits.memory-mb must be between 1 and {cap}"));
            }
        }
        if let Some(s) = self.limits.call_timeout_s {
            let cap = CALL_TIMEOUT_CAP.as_secs();
            if s == 0 || u64::from(s) > cap {
                p.push(format!("limits.call-timeout-s must be between 1 and {cap}"));
            }
        }

        if p.is_empty() {
            Ok(())
        } else {
            Err(ManifestError { problems: p })
        }
    }
}

fn check_file(p: &mut Vec<String>, package: Option<&Path>, what: &str, path: &str) {
    if !is_package_path(path) {
        p.push(format!("{what} `{path}` must be a path inside the package"));
    } else if let Some(dir) = package
        && !dir.join(path).is_file()
    {
        p.push(format!("{what} `{path}` is not in the package"));
    }
}

#[cfg(test)]
mod tests;
