//! BusinessTime, BusinessTimeRange and the few calendar helpers they need.
//!
//! Text form: `YYYY-MM-DDTHH:MM`, where `HH` is a signed i32 written with at
//! least two digits (`2026-08-31T-04:38`, `2026-08-30T26:01`). The minute is
//! always added: `T-04:38` is -4 hours plus 38 minutes, i.e. 3h22m before
//! midnight. Ordering is date first, then hour, then minute.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A calendar date, year 0000 to 9999, proleptic Gregorian.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub fn new(year: u16, month: u8, day: u8) -> Option<Date> {
        if year > 9999 || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// Parses `YYYY-MM-DD`.
    pub fn parse(s: &str) -> Option<Date> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let year = digits(&s[0..4])? as u16;
        let month = digits(&s[5..7])? as u8;
        let day = digits(&s[8..10])? as u8;
        Date::new(year, month, day)
    }

    /// Days since 1970-01-01.
    pub fn days_since_epoch(self) -> i64 {
        days_from_civil(self.year as i64, self.month as i64, self.day as i64)
    }

    pub fn from_days_since_epoch(days: i64) -> Option<Date> {
        let (y, m, d) = civil_from_days(days);
        if !(0..=9999).contains(&y) {
            return None;
        }
        Date::new(y as u16, m as u8, d as u8)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

pub fn is_leap(year: u16) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn digits(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

// Howard Hinnant's civil calendar algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A moment in the books: a date plus an hour that may run outside the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BusinessTime {
    pub date: Date,
    pub hour: i32,
    /// [0, 59]
    pub minute: u8,
}

impl BusinessTime {
    pub const MIN: BusinessTime = BusinessTime {
        date: Date { year: 0, month: 1, day: 1 },
        hour: i32::MIN,
        minute: 0,
    };
    pub const MAX: BusinessTime = BusinessTime {
        date: Date {
            year: 9999,
            month: 12,
            day: 31,
        },
        hour: i32::MAX,
        minute: 59,
    };

    pub fn new(date: Date, hour: i32, minute: u8) -> Option<BusinessTime> {
        (minute <= 59).then_some(BusinessTime { date, hour, minute })
    }

    /// The earliest moment of a date.
    pub fn start_of(date: Date) -> BusinessTime {
        BusinessTime {
            date,
            hour: i32::MIN,
            minute: 0,
        }
    }

    /// The latest moment of a date.
    pub fn end_of(date: Date) -> BusinessTime {
        BusinessTime {
            date,
            hour: i32::MAX,
            minute: 59,
        }
    }

    /// Builds from a date and signed minutes from the day's midnight.
    pub fn from_minutes(date: Date, minutes: i64) -> Option<BusinessTime> {
        let hour = minutes.div_euclid(60);
        let minute = minutes.rem_euclid(60) as u8;
        Some(BusinessTime {
            date,
            hour: i32::try_from(hour).ok()?,
            minute,
        })
    }

    /// The next minute on the same date, if the hour does not overflow.
    pub fn next_minute(self) -> Option<BusinessTime> {
        if self.minute < 59 {
            Some(BusinessTime {
                minute: self.minute + 1,
                ..self
            })
        } else {
            Some(BusinessTime {
                hour: self.hour.checked_add(1)?,
                minute: 0,
                ..self
            })
        }
    }
}

impl Ord for BusinessTime {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.date, self.hour, self.minute).cmp(&(other.date, other.hour, other.minute))
    }
}

impl PartialOrd for BusinessTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for BusinessTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.hour < 0 { "-" } else { "" };
        write!(f, "{}T{}{:02}:{:02}", self.date, sign, self.hour.unsigned_abs(), self.minute)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTimeError(pub String);

impl fmt::Display for ParseTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid BusinessTime \"{}\": expected YYYY-MM-DDTHH:MM with a signed hour",
            self.0
        )
    }
}

impl std::error::Error for ParseTimeError {}

impl FromStr for BusinessTime {
    type Err = ParseTimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseTimeError(s.to_string());
        let (date, time) = s.split_once('T').ok_or_else(err)?;
        let date = Date::parse(date).ok_or_else(err)?;
        let (hour, minute) = time.rsplit_once(':').ok_or_else(err)?;
        let (negative, hour_digits) = match hour.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, hour),
        };
        if hour_digits.len() < 2 || !hour_digits.bytes().all(|c| c.is_ascii_digit()) {
            return Err(err());
        }
        let magnitude: i64 = hour_digits.parse().map_err(|_| err())?;
        let hour = i32::try_from(if negative { -magnitude } else { magnitude }).map_err(|_| err())?;
        if minute.len() != 2 {
            return Err(err());
        }
        let minute = digits(minute).ok_or_else(err)? as u8;
        BusinessTime::new(date, hour, minute).ok_or_else(err)
    }
}

impl Serialize for BusinessTime {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BusinessTime {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// `from` inclusive, `until` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessTimeRange {
    pub from: BusinessTime,
    pub until: BusinessTime,
}

impl BusinessTimeRange {
    pub fn contains(&self, t: BusinessTime) -> bool {
        self.from <= t && t < self.until
    }
}

/// Whether `t` is inside a validity window.
pub fn is_valid_at(
    valid_from: BusinessTime,
    valid_until: Option<BusinessTime>,
    invalid_ranges: &[BusinessTimeRange],
    t: BusinessTime,
) -> bool {
    t >= valid_from && valid_until.is_none_or(|u| t < u) && !invalid_ranges.iter().any(|r| r.contains(t))
}

/// RFC 3339 UTC with milliseconds, from milliseconds since the Unix epoch.
pub fn rfc3339_from_unix_millis(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

pub fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn now_rfc3339() -> String {
    rfc3339_from_unix_millis(now_unix_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> BusinessTime {
        s.parse().unwrap()
    }

    #[test]
    fn parse_and_format_round_trip() {
        for s in [
            "2026-08-31T-04:38",
            "2026-08-30T26:01",
            "2026-01-01T00:00",
            "2026-01-01T-2147483648:00",
            "2026-01-01T2147483647:59",
        ] {
            assert_eq!(t(s).to_string(), s);
        }
    }

    #[test]
    fn rejects_bad_input() {
        for s in [
            "2026-02-30T00:00",
            "2026-01-01T0:00",
            "2026-01-01T00:60",
            "2026-01-01 00:00",
            "2026-01-01T00:0",
            "2026-01-01T2147483648:00",
        ] {
            assert!(s.parse::<BusinessTime>().is_err(), "{s}");
        }
    }

    #[test]
    fn date_wins() {
        assert!(t("2026-01-15T99:00") < t("2026-01-16T-99:00"));
        assert!(t("2026-01-15T-04:38") < t("2026-01-15T-04:39"));
        assert!(t("2026-01-15T-05:59") < t("2026-01-15T-04:00"));
    }

    #[test]
    fn minutes_are_additive() {
        let d = Date::parse("2026-01-01").unwrap();
        assert_eq!(BusinessTime::from_minutes(d, -202).unwrap().to_string(), "2026-01-01T-04:38");
        assert_eq!(t("2026-01-01T23:59").next_minute().unwrap().to_string(), "2026-01-01T24:00");
    }

    #[test]
    fn epoch_days() {
        let d = Date::parse("2026-11-12").unwrap();
        assert_eq!(Date::from_days_since_epoch(d.days_since_epoch()), Some(d));
        assert_eq!(rfc3339_from_unix_millis(0), "1970-01-01T00:00:00.000Z");
    }
}
