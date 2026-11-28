//! Usage statistics and plan limits (`docs/spec/03-kernel.md#usage`, ADR 0035).
//!
//! - Every `usage` event updates its turn's row in `usage_records`, in the same
//!   transaction ([`record`]). `usage` reports a turn's running total, so the row keeps the
//!   last one. Rows outlive their thread and hold numbers only.
//! - Providers report plan limits while a turn runs ([`Kernel::report_limits`]) or when the
//!   user asks ([`Kernel::usage_refresh`]); the kernel keeps the latest snapshot per
//!   provider and announces changes on the `usage` topic.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use kumikit_protocol::UsageEvent;
use kumikit_protocol::event::{Usage, UsageRecorded};
use kumikit_protocol::methods::{
    DailyUsage, ModelUsageRow, UsageSummary, UsageSummaryParams, UsageTotals,
};
use kumikit_protocol::model::{ModelUsage, ProviderLimits, ThreadId, Timestamp, UsageLimits};
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::broadcast;

use crate::error::{KernelError, KernelResult};
use crate::kernel::{Kernel, lock};

/// Capacity of the `usage` topic's channel; a slower listener resyncs.
pub(crate) const USAGE_EVENTS: usize = 64;
/// The longest `usage.summary` period, in days.
pub const MAX_SUMMARY_DAYS: u32 = 400;
/// A refresh this soon after the last one returns the stored snapshot.
pub const REFRESH_INTERVAL_SECS: i64 = 60;
/// Most windows kept from one snapshot.
const MAX_WINDOWS: usize = 32;
/// Longest label, group, id, plan or account kept, in bytes.
const MAX_TEXT: usize = 200;
/// Most models kept in one `usage` event.
pub(crate) const MAX_MODELS: usize = 32;

/// State of the usage feature, kept in the kernel.
#[derive(Debug)]
pub(crate) struct State {
    pub(crate) events: broadcast::Sender<UsageEvent>,
    /// When each provider was last refreshed on demand, by the kernel clock.
    refreshed: Mutex<HashMap<String, Timestamp>>,
    /// One lock per provider, so concurrent refreshes run once.
    refreshing: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for State {
    fn default() -> Self {
        State {
            events: broadcast::Sender::new(USAGE_EVENTS),
            refreshed: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(HashMap::new()),
        }
    }
}

/// Records a `usage` event on its turn's row. Called by `repo::append_event` in the event's
/// transaction. A usage event of a turn that has no row (none in this thread) is ignored.
pub(crate) fn record(conn: &Connection, thread: &ThreadId, usage: &Usage) -> KernelResult<()> {
    let turn: Option<(String, String, String)> = conn
        .query_row(
            "SELECT coalesce(t.provider, th.provider), coalesce(t.model, th.model), t.started
             FROM turns t JOIN threads th ON th.id = t.thread
             WHERE t.id = ?1 AND t.thread = ?2",
            params![usage.turn.as_str(), thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((provider, model, started)) = turn else {
        return Ok(());
    };
    let models = if usage.models.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&usage.models)?)
    };
    conn.execute(
        "INSERT INTO usage_records
             (turn, thread, provider, model, started, input_tokens, output_tokens,
              cached_input_tokens, cost, models)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (turn) DO UPDATE SET
             input_tokens = excluded.input_tokens,
             output_tokens = excluded.output_tokens,
             cached_input_tokens = excluded.cached_input_tokens,
             cost = excluded.cost,
             models = excluded.models",
        params![
            usage.turn.as_str(),
            thread.as_str(),
            provider,
            model,
            started,
            clamp(usage.input_tokens),
            clamp(usage.output_tokens),
            clamp(usage.cached_input_tokens.unwrap_or(0)),
            usage.cost.filter(|c| c.is_finite()),
            models,
        ],
    )?;
    Ok(())
}

