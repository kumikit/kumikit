//! WebSocket framing for the protocol: one JSON-RPC message per text frame, at most
//! 8 MiB, and keepalive pings (`docs/spec/02-protocol.md#remote-transport`).

use std::io;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Bytes, Message};

use crate::messages::{Close, MessageReader, MessageWriter, Received};
use crate::rpc::MAX_MESSAGE;

/// The WebSocket limits: 8 MiB per message and per frame.
pub fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Reads protocol messages from a WebSocket. A client silent for `idle` (not even a pong)
/// is treated as gone.
#[derive(Debug)]
pub struct WsReader<S> {
    stream: SplitStream<WebSocketStream<S>>,
    idle: Duration,
}

/// Writes protocol messages to a WebSocket, pinging every `ping`.
#[derive(Debug)]
pub struct WsWriter<S> {
    sink: SplitSink<WebSocketStream<S>, Message>,
    ping: Duration,
}

/// Splits an accepted, authenticated WebSocket into the server's reader and writer.
pub fn split<S>(
    ws: WebSocketStream<S>,
    ping: Duration,
    idle: Duration,
) -> (WsReader<S>, WsWriter<S>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (sink, stream) = ws.split();
    (WsReader { stream, idle }, WsWriter { sink, ping })
}

impl<S> MessageReader for WsReader<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    async fn recv(&mut self) -> io::Result<Received> {
        loop {
            let next = tokio::time::timeout(self.idle, self.stream.next())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "the client stopped answering")
                })?;
            return Ok(match next {
                None | Some(Ok(Message::Close(_))) => Received::Eof,
                Some(Ok(Message::Text(t))) => Received::Message(t.as_bytes().trim_ascii().to_vec()),
                Some(Ok(Message::Binary(_))) => {
                    Received::Refused("binary frames are not supported")
                }
                // Pings are answered by tungstenite; any frame counts as the client alive.
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                Some(Err(tungstenite::Error::Capacity(_))) => Received::Refused("message too long"),
                Some(Err(
                    tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed,
                )) => Received::Eof,
                Some(Err(e)) => return Err(io::Error::other(e)),
            });
        }
    }
}

impl<S> MessageWriter for WsWriter<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    async fn run(mut self, mut rx: mpsc::Receiver<String>, mut close: oneshot::Receiver<Close>) {
        let mut ping = tokio::time::interval(self.ping);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping.tick().await;
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(text) => {
                        if let Err(e) = self.sink.send(Message::text(text)).await {
                            tracing::debug!(error = %e, "WebSocket write failed");
                            return;
                        }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if let Err(e) = self.sink.send(Message::Ping(Bytes::new())).await {
                        tracing::debug!(error = %e, "WebSocket ping failed");
                        return;
                    }
                }
            }
        }
        tracing::debug!("WebSocket writer closing");
        let frame = close.try_recv().ok().map(|c| CloseFrame {
            code: CloseCode::from(c.code),
            reason: c.reason.into(),
        });
        let _ = self.sink.send(Message::Close(frame)).await;
        let _ = self.sink.close().await;
    }
}
