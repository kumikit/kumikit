//! Choosing a version, and comparing permissions (`docs/spec/10-distribution.md#install`,
//! `#updates`).

use kumikit_protocol::methods::{FilesAccess, Limits, Permissions};

use crate::index::{Index, Plugin, Version};

/// Whether `range` (a manifest's `api`) accepts the API `version`.
pub fn api_accepts(range: &str, version: &str) -> bool {
    match (
        semver::VersionReq::parse(range),
        semver::Version::parse(version),
    ) {
        (Ok(r), Ok(v)) => r.matches(&v),
        _ => false,
    }
}

fn parsed(v: &Version) -> Option<semver::Version> {
    semver::Version::parse(&v.version).ok()
}

/// The plugin with this id, if the index lists it.
pub fn find<'a>(index: &'a Index, id: &str) -> Option<&'a Plugin> {
    index.plugins.iter().find(|p| p.id == id)
}

/// Why no version can be installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The version is not listed.
    #[error("{id} has no version {version} in this registry")]
    NoSuchVersion {
        /// Plugin id.
        id: String,
        /// The version asked for.
        version: String,
    },
    /// The version was withdrawn.
    #[error("{id} {version} was withdrawn by its publisher")]
    Yanked {
        /// Plugin id.
        id: String,
        /// The version.
        version: String,
    },
    /// No version targets an API this Kumikit has.
    #[error("no version of {id} works with this Kumikit (plugin API {api})")]
    Incompatible {
        /// Plugin id.
        id: String,
        /// This Kumikit's API version.
        api: String,
    },
}

/// The version to install: `wanted` if given (never a yanked one), otherwise the highest
/// version that is not yanked and whose `api` accepts `api`.
pub fn choose<'a>(
    plugin: &'a Plugin,
    wanted: Option<&str>,
    api: &str,
) -> Result<&'a Version, ResolveError> {
    if let Some(w) = wanted {
        let v = plugin
            .versions
            .iter()
            .find(|v| v.version == w)
            .ok_or_else(|| ResolveError::NoSuchVersion {
                id: plugin.id.clone(),
                version: w.to_owned(),
            })?;
        if v.yanked {
            return Err(ResolveError::Yanked {
                id: plugin.id.clone(),
                version: w.to_owned(),
            });
        }
        return Ok(v);
    }
    plugin
        .versions
        .iter()
        .filter(|v| !v.yanked && api_accepts(&v.api, api))
        .filter_map(|v| parsed(v).map(|s| (s, v)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v)
        .ok_or_else(|| ResolveError::Incompatible {
            id: plugin.id.clone(),
            api: api.to_owned(),
        })
}

/// The update for a plugin installed at `current`: the highest version above it that is
/// not yanked and whose `api` accepts `api`. Never a lower version.
pub fn update<'a>(plugin: &'a Plugin, current: &str, api: &str) -> Option<&'a Version> {
    let current = semver::Version::parse(current).ok()?;
    plugin
        .versions
        .iter()
        .filter(|v| !v.yanked && api_accepts(&v.api, api))
        .filter_map(|v| parsed(v).map(|s| (s, v)))
        .filter(|(s, _)| *s > current)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, v)| v)
}

fn files_rank(f: FilesAccess) -> u8 {
    match f {
        FilesAccess::None => 0,
        FilesAccess::ReadWorkspace => 1,
        FilesAccess::Workspace => 2,
    }
}

/// Whether `new` asks for anything `old` did not: a program, host or secret, more file
/// access, tools, or a higher limit (10-distribution, Updates).
fn workspaces_rank(w: kumikit_protocol::methods::WorkspacesAccess) -> u8 {
    use kumikit_protocol::methods::WorkspacesAccess as W;
    match w {
        W::None => 0,
        W::Linked => 1,
        W::All => 2,
    }
}

