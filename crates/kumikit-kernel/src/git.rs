//! Git branch and worktree operations for workspaces, run through the kernel's process
//! supervisor (`docs/spec/03-kernel.md#processes`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kumikit_protocol::methods::{
    GitBranch, WorkspaceGitInfo, WorkspaceWorktreeCreateResult, WorktreeEntry, WorktreeListResult,
};
use kumikit_protocol::model::WorkspaceId;

use crate::error::{KernelError, KernelResult};
use crate::grants::Grants;
use crate::kernel::Kernel;
use crate::process::{OwnerId, ProcessError, SpawnContext, SpawnRequest};

const MAX_STDOUT: usize = 2 * 1024 * 1024;
const MAX_STDERR: usize = 32 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

struct GitOut {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Kernel {
    /// Resolves the directory to run Git commands in: the workspace root, or an optional
    /// linked worktree path belonging to the same Git repository.
    async fn resolve_git_target(
        &self,
        workspace: &WorkspaceId,
        path: Option<&str>,
    ) -> KernelResult<PathBuf> {
        let ws = self.workspace_get(workspace).await?;
        let ws_root = PathBuf::from(&ws.root);
        let Some(raw) = path.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(ws_root);
        };
        let target = PathBuf::from(raw);
        if !target.is_absolute() {
            return Err(KernelError::InvalidParams(format!(
                "worktree path must be absolute: {raw}"
            )));
        }
        let canon = tokio::task::spawn_blocking(move || std::fs::canonicalize(&target))
            .await
            .map_err(|e| KernelError::Io(e.to_string()))?
            .map_err(|e| KernelError::NotFound(format!("cannot open {raw}: {e}")))?;
        if canon == ws_root {
            return Ok(ws_root);
        }
        // Verify the target path belongs to the same Git repository common dir.
        let ws_common = self.git_common_dir(&ws_root).await?;
        let target_common = self.git_common_dir(&canon).await?;
        match (ws_common, target_common) {
            (Some(a), Some(b)) if a == b => Ok(canon),
            _ => Err(KernelError::InvalidParams(format!(
                "path `{raw}` does not belong to workspace {workspace}'s Git repository"
            ))),
        }
    }

