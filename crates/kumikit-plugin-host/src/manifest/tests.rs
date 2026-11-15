use super::*;

const PROVIDER: &str = r#"
id = "kumikit.provider-claude"
name = "Claude Code"
version = "0.3.0"
description = "Run Claude Code as a Kumikit provider."
license = "MIT"
repository = "https://github.com/kumikit/provider-claude"
kind = ["provider"]
api = "^0.1"

[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:provider/plugin@0.1.0"]

[activation]
on = ["startup", "provider:claude"]

[permissions]
spawn = ["claude"]
files = "workspace"
secrets = ["anthropic-api-key"]

[limits]
memory-mb = 128

[contributes]
providers = [{ id = "claude", name = "Claude Code" }]
commands = [{ id = "claude.login", title = "Sign in" }]
"#;

const DECLARATIVE: &str = r#"
id = "community.provider-gemini"
name = "Gemini CLI"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[permissions]
spawn = ["gemini"]
[contributes]
providers = [{ id = "gemini", name = "Gemini CLI", acp = { command = "gemini", args = ["--experimental-acp"] } }]
"#;

fn problems(text: &str) -> Vec<String> {
    match Manifest::parse(text) {
        Err(e) => e.problems,
        Ok(m) => m.check(None).err().map(|e| e.problems).unwrap_or_default(),
    }
}

/// Replaces `from` with `to` in `base` and returns the problems found.
fn with(base: &str, from: &str, to: &str) -> Vec<String> {
    assert!(base.contains(from), "{from}");
    problems(&base.replacen(from, to, 1))
}

fn has(problems: &[String], needle: &str) -> bool {
    problems.iter().any(|p| p.contains(needle))
}

#[test]
fn spec_examples_are_valid() {
    assert_eq!(problems(PROVIDER), Vec::<String>::new());
    assert_eq!(problems(DECLARATIVE), Vec::<String>::new());
    let m = Manifest::parse(DECLARATIVE).unwrap();
    let (key, table) = m.contributes.providers[0].backend().unwrap();
    assert_eq!(key, "acp");
    assert_eq!(table["command"], "gemini");
    let m = Manifest::parse(PROVIDER).unwrap();
    assert_eq!(m.memory_limit(), 128 << 20);
    assert_eq!(m.call_timeout(), crate::engine::DEFAULT_CALL_TIMEOUT);
    let g = m.permissions.to_grants();
    assert!(g.may_spawn("claude") && g.may_read_secret("anthropic-api-key"));
    assert_eq!(g.files, FilesGrant::Workspace);
}

#[test]
fn unknown_fields_are_refused() {
    assert!(!with(PROVIDER, "license = \"MIT\"", "licence = \"MIT\"").is_empty());
    assert!(
        !with(
            PROVIDER,
            "spawn = [\"claude\"]",
            "spawn = [\"claude\"]\nspwan = []"
        )
        .is_empty()
    );
}

#[test]
fn every_rule_has_a_message() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "id = \"kumikit.provider-claude\"",
            "id = \"Kumikit.Claude\"",
            "id `Kumikit.Claude`",
        ),
        (
            "id = \"kumikit.provider-claude\"",
            "id = \"noperiod\"",
            "<publisher>.<name>",
        ),
        ("version = \"0.3.0\"", "version = \"three\"", "not a semver"),
        ("api = \"^0.1\"", "api = \"^1.0\"", "does not include"),
        ("api = \"^0.1\"", "api = \"latest\"", "not a version range"),
        ("kind = [\"provider\"]", "kind = []", "kind is empty"),
        (
            "kind = [\"provider\"]",
            "kind = [\"theme\"]",
            "installed on a client",
        ),
        (
            "wasm = \"plugin.wasm\"",
            "wasm = \"../plugin.wasm\"",
            "inside the package",
        ),
        (
            "worlds = [\"kumikit:provider/plugin@0.1.0\"]",
            "worlds = [\"kumikit:provider/plugin@9.0.0\"]",
            "not supported",
        ),
        (
            "worlds = [\"kumikit:provider/plugin@0.1.0\"]",
            "worlds = [\"kumikit:plugin/plugin@0.1.0\"]",
            "must implement",
        ),
        (
            "on = [\"startup\", \"provider:claude\"]",
            "on = [\"provider:other\"]",
            "trigger `provider:other`",
        ),
        (
            "on = [\"startup\", \"provider:claude\"]",
            "on = [\"sometimes\"]",
            "trigger `sometimes`",
        ),
        (
            "spawn = [\"claude\"]",
            "spawn = [\"/usr/bin/claude\"]",
            "bare program name",
        ),
        (
            "secrets = [\"anthropic-api-key\"]",
            "secrets = [\"a key\"]",
            "secret name",
        ),
        ("memory-mb = 128", "memory-mb = 4096", "limits.memory-mb"),
        (
            "memory-mb = 128",
            "call-timeout-s = 0",
            "limits.call-timeout-s",
        ),
        (
            "{ id = \"claude\", name = \"Claude Code\" }",
            "{ id = \"Claude\", name = \"Claude Code\" }",
            "provider id `Claude`",
        ),
        (
            "providers = [{ id = \"claude\", name = \"Claude Code\" }]",
            "providers = [{ id = \"claude\", name = \"A\" }, { id = \"claude\", name = \"B\" }]",
            "appears twice",
        ),
    ];
    for (from, to, needle) in cases {
        let p = with(PROVIDER, from, to);
        assert!(has(&p, needle), "{to}: expected `{needle}` in {p:?}");
    }
}

