//! ISO-8601 calendar dates (`YYYY-MM-DD`) shared by journal note naming,
//! backup folder dating and the frontmatter `Date` property. Pure string ↔
//! [`NaiveDate`] conversion; no I/O, so it lives outside `nfs`/`system`.

use chrono::NaiveDate;

const ISO_DATE_FORMAT: &str = "%Y-%m-%d";

/// Parses a `YYYY-MM-DD` date; `None` for anything else, including a date
/// followed by a time component.
pub(crate) fn parse_iso_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, ISO_DATE_FORMAT).ok()
}

/// Formats `date` as zero-padded `YYYY-MM-DD`.
pub(crate) fn format_iso_date(date: NaiveDate) -> String {
    date.format(ISO_DATE_FORMAT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_iso_date() {
        assert_eq!(
            parse_iso_date("2024-01-31"),
            NaiveDate::from_ymd_opt(2024, 1, 31)
        );
    }

    #[test]
    fn rejects_trailing_time_and_garbage() {
        assert_eq!(parse_iso_date("2024-01-31T10:00"), None);
        assert_eq!(parse_iso_date("not a date"), None);
        assert_eq!(parse_iso_date("2024-02-30"), None);
    }

    #[test]
    fn formats_zero_padded() {
        let d = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
        assert_eq!(format_iso_date(d), "2024-03-05");
        assert_eq!(parse_iso_date(&format_iso_date(d)), Some(d));
    }
}
