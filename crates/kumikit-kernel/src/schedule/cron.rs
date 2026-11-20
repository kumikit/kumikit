//! Cron expressions for schedules (`docs/spec/03-kernel.md#schedules`): five fields
//! (minute, hour, day of month, month, day of week) with `*`, lists, ranges and steps, or
//! `@hourly`, `@daily`, `@weekly`, `@monthly`. No seconds, so the shortest interval is a
//! minute. Times are civil times in a time zone: a time skipped by a DST change runs as
//! if the clocks had not moved (02:30 → 03:30); a repeated hour runs once.

use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

/// A parsed cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minute: u64,
    hour: u64,
    dom: u64,
    month: u64,
    dow: u64,
    /// Whether day of month and day of week were both restricted (then either matches).
    dom_or_dow: bool,
    dom_any: bool,
    dow_any: bool,
}

fn field(text: &str, min: u32, max: u32, what: &str) -> Result<(u64, bool), String> {
    let mut bits = 0u64;
    let any = text == "*";
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s.parse().map_err(|_| format!("{what}: bad step `{s}`"))?;
                if step == 0 {
                    return Err(format!("{what}: a step cannot be 0"));
                }
                (r, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let a: u32 = a.parse().map_err(|_| format!("{what}: bad value `{a}`"))?;
            let b: u32 = b.parse().map_err(|_| format!("{what}: bad value `{b}`"))?;
            (a, b)
        } else {
            let v: u32 = range
                .parse()
                .map_err(|_| format!("{what}: bad value `{range}`"))?;
            (v, if step > 1 { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("{what}: `{part}` is outside {min}-{max}"));
        }
        let mut v = lo;
        while v <= hi {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok((bits, any))
}

impl Cron {
    /// Parses an expression.
    pub fn parse(text: &str) -> Result<Cron, String> {
        let text = text.trim();
        let expanded = match text {
            "@hourly" => "0 * * * *",
            "@daily" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            other => other,
        };
        let parts: Vec<&str> = expanded.split_whitespace().collect();
        let [m, h, dom, mon, dow] = parts[..] else {
            return Err(format!(
                "`{text}` needs five fields (minute hour day-of-month month day-of-week)"
            ));
        };
        let (minute, _) = field(m, 0, 59, "minute")?;
        let (hour, _) = field(h, 0, 23, "hour")?;
        let (dom, dom_any) = field(dom, 1, 31, "day of month")?;
        let (month, _) = field(mon, 1, 12, "month")?;
        let (mut dow, dow_any) = field(dow, 0, 7, "day of week")?;
        // 7 is Sunday too.
        if dow & (1 << 7) != 0 {
            dow |= 1;
        }
        Ok(Cron {
            minute,
            hour,
            dom,
            month,
            dow,
            dom_or_dow: !dom_any && !dow_any,
            dom_any,
            dow_any,
        })
    }

    fn day_matches(&self, d: Date) -> bool {
        let dom = self.dom & (1 << d.day()) != 0;
        let dow = self.dow & (1 << d.weekday().to_sunday_zero_offset()) != 0;
        if self.dom_or_dow {
            dom || dow
        } else {
            (self.dom_any || dom) && (self.dow_any || dow)
        }
    }

    /// The first time strictly after `after` that matches, in `tz`; `None` if there is
    /// none within eight years (an impossible date like 30 February).
    pub fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        let start = after.to_zoned(tz.clone()).datetime();
        // The next whole minute after `after`, in civil time.
        let mut t = start
            .with()
            .second(0)
            .subsec_nanosecond(0)
            .build()
            .ok()?
            .checked_add(1.minute())
            .ok()?;
        let limit = start.date().checked_add(8.years()).ok()?;
        while t.date() < limit {
            if self.month & (1 << t.month()) == 0 {
                let first = Date::new(t.year(), t.month(), 1).ok()?;
                t = first
                    .checked_add(1.month())
                    .ok()?
                    .to_datetime(Time::midnight());
                continue;
            }
            if !self.day_matches(t.date()) {
                t = t
                    .date()
                    .checked_add(1.day())
                    .ok()?
                    .to_datetime(Time::midnight());
                continue;
            }
            if self.hour & (1 << t.hour()) == 0 {
                t = DateTime::from_parts(t.date(), Time::new(t.hour(), 0, 0, 0).ok()?)
                    .checked_add(1.hour())
                    .ok()?;
                continue;
            }
            if self.minute & (1 << t.minute()) == 0 {
                t = t.checked_add(1.minute()).ok()?;
                continue;
            }
            let zoned: Zoned = t.to_zoned(tz.clone()).ok()?;
            let ts = zoned.timestamp();
            if ts > after {
                return Some(ts);
            }
            // A repeated hour maps back before `after`: try the next minute.
            t = t.checked_add(1.minute()).ok()?;
        }
        None
    }
}

/// A time zone by IANA name, or the core's local zone for `None`.
pub fn zone(name: Option<&str>) -> Result<TimeZone, String> {
    match name {
        None => Ok(TimeZone::system()),
        Some(n) => TimeZone::get(n).map_err(|_| format!("unknown time zone `{n}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn next(cron: &str, after: &str, tz: &str) -> String {
        let tz = TimeZone::get(tz).unwrap();
        Cron::parse(cron)
            .unwrap()
            .next_after(ts(after), &tz)
            .unwrap()
            .to_string()
    }

    #[test]
    fn expressions_parse_or_say_why_not() {
        for ok in [
            "* * * * *",
            "0 9 * * 1-5",
            "*/15 0-6,22-23 1,15 */2 7",
            "@hourly",
            "@daily",
            "@weekly",
            "@monthly",
            "5/10 * * * *",
        ] {
            assert!(Cron::parse(ok).is_ok(), "{ok}");
        }
        for (bad, why) in [
            ("* * * *", "five fields"),
            ("60 * * * *", "outside 0-59"),
            ("* 24 * * *", "outside 0-23"),
            ("* * 0 * *", "outside 1-31"),
            ("*/0 * * * *", "cannot be 0"),
            ("a * * * *", "bad value"),
            ("5-1 * * * *", "outside"),
        ] {
            let e = Cron::parse(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn next_runs_follow_the_fields() {
        assert_eq!(
            next("* * * * *", "2026-11-20T10:00:30Z", "UTC"),
            "2026-11-20T10:01:00Z"
        );
        assert_eq!(
            next("0 9 * * 1-5", "2026-11-20T10:00:00Z", "UTC"),
            "2026-11-23T09:00:00Z"
        ); // Fri → Mon
        assert_eq!(
            next("*/15 * * * *", "2026-11-20T10:00:00Z", "UTC"),
            "2026-11-20T10:15:00Z"
        );
        assert_eq!(
            next("@monthly", "2026-11-20T10:00:00Z", "UTC"),
            "2026-12-01T00:00:00Z"
        );
        assert_eq!(
            next("0 0 29 2 *", "2026-11-20T10:00:00Z", "UTC"),
            "2028-02-29T00:00:00Z"
        );
        // Day of month OR day of week when both are restricted.
        assert_eq!(
            next("0 0 1 * 1", "2026-11-20T10:00:00Z", "UTC"),
            "2026-11-23T00:00:00Z"
        );
        // 7 is Sunday.
        assert_eq!(
            next("0 0 * * 7", "2026-11-20T10:00:00Z", "UTC"),
            "2026-11-22T00:00:00Z"
        );
    }

    #[test]
    fn time_zones_and_dst() {
        // 09:00 in Belgrade is 08:00 UTC in winter.
        assert_eq!(
            next("0 9 * * *", "2026-11-20T10:00:00Z", "Europe/Belgrade"),
            "2026-11-21T08:00:00Z"
        );
        // 2027-03-28 02:30 does not exist in Europe/Belgrade: it runs at 03:30 (01:30 UTC).
        assert_eq!(
            next("30 2 * * *", "2027-03-27T12:00:00Z", "Europe/Belgrade"),
            "2027-03-28T01:30:00Z"
        );
        // 2027-10-31 02:30 happens twice; it runs once, at the first (00:30 UTC), and the
        // next run is the next day.
        assert_eq!(
            next("30 2 * * *", "2027-10-30T12:00:00Z", "Europe/Belgrade"),
            "2027-10-31T00:30:00Z"
        );
        assert_eq!(
            next("30 2 * * *", "2027-10-31T00:30:00Z", "Europe/Belgrade"),
            "2027-11-01T01:30:00Z"
        );
    }

    #[test]
    fn an_impossible_date_never_runs() {
        let tz = TimeZone::UTC;
        assert_eq!(
            Cron::parse("0 0 30 2 *")
                .unwrap()
                .next_after(ts("2026-11-20T00:00:00Z"), &tz),
            None
        );
        assert!(zone(Some("Mars/Olympus")).is_err());
    }
}
