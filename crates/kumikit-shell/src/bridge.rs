//! The connection bridge (ADR 0013): the webview sends and receives JSON-RPC lines
//! through the shell, which holds the sockets. The shell does not parse messages; it
//! frames them and enforces the protocol's size limit.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Longest message in either direction (`docs/spec/02-protocol.md#transports`).
pub const MAX_LINE: usize = 8 * 1024 * 1024;

/// Outgoing lines buffered per connection.
const OUTBOUND: usize = 256;

/// What the webview receives on a connection's channel, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Incoming {
    /// One message from the core.
    Line {
        /// The JSON text, without the newline.
        line: String,
    },
    /// The connection is gone; nothing follows.
    Closed {
        /// Why, in plain words.
        reason: String,
    },
}

/// Errors from [`Bridge::send`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BridgeError {
    /// No open connection has this id.
    #[error("no connection {0}")]
    NoConnection(String),
    /// The line has a newline in it or is too long.
    #[error("invalid message: {0}")]
    InvalidLine(&'static str),
}

/// Open connections by id. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct Bridge {
    inner: Arc<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    next: AtomicU64,
    conns: Mutex<HashMap<String, mpsc::Sender<String>>>,
}

impl Inner {
    fn conns(&self) -> MutexGuard<'_, HashMap<String, mpsc::Sender<String>>> {
        // The map stays consistent even if a holder panicked.
        self.conns.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Bridge {
    /// Takes over a connected stream. Every incoming line, then a final
    /// [`Incoming::Closed`], goes to `sink`. Returns the connection id.
    pub fn attach<S>(&self, stream: S, sink: impl Fn(Incoming) + Send + Sync + 'static) -> String
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        self.attach_explained(stream, sink, std::future::ready)
    }

    /// Like [`Bridge::attach`], with `explain` rewriting the reason the connection closed
    /// (an SSH connection reports ssh's own message).
    pub fn attach_explained<S, F>(
        &self,
        stream: S,
        sink: impl Fn(Incoming) + Send + Sync + 'static,
        explain: impl FnOnce(String) -> F + Send + 'static,
    ) -> String
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
        F: Future<Output = String> + Send,
    {
        let (read, write) = tokio::io::split(stream);
        self.attach_halves(read, write, sink, explain)
    }

    /// Like [`Bridge::attach_explained`], with the two directions as separate objects.
    /// The write side is dropped when the connection closes, which is what closes a pipe
    /// (a child's stdin does not close on `shutdown`).
    pub fn attach_halves<R, W, F>(
        &self,
        read: R,
        write: W,
        sink: impl Fn(Incoming) + Send + Sync + 'static,
        explain: impl FnOnce(String) -> F + Send + 'static,
    ) -> String
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Send + 'static,
        F: Future<Output = String> + Send,
    {
        let (id, rx) = self.register();
        let writer = tokio::spawn(write_loop(write, rx));
        let inner = Arc::clone(&self.inner);
        let conn = id.clone();
        tokio::spawn(async move {
            let reason = explain(read_loop(read, &sink).await).await;
            inner.conns().remove(&conn);
            writer.abort();
            sink(Incoming::Closed { reason });
        });
        id
    }

    /// Takes over an authenticated WebSocket to a remote core: one message per text
    /// frame. The shell pings every `ping` and closes the connection after `idle` without
    /// any frame from the core.
    pub fn attach_ws<S>(
        &self,
        ws: tokio_tungstenite::WebSocketStream<S>,
        ping: std::time::Duration,
        idle: std::time::Duration,
        sink: impl Fn(Incoming) + Send + Sync + 'static,
    ) -> String
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        use futures_util::StreamExt;
        let (id, rx) = self.register();
        let (write, read) = ws.split();
        let writer = tokio::spawn(ws_write_loop(write, rx, ping));
        let inner = Arc::clone(&self.inner);
        let conn = id.clone();
        tokio::spawn(async move {
            let reason = ws_read_loop(read, idle, &sink).await;
            inner.conns().remove(&conn);
            writer.abort();
            sink(Incoming::Closed { reason });
        });
        id
    }

