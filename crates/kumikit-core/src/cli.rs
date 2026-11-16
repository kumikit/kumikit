//! `kumikit plugin add/list/remove/pack` (`docs/spec/10-distribution.md#local-packages-before-signing`).
//!
//! The commands talk `plugin.*` to the running core. When none runs, they take the data
//! directory's lock and open the plugin host in-process, then do the same calls; both
//! paths share [`Backend`]. Prompts appear only on a terminal: without one, and without
//! `--yes`, nothing is granted and data is kept.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use kumikit_plugin_host::lifecycle::PluginHost;
use kumikit_plugin_host::manifest::Manifest;
use kumikit_plugin_host::package;
use kumikit_protocol::methods::{
    FilesAccess, InstalledPlugin, Method, Permissions, PluginEnable, PluginInstall, PluginList,
    PluginState, PluginUninstall,
};
use kumikit_protocol::{ErrorCode, PROTOCOL_VERSION, RpcError};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::lock::DataDirLock;

/// Where the core is.
#[derive(Debug, Clone)]
pub struct Target {
    /// The core's data directory (used when no core runs).
    pub data_dir: PathBuf,
    /// The core's socket or pipe.
    pub address: String,
}

/// Where the commands write. Tests capture it.
pub trait Out {
    /// Writes one line.
    fn line(&mut self, text: &str);
}

/// Standard output.
#[derive(Debug)]
pub struct Stdout;

impl Out for Stdout {
    #[allow(clippy::print_stdout)]
    fn line(&mut self, text: &str) {
        println!("{text}");
    }
}

/// Asks yes/no questions on a terminal; answers no when stdin is not one.
fn confirm(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    #[allow(clippy::print_stdout)]
    {
        print!("{question} [y/N] ");
    }
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

/// A running core reached over the local transport, or the data directory opened here.
enum Backend {
    Remote(Rpc),
    Local {
        host: PluginHost,
        _lock: DataDirLock,
    },
}

/// A minimal NDJSON JSON-RPC client.
struct Rpc {
    reader: Box<dyn tokio::io::AsyncBufRead + Unpin + Send>,
    writer: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    next: i64,
}

impl Rpc {
    async fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, RpcError> {
        let value = self
            .request(M::NAME, serde_json::to_value(params).map_err(internal)?)
            .await?;
        serde_json::from_value(value).map_err(internal)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.next += 1;
        let id = self.next;
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(internal)?;
        self.writer.flush().await.map_err(internal)?;
        loop {
            let mut buf = String::new();
            if self.reader.read_line(&mut buf).await.map_err(internal)? == 0 {
                return Err(RpcError::new(
                    ErrorCode::Internal,
                    "the core closed the connection",
                ));
            }
            let v: Value = serde_json::from_str(&buf).map_err(internal)?;
            if v["id"] != id {
                continue; // a notification
            }
            if let Some(e) = v.get("error") {
                let code: ErrorCode = serde_json::from_value(e["data"]["code"].clone())
                    .unwrap_or(ErrorCode::Internal);
                let message = e["message"].as_str().unwrap_or_default().to_owned();
                return Err(RpcError::new(code, message));
            }
            return Ok(v["result"].clone());
        }
    }
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Internal, e.to_string())
}

fn rpc_err(e: RpcError) -> anyhow::Error {
    anyhow!("{}", e.message)
}

impl Backend {
    async fn open(target: &Target) -> Result<Backend> {
        if let Ok(stream) = crate::transport::connect(&target.address).await {
            let (read, write) = tokio::io::split(stream);
            let mut rpc = Rpc {
                reader: Box::new(BufReader::new(read)),
                writer: Box::new(write),
                next: 0,
            };
            rpc.request(
                "core.hello",
                json!({"protocol": PROTOCOL_VERSION.to_string(),
                       "client": {"name": "kumikit-cli", "version": env!("CARGO_PKG_VERSION")}}),
            )
            .await
            .map_err(rpc_err)
            .context("the running core refused the connection")?;
            return Ok(Backend::Remote(rpc));
        }
        // No core runs: work on the data directory, holding its lock.
        crate::serve::prepare_data_dir(&target.data_dir)?;
        let lock = DataDirLock::acquire(&target.data_dir)?;
        let host = crate::serve::open_core(&target.data_dir).await?;
        Ok(Backend::Local { host, _lock: lock })
    }