    async fn git_common_dir(&self, dir: &Path) -> KernelResult<Option<PathBuf>> {
        let out = match self
            .run_git(
                dir,
                vec!["rev-parse".into(), "--git-common-dir".into()],
                true,
            )
            .await
        {
            Ok(out) if out.code == Some(0) => out,
            Ok(_) | Err(KernelError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let raw = out.stdout.trim().to_owned();
        if raw.is_empty() {
            return Ok(None);
        }
        let base = dir.to_path_buf();
        let resolved = tokio::task::spawn_blocking(move || {
            let pb = PathBuf::from(&raw);
            let full = if pb.is_absolute() { pb } else { base.join(pb) };
            std::fs::canonicalize(&full).unwrap_or(full)
        })
        .await
        .map_err(|e| KernelError::Io(e.to_string()))?;
        Ok(Some(resolved))
    }

    /// Inspects a workspace's (or linked worktree's) Git branch, worktree status, and
    /// local branches.
    ///
    /// Returns `is_repo: false` when the workspace is not inside a Git repository (or
    /// when `git` is unavailable on `PATH`).
    pub async fn workspace_git_status(
        &self,
        workspace: &WorkspaceId,
        path: Option<&str>,
    ) -> KernelResult<WorkspaceGitInfo> {
        let root = self.resolve_git_target(workspace, path).await?;
        self.git_status_for_root(&root).await
    }

    /// Switches a workspace (or linked worktree) to an existing local Git branch and
    /// returns the refreshed Git status.
    pub async fn workspace_git_checkout(
        &self,
        workspace: &WorkspaceId,
        branch: &str,
        path: Option<&str>,
    ) -> KernelResult<WorkspaceGitInfo> {
        let branch = validate_branch_name(branch)?;
        let root = self.resolve_git_target(workspace, path).await?;

        let check = self
            .run_git(
                &root,
                vec!["check-ref-format".into(), "--branch".into(), branch.clone()],
                true,
            )
            .await?;
        if check.code != Some(0) {
            return Err(KernelError::InvalidParams(format!(
                "invalid branch name `{branch}`"
            )));
        }

        let out = self
            .run_git(&root, vec!["checkout".into(), branch.clone()], false)
            .await?;
        if out.code != Some(0) {
            let msg = out.stderr.trim();
            return Err(KernelError::Conflict(if msg.is_empty() {
                format!("git checkout `{branch}` failed")
            } else {
                msg.to_owned()
            }));
        }

        self.git_status_for_root(&root).await
    }

    /// Creates a linked Git worktree under `<homeDir>/worktrees/<repo>/<branch-slug>`,
    /// opens it as a workspace on this core, and returns the workspace plus its Git state.
    pub async fn workspace_git_worktree_create(
        &self,
        workspace: &WorkspaceId,
        branch: &str,
        base_branch: Option<&str>,
    ) -> KernelResult<WorkspaceWorktreeCreateResult> {
        let branch = validate_branch_name(branch)?;
        let base = match base_branch.map(str::trim).filter(|s| !s.is_empty()) {
            Some(b) => Some(validate_ref_name(b)?),
            None => None,
        };

        let ws = self.workspace_get(workspace).await?;
        let root = PathBuf::from(&ws.root);

        let check = self
            .run_git(
                &root,
                vec!["check-ref-format".into(), "--branch".into(), branch.clone()],
                true,
            )
            .await?;
        if check.code != Some(0) {
            return Err(KernelError::InvalidParams(format!(
                "invalid branch name `{branch}`"
            )));
        }

        // Verify this is a Git repo and locate the main repository name from common dir.
        let probe = self
            .run_git(
                &root,
                vec![
                    "rev-parse".into(),
                    "--show-toplevel".into(),
                    "--git-common-dir".into(),
                ],
                true,
            )
            .await?;
        if probe.code != Some(0) {
            return Err(KernelError::InvalidParams(
                "workspace is not a Git repository".into(),
            ));
        }
        let mut lines = probe.stdout.lines();
        let toplevel = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                KernelError::InvalidParams("workspace is not a Git repository".into())
            })?;
        let common_dir = lines.next().map(str::trim).unwrap_or(".git");
        let repo_name = derive_repo_name(&root, toplevel, common_dir);

        // Compute destination directory under `<homeDir>/worktrees/<repo>/<branch-slug>`.
        let cfg = self.core_config().await?;
        let home = PathBuf::from(&cfg.home_dir);
        let slug = sanitize_path_segment(&branch);
        let repo_seg = sanitize_path_segment(&repo_name);
        let parent_dir = home.join("worktrees").join(&repo_seg);
        let target_dir = tokio::task::spawn_blocking(move || -> Result<PathBuf, std::io::Error> {
            std::fs::create_dir_all(&parent_dir)?;
            let mut candidate = parent_dir.join(&slug);
            let mut n = 2u32;
            while candidate.exists() {
                candidate = parent_dir.join(format!("{slug}-{n}"));
                n += 1;
            }
            Ok(candidate)
        })
        .await
        .map_err(|e| KernelError::Io(format!("worktree task failed: {e}")))?
        .map_err(|e| {
            KernelError::InvalidParams(format!(
                "cannot prepare worktree directory under `{}`: {e}",
                cfg.home_dir
            ))
        })?;

        let target_str = target_dir
            .to_str()
            .ok_or_else(|| KernelError::InvalidParams("worktree path is not valid UTF-8".into()))?
            .to_owned();

        // Check whether the requested branch already exists locally.
        let exists = self
            .run_git(
                &root,
                vec![
                    "show-ref".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    format!("refs/heads/{branch}"),
                ],
                true,
            )
            .await?
            .code
            == Some(0);

        let mut args = vec!["worktree".into(), "add".into()];
        if exists {
            args.push(target_str.clone());
            args.push(branch.clone());
        } else {
            args.push("-b".into());
            args.push(branch.clone());
            args.push(target_str.clone());
            if let Some(base_ref) = base {
                args.push(base_ref);
            }
        }

        let created = self.run_git(&root, args, false).await?;
        if created.code != Some(0) {
            let msg = created.stderr.trim();
            return Err(KernelError::Conflict(if msg.is_empty() {
                format!("git worktree add failed for `{branch}`")
            } else {
                msg.to_owned()
            }));
        }

        let canonical = tokio::task::spawn_blocking(move || std::fs::canonicalize(&target_dir))
            .await
            .map_err(|e| KernelError::Io(e.to_string()))?
            .map_err(|e| KernelError::Io(format!("cannot canonicalize created worktree: {e}")))?;
        let canonical_str = canonical
            .to_str()
            .ok_or_else(|| KernelError::InvalidParams("worktree path is not valid UTF-8".into()))?;
        let path = match canonical_str.strip_prefix(r"\\?\") {
            Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
            _ => canonical_str.to_owned(),
        };
        let git = self.git_status_for_root(Path::new(&path)).await?;
        Ok(WorkspaceWorktreeCreateResult { path, git })
    }