    fn register(&self) -> (String, mpsc::Receiver<String>) {
        let id = format!("c{}", self.inner.next.fetch_add(1, Ordering::Relaxed) + 1);
        let (tx, rx) = mpsc::channel(OUTBOUND);
        self.inner.conns().insert(id.clone(), tx);
        (id, rx)
    }

    /// Sends one message. The line must be one JSON text without a newline.
    pub async fn send(&self, id: &str, line: String) -> Result<(), BridgeError> {
        if line.contains('\n') {
            return Err(BridgeError::InvalidLine("it contains a newline"));
        }
        if line.len() > MAX_LINE {
            return Err(BridgeError::InvalidLine("it is longer than 8 MiB"));
        }
        let tx = self
            .inner
            .conns()
            .get(id)
            .cloned()
            .ok_or_else(|| BridgeError::NoConnection(id.to_owned()))?;
        tx.send(line)
            .await
            .map_err(|_| BridgeError::NoConnection(id.to_owned()))
    }

    /// Closes a connection; its channel then gets [`Incoming::Closed`]. Returns whether
    /// it was open.
    pub fn close(&self, id: &str) -> bool {
        // Dropping the sender ends the writer, which shuts the stream down; the core then
        // closes its side and the reader reports it.
        self.inner.conns().remove(id).is_some()
    }

    /// The number of open connections.
    pub fn len(&self) -> usize {
        self.inner.conns().len()
    }

    /// Whether no connection is open.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

async fn read_loop<R: AsyncRead + Unpin>(read: R, sink: &impl Fn(Incoming)) -> String {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    loop {
        match read_line(&mut reader, &mut buf).await {
            Ok(Some(())) => {
                let text = String::from_utf8_lossy(&buf);
                let line = text.trim_end_matches('\r');
                if !line.trim().is_empty() {
                    sink(Incoming::Line {
                        line: line.to_owned(),
                    });
                }
            }
            Ok(None) => return "the core closed the connection".into(),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                return "the core sent a message longer than 8 MiB".into();
            }
            Err(e) => return format!("the connection failed: {e}"),
        }
    }
}

/// Reads one line into `buf` (without the newline). `None` at the end of the stream;
/// `InvalidData` if the line exceeds [`MAX_LINE`].
async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> io::Result<Option<()>> {
    buf.clear();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { None } else { Some(()) });
        }
        let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i, true),
            None => (available.len(), false),
        };
        if buf.len() + chunk > MAX_LINE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        buf.extend_from_slice(&available[..chunk]);
        r.consume(if done { chunk + 1 } else { chunk });
        if done {
            return Ok(Some(()));
        }
    }
}

async fn ws_read_loop<S>(
    mut read: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<S>>,
    idle: std::time::Duration,
    sink: &impl Fn(Incoming),
) -> String
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{Error, Message};
    loop {
        let Ok(next) = tokio::time::timeout(idle, read.next()).await else {
            return "the core stopped answering".into();
        };
        match next {
            Some(Ok(Message::Text(t))) => {
                let line = t.as_str().trim();
                if !line.is_empty() {
                    sink(Incoming::Line {
                        line: line.to_owned(),
                    });
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
            Some(Ok(Message::Binary(_))) => return "the core sent a binary frame".into(),
            Some(Ok(Message::Close(frame))) => {
                return match frame {
                    Some(f) if u16::from(f.code) == kumikit_protocol::remote::CLOSE_REVOKED => {
                        "this device was revoked".into()
                    }
                    Some(f) if !f.reason.is_empty() => {
                        format!("the core closed the connection: {}", f.reason)
                    }
                    _ => "the core closed the connection".into(),
                };
            }
            Some(Err(Error::Capacity(_))) => {
                return "the core sent a message longer than 8 MiB".into();
            }
            Some(Err(e)) => return format!("the connection failed: {e}"),
            None => return "the core closed the connection".into(),
        }
    }
}

async fn ws_write_loop<S>(
    mut write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<S>,
        tokio_tungstenite::tungstenite::Message,
    >,
    mut rx: mpsc::Receiver<String>,
    ping: std::time::Duration,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{Bytes, Message};
    let mut tick = tokio::time::interval(ping);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Some(line) => {
                    if write.send(Message::text(line)).await.is_err() {
                        return;
                    }
                }
                None => break,
            },
            _ = tick.tick() => {
                if write.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
        }
    }
    let _ = write.send(Message::Close(None)).await;
    let _ = write.close().await;
}

