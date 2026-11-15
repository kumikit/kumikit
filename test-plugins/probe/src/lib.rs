//! Test plugin: a deterministic provider. The session's model selects the scenario.
//!
//! - `echo`: streams the input back word by word, then `text.done`.

use kumikit_provider::{
    Error, ErrorCode, Event, Lifecycle, Model, Provider, ProviderRef, SessionConfig, UserInput,
    events,
};

struct Probe;

impl Lifecycle for Probe {
    fn activate() -> Result<(), Error> {
        Ok(())
    }

    fn deactivate() {}
}

impl Provider for Probe {
    fn list_models(_provider: ProviderRef) -> Result<Vec<Model>, Error> {
        Ok(vec![Model {
            id: "echo".into(),
            label: "Echo".into(),
            capabilities: Vec::new(),
        }])
    }

    fn start_session(_provider: ProviderRef, config: SessionConfig) -> Result<String, Error> {
        Ok(format!("probe-{}", config.thread))
    }

    fn send(_session: String, input: UserInput) -> Result<(), Error> {
        for word in input.text.split_inclusive(' ') {
            events::emit(&Event::TextDelta(word.to_owned()))?;
        }
        events::emit(&Event::TextDone(input.text))
            .map_err(|e| Error::new(ErrorCode::Internal, e.message))
    }

    fn end_session(_session: String) {}
}

kumikit_provider::export_provider!(Probe);
