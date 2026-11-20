//! Schedules (`docs/spec/03-kernel.md#schedules`, ADR 0024): rows in the core database,
//! one scheduler task that sleeps until the earliest enabled run, and a runner (the plugin
//! host) that wakes the owning plugin. With no enabled schedule nothing is armed.
//!
//! [`Kernel::schedule_tick`] holds the timing rules and takes the time as an argument, so
//! tests drive it without waiting.

pub mod cron;

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use jiff::{SignedDuration, Timestamp};
use rusqlite::{OptionalExtension, params};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::{KernelError, KernelResult};
use crate::kernel::{Kernel, lock};
use crate::provider::BoxFuture;

/// Most schedules one plugin may have.
pub const MAX_PER_PLUGIN: usize = 64;
/// A run this late (core down, machine asleep) counts as missed.
pub const LATE: SignedDuration = SignedDuration::from_mins(5);
/// Prefix of the keys of manifest triggers.
pub const MANIFEST_PREFIX: &str = "manifest:";

/// What to do with a run that was missed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missed {
    /// Record it as missed.
    Skip,
    /// Run once now.
    Once,
}

impl Missed {
    fn as_str(self) -> &'static str {
        match self {
            Missed::Skip => "skip",
            Missed::Once => "once",
        }
    }
}

/// Who made a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A `schedule:<cron>` trigger in the manifest.
    Manifest,
    /// The plugin, through `kumikit:host/timers`.
    Plugin,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Manifest => "manifest",
            Source::Plugin => "plugin",
        }
    }
}

/// What a schedule is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The cron expression.
    pub cron: String,
    /// IANA zone, or the core's local zone.
    pub zone: Option<String>,
    /// What it does, in the plugin's words.
    pub description: String,
    /// Missed-run policy.
    pub missed: Missed,
}

/// A stored schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The owning plugin.
    pub plugin: String,
    /// Its key.
    pub key: String,
    /// What it is.
    pub spec: Spec,
    /// Whether it runs.
    pub enabled: bool,
    /// Who made it.
    pub source: Source,
    /// Next run, RFC 3339 UTC.
    pub next_run: Option<String>,
    /// Last run (or missed or skipped run), RFC 3339 UTC.
    pub last_run: Option<String>,
    /// `ran`, `failed`, `missed` or `skipped`.
    pub last_outcome: Option<String>,
}

/// What wakes plugins: the plugin host.
pub trait Runner: Send + Sync {
    /// Whether the plugin is enabled and granted `schedules`.
    fn runnable(&self, plugin: &str) -> bool;
    /// Calls the plugin's `timers.fire`; resolves when the call returns.
    fn fire(
        &self,
        plugin: String,
        key: String,
        due: String,
    ) -> BoxFuture<'static, Result<(), String>>;
}

/// The kernel's scheduler state.
#[derive(Default)]
pub(crate) struct State {
    runner: OnceLock<Arc<dyn Runner>>,
    running: Mutex<HashSet<(String, String)>>,
    changed: Arc<Notify>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("schedule::State")
            .field("runner", &self.runner.get().is_some())
            .finish_non_exhaustive()
    }
}

/// What a tick did with one schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It was due and its plugin was woken.
    Fired {
        plugin: String,
        key: String,
        due: String,
    },
    /// It was due too long ago and its policy is `skip`.
    Missed {
        plugin: String,
        key: String,
        due: String,
    },
    /// It was due while its previous run was still going.
    Skipped {
        plugin: String,
        key: String,
        due: String,
    },
}

fn parse_ts(s: &str) -> Option<Timestamp> {
    s.parse().ok()
}

fn next_run(spec: &Spec, after: Timestamp) -> KernelResult<Option<String>> {
    let cron = cron::Cron::parse(&spec.cron).map_err(KernelError::InvalidParams)?;
    let zone = cron::zone(spec.zone.as_deref()).map_err(KernelError::InvalidParams)?;
    Ok(cron.next_after(after, &zone).map(|t| t.to_string()))
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let missed: String = r.get(5)?;
    let source: String = r.get(7)?;
    Ok(Row {
        plugin: r.get(0)?,
        key: r.get(1)?,
        spec: Spec {
            cron: r.get(2)?,
            zone: r.get(3)?,
            description: r.get(4)?,
            missed: if missed == "once" {
                Missed::Once
            } else {
                Missed::Skip
            },
        },
        enabled: r.get::<_, i64>(6)? != 0,
        source: if source == "manifest" {
            Source::Manifest
        } else {
            Source::Plugin
        },
        next_run: r.get(8)?,
        last_run: r.get(9)?,
        last_outcome: r.get(10)?,
    })
}

