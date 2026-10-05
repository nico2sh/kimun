//! [`PropertyDateTime`]: a date-time property as written — local, with no
//! offset (how Obsidian writes its Date & time property), or with an offset —
//! so a value read from or written to a note keeps its meaning.

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Timelike, Utc};

/// A date-time property value: a wall-clock time, plus its UTC offset when
/// one was written. Searching and sorting compare [`Self::to_utc`], which
/// reads a local value as UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PropertyDateTime {
    local: NaiveDateTime,
    offset: Option<FixedOffset>,
}

impl PropertyDateTime {
    /// A local date-time, with no offset.
    pub fn local(local: NaiveDateTime) -> Self {
        Self {
            local,
            offset: None,
        }
    }

    /// A date-time with an explicit offset.
    pub fn with_offset(dt: DateTime<FixedOffset>) -> Self {
        Self {
            local: dt.naive_local(),
            offset: Some(*dt.offset()),
        }
    }

    /// The wall-clock time as written.
    pub fn naive_local(&self) -> NaiveDateTime {
        self.local
    }

    /// The offset, when one was written.
    pub fn offset(&self) -> Option<FixedOffset> {
        self.offset
    }

    /// The instant this value names, reading a local value as UTC — what
    /// property comparisons and sorting use.
    pub fn to_utc(self) -> DateTime<Utc> {
        match self.offset {
            // A fixed offset has exactly one mapping for every local time.
            Some(offset) => offset
                .from_local_datetime(&self.local)
                .single()
                .map_or_else(|| self.local.and_utc(), |dt| dt.with_timezone(&Utc)),
            None => self.local.and_utc(),
        }
    }

    /// Parses an RFC3339 date-time (its offset kept; `Z` is an offset of 0)
    /// or an offset-less `YYYY-MM-DDTHH:MM[:SS[.f]]` (a local time). The `T`
    /// and `Z` are case-insensitive.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_uppercase();
        if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
            return Some(Self::with_offset(dt));
        }
        ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
            .iter()
            .find_map(|f| NaiveDateTime::parse_from_str(&s, f).ok())
            .map(Self::local)
    }

    /// The value with seconds always written — for TOML, whose date-times
    /// require them.
    pub(crate) fn to_string_with_seconds(self) -> String {
        self.render(true)
    }

    fn render(self, always_seconds: bool) -> String {
        let pattern = if self.local.nanosecond() != 0 {
            "%Y-%m-%dT%H:%M:%S%.f"
        } else if self.local.second() != 0 || always_seconds || self.offset.is_some() {
            "%Y-%m-%dT%H:%M:%S"
        } else {
            "%Y-%m-%dT%H:%M"
        };
        let local = self.local.format(pattern).to_string();
        match self.offset {
            None => local,
            Some(offset) if offset.local_minus_utc() == 0 => format!("{local}Z"),
            Some(offset) => format!("{local}{offset}"),
        }
    }
}

/// `2024-03-01T14:30` (local, seconds omitted when zero, as Obsidian writes
/// it), `2024-03-01T14:30:00Z`, `2024-03-01T14:30:15+02:00` (RFC3339 needs the
/// seconds once there is an offset).
impl fmt::Display for PropertyDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(false))
    }
}

impl From<DateTime<Utc>> for PropertyDateTime {
    fn from(dt: DateTime<Utc>) -> Self {
        Self::with_offset(dt.fixed_offset())
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn naive(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2024, 3, 1)
            .unwrap()
            .and_hms_opt(h, m, s)
            .unwrap()
    }

    #[test]
    fn keeps_local_and_offset_as_written() {
        let local = PropertyDateTime::parse("2024-03-01T14:30").unwrap();
        assert_eq!(local, PropertyDateTime::local(naive(14, 30, 0)));
        assert_eq!(local.to_string(), "2024-03-01T14:30");
        assert_eq!(local.to_string_with_seconds(), "2024-03-01T14:30:00");

        let offset = PropertyDateTime::parse("2024-03-01T14:30:15+02:00").unwrap();
        assert_eq!(offset.offset(), FixedOffset::east_opt(7200));
        assert_eq!(offset.to_string(), "2024-03-01T14:30:15+02:00");

        let utc = PropertyDateTime::parse("2024-03-01t14:30:00z").unwrap();
        assert_eq!(utc.to_string(), "2024-03-01T14:30:00Z");
        assert!(PropertyDateTime::parse("2024-03-01").is_none());
        assert!(PropertyDateTime::parse("soon").is_none());
    }

    #[test]
    fn compares_as_utc_reading_local_as_utc() {
        let offset = PropertyDateTime::parse("2024-03-01T14:30:00+02:00").unwrap();
        assert_eq!(offset.to_utc(), naive(12, 30, 0).and_utc());
        let local = PropertyDateTime::parse("2024-03-01T14:30").unwrap();
        assert_eq!(local.to_utc(), naive(14, 30, 0).and_utc());
    }
}
