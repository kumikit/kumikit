//! Outgoing HTTP over `wasi:http` (`docs/spec/04-plugin-host.md#outgoing-http`).
//!
//! The host sends requests only to hosts in the plugin's `permissions.network`, over
//! `https`, and does not follow redirects. [`send`] blocks until the whole response is
//! read; [`start`] returns a [`Pending`] whose [`Pending::pollable`] can be waited on
//! together with other pollables (a bot waits on its chat service and the kernel at once).
//!
//! Natively (unit tests of plugin crates) every call fails: there is no host.

use std::time::Duration;

/// Largest response body read (the host caps it at 16 MiB too).
pub const MAX_BODY: usize = 16 << 20;

/// A request.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// `GET`, `POST`, … (default `GET`).
    pub method: String,
    /// `https://host[:port]/path?query`.
    pub url: String,
    /// Header names and values.
    pub headers: Vec<(String, String)>,
    /// The body (empty for none).
    pub body: Vec<u8>,
    /// How long to wait for the response to start, and between chunks (the host caps it
    /// at 120 s).
    pub timeout: Option<Duration>,
}

impl Request {
    /// A `GET` of `url`.
    pub fn get(url: impl Into<String>) -> Self {
        Request {
            method: "GET".into(),
            url: url.into(),
            ..Default::default()
        }
    }

    /// A `POST` of `body` to `url`, with a `content-type`.
    pub fn post(url: impl Into<String>, content_type: &str, body: Vec<u8>) -> Self {
        Request {
            method: "POST".into(),
            url: url.into(),
            headers: vec![("content-type".into(), content_type.into())],
            body,
            timeout: None,
        }
    }

    /// Adds a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the timeout.
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = Some(t);
        self
    }
}

/// A response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Header names (lowercase) and values.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// The first value of a header, by lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request failed. `denied` is true when the host refused it by policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    /// What happened.
    pub message: String,
    /// The host refused the request (not in `permissions.network`, not `https`, a private
    /// address).
    pub denied: bool,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

fn fail(message: impl Into<String>) -> HttpError {
    HttpError {
        message: message.into(),
        denied: false,
    }
}

/// Splits `https://host:port/path?q` into scheme, authority and path with query.
pub fn split_url(url: &str) -> Result<(&str, &str, &str), HttpError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| fail(format!("not an absolute URL: {url}")))?;
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], &rest[i..]),
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(fail(format!("no host in {url}")));
    }
    Ok((scheme, authority, path))
}