fn clamp(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn unclamp(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// One turn's usage, as stored.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub(crate) thread: String,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) started: String,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cost: Option<f64>,
    pub(crate) models: Vec<ModelUsage>,
}

fn records_since(
    conn: &Connection,
    since: &str,
    provider: Option<&str>,
) -> KernelResult<Vec<Record>> {
    let mut stmt = conn.prepare(
        "SELECT thread, provider, model, started, input_tokens, output_tokens,
                cached_input_tokens, cost, models
         FROM usage_records
         WHERE started >= ?1 AND (?2 IS NULL OR provider = ?2)
         ORDER BY started",
    )?;
    let rows = stmt.query_map(params![since, provider], |r| {
        Ok((
            Record {
                thread: r.get(0)?,
                provider: r.get(1)?,
                model: r.get(2)?,
                started: r.get(3)?,
                input_tokens: unclamp(r.get(4)?),
                output_tokens: unclamp(r.get(5)?),
                cached_input_tokens: unclamp(r.get(6)?),
                cost: r.get(7)?,
                models: Vec::new(),
            },
            r.get::<_, Option<String>>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (mut record, models) = row?;
        if let Some(json) = models {
            // A breakdown that no longer parses still counts under the turn's model.
            record.models = serde_json::from_str(&json).unwrap_or_default();
        }
        out.push(record);
    }
    Ok(out)
}

fn limits_row(conn: &Connection, provider: &str) -> KernelResult<Option<(Option<String>, String)>> {
    Ok(conn
        .query_row(
            "SELECT limits, observed FROM provider_limits WHERE provider = ?1",
            [provider],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn limits_put(
    conn: &Connection,
    provider: &str,
    limits: Option<&UsageLimits>,
    observed: &Timestamp,
) -> KernelResult<()> {
    let json = limits.map(serde_json::to_string).transpose()?;
    conn.execute(
        "INSERT INTO provider_limits (provider, limits, observed) VALUES (?1, ?2, ?3)
         ON CONFLICT (provider) DO UPDATE SET limits = excluded.limits,
                                              observed = excluded.observed",
        params![provider, json, observed.0],
    )?;
    Ok(())
}

fn to_provider_limits(provider: &str, row: Option<(Option<String>, String)>) -> ProviderLimits {
    match row {
        None => ProviderLimits {
            provider: provider.to_owned(),
            limits: None,
            observed: None,
        },
        Some((limits, observed)) => ProviderLimits {
            provider: provider.to_owned(),
            limits: limits.and_then(|j| serde_json::from_str(&j).ok()),
            observed: Some(Timestamp(observed)),
        },
    }
}

fn cut(s: String) -> String {
    if s.len() <= MAX_TEXT {
        return s;
    }
    let mut end = MAX_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// Checks and tidies a snapshot from a provider: percentages within 0–100 (a NaN is
/// refused), at most [`MAX_WINDOWS`] windows, texts cut to [`MAX_TEXT`], an account URL
/// only when it is `https`, and reset times that parse.
pub fn sanitize_limits(mut limits: UsageLimits) -> KernelResult<UsageLimits> {
    if limits.windows.len() > MAX_WINDOWS {
        return Err(KernelError::InvalidParams(format!(
            "at most {MAX_WINDOWS} limit windows"
        )));
    }
    for w in &mut limits.windows {
        if !w.used_percent.is_finite() {
            return Err(KernelError::InvalidParams(format!(
                "window {} has no valid percentage",
                w.id
            )));
        }
        w.used_percent = w.used_percent.clamp(0.0, 100.0);
        w.id = cut(std::mem::take(&mut w.id));
        w.label = cut(std::mem::take(&mut w.label));
        w.group = w.group.take().map(cut);
        w.model = w.model.take().map(cut);
        if let Some(at) = &w.resets_at
            && at.0.parse::<jiff::Timestamp>().is_err()
        {
            return Err(KernelError::InvalidParams(format!(
                "window {} has a reset time that is not RFC 3339: {}",
                w.id, at.0
            )));
        }
    }
    limits.plan = limits.plan.take().map(cut);
    limits.account = limits.account.take().map(cut);
    limits.account_url = limits.account_url.take().filter(|u| {
        u.starts_with("https://") && u.len() <= 2048 && !u.contains(char::is_whitespace)
    });
    Ok(limits)
}

/// The first instant counted by a `days`-long period ending today, in the client's time
/// zone, as a stored timestamp string.
fn window_start(now: &Timestamp, days: u32, offset_minutes: i32) -> KernelResult<String> {
    let now: jiff::Timestamp = now
        .0
        .parse()
        .map_err(|e| KernelError::Io(format!("cannot read the clock: {e}")))?;
    let tz = fixed_zone(offset_minutes)?;
    let today = now.to_zoned(tz.clone()).date();
    let first = today
        .checked_sub(jiff::Span::new().days(i64::from(days.saturating_sub(1))))
        .map_err(|e| KernelError::InvalidParams(e.to_string()))?;
    let start = first
        .to_zoned(tz)
        .map_err(|e| KernelError::InvalidParams(e.to_string()))?
        .timestamp();
    Ok(start.strftime("%Y-%m-%dT%H:%M:%S.000Z").to_string())
}

fn fixed_zone(offset_minutes: i32) -> KernelResult<jiff::tz::TimeZone> {
    if !(-18 * 60..=18 * 60).contains(&offset_minutes) {
        return Err(KernelError::InvalidParams(
            "tzOffsetMinutes must be between -1080 and 1080".into(),
        ));
    }
    let offset = jiff::tz::Offset::from_seconds(offset_minutes * 60)
        .map_err(|e| KernelError::InvalidParams(e.to_string()))?;
    Ok(jiff::tz::TimeZone::fixed(offset))
}

fn add(t: &mut UsageTotals, input: u64, output: u64, cached: u64, cost: Option<f64>) {
    t.input_tokens = t.input_tokens.saturating_add(input);
    t.output_tokens = t.output_tokens.saturating_add(output);
    t.cached_input_tokens = t.cached_input_tokens.saturating_add(cached);
    if let Some(c) = cost {
        t.cost = Some(t.cost.unwrap_or(0.0) + c);
    }
}

/// Adds up records into a summary; `daily` uses days at `offset_minutes` from UTC. Pure,
/// so the rules are unit-tested here: a turn with a `models` breakdown counts per model,
/// one without counts under its own model, and each turn counts once in `turns`.
pub(crate) fn summarize(records: &[Record], offset_minutes: i32) -> KernelResult<UsageSummary> {
    let tz = fixed_zone(offset_minutes)?;
    let mut totals = UsageTotals::default();
    let mut threads = BTreeSet::new();
    let mut models: BTreeMap<(String, String), UsageTotals> = BTreeMap::new();
    let mut daily: BTreeMap<String, (u64, Option<f64>)> = BTreeMap::new();
    for r in records {
        add(
            &mut totals,
            r.input_tokens,
            r.output_tokens,
            r.cached_input_tokens,
            r.cost,
        );
        totals.turns = totals.turns.saturating_add(1);
        threads.insert(r.thread.as_str());
        if r.models.is_empty() {
            let row = models
                .entry((r.provider.clone(), r.model.clone()))
                .or_default();
            add(
                row,
                r.input_tokens,
                r.output_tokens,
                r.cached_input_tokens,
                r.cost,
            );
            row.turns = row.turns.saturating_add(1);
        } else {
            for m in &r.models {
                let row = models
                    .entry((r.provider.clone(), m.model.clone()))
                    .or_default();
                add(
                    row,
                    m.input_tokens,
                    m.output_tokens,
                    m.cached_input_tokens.unwrap_or(0),
                    m.cost,
                );
                row.turns = row.turns.saturating_add(1);
            }
        }
        let Ok(at) = r.started.parse::<jiff::Timestamp>() else {
            continue;
        };
        let date = at.to_zoned(tz.clone()).date().to_string();
        let day = daily.entry(date).or_insert((0, None));
        day.0 = day
            .0
            .saturating_add(r.input_tokens.saturating_add(r.output_tokens));
        if let Some(c) = r.cost {
            day.1 = Some(day.1.unwrap_or(0.0) + c);
        }
    }
    let mut models: Vec<ModelUsageRow> = models
        .into_iter()
        .map(|((provider, model), totals)| ModelUsageRow {
            provider,
            model,
            totals,
        })
        .collect();
    models.sort_by(|a, b| {
        let tokens = |r: &ModelUsageRow| r.totals.input_tokens + r.totals.output_tokens;
        tokens(b)
            .cmp(&tokens(a))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model.cmp(&b.model))
    });
    Ok(UsageSummary {
        totals,
        threads: u32::try_from(threads.len()).unwrap_or(u32::MAX),
        models,
        daily: daily
            .into_iter()
            .map(|(date, (tokens, cost))| DailyUsage { date, tokens, cost })
            .collect(),
    })
}

impl Kernel {
    /// Live changes for the `usage` topic. Nothing is kept for late listeners.
    pub fn usage_changes(&self) -> broadcast::Receiver<UsageEvent> {
        self.inner.usage.events.subscribe()
    }

    pub(crate) fn announce_usage(&self, event: UsageEvent) {
        // No listener is fine.
        let _ = self.inner.usage.events.send(event);
    }

    /// Announces `usage.recorded` for a turn's usage that was just committed.
    pub(crate) async fn usage_recorded(&self, thread: &ThreadId, usage: &Usage) {
        let turn = usage.turn.clone();
        let provider = self
            .inner
            .store
            .read(move |c| {
                Ok::<_, KernelError>(
                    c.query_row(
                        "SELECT provider FROM usage_records WHERE turn = ?1",
                        [turn.as_str()],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?,
                )
            })
            .await;
        if let Ok(Some(provider)) = provider {
            self.announce_usage(UsageEvent::Recorded(UsageRecorded {
                provider,
                thread: thread.clone(),
            }));
        }
    }

    /// `usage.summary`: the usage of the turns this core ran in the last `days` days.
    pub async fn usage_summary(&self, params: UsageSummaryParams) -> KernelResult<UsageSummary> {
        if !(1..=MAX_SUMMARY_DAYS).contains(&params.days) {
            return Err(KernelError::InvalidParams(format!(
                "days must be between 1 and {MAX_SUMMARY_DAYS}"
            )));
        }
        let offset = params.tz_offset_minutes.unwrap_or(0);
        let since = window_start(&self.now(), params.days, offset)?;
        let provider = params.provider;
        let records = self
            .inner
            .store
            .read(move |c| records_since(c, &since, provider.as_deref()))
            .await?;
        summarize(&records, offset)
    }

    /// `usage.limits`: the latest snapshot of each registered provider, or of one.
    pub async fn usage_limits(&self, provider: Option<&str>) -> KernelResult<Vec<ProviderLimits>> {
        let ids: Vec<String> = match provider {
            Some(id) => {
                self.require_provider(id)?;
                vec![id.to_owned()]
            }
            None => self.provider_list().into_iter().map(|p| p.id).collect(),
        };
        self.inner
            .store
            .read(move |c| {
                ids.iter()
                    .map(|id| Ok(to_provider_limits(id, limits_row(c, id)?)))
                    .collect::<KernelResult<Vec<_>>>()
            })
            .await
    }

    /// Stores a provider's snapshot and announces it. `None` records that the provider
    /// has no plan limits to show.
    pub async fn report_limits(
        &self,
        provider: &str,
        limits: Option<UsageLimits>,
    ) -> KernelResult<ProviderLimits> {
        let limits = limits.map(sanitize_limits).transpose()?;
        let observed = self.now();
        let id = provider.to_owned();
        let (l, o) = (limits.clone(), observed.clone());
        self.inner
            .store
            .write(move |c| limits_put(c, &id, l.as_ref(), &o))
            .await?;
        let snapshot = ProviderLimits {
            provider: provider.to_owned(),
            limits,
            observed: Some(observed),
        };
        self.announce_usage(UsageEvent::Limits(snapshot.clone()));
        Ok(snapshot)
    }

    /// `usage.refresh`: asks the provider to read its plan limits now. Within
    /// [`REFRESH_INTERVAL_SECS`] of the last refresh, returns the stored snapshot instead,
    /// and concurrent refreshes of one provider run once.
    pub async fn usage_refresh(&self, provider: &str) -> KernelResult<ProviderLimits> {
        let p = self.require_provider(provider)?;
        if !p.reads_limits() {
            return Err(KernelError::Unsupported(format!(
                "provider {provider} cannot read its plan limits"
            )));
        }
        let gate = lock(&self.inner.usage.refreshing)
            .entry(provider.to_owned())
            .or_default()
            .clone();
        let _running = gate.lock().await;
        let last = lock(&self.inner.usage.refreshed).get(provider).cloned();
        if let Some(last) = last
            && seconds_between(&last, &self.now()).is_some_and(|s| s < REFRESH_INTERVAL_SECS)
        {
            return Ok(self
                .usage_limits(Some(provider))
                .await?
                .pop()
                .unwrap_or(ProviderLimits {
                    provider: provider.to_owned(),
                    limits: None,
                    observed: None,
                }));
        }
        lock(&self.inner.usage.refreshed).insert(provider.to_owned(), self.now());
        let limits = p
            .read_limits()
            .await
            .map_err(|e| KernelError::PluginFailed(e.message))?;
        self.report_limits(provider, limits).await
    }

    fn require_provider(
        &self,
        id: &str,
    ) -> KernelResult<std::sync::Arc<dyn crate::provider::Provider>> {
        self.lock_providers()
            .iter()
            .find(|p| p.id() == id)
            .cloned()
            .ok_or_else(|| KernelError::NotFound(format!("no provider {id}")))
    }
}

fn seconds_between(a: &Timestamp, b: &Timestamp) -> Option<i64> {
    let a: jiff::Timestamp = a.0.parse().ok()?;
    let b: jiff::Timestamp = b.0.parse().ok()?;
    Some(b.as_second() - a.as_second())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kumikit_protocol::model::LimitWindow;

    fn rec(thread: &str, model: &str, started: &str, input: u64, output: u64) -> Record {
        Record {
            thread: thread.into(),
            provider: "p/x".into(),
            model: model.into(),
            started: started.into(),
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
            cost: None,
            models: Vec::new(),
        }
    }

    #[test]
    fn summary_counts_turns_threads_and_models() {
        let mut a = rec("a", "big", "2026-11-15T10:00:00.000Z", 100, 10);
        a.cost = Some(1.0);
        a.cached_input_tokens = 80;
        a.models = vec![
            ModelUsage {
                model: "big".into(),
                input_tokens: 90,
                output_tokens: 8,
                cached_input_tokens: Some(75),
                cost: Some(0.9),
            },
            ModelUsage {
                model: "small".into(),
                input_tokens: 10,
                output_tokens: 2,
                cached_input_tokens: Some(5),
                cost: Some(0.1),
            },
        ];
        let b = rec("a", "big", "2026-11-15T11:00:00.000Z", 50, 5);
        let c = rec("b", "small", "2026-11-16T09:00:00.000Z", 20, 2);
        let s = summarize(&[a, b, c], 0).unwrap();
        assert_eq!(s.totals.turns, 3);
        assert_eq!(s.totals.input_tokens, 170);
        assert_eq!(s.totals.output_tokens, 17);
        assert_eq!(s.totals.cached_input_tokens, 80);
        assert_eq!(s.totals.cost, Some(1.0));
        assert_eq!(s.threads, 2);
        let rows: Vec<(&str, u64, u32)> = s
            .models
            .iter()
            .map(|r| (r.model.as_str(), r.totals.input_tokens, r.totals.turns))
            .collect();
        assert_eq!(rows, vec![("big", 140, 2), ("small", 30, 2)]);
        let days: Vec<(&str, u64)> = s
            .daily
            .iter()
            .map(|d| (d.date.as_str(), d.tokens))
            .collect();
        assert_eq!(days, vec![("2026-11-15", 165), ("2026-11-16", 22)]);
        assert_eq!(s.daily[0].cost, Some(1.0));
        assert_eq!(s.daily[1].cost, None);
    }

    #[test]
    fn daily_uses_the_clients_days() {
        let a = rec("a", "m", "2026-11-15T23:30:00.000Z", 1, 0);
        let s = summarize(std::slice::from_ref(&a), 60).unwrap();
        assert_eq!(s.daily[0].date, "2026-11-16");
        let s = summarize(&[a], -60).unwrap();
        assert_eq!(s.daily[0].date, "2026-11-15");
    }

    #[test]
    fn window_starts_at_local_midnight() {
        let now = Timestamp("2026-11-15T10:00:00.000Z".into());
        assert_eq!(
            window_start(&now, 1, 0).unwrap(),
            "2026-11-15T00:00:00.000Z"
        );
        assert_eq!(
            window_start(&now, 30, 0).unwrap(),
            "2026-10-17T00:00:00.000Z"
        );
        // UTC+2: local midnight is 22:00 the day before.
        assert_eq!(
            window_start(&now, 1, 120).unwrap(),
            "2026-11-14T22:00:00.000Z"
        );
        assert!(window_start(&now, 1, 5000).is_err());
    }

    fn window(pct: f64) -> LimitWindow {
        LimitWindow {
            id: "w".into(),
            label: "5 hours".into(),
            group: None,
            used_percent: pct,
            resets_at: None,
            model: None,
        }
    }

    #[test]
    fn snapshots_are_tidied() {
        let l = sanitize_limits(UsageLimits {
            plan: Some("x".repeat(500)),
            account: None,
            account_url: Some("http://example.com".into()),
            windows: vec![window(140.0), window(-3.0)],
        })
        .unwrap();
        assert_eq!(l.plan.unwrap().len(), MAX_TEXT);
        assert_eq!(l.account_url, None);
        assert_eq!(l.windows[0].used_percent, 100.0);
        assert_eq!(l.windows[1].used_percent, 0.0);
        let https = sanitize_limits(UsageLimits {
            account_url: Some("https://example.com/a".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(https.account_url.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn bad_snapshots_are_refused() {
        let nan = UsageLimits {
            windows: vec![window(f64::NAN)],
            ..Default::default()
        };
        assert!(sanitize_limits(nan).is_err());
        let mut w = window(1.0);
        w.resets_at = Some(Timestamp("tomorrow".into()));
        let bad_time = UsageLimits {
            windows: vec![w],
            ..Default::default()
        };
        assert!(sanitize_limits(bad_time).is_err());
        let many = UsageLimits {
            windows: vec![window(1.0); MAX_WINDOWS + 1],
            ..Default::default()
        };
        assert!(sanitize_limits(many).is_err());
    }
}