    /// Lists all managed Git worktrees under `<homeDir>/worktrees/` with disk usage and
    /// linked thread counts (largest first).
    pub async fn list_managed_worktrees(&self) -> KernelResult<WorktreeListResult> {
        let cfg = self.core_config().await?;
        let wt_root = PathBuf::from(&cfg.home_dir).join("worktrees");
        let root_str = wt_root.to_string_lossy().into_owned();
        let by_path = self
            .inner
            .store
            .read(crate::repo::threads_by_worktree_path)
            .await?;
        let running_threads: BTreeSet<String> = self
            .lock_runs()
            .keys()
            .map(|id| id.as_str().to_owned())
            .collect();

        let scanned = tokio::task::spawn_blocking(move || scan_worktrees(&wt_root))
            .await
            .map_err(|e| KernelError::Io(format!("worktree scan failed: {e}")))?;

        let mut entries = Vec::with_capacity(scanned.len());
        let mut total_bytes = 0u64;
        for item in scanned {
            let linked = by_path.get(&item.path);
            let thread_count = linked.map_or(0, |v| u32::try_from(v.len()).unwrap_or(u32::MAX));
            let in_use =
                linked.is_some_and(|v| v.iter().any(|tid| running_threads.contains(tid.as_str())));
            total_bytes = total_bytes.saturating_add(item.size_bytes);
            entries.push(WorktreeEntry {
                path: item.path,
                repo: item.repo,
                name: item.name,
                branch: item.branch,
                size_bytes: item.size_bytes,
                thread_count,
                in_use,
            });
        }

        entries.sort_by(|a, b| {
            b.size_bytes
                .cmp(&a.size_bytes)
                .then_with(|| a.repo.cmp(&b.repo))
                .then_with(|| a.name.cmp(&b.name))
        });

        Ok(WorktreeListResult {
            root: root_str,
            worktrees: entries,
            total_bytes,
        })
    }

    /// Removes a managed Git worktree directory strictly inside `<homeDir>/worktrees/`.
    /// Refuses if any linked thread currently has a running turn.
    pub async fn delete_managed_worktree(&self, path: &str) -> KernelResult<()> {
        let target = self.verify_managed_worktree_path(path).await?;
        let target_str = target
            .to_str()
            .ok_or_else(|| KernelError::InvalidParams("worktree path is not valid UTF-8".into()))?
            .to_owned();

        let by_path = self
            .inner
            .store
            .read(crate::repo::threads_by_worktree_path)
            .await?;
        if let Some(linked) = by_path.get(&target_str) {
            let runs = self.lock_runs();
            if linked.iter().any(|tid| runs.contains_key(tid)) {
                return Err(KernelError::Conflict(
                    "cannot delete a worktree while a thread using it is running".into(),
                ));
            }
        }

        self.remove_verified_worktree(&target).await?;
        let clear_path = target_str;
        self.inner
            .store
            .write(move |c| crate::repo::threads_clear_worktree_path(c, &clear_path))
            .await?;
        Ok(())
    }

    /// Best-effort cleanup called when deleting a thread whose `worktree_path` is no
    /// longer referenced by any other thread. Only deletes directories strictly inside
    /// `<homeDir>/worktrees/`.
    pub(crate) async fn remove_worktree_if_managed(&self, path: &str) {
        let Ok(target) = self.verify_managed_worktree_path(path).await else {
            return;
        };
        if let Err(e) = self.remove_verified_worktree(&target).await {
            tracing::warn!(path = %target.display(), error = %e, "could not remove worktree directory");
        }
    }