    async fn list(&mut self) -> Result<Vec<InstalledPlugin>> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginList>(&Default::default())
                .await
                .map_err(rpc_err)?
                .plugins),
            Backend::Local { host, .. } => Ok(crate::plugins::list(host)),
        }
    }

    async fn install(&mut self, path: &str) -> Result<InstalledPlugin, RpcError> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginInstall>(&kumikit_protocol::methods::PluginInstallParams {
                    path: path.to_owned(),
                })
                .await?
                .plugin),
            Backend::Local { host, .. } => crate::plugins::install(host, path).await,
        }
    }

    async fn enable(&mut self, id: &str, grant: &Permissions) -> Result<InstalledPlugin> {
        match self {
            Backend::Remote(rpc) => Ok(rpc
                .call::<PluginEnable>(&kumikit_protocol::methods::PluginEnableParams {
                    plugin: id.to_owned(),
                    grant: grant.clone(),
                })
                .await
                .map_err(rpc_err)?
                .plugin),
            Backend::Local { host, .. } => crate::plugins::enable(host, id, grant)
                .await
                .map_err(rpc_err),
        }
    }

    async fn uninstall(&mut self, id: &str, delete_data: bool) -> Result<()> {
        match self {
            Backend::Remote(rpc) => {
                rpc.call::<PluginUninstall>(&kumikit_protocol::methods::PluginUninstallParams {
                    plugin: id.to_owned(),
                    delete_data: Some(delete_data),
                })
                .await
                .map_err(rpc_err)?;
                Ok(())
            }
            Backend::Local { host, .. } => host
                .uninstall(id, delete_data)
                .await
                .map_err(|e| anyhow!("{e}")),
        }
    }
}

/// The permissions in plain words, one line each (`docs/spec/04-plugin-host.md#permissions`).
pub fn describe(p: &Permissions) -> Vec<String> {
    let mut lines = Vec::new();
    for prog in &p.spawn {
        lines.push(format!(
            "can start the `{prog}` program, which runs with your full user access, outside the sandbox"
        ));
    }
    for host in &p.network {
        lines.push(format!("can connect to {host}"));
    }
    match p.files {
        FilesAccess::None => {}
        FilesAccess::ReadWorkspace => lines.push("can read files in your workspaces".into()),
        FilesAccess::Workspace => {
            lines.push("can read and change files in your workspaces".into());
        }
    }
    for s in &p.secrets {
        lines.push(format!("can read the secret `{s}`"));
    }
    if p.tools {
        lines.push("can call tools that other plugins contribute".into());
    }
    if let Some(l) = p.limits {
        if let Some(mb) = l.memory_mb {
            lines.push(format!(
                "may use up to {mb} MiB of memory per instance (default 64)"
            ));
        }
        if let Some(s) = l.call_timeout_s {
            lines.push(format!("may compute for up to {s} s per call (default 10)"));
        }
    }
    if lines.is_empty() {
        lines.push("needs no permissions".into());
    }
    lines
}

/// Reads the manifest inside a package without installing it.
fn peek_manifest(file: &Path) -> Result<Manifest> {
    crate::plugins::peek_manifest(file).map_err(|e| anyhow!(e))
}

