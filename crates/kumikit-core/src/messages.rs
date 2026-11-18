//! Message framing (`docs/spec/02-protocol.md#transports`): the JSON-RPC server reads and
//! writes whole messages; transports decide how a message is framed. Newline-delimited
//! streams (socket, pipe, `kumikit bridge`) live here; WebSocket frames live with the
//! remote transport.

use std::future::Future;
use std::io;

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
};
use tokio::sync::{mpsc, oneshot};

use crate::rpc::MAX_MESSAGE;

/// What a reader got.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// One message, without surrounding whitespace (empty messages are skipped by the
    /// server).
    Message(Vec<u8>),
    /// The client closed the connection.
    Eof,
    /// The message breaks a framing rule (too long, not text); the connection must close
    /// after an `invalid_request` with this reason.
    Refused(&'static str),
}

/// Reads whole messages from a client.
pub trait MessageReader: Send + 'static {
    /// The next message.
    fn recv(&mut self) -> impl Future<Output = io::Result<Received>> + Send;
}

/// Why the server closes a connection, for transports that can say so (a WebSocket
/// close frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Close {
    /// WebSocket close code.
    pub code: u16,
    /// Reason, in plain words.
    pub reason: String,
}

/// Writes messages to a client, in order, until the channel closes.
pub trait MessageWriter: Send + 'static {
    /// Sends every message from `rx`; returns when it closes or the transport fails, after
    /// closing the transport. If `close` holds a reason by then, the transport says it.
    fn run(
        self,
        rx: mpsc::Receiver<String>,
        close: oneshot::Receiver<Close>,
    ) -> impl Future<Output = ()> + Send;
}

/// Newline-delimited JSON on a byte stream.
#[derive(Debug)]
pub struct LineReader<R> {
    reader: BufReader<R>,
    buf: Vec<u8>,
}

impl<R: AsyncRead + Send + Unpin + 'static> LineReader<R> {
    /// Reads lines from `read`.
    pub fn new(read: R) -> Self {
        LineReader {
            reader: BufReader::new(read),
            buf: Vec::new(),
        }
    }
}

impl<R: AsyncRead + Send + Unpin + 'static> MessageReader for LineReader<R> {
    async fn recv(&mut self) -> io::Result<Received> {
        Ok(
            match read_line(&mut self.reader, &mut self.buf, MAX_MESSAGE).await? {
                Line::Data => Received::Message(trim_line(&self.buf).to_vec()),
                Line::Eof => Received::Eof,
                Line::TooLong => Received::Refused("message too long"),
            },
        )
    }
}

/// Newline-delimited JSON on a byte stream.
#[derive(Debug)]
pub struct LineWriter<W> {
    write: W,
}

impl<W: AsyncWrite + Send + 'static> LineWriter<W> {
    /// Writes lines to `write`.
    pub fn new(write: W) -> Self {
        LineWriter { write }
    }
}

impl<W: AsyncWrite + Send + 'static> MessageWriter for LineWriter<W> {
    async fn run(self, rx: mpsc::Receiver<String>, _close: oneshot::Receiver<Close>) {
        write_loop(self.write, rx).await;
    }
}

async fn write_loop<W: AsyncWrite>(write: W, mut rx: mpsc::Receiver<String>) {
    let out = BufWriter::new(write);
    tokio::pin!(out);
    while let Some(line) = rx.recv().await {
        let mut ok = write_one(&mut out, &line).await;
        // Write whatever else is ready before flushing once.
        while ok {
            match rx.try_recv() {
                Ok(next) => ok = write_one(&mut out, &next).await,
                Err(_) => break,
            }
        }
        if !ok || out.flush().await.is_err() {
            return;
        }
    }
    let _ = out.shutdown().await;
}

async fn write_one<W: AsyncWrite>(out: &mut std::pin::Pin<&mut BufWriter<W>>, line: &str) -> bool {
    out.write_all(line.as_bytes()).await.is_ok() && out.write_all(b"\n").await.is_ok()
}

/// Result of reading one line.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// A line (possibly the last one, without a newline) is in the buffer.
    Data,
    /// The stream ended.
    Eof,
    /// The line exceeds the limit; the stream is left mid-line.
    TooLong,
}

async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> io::Result<Line> {
    buf.clear();
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() {
                Line::Eof
            } else {
                Line::Data
            });
        }
        let (chunk, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i, true),
            None => (available.len(), false),
        };
        if buf.len() + chunk > max {
            return Ok(Line::TooLong);
        }
        buf.extend_from_slice(&available[..chunk]);
        r.consume(if done { chunk + 1 } else { chunk });
        if done {
            return Ok(Line::Data);
        }
    }
}

fn trim_line(buf: &[u8]) -> &[u8] {
    let end = buf
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    let start = buf
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(end);
    &buf[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_are_messages() {
        let (ours, mut theirs) = tokio::io::duplex(1024);
        let mut r = LineReader::new(ours);
        theirs
            .write_all(b"  {\"a\":1}\r\n\n{\"b\":2}")
            .await
            .unwrap();
        drop(theirs);
        assert_eq!(
            r.recv().await.unwrap(),
            Received::Message(b"{\"a\":1}".to_vec())
        );
        assert_eq!(r.recv().await.unwrap(), Received::Message(Vec::new()));
        assert_eq!(
            r.recv().await.unwrap(),
            Received::Message(b"{\"b\":2}".to_vec())
        );
        assert_eq!(r.recv().await.unwrap(), Received::Eof);
    }

    #[tokio::test]
    async fn an_oversized_line_is_refused() {
        let (ours, mut theirs) = tokio::io::duplex(64 * 1024);
        let mut r = LineReader::new(ours);
        tokio::spawn(async move {
            let _ = theirs.write_all(&vec![b'x'; MAX_MESSAGE + 1]).await;
        });
        assert_eq!(
            r.recv().await.unwrap(),
            Received::Refused("message too long")
        );
    }

    #[tokio::test]
    async fn the_writer_ends_each_message_with_a_newline() {
        let (ours, theirs) = tokio::io::duplex(1024);
        let (tx, rx) = mpsc::channel(4);
        let (_close, closed) = oneshot::channel();
        let task = tokio::spawn(LineWriter::new(ours).run(rx, closed));
        tx.send("{}".into()).await.unwrap();
        tx.send("[]".into()).await.unwrap();
        drop(tx);
        task.await.unwrap();
        let mut out = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut BufReader::new(theirs), &mut out)
            .await
            .unwrap();
        assert_eq!(out, "{}\n[]\n");
    }
}
