//! A minimal Kumikit provider: it answers every message with the message itself.
//!
//! Replace `send` with your agent. A provider that runs a program asks for it in
//! `package/plugin.toml` (`[permissions] spawn`) and starts it with `process::spawn`;
//! every host call is checked against what the user granted.

use kumikit_provider::{
    Error, Event, Plugin, Model, Provider, ProviderRef, SessionConfig, UserInput, events,
};

struct Echo;

impl Plugin for Echo {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl Provider for Echo {
    fn list_models(_provider: ProviderRef) -> Result<Vec<Model>, Error> {
        Ok(vec![Model {
            id: "echo".into(),
            label: "Echo".into(),
            capabilities: Vec::new(),
        }])
    }

    fn start_session(_provider: ProviderRef, config: SessionConfig) -> Result<String, Error> {
        Ok(config.thread)
    }

    /// Runs one turn: stream the reply, then say it is done.
    fn send(_session: String, input: UserInput) -> Result<(), Error> {
        for word in input.text.split_inclusive(' ') {
            events::emit(&Event::TextDelta(word.to_owned()))?;
        }
        events::emit(&Event::TextDone(input.text))
    }

    fn end_session(_session: String) {}
}

kumikit_provider::export_provider!(Echo);
