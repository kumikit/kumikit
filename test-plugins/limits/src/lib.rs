//! Test plugin: the plan limits reader of a declarative provider (ADR 0035). Its
//! component implements only the base world and exports `kumikit:usage/probe`; the
//! provider's sessions run on its backend. Built only with the public SDKs.
//!
//! `read-limits` answers from the provider's backend table: `env.LIMITS = "broken"` fails
//! with `io`, `env.LIMITS = "none"` says there are no plan limits, anything else
//! returns one grouped window at 33 %.

use kumikit_plugin::{Error, ErrorCode, Plugin, SpawnRequest, process};
use kumikit_provider::ProviderRef;
use kumikit_provider::usage::{LimitWindow, Limits, LimitsReader};

struct Reader;

impl Plugin for Reader {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl LimitsReader for Reader {
    fn read_limits(provider: ProviderRef) -> Result<Option<Limits>, Error> {
        let config = provider.config.unwrap_or_default();
        if config.contains(r#""LIMITS":"broken""#) {
            return Err(Error::new(ErrorCode::Io, "the usage report failed"));
        }
        if config.contains(r#""LIMITS":"none""#) {
            return Ok(None);
        }
        if config.contains(r#""LIMITS":"trap""#) {
            panic!("limits reader trap");
        }
        if config.contains(r#""LIMITS":"hang""#) {
            let child = process::spawn(&SpawnRequest {
                program: "fake-acp-agent".into(),
                args: vec!["hang".into()],
                cwd: None,
                env: Vec::new(),
                secret_env: Vec::new(),
            })?;
            let _ = child.read_stdout(1)?;
            return Ok(None);
        }
        if config.contains(r#""LIMITS":"invalid""#) {
            return Ok(Some(Limits {
                plan: None,
                account: None,
                account_url: None,
                windows: vec![LimitWindow {
                    id: "bad".into(),
                    label: "Bad".into(),
                    group: None,
                    used_percent: f32::NAN,
                    resets_at: None,
                    model: None,
                }],
            }));
        }
        Ok(Some(Limits {
            plan: None,
            account: None,
            account_url: Some("https://example.com/plans".into()),
            windows: vec![LimitWindow {
                id: format!("{}-weekly", provider.id),
                label: "Weekly".into(),
                group: Some("Gemini models".into()),
                used_percent: 33.0,
                resets_at: None,
                model: Some("gemini".into()),
            }],
        }))
    }
}

kumikit_plugin::export_plugin!(Reader);
kumikit_provider::export_limits_reader!(Reader);
