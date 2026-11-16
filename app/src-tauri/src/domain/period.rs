//! Storage buckets and the calendar periods the views add them up into.
//!
//! Traffic is stored in 15-minute buckets keyed by their UTC start. Every time zone in use is a
//! whole number of quarter hours from UTC, so local days, weeks and months always begin on a
//! bucket boundary and a period is an exact sum of buckets, also after the time zone changes.

use jiff::civil::{Date, Weekday};
use jiff::{ToSpan, ZonedDateTime};

pub const BUCKET_SECS: i64 = 15 * 60;

/// Start of the bucket holding `ts` (unix seconds).
pub fn bucket_of(ts: i64) -> i64 {
    ts - ts.rem_euclid(BUCKET_SECS)
}

/// A half-open span of unix seconds, `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: i64,
    pub end: i64,
}

impl Range {
    pub fn contains(&self, ts: i64) -> bool {
        self.start <= ts && ts < self.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
    Year,
}

fn secs(z: &ZonedDateTime) -> i64 {
    z.timestamp().as_second()
}

fn span(start: &ZonedDateTime, next: ZonedDateTime) -> Range {
    Range { start: secs(start), end: secs(&next) }
}

/// The period of the given kind that contains `now`, in `now`'s time zone. Calendar arithmetic
/// keeps a day a calendar day across daylight-saving changes (23 or 25 hours).
pub fn current(period: Period, now: &ZonedDateTime, week_start: Weekday) -> Result<Range, jiff::Error> {
    let day = now.start_of_day()?;
    Ok(match period {
        Period::Day => span(&day, day.checked_add(1.day())?),
        Period::Week => {
            let back = now.weekday().since(week_start);
            let start = day.checked_sub(i64::from(back).days())?;
            span(&start, start.checked_add(1.week())?)
        }
        Period::Month => {
            let start = now.first_of_month()?.start_of_day()?;
            span(&start, start.checked_add(1.month())?)
        }
        Period::Year => {
            let start = now.first_of_year()?.start_of_day()?;
            span(&start, start.checked_add(1.year())?)
        }
    })
}

/// The last `n` calendar days up to and including today, oldest first.
pub fn last_days(now: &ZonedDateTime, n: usize) -> Result<Vec<(Date, Range)>, jiff::Error> {
    let today = now.start_of_day()?;
    let mut out = Vec::with_capacity(n);
    for back in (0..n as i64).rev() {
        let start = today.checked_sub(back.days())?;
        out.push((start.date(), span(&start, start.checked_add(1.day())?)));
    }
    Ok(out)
}

/// Every hour of today, as wall-clock hours (a daylight-saving day has 23 or 25 of them).
pub fn hours_of_today(now: &ZonedDateTime) -> Result<Vec<Range>, jiff::Error> {
    let day = current(Period::Day, now, Weekday::Monday)?;
    let start = now.start_of_day()?;
    let mut out = Vec::with_capacity(25);
    let mut t = start.timestamp();
    while t.as_second() < day.end {
        let next = t.checked_add(1.hour())?;
        out.push(Range { start: t.as_second(), end: next.as_second().min(day.end) });
        t = next;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn at(tz: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> ZonedDateTime {
        date(y, m, d).at(h, min, 0, 0).in_tz(tz).unwrap()
    }

    fn ts(tz: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
        secs(&at(tz, y, m, d, h, min))
    }

    #[test]
    fn bucket_of_aligns_down_to_quarter_hours() {
        // Arrange
        let start = 1_700_000_100 - 1_700_000_100 % BUCKET_SECS;

        // Act
        let inside = [start, start + 1, start + BUCKET_SECS - 1].map(bucket_of);
        let next = bucket_of(start + BUCKET_SECS);

        // Assert
        assert_eq!(inside, [start; 3]);
        assert_eq!(next, start + BUCKET_SECS);
    }

    #[test]
    fn day_boundaries_fall_on_buckets_in_quarter_hour_zones() {
        // Arrange
        let zones = ["Asia/Kolkata", "Asia/Kathmandu", "Australia/Eucla", "America/St_Johns", "Asia/Shanghai"];

        for tz in zones {
            // Act
            let day = current(Period::Day, &at(tz, 2026, 3, 9, 15, 0), Weekday::Monday).unwrap();

            // Assert
            assert_eq!(bucket_of(day.start), day.start, "{tz}");
            assert_eq!(bucket_of(day.end), day.end, "{tz}");
        }
    }

    #[test]
    fn week_starts_on_the_chosen_weekday() {
        // Arrange: Wednesday 2026-11-18.
        let now = at("Asia/Shanghai", 2026, 11, 18, 10, 0);

        // Act
        let monday = current(Period::Week, &now, Weekday::Monday).unwrap();
        let sunday = current(Period::Week, &now, Weekday::Sunday).unwrap();

        // Assert
        assert_eq!(monday.start, ts("Asia/Shanghai", 2026, 11, 16, 0, 0));
        assert_eq!(monday.end, ts("Asia/Shanghai", 2026, 11, 23, 0, 0));
        assert_eq!(sunday.start, ts("Asia/Shanghai", 2026, 11, 15, 0, 0));
    }

    #[test]
    fn week_containing_its_first_day_starts_that_day() {
        // Arrange: a Monday.
        let now = at("Asia/Shanghai", 2026, 11, 16, 0, 0);

        // Act
        let week = current(Period::Week, &now, Weekday::Monday).unwrap();

        // Assert
        assert_eq!(week.start, secs(&now));
    }

    #[test]
    fn month_and_year_follow_the_calendar() {
        // Arrange
        let now = at("Asia/Shanghai", 2028, 2, 20, 9, 0);

        // Act
        let month = current(Period::Month, &now, Weekday::Monday).unwrap();
        let year = current(Period::Year, &now, Weekday::Monday).unwrap();

        // Assert: 2028 is a leap year.
        assert_eq!(month.end - month.start, 29 * 86_400);
        assert_eq!(year.start, ts("Asia/Shanghai", 2028, 1, 1, 0, 0));
        assert_eq!(year.end, ts("Asia/Shanghai", 2029, 1, 1, 0, 0));
    }

    #[test]
    fn daylight_saving_days_are_calendar_days() {
        // Arrange: Europe/Berlin springs forward on 2026-03-29 and falls back on 2026-10-25.
        let spring = at("Europe/Berlin", 2026, 3, 29, 12, 0);
        let autumn = at("Europe/Berlin", 2026, 10, 25, 12, 0);

        // Act
        let short = current(Period::Day, &spring, Weekday::Monday).unwrap();
        let long = current(Period::Day, &autumn, Weekday::Monday).unwrap();
        let hours = (hours_of_today(&spring).unwrap().len(), hours_of_today(&autumn).unwrap().len());

        // Assert
        assert_eq!(short.end - short.start, 23 * 3_600);
        assert_eq!(long.end - long.start, 25 * 3_600);
        assert_eq!(hours, (23, 25));
    }

    #[test]
    fn last_days_are_consecutive_and_end_today() {
        // Arrange
        let now = at("Asia/Shanghai", 2026, 11, 16, 20, 30);
        let n = 30;

        // Act
        let days = last_days(&now, n).unwrap();

        // Assert
        assert_eq!(days.len(), n);
        assert_eq!(days.last().unwrap().0, date(2026, 11, 16));
        assert!(days.windows(2).all(|w| w[0].1.end == w[1].1.start));
        assert!(days.last().unwrap().1.contains(secs(&now)));
    }
}