/// Sends a request and reads the whole response.
pub fn send(req: &Request) -> Result<Response, HttpError> {
    start(req)?.wait()
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{HttpError, MAX_BODY, Request, Response, fail, split_url};
    use wasip2::http::outgoing_handler;
    use wasip2::http::types::{
        ErrorCode, Fields, FutureIncomingResponse, Method, OutgoingBody, OutgoingRequest,
        RequestOptions, Scheme,
    };
    use wasip2::io::poll::Pollable;
    use wasip2::io::streams::StreamError;

    /// A request on its way; see [`super::start`].
    #[derive(Debug)]
    pub struct Pending {
        future: FutureIncomingResponse,
    }

    fn method(m: &str) -> Method {
        match m.to_ascii_uppercase().as_str() {
            "" | "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            other => Method::Other(other.to_owned()),
        }
    }

    fn error(e: ErrorCode) -> HttpError {
        let denied = matches!(
            e,
            ErrorCode::HttpRequestDenied | ErrorCode::DestinationIpProhibited
        );
        HttpError {
            message: format!("{e:?}"),
            denied,
        }
    }

    pub fn start(req: &Request) -> Result<Pending, HttpError> {
        let (scheme, authority, path) = split_url(&req.url)?;
        let headers = Fields::new();
        for (n, v) in &req.headers {
            headers
                .append(n, v.as_bytes())
                .map_err(|e| fail(format!("header {n}: {e:?}")))?;
        }
        let r = OutgoingRequest::new(headers);
        r.set_method(&method(&req.method))
            .map_err(|()| fail("bad method"))?;
        let scheme = match scheme {
            "https" => Scheme::Https,
            "http" => Scheme::Http,
            other => Scheme::Other(other.to_owned()),
        };
        r.set_scheme(Some(&scheme))
            .map_err(|()| fail("bad scheme"))?;
        r.set_authority(Some(authority))
            .map_err(|()| fail("bad host"))?;
        r.set_path_with_query(Some(path))
            .map_err(|()| fail("bad path"))?;
        let body = r.body().map_err(|()| fail("no request body"))?;
        let options = RequestOptions::new();
        if let Some(t) = req.timeout {
            let ns = u64::try_from(t.as_nanos()).unwrap_or(u64::MAX);
            let _ = options.set_first_byte_timeout(Some(ns));
            let _ = options.set_between_bytes_timeout(Some(ns));
        }
        let future = outgoing_handler::handle(r, Some(options)).map_err(error)?;
        if !req.body.is_empty() {
            let stream = body.write().map_err(|()| fail("cannot write the body"))?;
            for chunk in req.body.chunks(4096) {
                stream
                    .blocking_write_and_flush(chunk)
                    .map_err(|e| fail(format!("writing the body: {e:?}")))?;
            }
            drop(stream);
        }
        OutgoingBody::finish(body, None).map_err(error)?;
        Ok(Pending { future })
    }

    impl Pending {
        /// Ready when the response's headers arrived (or the request failed).
        pub fn pollable(&self) -> Pollable {
            self.future.subscribe()
        }

        /// Whether [`Pending::wait`] would not block for the headers.
        pub fn is_ready(&self) -> bool {
            self.future.subscribe().ready()
        }

        /// Waits for the response and reads its body.
        pub fn wait(self) -> Result<Response, HttpError> {
            self.future.subscribe().block();
            let response = match self.future.get() {
                Some(Ok(Ok(r))) => r,
                Some(Ok(Err(e))) => return Err(error(e)),
                Some(Err(())) => return Err(fail("the response was already taken")),
                None => return Err(fail("the response is not ready")),
            };
            let status = response.status();
            let headers = response
                .headers()
                .entries()
                .into_iter()
                .map(|(n, v)| {
                    (
                        n.to_ascii_lowercase(),
                        String::from_utf8_lossy(&v).into_owned(),
                    )
                })
                .collect();
            let incoming = response.consume().map_err(|()| fail("no response body"))?;
            let stream = incoming.stream().map_err(|()| fail("no response body"))?;
            let mut body = Vec::new();
            loop {
                match stream.blocking_read(64 * 1024) {
                    Ok(chunk) => {
                        body.extend_from_slice(&chunk);
                        if body.len() > MAX_BODY {
                            return Err(fail("the response is larger than 16 MiB"));
                        }
                    }
                    Err(StreamError::Closed) => break,
                    Err(StreamError::LastOperationFailed(e)) => {
                        return Err(fail(format!(
                            "reading the response: {}",
                            e.to_debug_string()
                        )));
                    }
                }
            }
            Ok(Response {
                status,
                headers,
                body,
            })
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{HttpError, Request, Response, fail};

    /// A request on its way; natively there is none.
    #[derive(Debug)]
    pub struct Pending(());

    pub fn start(_req: &Request) -> Result<Pending, HttpError> {
        Err(fail("outgoing HTTP needs the plugin host (wasm32)"))
    }

    impl Pending {
        /// Whether [`Pending::wait`] would not block.
        pub fn is_ready(&self) -> bool {
            true
        }

        /// Waits for the response.
        pub fn wait(self) -> Result<Response, HttpError> {
            Err(fail("outgoing HTTP needs the plugin host (wasm32)"))
        }
    }
}

pub use imp::Pending;

/// Starts a request; read the response with [`Pending::wait`].
pub fn start(req: &Request) -> Result<Pending, HttpError> {
    imp::start(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_split_into_their_parts() {
        assert_eq!(
            split_url("https://api.telegram.org/bot1:x/getMe").unwrap(),
            ("https", "api.telegram.org", "/bot1:x/getMe")
        );
        assert_eq!(
            split_url("https://h:8443").unwrap(),
            ("https", "h:8443", "/")
        );
        assert_eq!(split_url("https://h?x=1").unwrap(), ("https", "h", "?x=1"));
        assert!(split_url("api.telegram.org/x").is_err());
        assert!(split_url("https:///x").is_err());
    }

    #[test]
    fn natively_there_is_no_host() {
        assert!(send(&Request::get("https://example.org/")).is_err());
    }
}