/// Whether `new` asks for more than `old`.
pub fn widens(old: &Permissions, new: &Permissions) -> bool {
    let adds = |a: &[String], b: &[String]| b.iter().any(|x| !a.contains(x));
    let raised = |a: Option<u32>, b: Option<u32>| match (a, b) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(x), Some(y)) => y > x,
    };
    let none = Limits {
        memory_mb: None,
        call_timeout_s: None,
    };
    let (ol, nl) = (old.limits.unwrap_or(none), new.limits.unwrap_or(none));
    adds(&old.spawn, &new.spawn)
        || adds(&old.network, &new.network)
        || adds(&old.secrets, &new.secrets)
        || files_rank(new.files) > files_rank(old.files)
        || (new.tools && !old.tools)
        || adds(&old.client, &new.client)
        || workspaces_rank(new.workspaces) > workspaces_rank(old.workspaces)
        || (new.schedules && !old.schedules)
        || raised(ol.memory_mb, nl.memory_mb)
        || raised(ol.call_timeout_s, nl.call_timeout_s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(v: &str, api: &str, yanked: bool) -> Version {
        Version {
            version: v.into(),
            url: format!("https://x/{v}.kkp"),
            sha256: "00".into(),
            api: api.into(),
            requested: None,
            ui: None,
            yanked,
        }
    }

    fn plugin(versions: Vec<Version>) -> Plugin {
        Plugin {
            id: "acme.probe".into(),
            name: "Probe".into(),
            description: None,
            repository: None,
            license: None,
            kind: vec!["provider".into()],
            versions,
        }
    }

    #[test]
    fn chooses_the_latest_compatible_unyanked_version() {
        let p = plugin(vec![
            version("0.1.0", "^0.1", false),
            version("0.10.0", "^0.1", false),
            version("0.11.0", "^0.1", true),
            version("1.0.0", "^1", false),
            version("0.9.0", "^0.1", false),
        ]);
        assert_eq!(choose(&p, None, "0.1.0").unwrap().version, "0.10.0");
        assert_eq!(choose(&p, Some("0.1.0"), "0.1.0").unwrap().version, "0.1.0");
        assert!(matches!(
            choose(&p, Some("0.11.0"), "0.1.0"),
            Err(ResolveError::Yanked { .. })
        ));
        assert!(matches!(
            choose(&p, Some("9.9.9"), "0.1.0"),
            Err(ResolveError::NoSuchVersion { .. })
        ));
        assert!(matches!(
            choose(&p, None, "2.0.0"),
            Err(ResolveError::Incompatible { .. })
        ));
    }

    #[test]
    fn updates_never_downgrade_and_skip_yanked() {
        let p = plugin(vec![
            version("0.1.0", "^0.1", false),
            version("0.2.0", "^0.1", true),
            version("0.3.0", "^0.1", false),
        ]);
        assert_eq!(update(&p, "0.1.0", "0.1.0").unwrap().version, "0.3.0");
        assert_eq!(update(&p, "0.3.0", "0.1.0"), None);
        assert_eq!(update(&p, "0.4.0", "0.1.0"), None, "never a lower version");
        let only_yanked = plugin(vec![
            version("0.1.0", "^0.1", false),
            version("0.2.0", "^0.1", true),
        ]);
        assert_eq!(update(&only_yanked, "0.1.0", "0.1.0"), None);
    }

    #[test]
    fn widening() {
        let base = Permissions {
            spawn: vec!["claude".into()],
            files: FilesAccess::ReadWorkspace,
            ..Permissions::default()
        };
        assert!(!widens(&base, &base));
        assert!(!widens(&base, &Permissions::default()), "narrower");
        let cases = [
            Permissions {
                spawn: vec!["claude".into(), "sh".into()],
                ..base.clone()
            },
            Permissions {
                network: vec!["api.example.com".into()],
                ..base.clone()
            },
            Permissions {
                secrets: vec!["k".into()],
                ..base.clone()
            },
            Permissions {
                files: FilesAccess::Workspace,
                ..base.clone()
            },
            Permissions {
                tools: true,
                ..base.clone()
            },
            Permissions {
                limits: Some(Limits {
                    memory_mb: Some(128),
                    call_timeout_s: None,
                }),
                ..base.clone()
            },
        ];
        for c in cases {
            assert!(widens(&base, &c), "{c:?}");
        }
        let high = Permissions {
            limits: Some(Limits {
                memory_mb: Some(128),
                call_timeout_s: Some(20),
            }),
            ..base.clone()
        };
        let lower = Permissions {
            limits: Some(Limits {
                memory_mb: Some(96),
                call_timeout_s: Some(20),
            }),
            ..base
        };
        assert!(!widens(&high, &lower));
    }
}
