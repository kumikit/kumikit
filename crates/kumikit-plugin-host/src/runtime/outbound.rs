//! Outgoing HTTP for plugins (`docs/spec/04-plugin-host.md#outgoing-http`, ADR 0023).
//!
//! Guests use `wasi:http/outgoing-handler`; `wasmtime-wasi-http` hands every request to
//! [`Outbound::send_request`], which applies the plugin's `network` grant before anything
//! leaves the core. Logs name only the scheme and host of a request: paths can hold
//! tokens.

use std::future::Future;

use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

/// The sending side of one instance's `wasi:http`.
#[derive(Debug, Clone)]
pub struct Outbound {
    /// The plugin whose code runs, for logs.
    pub plugin: String,
}

type ResponseFuture = Box<
    dyn Future<
            Output = Result<
                (
                    http::Response<WasiBody>,
                    Box<dyn Future<Output = Result<(), Error>> + Send>,
                ),
                Error,
            >,
        > + Send,
>;

impl WasiHttpHooks for Outbound {
    fn is_supported_scheme(&mut self, scheme: &http::uri::Scheme) -> bool {
        *scheme == http::uri::Scheme::HTTPS
    }

    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        _options: Option<RequestOptions>,
        _fut: Box<dyn Future<Output = Result<(), Error>> + Send>,
    ) -> ResponseFuture {
        let host = request.uri().host().unwrap_or_default().to_owned();
        tracing::warn!(plugin = %self.plugin, %host, "outgoing request refused: not supported yet");
        Box::new(async { Err(Error::HttpRequestDenied) })
    }
}
