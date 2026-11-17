//! Test only: `net-cut --to <host:port>` prints `ready <addr>`, then reads `cut` and
//! `restore` lines on stdin (answering `ok` to each) until stdin closes.

use std::io::Write;

use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let target = args
        .iter()
        .position(|a| a == "--to")
        .and_then(|i| args.get(i + 1))
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| std::io::Error::other("usage: net-cut --to <ip:port>"))?;
    let proxy = net_cut::Proxy::start(target).await?;
    let mut out = std::io::stdout();
    writeln!(out, "ready {}", proxy.addr())?;
    out.flush()?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        match line.trim() {
            "cut" => proxy.cut(),
            "restore" => proxy.restore(),
            other => {
                writeln!(out, "unknown command {other}")?;
                continue;
            }
        }
        writeln!(out, "ok")?;
        out.flush()?;
    }
    Ok(())
}