/// `kumikit plugin add <file> [--yes]`.
pub async fn add(target: &Target, file: &Path, yes: bool, out: &mut dyn Out) -> Result<()> {
    let file =
        std::fs::canonicalize(file).with_context(|| format!("cannot read {}", file.display()))?;
    let manifest = peek_manifest(&file)?;
    let mut backend = Backend::open(target).await?;
    let path = file
        .to_str()
        .ok_or_else(|| anyhow!("the path is not UTF-8"))?;
    let plugin = match backend.install(path).await {
        Ok(p) => p,
        Err(e) if e.data.code == ErrorCode::Conflict => {
            // Installed earlier; finish granting it if that never happened.
            let existing = backend
                .list()
                .await?
                .into_iter()
                .find(|p| p.id == manifest.id)
                .ok_or_else(|| rpc_err(e))?;
            if existing.state == PluginState::Enabled {
                out.line(&format!(
                    "{} {} is already installed and enabled.",
                    existing.id, existing.version
                ));
                return Ok(());
            }
            if existing.version != manifest.version {
                bail!(
                    "{} {} is installed; remove it first to install {}",
                    existing.id,
                    existing.version,
                    manifest.version
                );
            }
            existing
        }
        Err(e) => return Err(rpc_err(e)),
    };

    out.line(&format!(
        "{} {} ({})",
        plugin.name, plugin.version, plugin.id
    ));
    if let Some(d) = &plugin.description {
        out.line(d);
    }
    let publisher = plugin.id.split('.').next().unwrap_or_default();
    out.line(&format!("Publisher: {publisher}"));
    if let Some(r) = &plugin.repository {
        out.line(&format!("Repository: {r}"));
    }
    out.line("This package is not signed; Kumikit cannot check who made it.");
    out.line("It asks for these permissions. It:");
    for line in describe(&plugin.requested) {
        out.line(&format!("  - {line}"));
    }
    if !yes && !confirm("Grant these permissions and enable it?") {
        out.line(&format!(
            "{} is installed but not enabled. Run `kumikit plugin add {} --yes` to grant and enable it.",
            plugin.id,
            file.display()
        ));
        return Ok(());
    }
    let enabled = backend.enable(&plugin.id, &plugin.requested).await?;
    out.line(&format!("Enabled {}.", enabled.id));
    for p in &enabled.contributes.providers {
        out.line(&format!("Provider: {p}"));
    }
    Ok(())
}

/// `kumikit plugin list`.
pub async fn list(target: &Target, out: &mut dyn Out) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let plugins = backend.list().await?;
    if plugins.is_empty() {
        out.line("No plugins are installed.");
    }
    for p in plugins {
        let state = match p.state {
            PluginState::Installed => "installed (not enabled)",
            PluginState::Enabled => "enabled",
            PluginState::Disabled => "disabled",
        };
        out.line(&format!("{} {}  {state}", p.id, p.version));
        for prov in &p.contributes.providers {
            out.line(&format!("  provider {prov}"));
        }
        if let Some(f) = &p.failure {
            out.line(&format!("  failed: {}", f.message));
        }
        if let Some(e) = &p.error {
            out.line(&format!("  cannot be loaded: {e}"));
        }
    }
    Ok(())
}

/// `kumikit plugin remove <id> [--yes] [--keep-data]`.
pub async fn remove(
    target: &Target,
    id: &str,
    yes: bool,
    keep_data: bool,
    out: &mut dyn Out,
) -> Result<()> {
    let mut backend = Backend::open(target).await?;
    let delete_data = !keep_data
        && (yes
            || confirm(&format!(
                "Also delete {id}'s data (its stored state and files)?"
            )));
    backend.uninstall(id, delete_data).await?;
    out.line(&if delete_data {
        format!("Removed {id} and its data.")
    } else {
        format!("Removed {id}; its data was kept.")
    });
    Ok(())
}

/// `kumikit plugin pack <dir> [-o <file>]`: validates the manifest and writes a `.kkp`.
pub fn pack(dir: &Path, output: Option<&Path>, out: &mut dyn Out) -> Result<()> {
    let manifest = Manifest::load(dir).map_err(|e| anyhow!("{e}"))?;
    let file = output.map_or_else(
        || PathBuf::from(format!("{}-{}.kkp", manifest.id, manifest.version)),
        Path::to_path_buf,
    );
    package::pack(dir, &file).map_err(|e| anyhow!("{e}"))?;
    out.line(&format!("Wrote {}", file.display()));
    Ok(())
}