async fn write_loop<W: AsyncWrite>(write: W, mut rx: mpsc::Receiver<String>) {
    tokio::pin!(write);
    while let Some(line) = rx.recv().await {
        let ok = write.write_all(line.as_bytes()).await.is_ok()
            && write.write_all(b"\n").await.is_ok()
            && write.flush().await.is_ok();
        if !ok {
            return;
        }
    }
    let _ = write.shutdown().await;
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    fn collect() -> (
        impl Fn(Incoming) + Send + Sync + 'static,
        mpsc::UnboundedReceiver<Incoming>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (move |m| drop(tx.send(m)), rx)
    }

    #[tokio::test]
    async fn lines_go_both_ways_in_order() {
        let (ours, theirs) = tokio::io::duplex(1024);
        let bridge = Bridge::default();
        let (sink, mut rx) = collect();
        let id = bridge.attach(ours, sink);
        let (read, mut write) = tokio::io::split(theirs);
        let mut lines = BufReader::new(read).lines();

        bridge.send(&id, r#"{"a":1}"#.into()).await.unwrap();
        bridge.send(&id, r#"{"b":2}"#.into()).await.unwrap();
        assert_eq!(lines.next_line().await.unwrap().unwrap(), r#"{"a":1}"#);
        assert_eq!(lines.next_line().await.unwrap().unwrap(), r#"{"b":2}"#);

        write
            .write_all(b"{\"x\":1}\r\n\n{\"y\":2}\n")
            .await
            .unwrap();
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Line {
                line: r#"{"x":1}"#.into()
            }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Line {
                line: r#"{"y":2}"#.into()
            }
        );
        drop(write);
        drop(lines);
        assert!(matches!(rx.recv().await.unwrap(), Incoming::Closed { .. }));
        assert!(bridge.is_empty(), "a closed connection is forgotten");
        assert_eq!(
            bridge.send(&id, "{}".into()).await,
            Err(BridgeError::NoConnection(id))
        );
    }

    #[tokio::test]
    async fn bad_outgoing_lines_are_refused() {
        let (ours, _theirs) = tokio::io::duplex(64);
        let bridge = Bridge::default();
        let (sink, _rx) = collect();
        let id = bridge.attach(ours, sink);
        assert_eq!(
            bridge.send(&id, "{}\n{}".into()).await,
            Err(BridgeError::InvalidLine("it contains a newline"))
        );
        assert_eq!(
            bridge.send(&id, "x".repeat(MAX_LINE + 1)).await,
            Err(BridgeError::InvalidLine("it is longer than 8 MiB"))
        );
    }

    #[tokio::test]
    async fn an_oversized_incoming_line_closes_the_connection() {
        let (ours, mut theirs) = tokio::io::duplex(64 * 1024);
        let bridge = Bridge::default();
        let (sink, mut rx) = collect();
        bridge.attach(ours, sink);
        let big = vec![b'x'; MAX_LINE + 1];
        tokio::spawn(async move {
            let _ = theirs.write_all(&big).await;
        });
        assert_eq!(
            rx.recv().await.unwrap(),
            Incoming::Closed {
                reason: "the core sent a message longer than 8 MiB".into()
            }
        );
    }

    #[tokio::test]
    async fn closing_reports_closed() {
        let (ours, theirs) = tokio::io::duplex(64);
        let bridge = Bridge::default();
        let (sink, mut rx) = collect();
        let id = bridge.attach(ours, sink);
        // The other side closes when it sees our side shut down, like the core does.
        tokio::spawn(async move {
            let mut lines = BufReader::new(theirs).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        assert!(bridge.close(&id));
        assert!(!bridge.close(&id));
        assert!(matches!(rx.recv().await.unwrap(), Incoming::Closed { .. }));
    }
}