    /// Verifies that `path` exists and resolves to a leaf worktree directory
    /// `<homeDir>/worktrees/<repo>/<worktree>`.
    async fn verify_managed_worktree_path(&self, raw: &str) -> KernelResult<PathBuf> {
        let trimmed = raw.trim().to_owned();
        if trimmed.is_empty() {
            return Err(KernelError::InvalidParams("worktree path is empty".into()));
        }
        let cfg = self.core_config().await?;
        let wt_root = PathBuf::from(&cfg.home_dir).join("worktrees");
        let candidate = PathBuf::from(&trimmed);
        if !candidate.is_absolute() {
            return Err(KernelError::InvalidParams(format!(
                "worktree path must be absolute: {raw}"
            )));
        }
        tokio::task::spawn_blocking(move || {
            let canon_root = std::fs::canonicalize(&wt_root).map_err(|_| {
                KernelError::NotFound(format!(
                    "worktrees directory does not exist: {}",
                    wt_root.display()
                ))
            })?;
            let canon_target = std::fs::canonicalize(&candidate).map_err(|e| {
                KernelError::NotFound(format!("cannot find worktree `{trimmed}`: {e}"))
            })?;
            if !canon_target.is_dir() {
                return Err(KernelError::InvalidParams(format!(
                    "not a directory: `{trimmed}`"
                )));
            }
            let rel = canon_target.strip_prefix(&canon_root).map_err(|_| {
                KernelError::InvalidParams(format!(
                    "refusing to delete directory outside `{}`",
                    canon_root.display()
                ))
            })?;
            // Require `<repo>/<worktree>` depth (>= 2 components) so `<wt_root>` or `<repo>`
            // itself is never deleted directly.
            if rel.components().count() < 2 {
                return Err(KernelError::InvalidParams(
                    "path must point to a worktree inside `<homeDir>/worktrees/<repo>/<name>`"
                        .into(),
                ));
            }
            Ok(canon_target)
        })
        .await
        .map_err(|e| KernelError::Io(e.to_string()))?
    }

    async fn remove_verified_worktree(&self, target: &Path) -> KernelResult<()> {
        let target_buf = target.to_path_buf();
        let main_repo = tokio::task::spawn_blocking({
            let target_buf = target_buf.clone();
            move || main_repo_from_worktree(&target_buf)
        })
        .await
        .unwrap_or(None);

        // First ask `git worktree remove --force` in the parent repository so git updates
        // `.git/worktrees/` cleanly.
        if let Some(repo_root) = &main_repo
            && let Some(target_str) = target.to_str()
        {
            let _ = self
                .run_git(
                    repo_root,
                    vec![
                        "worktree".into(),
                        "remove".into(),
                        "--force".into(),
                        target_str.to_owned(),
                    ],
                    false,
                )
                .await;
        }

        // If the directory still exists (e.g. orphaned worktree whose parent repo moved),
        // remove it directly and clean up an empty `<repo>` parent folder.
        let wt_root = PathBuf::from(&self.core_config().await?.home_dir).join("worktrees");
        tokio::task::spawn_blocking(move || -> Result<(), std::io::Error> {
            if target_buf.exists() {
                std::fs::remove_dir_all(&target_buf)?;
            }
            if let Some(repo_dir) = target_buf.parent()
                && let Ok(canon_root) = std::fs::canonicalize(&wt_root)
                && let Ok(canon_repo) = std::fs::canonicalize(repo_dir)
                && canon_repo != canon_root
                && canon_repo.starts_with(&canon_root)
            {
                let _ = std::fs::remove_dir(&canon_repo);
            }
            Ok(())
        })
        .await
        .map_err(|e| KernelError::Io(e.to_string()))?
        .map_err(|e| KernelError::Io(format!("failed to remove worktree directory: {e}")))?;

        if let Some(repo_root) = &main_repo {
            let _ = self
                .run_git(repo_root, vec!["worktree".into(), "prune".into()], false)
                .await;
        }
        Ok(())
    }

