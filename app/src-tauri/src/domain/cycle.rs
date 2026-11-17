//! Billing cycles: when a subscription's allowance resets, and how much of it is used.

use super::counter::Bytes;
use super::period::Range;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{ToSpan, ZonedDateTime};
use serde::{Deserialize, Serialize};

/// When a cycle starts over. Times are local wall-clock times in quarter hours, the resolution
/// traffic is stored at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reset {
    /// On `day` of every month at `hour:minute`; in shorter months on their last day.
    #[serde(rename_all = "camelCase")]
    Monthly { day: i8, hour: i8, minute: i8 },
    /// Every `days` days, counted from `start` (`YYYY-MM-DD`) at `hour:minute`.
    #[serde(rename_all = "camelCase")]
    EveryDays { start: String, hour: i8, minute: i8, days: i32 },
}

/// Which traffic counts against the allowance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CountMode {
    /// Upload and download, as most providers count.
    #[default]
    Both,
    DownloadOnly,
}

impl CountMode {
    pub fn used(self, b: Bytes) -> u64 {
        match self {
            CountMode::Both => b.up + b.down,
            CountMode::DownloadOnly => b.down,
        }
    }
}

fn time_ok(hour: i8, minute: i8) -> bool {
    (0..24).contains(&hour) && [0, 15, 30, 45].contains(&minute)
}

impl Reset {
    /// Why the rule cannot be used, if it cannot.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Reset::Monthly { day, hour, minute } => {
                if !(1..=31).contains(day) {
                    return Err("每月的重置日应在 1 到 31 之间。".into());
                }
                if !time_ok(*hour, *minute) {
                    return Err("重置时间应为整刻钟，例如 00:00、08:30。".into());
                }
            }
            Reset::EveryDays { start, hour, minute, days } => {
                start.parse::<Date>().map_err(|_| "起始日期无效。")?;
                if !(1..=366).contains(days) {
                    return Err("周期天数应在 1 到 366 之间。".into());
                }
                if !time_ok(*hour, *minute) {
                    return Err("重置时间应为整刻钟，例如 00:00、08:30。".into());
                }
            }
        }
        Ok(())
    }

    /// The cycle that contains `now`, in `now`'s time zone.
    pub fn current(&self, now: &ZonedDateTime) -> Result<Range, String> {
        self.check()?;
        let e = |e: jiff::Error| e.to_string();
        let tz = now.time_zone().clone();
        let secs = |z: &ZonedDateTime| z.timestamp().as_second();
        match self {
            Reset::Monthly { day, hour, minute } => {
                let at = |year: i16, month: i8| -> Result<ZonedDateTime, String> {
                    let first = Date::new(year, month, 1).map_err(e)?;
                    let d = Date::new(year, month, (*day).min(first.days_in_month())).map_err(e)?;
                    d.at(*hour, *minute, 0, 0).to_zoned_datetime(tz.clone()).map_err(e)
                };
                let shift = |(y, m): (i16, i8), by: i8| -> (i16, i8) {
                    let n = i32::from(y) * 12 + i32::from(m) - 1 + i32::from(by);
                    ((n.div_euclid(12)) as i16, (n.rem_euclid(12) + 1) as i8)
                };
                let here = (now.year(), now.month());
                let this = at(here.0, here.1)?;
                let start_month = if secs(&this) <= secs(now) { here } else { shift(here, -1) };
                let start = at(start_month.0, start_month.1)?;
                let next = shift(start_month, 1);
                let end = at(next.0, next.1)?;
                Ok(Range { start: secs(&start), end: secs(&end) })
            }
            Reset::EveryDays { start, hour, minute, days } => {
                let first = start.parse::<Date>().map_err(|_| "起始日期无效。")?;
                let origin = first.at(*hour, *minute, 0, 0).to_zoned_datetime(tz.clone()).map_err(e)?;
                let elapsed = i64::from(first.until(now.date()).map_err(e)?.get_days());
                let mut k = elapsed.div_euclid(i64::from(*days));
                let begin = |k: i64| origin.checked_add((k * i64::from(*days)).days()).map_err(e);
                let mut start = begin(k)?;
                if secs(&start) > secs(now) {
                    k -= 1;
                    start = begin(k)?;
                }
                let end = begin(k + 1)?;
                Ok(Range { start: secs(&start), end: secs(&end) })
            }
        }
    }
}

/// What a provider reports about a subscription (the `subscription-userinfo` header): its own
/// count of the traffic, from every device using the subscription.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reported {
    pub upload: u64,
    pub download: u64,
    /// Allowance in bytes; 0 when the provider gives none.
    pub total: u64,
    /// Unix seconds; `None` when the subscription does not expire.
    pub expire: Option<i64>,
    /// When the client last fetched the subscription, unix seconds.
    pub updated: Option<i64>,
}

/// A rule's cycle at a unix second, in a zone (for callers outside the domain).
pub fn current_at(reset: &Reset, ts: i64, tz: &TimeZone) -> Result<Range, String> {
    let now = jiff::Timestamp::from_second(ts).map_err(|e| e.to_string())?.to_zoned_datetime(tz.clone());
    reset.current(&now)
}

/// Usage at the end of the cycle if it goes on at the pace so far; `None` while too little of
/// the cycle has passed (under a day) to say.
pub fn projected(used: u64, cycle: Range, now: i64) -> Option<u64> {
    let elapsed = now - cycle.start;
    if elapsed < 86_400 || now >= cycle.end {
        return None;
    }
    let total = (cycle.end - cycle.start) as f64;
    Some((used as f64 * total / elapsed as f64).round() as u64)
}

