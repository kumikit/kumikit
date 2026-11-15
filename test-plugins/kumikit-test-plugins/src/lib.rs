//! Paths to the test plugins, built for `wasm32-wasip2` by this crate's build script.
//! A dev-dependency only; nothing shipped depends on it.

/// The smallest plugin (`test-plugins/hello`): implements `kumikit:plugin/plugin` and
/// logs on activation.
pub const HELLO_WASM: &str = env!("KUMIKIT_TEST_WASM_KUMIKIT_TEST_HELLO");

/// A deterministic provider (`test-plugins/probe`): implements
/// `kumikit:provider/plugin`; the model selects a scenario.
pub const PROBE_WASM: &str = env!("KUMIKIT_TEST_WASM_KUMIKIT_TEST_PROBE");

/// The fake agent program (`test-plugins/fake-acp-agent`), built for the host.
pub const FAKE_ACP_AGENT: &str = env!("KUMIKIT_TEST_BIN_FAKE_ACP_AGENT");
