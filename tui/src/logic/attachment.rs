//! How an **Attachment view** words an attachment's metadata (CONTEXT.md):
//! its size, its last-modified time and its type, the same in every frontend.
//! The details themselves are core's (`NoteVault::get_attachment_details`).

/// Formats a byte count as a human-readable size (`2.3 MB`, `512 B`).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// Formats a Unix-second timestamp as a locale-agnostic `YYYY-MM-DD HH:MM`
/// (UTC).
pub fn format_mtime(secs: u64) -> String {
    match chrono::DateTime::from_timestamp(secs as i64, 0) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown".to_string(),
    }
}

/// The attachment's type as shown: its extension in capitals (`PDF`), or
/// `(no extension)`.
pub fn type_label(extension: Option<&str>) -> String {
    match extension {
        Some(ext) => ext.to_uppercase(),
        None => "(no extension)".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_scales_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(2_411_724), "2.3 MB");
    }

    #[test]
    fn mtime_is_utc_minutes() {
        assert_eq!(format_mtime(0), "1970-01-01 00:00");
        assert_eq!(format_mtime(1_700_000_000), "2023-11-14 22:13");
    }

    #[test]
    fn type_is_the_extension_or_says_there_is_none() {
        assert_eq!(type_label(Some("pdf")), "PDF");
        assert_eq!(type_label(None), "(no extension)");
    }
}
