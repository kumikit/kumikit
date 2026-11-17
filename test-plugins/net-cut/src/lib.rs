//! Test only: a loopback TCP proxy whose network can be cut, for the M3 exit tests
//! (`docs/plans/2026-11-17-m3-multiple-cores.md`, Tests).
//!
//! A cut is a black hole, which is what a lost network looks like: live connections stay
//! open but no byte passes in either direction, and new connections wait unanswered in
//! the listen backlog. Nothing is closed, so a client notices only through its own
//! keepalive and timeouts. Restoring closes every connection from before the cut (as a
//! network coming back after the peers gave up would) and serves new ones again.

use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// The network's state: whether it is cut, and how many times it was restored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct State {
    cut: bool,
    epoch: u64,
}

/// A running proxy. Dropping it stops accepting; connections end with their peers.
#[derive(Debug)]
pub struct Proxy {
    addr: SocketAddr,
    state: watch::Sender<State>,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    /// Listens on `127.0.0.1:0` and forwards to `target`.
    pub async fn start(target: SocketAddr) -> io::Result<Proxy> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let (state, watch) = watch::channel(State::default());
        let task = tokio::spawn(accept(listener, target, watch));
        Ok(Proxy { addr, state, task })
    }

    /// Where clients connect.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Cuts the network: nothing passes until [`Proxy::restore`], and new connections wait
    /// unaccepted.
    pub fn cut(&self) {
        self.state.send_modify(|s| s.cut = true);
    }

    /// Restores it: every connection from before closes, new ones are forwarded.
    pub fn restore(&self) {
        self.state.send_modify(|s| {
            if s.cut {
                s.cut = false;
                s.epoch += 1;
            }
        });
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept(listener: TcpListener, target: SocketAddr, mut state: watch::Receiver<State>) {
    loop {
        // While cut, connections wait in the listen backlog, unanswered.
        if state.wait_for(|s| !s.cut).await.is_err() {
            return;
        }
        let Ok((client, _)) = listener.accept().await else {
            return;
        };
        let epoch = state.borrow().epoch;
        tokio::spawn(connection(client, target, state.clone(), epoch));
    }
}

async fn connection(
    mut client: TcpStream,
    target: SocketAddr,
    mut state: watch::Receiver<State>,
    epoch: u64,
) {
    let Ok(mut server) = TcpStream::connect(target).await else {
        return;
    };
    let (mut a, mut b) = ([0u8; 16 * 1024], [0u8; 16 * 1024]);
    loop {
        {
            let now = *state.borrow_and_update();
            if now.epoch != epoch {
                // Cut and restored since this connection began: it is gone.
                return;
            }
            if now.cut {
                // A black hole: read nothing, write nothing, close nothing, until restored.
                let _ = state.wait_for(|s| s.epoch != epoch).await;
                return;
            }
        }
        tokio::select! {
            // The network's state wins over pending bytes, so nothing passes after a cut.
            biased;
            changed = state.changed() => if changed.is_err() { return },
            n = client.read(&mut a) => match n {
                Ok(0) | Err(_) => return,
                Ok(n) => if server.write_all(&a[..n]).await.is_err() { return },
            },
            n = server.read(&mut b) => match n {
                Ok(0) | Err(_) => return,
                Ok(n) => if client.write_all(&b[..n]).await.is_err() { return },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An echo server that reports every byte it receives.
    async fn echo() -> (SocketAddr, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = s.read(&mut buf).await {
                        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                        let _ = tx.send(buf[..n].to_vec());
                    }
                });
            }
        });
        (addr, rx)
    }

    async fn round_trip(c: &mut TcpStream, bytes: &[u8]) {
        c.write_all(bytes).await.unwrap();
        let mut got = vec![0u8; bytes.len()];
        c.read_exact(&mut got).await.unwrap();
        assert_eq!(got, bytes);
    }

    async fn step<T>(what: &str, f: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(std::time::Duration::from_secs(10), f)
            .await
            .unwrap_or_else(|_| panic!("stuck: {what}"))
    }

    #[tokio::test]
    async fn forwards_cuts_and_restores() {
        let (target, mut seen) = echo().await;
        let proxy = Proxy::start(target).await.unwrap();
        let mut live = step("connect", TcpStream::connect(proxy.addr()))
            .await
            .unwrap();
        step("first round trip", round_trip(&mut live, b"one")).await;
        assert_eq!(step("echo saw one", seen.recv()).await.unwrap(), b"one");

        proxy.cut();
        // Written into the black hole; never reaches the server.
        live.write_all(b"lost").await.unwrap();
        // A connection made during the cut waits unanswered.
        let mut waiting = step("connect during the cut", TcpStream::connect(proxy.addr()))
            .await
            .unwrap();
        waiting.write_all(b"late").await.unwrap();

        proxy.restore();
        // The connection from before the cut is gone, and what it sent never got through.
        let mut rest = Vec::new();
        assert_eq!(
            step("live closes", live.read_to_end(&mut rest))
                .await
                .unwrap_or(0),
            0
        );
        // The waiting one is answered once the network is back, like a retried connect.
        let mut got = [0u8; 4];
        step("late answer", waiting.read_exact(&mut got))
            .await
            .unwrap();
        assert_eq!(&got, b"late");
        assert_eq!(
            step("echo saw late", seen.recv()).await.unwrap(),
            b"late",
            "the cut bytes were never forwarded"
        );
        let mut again = step("connect again", TcpStream::connect(proxy.addr()))
            .await
            .unwrap();
        step("second round trip", round_trip(&mut again, b"two")).await;
        assert_eq!(step("echo saw two", seen.recv()).await.unwrap(), b"two");
    }
}