    async fn git_status_for_root(&self, root: &Path) -> KernelResult<WorkspaceGitInfo> {
        let probe = match self
            .run_git(
                root,
                vec![
                    "rev-parse".into(),
                    "--is-inside-work-tree".into(),
                    "--show-toplevel".into(),
                    "--git-dir".into(),
                    "--git-common-dir".into(),
                ],
                true,
            )
            .await
        {
            Ok(out) => out,
            Err(KernelError::NotFound(_)) => return Ok(WorkspaceGitInfo::default()),
            Err(e) => return Err(e),
        };

        if probe.code != Some(0) {
            return Ok(WorkspaceGitInfo::default());
        }

        let mut lines = probe.stdout.lines();
        if lines.next().map(str::trim) != Some("true") {
            return Ok(WorkspaceGitInfo::default());
        }
        let repo_root = lines.next().map(|s| s.trim().to_owned());
        let git_dir = lines.next().map(str::trim).unwrap_or(".git").to_owned();
        let git_common_dir = lines.next().map(str::trim).unwrap_or(".git").to_owned();

        let root_buf = root.to_path_buf();
        let is_worktree = tokio::task::spawn_blocking(move || {
            let resolve = |p: &str| {
                let pb = PathBuf::from(p);
                let full = if pb.is_absolute() {
                    pb
                } else {
                    root_buf.join(pb)
                };
                std::fs::canonicalize(&full).unwrap_or(full)
            };
            resolve(&git_dir) != resolve(&git_common_dir)
        })
        .await
        .unwrap_or(false);

        // Current branch or detached short SHA.
        let sym = self
            .run_git(
                root,
                vec![
                    "symbolic-ref".into(),
                    "--quiet".into(),
                    "--short".into(),
                    "HEAD".into(),
                ],
                true,
            )
            .await?;
        let (branch, detached) = if sym.code == Some(0) && !sym.stdout.trim().is_empty() {
            (Some(sym.stdout.trim().to_owned()), false)
        } else {
            let sha = self
                .run_git(
                    root,
                    vec!["rev-parse".into(), "--short".into(), "HEAD".into()],
                    true,
                )
                .await?;
            if sha.code == Some(0) && !sha.stdout.trim().is_empty() {
                (Some(sha.stdout.trim().to_owned()), true)
            } else {
                // Unborn branch on a fresh `git init` repo with no commits yet.
                (None, false)
            }
        };

        // List local branches with optional worktree paths.
        let raw_branches = self
            .run_git(
                root,
                vec![
                    "for-each-ref".into(),
                    "--format=%(HEAD)%09%(refname:short)%09%(worktreepath)".into(),
                    "refs/heads".into(),
                ],
                true,
            )
            .await?;
        let mut branches = if raw_branches.code == Some(0) {
            parse_branch_lines(&raw_branches.stdout)
        } else {
            let fallback = self
                .run_git(
                    root,
                    vec![
                        "for-each-ref".into(),
                        "--format=%(HEAD)%09%(refname:short)".into(),
                        "refs/heads".into(),
                    ],
                    true,
                )
                .await?;
            if fallback.code == Some(0) {
                parse_branch_lines(&fallback.stdout)
            } else {
                Vec::new()
            }
        };

        // Determine default branch (`origin/HEAD`, else `main`, else `master`).
        let origin_head = self
            .run_git(
                root,
                vec![
                    "symbolic-ref".into(),
                    "--quiet".into(),
                    "--short".into(),
                    "refs/remotes/origin/HEAD".into(),
                ],
                true,
            )
            .await?;
        let mut default_branch = if origin_head.code == Some(0) {
            let trimmed = origin_head.stdout.trim();
            trimmed
                .strip_prefix("origin/")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        } else {
            None
        };
        if default_branch.is_none() {
            if branches.iter().any(|b| b.name == "main") {
                default_branch = Some("main".to_owned());
            } else if branches.iter().any(|b| b.name == "master") {
                default_branch = Some("master".to_owned());
            }
        }

        // Sort: current branch first, then default branch, then alphabetical.
        let def_ref = default_branch.as_deref();
        branches.sort_by(|a, b| {
            let rank = |br: &GitBranch| {
                if br.current {
                    0u8
                } else if def_ref == Some(br.name.as_str()) {
                    1u8
                } else {
                    2u8
                }
            };
            rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
        });

        Ok(WorkspaceGitInfo {
            is_repo: true,
            branch,
            detached,
            is_worktree,
            repo_root,
            default_branch,
            branches,
        })
    }