/// Alert thresholds (percent of the quota) newly crossed: reached now, above `already`.
pub fn crossed(used: u64, quota: u64, thresholds: &[u8], already: u8) -> Option<u8> {
    if quota == 0 {
        return None;
    }
    let pct = used as f64 / quota as f64 * 100.0;
    thresholds.iter().copied().filter(|t| *t > already && pct >= f64::from(*t)).max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn at(y: i16, m: i8, d: i8, h: i8, min: i8) -> ZonedDateTime {
        date(y, m, d).at(h, min, 0, 0).in_tz("Asia/Shanghai").unwrap()
    }

    fn ts(y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
        at(y, m, d, h, min).timestamp().as_second()
    }

    fn monthly(day: i8, hour: i8, minute: i8) -> Reset {
        Reset::Monthly { day, hour, minute }
    }

    #[test]
    fn a_monthly_cycle_runs_from_this_months_reset_to_the_next() {
        // Act
        let r = monthly(26, 17, 0).current(&at(2026, 11, 28, 9, 0)).unwrap();

        // Assert
        assert_eq!(r, Range { start: ts(2026, 11, 26, 17, 0), end: ts(2026, 12, 26, 17, 0) });
    }

    #[test]
    fn before_the_reset_time_the_previous_cycle_still_runs() {
        // Act: the 26th, but before 17:00.
        let r = monthly(26, 17, 0).current(&at(2026, 11, 26, 16, 45)).unwrap();

        // Assert
        assert_eq!(r.start, ts(2026, 10, 26, 17, 0));
        assert_eq!(r.end, ts(2026, 11, 26, 17, 0));
    }

    #[test]
    fn a_reset_day_past_the_months_end_falls_on_its_last_day() {
        // Act: day 31 in February of a leap year and across the year end.
        let feb = monthly(31, 0, 0).current(&at(2028, 2, 29, 12, 0)).unwrap();
        let january = monthly(31, 0, 0).current(&at(2027, 1, 10, 12, 0)).unwrap();

        // Assert
        assert_eq!(feb, Range { start: ts(2028, 2, 29, 0, 0), end: ts(2028, 3, 31, 0, 0) });
        assert_eq!(january, Range { start: ts(2026, 12, 31, 0, 0), end: ts(2027, 1, 31, 0, 0) });
    }

    #[test]
    fn cycles_follow_local_time_across_daylight_saving() {
        // Arrange: Berlin springs forward on 2026-03-29.
        let now = date(2026, 4, 2).at(12, 0, 0, 0).in_tz("Europe/Berlin").unwrap();

        // Act
        let r = monthly(15, 0, 0).current(&now).unwrap();

        // Assert: 31 calendar days, one hour short.
        assert_eq!(r.end - r.start, 31 * 86_400 - 3_600);
    }

    #[test]
    fn every_n_days_counts_from_the_start_date() {
        // Arrange
        let rule = Reset::EveryDays { start: "2026-11-01".into(), hour: 8, minute: 0, days: 30 };

        // Act
        let first = rule.current(&at(2026, 11, 1, 8, 0)).unwrap();
        let second = rule.current(&at(2026, 12, 15, 12, 0)).unwrap();
        let early = rule.current(&at(2026, 12, 1, 7, 0)).unwrap();

        // Assert
        assert_eq!(first, Range { start: ts(2026, 11, 1, 8, 0), end: ts(2026, 12, 1, 8, 0) });
        assert_eq!(second, Range { start: ts(2026, 12, 1, 8, 0), end: ts(2026, 12, 31, 8, 0) });
        assert_eq!(early, first);
    }

    #[test]
    fn rules_outside_the_stored_resolution_are_refused() {
        // Act
        let bad = [monthly(0, 0, 0), monthly(32, 0, 0), monthly(1, 24, 0), monthly(1, 8, 10)].map(|r| r.check().is_err());
        let bad_days = Reset::EveryDays { start: "2026-11-01".into(), hour: 0, minute: 0, days: 0 }.check();

        // Assert
        assert_eq!(bad, [true; 4]);
        assert!(bad_days.is_err());
    }

    #[test]
    fn rules_round_trip_as_tagged_json() {
        // Arrange
        let rule = monthly(15, 8, 30);

        // Act
        let json = serde_json::to_string(&rule).unwrap();

        // Assert
        assert_eq!(json, r#"{"kind":"monthly","day":15,"hour":8,"minute":30}"#);
        assert_eq!(serde_json::from_str::<Reset>(&json).unwrap(), rule);
    }

    #[test]
    fn projection_extends_the_pace_so_far() {
        // Arrange: 10 of 30 days passed.
        let cycle = Range { start: 0, end: 30 * 86_400 };

        // Act
        let pace = projected(100, cycle, 10 * 86_400);
        let too_early = projected(100, cycle, 3_600);

        // Assert
        assert_eq!(pace, Some(300));
        assert_eq!(too_early, None);
    }

    #[test]
    fn each_threshold_is_crossed_once() {
        // Arrange
        let thresholds = [80, 100];
        let quota = 1_000;

        // Act / Assert
        assert_eq!(crossed(799, quota, &thresholds, 0), None);
        assert_eq!(crossed(800, quota, &thresholds, 0), Some(80));
        assert_eq!(crossed(900, quota, &thresholds, 80), None);
        assert_eq!(crossed(1_200, quota, &thresholds, 0), Some(100));
        assert_eq!(crossed(1_200, 0, &thresholds, 0), None);
    }

    #[test]
    fn download_only_ignores_upload() {
        // Act / Assert
        assert_eq!(CountMode::DownloadOnly.used(Bytes::new(5, 7)), 7);
        assert_eq!(CountMode::Both.used(Bytes::new(5, 7)), 12);
    }
}