#[test]
fn declarative_rules() {
    let p = with(DECLARATIVE, "acp = { command", "acp = 3, other = { command");
    assert!(has(&p, "several backend keys"), "{p:?}");
    let p = with(
        DECLARATIVE,
        "[permissions]",
        "[activation]\non = [\"startup\"]\n[permissions]",
    );
    assert!(has(&p, "need a runtime"), "{p:?}");
    let p = with(
        DECLARATIVE,
        "providers = [{ id = \"gemini\", name = \"Gemini CLI\", acp = { command = \"gemini\", args = [\"--experimental-acp\"] } }]",
        "providers = [{ id = \"gemini\", name = \"Gemini CLI\" }]",
    );
    assert!(has(&p, "must implement"), "{p:?}");
}

#[test]
fn backends_need_the_provider_world() {
    let backend = r#"
id = "kumikit.provider-acp"
name = "ACP"
version = "0.1.0"
kind = ["provider"]
api = "^0.1"
[runtime]
wasm = "plugin.wasm"
worlds = ["kumikit:provider/plugin@0.1.0"]
[contributes]
provider-backends = [{ key = "acp" }]
"#;
    assert_eq!(problems(backend), Vec::<String>::new());
    let p = with(
        backend,
        "kumikit:provider/plugin@0.1.0",
        "kumikit:plugin/plugin@0.1.0",
    );
    assert!(has(&p, "serves provider backends"), "{p:?}");
    let p = with(backend, "key = \"acp\"", "key = \"name\"");
    assert!(has(&p, "backend key `name`"), "{p:?}");
}

#[test]
fn package_files_must_exist() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST_FILE), PROVIDER).unwrap();
    let err = Manifest::load(dir.path()).unwrap_err();
    assert!(has(&err.problems, "not in the package"), "{err:?}");
    std::fs::write(dir.path().join("plugin.wasm"), b"").unwrap();
    Manifest::load(dir.path()).unwrap();
}

#[test]
fn sidecars_are_refused_for_now() {
    let p = with(
        PROVIDER,
        "wasm = \"plugin.wasm\"",
        "sidecar = { command = \"node\" }",
    );
    assert!(has(&p, "sidecar plugins are not supported"), "{p:?}");
}