    async fn run_git(
        &self,
        root: &Path,
        args: Vec<String>,
        read_only: bool,
    ) -> KernelResult<GitOut> {
        let owner = OwnerId(format!("kernel-git-{}", self.new_id()));
        let mut spawn = BTreeSet::new();
        spawn.insert("git".to_owned());
        let grants = Grants {
            spawn,
            ..Default::default()
        };
        let workspaces = [root.to_path_buf()];
        let no_secret = |_: &str| None;
        let ctx = SpawnContext {
            plugin: "kernel",
            owner: &owner,
            grants: &grants,
            workspaces: &workspaces,
            secret: &no_secret,
        };

        let mut full_args = vec![
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.hooksPath=/dev/null".into(),
        ];
        full_args.extend(args);

        let mut env = vec![("GIT_TERMINAL_PROMPT".into(), "0".into())];
        if read_only {
            env.push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));
        }

        let req = SpawnRequest {
            program: "git".into(),
            args: full_args,
            cwd: None,
            env,
            secret_env: Vec::new(),
        };

        let child = self
            .inner
            .supervisor
            .spawn(&ctx, req)
            .await
            .map_err(|e| match e {
                ProcessError::NotFound(m) => KernelError::NotFound(m),
                ProcessError::InvalidArgument(m) => KernelError::InvalidParams(m),
                ProcessError::PermissionDenied(m) => KernelError::Conflict(m),
                ProcessError::Io(m) => KernelError::Io(m),
            })?;

        let run = async {
            child.close_stdin().await;
            let mut stdout = Vec::new();
            while let Some(chunk) = child.read_stdout(64 * 1024).await {
                if stdout.len() < MAX_STDOUT {
                    let take = (MAX_STDOUT - stdout.len()).min(chunk.len());
                    stdout.extend_from_slice(&chunk[..take]);
                }
            }
            let status = child.wait().await;
            let mut stderr = Vec::new();
            while let Some(chunk) = child.read_stderr(16 * 1024).await {
                if stderr.len() < MAX_STDERR {
                    let take = (MAX_STDERR - stderr.len()).min(chunk.len());
                    stderr.extend_from_slice(&chunk[..take]);
                }
            }
            GitOut {
                code: status.code,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            }
        };

        let res = tokio::time::timeout(GIT_TIMEOUT, run).await;
        self.inner.supervisor.kill_owner(&owner).await;
        res.map_err(|_| KernelError::Io("git command timed out".into()))
    }
}

fn parse_branch_lines(text: &str) -> Vec<GitBranch> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let head = parts.next()?;
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let wt = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            Some(GitBranch {
                name: name.to_owned(),
                current: head.trim() == "*",
                worktree_path: wt,
            })
        })
        .collect()
}

fn validate_branch_name(raw: &str) -> KernelResult<String> {
    let branch = raw.trim();
    if branch.is_empty()
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || branch.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(KernelError::InvalidParams(format!(
            "invalid branch name `{raw}`"
        )));
    }
    Ok(branch.to_owned())
}

fn validate_ref_name(raw: &str) -> KernelResult<String> {
    let r = raw.trim();
    if r.is_empty() || r.starts_with('-') || r.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(KernelError::InvalidParams(format!(
            "invalid git ref `{raw}`"
        )));
    }
    Ok(r.to_owned())
}

fn derive_repo_name(root: &Path, toplevel: &str, common_dir: &str) -> String {
    let common_path = {
        let pb = PathBuf::from(common_dir);
        let full = if pb.is_absolute() { pb } else { root.join(pb) };
        std::fs::canonicalize(&full).unwrap_or(full)
    };
    if common_path.file_name().and_then(|n| n.to_str()) == Some(".git")
        && let Some(parent) = common_path.parent()
        && let Some(name) = parent.file_name()
    {
        return name.to_string_lossy().into_owned();
    }
    Path::new(toplevel)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_owned())
}

fn sanitize_path_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        let keep = ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-';
        if keep {
            out.push(ch);
            prev_dash = ch == '-';
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.');
    if trimmed.is_empty() {
        "worktree".to_owned()
    } else {
        trimmed.to_owned()
    }
}

struct ScannedWorktree {
    path: String,
    repo: String,
    name: String,
    branch: Option<String>,
    size_bytes: u64,
}