const COLUMNS: &str = "plugin, key, cron, zone, description, missed, enabled, source, next_run, last_run, last_outcome";

impl Kernel {
    fn schedules(&self) -> &State {
        &self.inner.schedules
    }

    /// Wakes the scheduler task to re-read the table (after any change, and when a
    /// plugin's state changes).
    pub fn schedules_changed(&self) {
        self.schedules().changed.notify_one();
    }

    /// Every schedule on the core, by plugin and key.
    pub async fn schedule_list(&self) -> KernelResult<Vec<Row>> {
        self.inner
            .store
            .read(|c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT {COLUMNS} FROM schedules ORDER BY plugin, key"
                ))?;
                let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
                Ok::<_, KernelError>(rows)
            })
            .await
    }

    async fn schedule_get(&self, plugin: &str, key: &str) -> KernelResult<Option<Row>> {
        let (p, k) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .read(move |c| {
                Ok::<_, KernelError>(
                    c.query_row(
                        &format!("SELECT {COLUMNS} FROM schedules WHERE plugin = ?1 AND key = ?2"),
                        [&p, &k],
                        row,
                    )
                    .optional()?,
                )
            })
            .await
    }

    /// Adds or changes a schedule; returns its next run. Keeps `enabled` and the last run
    /// of an existing one. Fails with `invalid_params` for a bad cron or zone and `limit`
    /// past [`MAX_PER_PLUGIN`].
    pub async fn schedule_set(
        &self,
        plugin: &str,
        key: &str,
        spec: Spec,
        source: Source,
        now: Timestamp,
    ) -> KernelResult<Option<String>> {
        if key.is_empty() || key.len() > 128 {
            return Err(KernelError::InvalidParams(
                "a schedule key is 1 to 128 characters".into(),
            ));
        }
        let next = next_run(&spec, now)?;
        let existing = self.schedule_get(plugin, key).await?;
        if existing.is_none() {
            let count = self
                .schedule_list()
                .await?
                .iter()
                .filter(|r| r.plugin == plugin)
                .count();
            if count >= MAX_PER_PLUGIN {
                return Err(KernelError::Limit(format!(
                    "a plugin has at most {MAX_PER_PLUGIN} schedules"
                )));
            }
        }
        let (p, k, n) = (plugin.to_owned(), key.to_owned(), next.clone());
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "INSERT INTO schedules (plugin, key, cron, zone, description, missed, enabled, source, next_run)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8)
                     ON CONFLICT (plugin, key) DO UPDATE SET cron = excluded.cron, zone = excluded.zone,
                       description = excluded.description, missed = excluded.missed,
                       source = excluded.source, next_run = excluded.next_run",
                    params![p, k, spec.cron, spec.zone, spec.description, spec.missed.as_str(), source.as_str(), n],
                )?;
                Ok::<_, KernelError>(())
            })
            .await?;
        self.schedules_changed();
        Ok(next)
    }

    /// Removes a schedule; a missing one is fine.
    pub async fn schedule_delete(&self, plugin: &str, key: &str) -> KernelResult<()> {
        let (p, k) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "DELETE FROM schedules WHERE plugin = ?1 AND key = ?2",
                    [&p, &k],
                )?;
                Ok::<_, KernelError>(())
            })
            .await?;
        self.schedules_changed();
        Ok(())
    }

    /// Pauses or resumes a schedule (`not_found` if there is none). Resuming computes the
    /// next run from `now`.
    pub async fn schedule_set_enabled(
        &self,
        plugin: &str,
        key: &str,
        enabled: bool,
        now: Timestamp,
    ) -> KernelResult<()> {
        let r = self
            .schedule_get(plugin, key)
            .await?
            .ok_or_else(|| KernelError::NotFound(format!("{plugin} has no schedule {key}")))?;
        let next = if enabled {
            next_run(&r.spec, now)?
        } else {
            r.next_run.clone()
        };
        let (p, k) = (plugin.to_owned(), key.to_owned());
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "UPDATE schedules SET enabled = ?3, next_run = ?4 WHERE plugin = ?1 AND key = ?2",
                    params![p, k, i64::from(enabled), next],
                )?;
                Ok::<_, KernelError>(())
            })
            .await?;
        self.schedules_changed();
        Ok(())
    }

    /// Makes the manifest's `schedule:<cron>` triggers the plugin's manifest schedules
    /// (keys `manifest:<n>`), removing ones the manifest no longer has.
    pub async fn schedule_sync_manifest(
        &self,
        plugin: &str,
        crons: &[String],
        now: Timestamp,
    ) -> KernelResult<()> {
        for (n, cron) in crons.iter().enumerate() {
            let key = format!("{MANIFEST_PREFIX}{n}");
            let same = self
                .schedule_get(plugin, &key)
                .await?
                .is_some_and(|r| r.spec.cron == *cron);
            if !same {
                let spec = Spec {
                    cron: cron.clone(),
                    zone: None,
                    description: format!("On the schedule `{cron}` from its manifest"),
                    missed: Missed::Skip,
                };
                self.schedule_set(plugin, &key, spec, Source::Manifest, now)
                    .await?;
            }
        }
        for r in self.schedule_list().await? {
            if r.plugin == plugin
                && r.source == Source::Manifest
                && r.key
                    .strip_prefix(MANIFEST_PREFIX)
                    .and_then(|n| n.parse::<usize>().ok())
                    .is_none_or(|n| n >= crons.len())
            {
                self.schedule_delete(plugin, &r.key).await?;
            }
        }
        Ok(())
    }

    /// Removes every schedule of a plugin (uninstall).
    pub async fn schedules_delete_plugin(&self, plugin: &str) -> KernelResult<()> {
        let p = plugin.to_owned();
        self.inner
            .store
            .write(move |c| {
                c.execute("DELETE FROM schedules WHERE plugin = ?1", [&p])?;
                Ok::<_, KernelError>(())
            })
            .await?;
        self.schedules_changed();
        Ok(())
    }

    fn runnable(&self, plugin: &str) -> bool {
        self.schedules()
            .runner
            .get()
            .is_some_and(|r| r.runnable(plugin))
    }

    /// When the scheduler must wake next: the earliest next run of an enabled schedule
    /// whose plugin may run. `None` arms no timer.
    pub async fn schedule_wake(&self) -> KernelResult<Option<Timestamp>> {
        Ok(self
            .schedule_list()
            .await?
            .into_iter()
            .filter(|r| r.enabled && self.runnable(&r.plugin))
            .filter_map(|r| r.next_run.as_deref().and_then(parse_ts))
            .min())
    }

    async fn record(
        &self,
        plugin: &str,
        key: &str,
        next: Option<String>,
        last: Option<(&str, &str)>,
    ) -> KernelResult<()> {
        let (p, k) = (plugin.to_owned(), key.to_owned());
        let (when, outcome) = match last {
            Some((w, o)) => (Some(w.to_owned()), Some(o.to_owned())),
            None => (None, None),
        };
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "UPDATE schedules SET next_run = ?3,
                       last_run = COALESCE(?4, last_run), last_outcome = COALESCE(?5, last_outcome)
                     WHERE plugin = ?1 AND key = ?2",
                    params![p, k, next, when, outcome],
                )?;
                Ok::<_, KernelError>(())
            })
            .await
    }

    /// Records how a run went, leaving the next run as the tick set it.
    async fn record_last(
        &self,
        plugin: &str,
        key: &str,
        due: &str,
        outcome: &str,
    ) -> KernelResult<()> {
        let (p, k, d, o) = (
            plugin.to_owned(),
            key.to_owned(),
            due.to_owned(),
            outcome.to_owned(),
        );
        self.inner
            .store
            .write(move |c| {
                c.execute(
                    "UPDATE schedules SET last_run = ?3, last_outcome = ?4 WHERE plugin = ?1 AND key = ?2",
                    params![p, k, d, o],
                )?;
                Ok::<_, KernelError>(())
            })
            .await
    }

    fn start_fire(&self, plugin: &str, key: &str, due: &str) -> bool {
        let Some(runner) = self.schedules().runner.get().cloned() else {
            return false;
        };
        let id = (plugin.to_owned(), key.to_owned());
        if !lock(&self.schedules().running).insert(id.clone()) {
            return false;
        }
        let kernel = self.clone();
        let due = due.to_owned();
        tokio::spawn(async move {
            let result = runner.fire(id.0.clone(), id.1.clone(), due.clone()).await;
            let outcome = match &result {
                Ok(()) => "ran",
                Err(e) => {
                    tracing::warn!(plugin = %id.0, key = %id.1, error = %e, "a scheduled run failed");
                    "failed"
                }
            };
            lock(&kernel.schedules().running).remove(&id);
            let _ = kernel.record_last(&id.0, &id.1, &due, outcome).await;
        });
        true
    }

    /// Handles every schedule due at `now`: fires it, or records it as missed (late and
    /// `skip`) or skipped (still running), and moves its next run past `now`.
    pub async fn schedule_tick(&self, now: Timestamp) -> KernelResult<Vec<Outcome>> {
        let mut out = Vec::new();
        for r in self.schedule_list().await? {
            if !r.enabled || !self.runnable(&r.plugin) {
                continue;
            }
            let Some(due) = r.next_run.as_deref().and_then(parse_ts) else {
                continue;
            };
            if due > now {
                continue;
            }
            let due_s = due.to_string();
            let next = next_run(&r.spec, now)?;
            let late = now.duration_since(due) > LATE;
            let (plugin, key) = (r.plugin.clone(), r.key.clone());
            if late && r.spec.missed == Missed::Skip {
                self.record(&plugin, &key, next, Some((&due_s, "missed")))
                    .await?;
                out.push(Outcome::Missed {
                    plugin,
                    key,
                    due: due_s,
                });
            } else if self.start_fire(&plugin, &key, &due_s) {
                self.record(&plugin, &key, next, None).await?;
                out.push(Outcome::Fired {
                    plugin,
                    key,
                    due: due_s,
                });
            } else {
                self.record(&plugin, &key, next, Some((&due_s, "skipped")))
                    .await?;
                out.push(Outcome::Skipped {
                    plugin,
                    key,
                    due: due_s,
                });
            }
        }
        Ok(out)
    }

    /// Runs a schedule now, whatever its time ("Run now"). `conflict` while its last run
    /// is still going or its plugin may not run.
    pub async fn schedule_run_now(
        &self,
        plugin: &str,
        key: &str,
        now: Timestamp,
    ) -> KernelResult<()> {
        self.schedule_get(plugin, key)
            .await?
            .ok_or_else(|| KernelError::NotFound(format!("{plugin} has no schedule {key}")))?;
        if !self.runnable(plugin) {
            return Err(KernelError::Conflict(format!(
                "{plugin} is not enabled with schedules"
            )));
        }
        if !self.start_fire(plugin, key, &now.to_string()) {
            return Err(KernelError::Conflict("its last run is still going".into()));
        }
        Ok(())
    }

    /// Installs the runner and starts the scheduler task. Called once by the plugin host.
    pub fn start_scheduler(&self, runner: Arc<dyn Runner>) {
        if self.schedules().runner.set(runner).is_err() {
            return;
        }
        let kernel = self.clone();
        let changed = Arc::clone(&self.schedules().changed);
        let task = tokio::spawn(async move {
            loop {
                // A change between the read and the wait still wakes it: `notify_one`
                // keeps a permit.
                let wake = match kernel.schedule_wake().await {
                    Ok(w) => w,
                    Err(e) => {
                        tracing::debug!(error = %e, "scheduler stops");
                        return;
                    }
                };
                match wake {
                    None => changed.notified().await,
                    Some(at) => {
                        let wait = Timestamp::now().duration_until(at);
                        let wait = std::time::Duration::try_from(wait).unwrap_or_default();
                        tokio::select! {
                            () = tokio::time::sleep(wait) => {}
                            () = changed.notified() => continue,
                        }
                    }
                }
                if let Err(e) = kernel.schedule_tick(Timestamp::now()).await {
                    tracing::debug!(error = %e, "scheduler stops");
                    return;
                }
            }
        });
        *lock(&self.schedules().task) = Some(task);
    }

    /// Installs the runner without spawning the wall-clock task, so tests that drive
    /// `schedule_tick` with explicit timestamps do not race with real time.
    #[cfg(test)]
    pub(crate) fn set_schedule_runner(&self, runner: Arc<dyn Runner>) {
        let _ = self.schedules().runner.set(runner);
    }

    /// Stops the scheduler task (kernel close).
    pub(crate) fn stop_scheduler(&self) {
        if let Some(t) = lock(&self.schedules().task).take() {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests;