fn scan_worktrees(wt_root: &Path) -> Vec<ScannedWorktree> {
    let Ok(repos) = std::fs::read_dir(wt_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for repo_entry in repos.flatten() {
        let repo_path = repo_entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&repo_path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let repo_name = repo_entry.file_name().to_string_lossy().into_owned();
        let Ok(wts) = std::fs::read_dir(&repo_path) else {
            continue;
        };
        for wt_entry in wts.flatten() {
            let wt_path = wt_entry.path();
            let Ok(wt_meta) = std::fs::symlink_metadata(&wt_path) else {
                continue;
            };
            if !wt_meta.is_dir() {
                continue;
            }
            let canon = std::fs::canonicalize(&wt_path).unwrap_or(wt_path);
            let raw_str = canon.to_string_lossy();
            let clean_path = match raw_str.strip_prefix(r"\\?\") {
                Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
                _ => raw_str.into_owned(),
            };
            let name = wt_entry.file_name().to_string_lossy().into_owned();
            let branch = read_worktree_branch(&canon);
            let size_bytes = dir_size_bytes(&canon);
            out.push(ScannedWorktree {
                path: clean_path,
                repo: repo_name.clone(),
                name,
                branch,
                size_bytes,
            });
        }
    }
    out
}

/// Reads the checked-out branch name (or short SHA) directly from `<wt>/.git` without
/// spawning `git`.
fn read_worktree_branch(wt_dir: &Path) -> Option<String> {
    let git_dir = resolve_gitdir_file(wt_dir)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let trimmed = head.trim();
    if let Some(r) = trimmed.strip_prefix("ref:") {
        let target = r.trim();
        Some(
            target
                .strip_prefix("refs/heads/")
                .unwrap_or(target)
                .to_owned(),
        )
    } else if trimmed.len() >= 7 {
        Some(trimmed[..7].to_owned())
    } else if !trimmed.is_empty() {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

/// Resolves the `gitdir:` target inside a linked worktree's `.git` file.
fn resolve_gitdir_file(wt_dir: &Path) -> Option<PathBuf> {
    let dot_git = wt_dir.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let line = text.lines().next()?.trim();
    let target = line.strip_prefix("gitdir:")?.trim();
    let pb = PathBuf::from(target);
    let resolved = if pb.is_absolute() {
        pb
    } else {
        wt_dir.join(pb)
    };
    Some(std::fs::canonicalize(&resolved).unwrap_or(resolved))
}

/// Finds the primary repository working directory that owns a linked worktree.
fn main_repo_from_worktree(wt_dir: &Path) -> Option<PathBuf> {
    let git_dir = resolve_gitdir_file(wt_dir)?;
    // Linked worktrees store their metadata in `<repo>/.git/worktrees/<name>`.
    let worktrees_dir = git_dir.parent()?;
    if worktrees_dir.file_name().and_then(|n| n.to_str()) != Some("worktrees") {
        return None;
    }
    let dot_git = worktrees_dir.parent()?;
    if dot_git.file_name().and_then(|n| n.to_str()) != Some(".git") {
        return None;
    }
    let repo_root = dot_git.parent()?;
    repo_root.is_dir().then(|| repo_root.to_path_buf())
}

/// Recursively sums file sizes inside `root` without following symlinks.
fn dir_size_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            } else if meta.is_dir() {
                stack.push(p);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::kernel::KernelConfig;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .args(args)
            .output()
            .expect("git should be installed for tests");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn non_git_workspace_reports_is_repo_false() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("plain");
        std::fs::create_dir(&proj).unwrap();

        let k = Kernel::open(KernelConfig::new(tmp.path().join("db")))
            .await
            .unwrap();
        let ws = k.workspace_open(proj.to_str().unwrap()).await.unwrap();
        let info = k.workspace_git_status(&ws.id, None).await.unwrap();
        assert!(!info.is_repo);
        assert_eq!(info.branch, None);
        assert!(info.branches.is_empty());
    }

    #[tokio::test]
    async fn git_status_checkout_and_worktree_under_home_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("my-repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "initial"]);
        git(&repo, &["branch", "feature/ui"]);

        let mut cfg = KernelConfig::new(tmp.path().join("db"));
        let custom_home = tmp.path().join("kk-home");
        cfg.default_home_dir = custom_home.clone();
        let k = Kernel::open(cfg).await.unwrap();

        // Core config defaults and overrides.
        let cc = k.core_config().await.unwrap();
        assert_eq!(PathBuf::from(&cc.home_dir), custom_home);
        let override_home = tmp.path().join("custom-kk");
        let cc2 = k
            .set_core_config(Some(override_home.to_str().unwrap().into()))
            .await
            .unwrap();
        assert_eq!(PathBuf::from(&cc2.home_dir), override_home);

        // Status on main.
        let ws = k.workspace_open(repo.to_str().unwrap()).await.unwrap();
        let st = k.workspace_git_status(&ws.id, None).await.unwrap();
        assert!(st.is_repo);
        assert!(!st.is_worktree);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.default_branch.as_deref(), Some("main"));
        assert_eq!(st.branches.len(), 2);

        // Checkout feature/ui and back to main.
        let st = k
            .workspace_git_checkout(&ws.id, "feature/ui", None)
            .await
            .unwrap();
        assert_eq!(st.branch.as_deref(), Some("feature/ui"));
        let st = k
            .workspace_git_checkout(&ws.id, "main", None)
            .await
            .unwrap();
        assert_eq!(st.branch.as_deref(), Some("main"));

        // Create a worktree from main -> lands under `<override_home>/worktrees/my-repo/feat-wt`
        // WITHOUT registering a separate workspace.
        let wt = k
            .workspace_git_worktree_create(&ws.id, "feat/wt", Some("main"))
            .await
            .unwrap();
        assert!(wt.git.is_repo);
        assert!(wt.git.is_worktree);
        assert_eq!(wt.git.branch.as_deref(), Some("feat/wt"));
        let raw_expected = std::fs::canonicalize(&override_home)
            .unwrap()
            .join("worktrees")
            .join("my-repo")
            .join("feat-wt");
        let raw_expected_str = raw_expected.to_string_lossy();
        let expected_prefix = match raw_expected_str.strip_prefix(r"\\?\") {
            Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
            _ => raw_expected.clone(),
        };
        assert_eq!(PathBuf::from(&wt.path), expected_prefix);

        // Only the parent workspace is registered; worktree threads live under it.
        let all_ws = k.workspace_list().await.unwrap();
        assert_eq!(all_ws.len(), 1);
        assert_eq!(all_ws[0].id, ws.id);

        // Querying status with the worktree path inspects the linked worktree.
        let wt_st = k
            .workspace_git_status(&ws.id, Some(&wt.path))
            .await
            .unwrap();
        assert!(wt_st.is_worktree);
        assert_eq!(wt_st.branch.as_deref(), Some("feat/wt"));

        // Creating a thread with worktree_path persists it under the parent workspace,
        // and archive -> unarchive -> delete lifecycle works cleanly.
        let thread = k
            .thread_insert_with_worktree(
                &ws.id,
                "p".into(),
                "m".into(),
                Some("wt task".into()),
                None,
                Some(wt.path.clone()),
            )
            .await
            .unwrap();
        assert_eq!(thread.workspace, ws.id);
        assert_eq!(thread.worktree_path.as_deref(), Some(wt.path.as_str()));

        k.thread_archive(&thread.id).await.unwrap();
        let archived = k.thread_list(&ws.id, true).await.unwrap();
        assert_eq!(
            archived[0].status,
            kumikit_protocol::model::ThreadStatus::Archived
        );

        let restored = k.thread_unarchive(&thread.id).await.unwrap();
        assert_eq!(restored.status, kumikit_protocol::model::ThreadStatus::Idle);

        // Managed worktree list reports the created worktree, its branch, disk usage, and thread count.
        let listed = k.list_managed_worktrees().await.unwrap();
        assert_eq!(listed.worktrees.len(), 1);
        assert_eq!(listed.worktrees[0].path, wt.path);
        assert_eq!(listed.worktrees[0].repo, "my-repo");
        assert_eq!(listed.worktrees[0].branch.as_deref(), Some("feat/wt"));
        assert_eq!(listed.worktrees[0].thread_count, 1);
        assert!(listed.worktrees[0].size_bytes > 0);
        assert_eq!(listed.total_bytes, listed.worktrees[0].size_bytes);

        // Deleting the last thread using a managed worktree also deletes its worktree folder.
        k.thread_delete(&thread.id).await.unwrap();
        assert!(k.thread_list(&ws.id, true).await.unwrap().is_empty());
        assert!(
            !Path::new(&wt.path).exists(),
            "worktree folder should be deleted with its thread"
        );
        assert!(
            k.list_managed_worktrees()
                .await
                .unwrap()
                .worktrees
                .is_empty()
        );

        // Creating another worktree and deleting it directly via `delete_managed_worktree`.
        let wt2 = k
            .workspace_git_worktree_create(&ws.id, "feat/wt2", Some("main"))
            .await
            .unwrap();
        assert!(Path::new(&wt2.path).exists());
        // Refuses paths outside `<homeDir>/worktrees/`.
        assert!(
            k.delete_managed_worktree(repo.to_str().unwrap())
                .await
                .is_err()
        );
        k.delete_managed_worktree(&wt2.path).await.unwrap();
        assert!(!Path::new(&wt2.path).exists());

        // Resetting home dir restores the default.
        let cc3 = k.set_core_config(None).await.unwrap();
        assert_eq!(PathBuf::from(&cc3.home_dir), custom_home);
    }

    #[test]
    fn sanitize_segments() {
        assert_eq!(
            sanitize_path_segment("feat/ui-worktree"),
            "feat-ui-worktree"
        );
        assert_eq!(sanitize_path_segment("..."), "worktree");
        assert_eq!(sanitize_path_segment("m6-ui-design"), "m6-ui-design");
    }
}
