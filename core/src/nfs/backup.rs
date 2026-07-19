use std::path::Path;

use super::{resolve_path_on_disk, VaultPath};
use crate::error::FSError;

/// How long automated-edit backups are retained before the lazy purge reclaims
/// them. Counted in whole days against the UTC backup date.
const BACKUP_RETENTION_DAYS: i64 = 30;

/// The last `(backups_root, date)` purged in this process. The sweep is
/// de-duplicated against this so it runs at most once per vault per UTC day
/// rather than on every backup write — a single hub-note rename can back up
/// thousands of victims in a row, and each would otherwise re-scan the root.
static LAST_PURGE: std::sync::LazyLock<
    std::sync::Mutex<Option<(std::path::PathBuf, chrono::NaiveDate)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// Best-effort sweep of the backups root: removes every `<YYYY-MM-DD>` directory
/// whose date is older than [`BACKUP_RETENTION_DAYS`]. Runs at most once per
/// backups root per UTC day per process (see [`LAST_PURGE`]). Never fails the
/// caller — backups are housekeeping, and a purge error must not block (and
/// thereby abort) the edit that triggered it.
async fn purge_old_backups(backups_root: &Path) {
    let today = chrono::Utc::now().date_naive();
    // Skip if we already swept this root today (in this process). The marker is
    // only stamped AFTER a successful sweep below, so a transient failure (e.g.
    // the dir not existing yet, or a read error) is retried on the next backup.
    if LAST_PURGE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|(root, day)| root == backups_root && *day == today)
    {
        return;
    }
    let cutoff = today - chrono::Duration::days(BACKUP_RETENTION_DAYS);
    let mut entries = match tokio::fs::read_dir(backups_root).await {
        Ok(e) => e,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        if let Ok(date) = chrono::NaiveDate::parse_from_str(&name.to_string_lossy(), "%Y-%m-%d") {
            if date < cutoff {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
            }
        }
    }
    *LAST_PURGE.lock().unwrap() = Some((backups_root.to_path_buf(), today));
}

/// Atomically reserves a free backup destination: tries the mirrored name first,
/// then time-and-counter-suffixed variants, each via `create_new` so two writers
/// racing on the same note get distinct files and no pre-image is ever clobbered.
/// Returns the reserved (now-empty) path for the caller to copy into.
async fn reserve_backup_dest(base: &Path) -> Result<std::path::PathBuf, FSError> {
    let mut candidate = base.to_path_buf();
    let mut attempt: u32 = 0;
    loop {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let ts = chrono::Utc::now().format("%H%M%S%6f");
                let mut name = base.file_name().unwrap_or_default().to_os_string();
                name.push(format!(".{ts}.{attempt}"));
                candidate = base.with_file_name(name);
                attempt = attempt.wrapping_add(1);
            }
            Err(e) => return Err(FSError::ReadFileError(e)),
        }
    }
}

/// Copies the current on-disk content of the note at `path` into a hidden, dated
/// backup directory inside the vault, before the note is overwritten or deleted.
/// The backup lives at `<workspace>/.kimun/backups/<YYYY-MM-DD>/<note>` — the
/// note's on-disk path mirrored under the (UTC) date. The destination is claimed
/// atomically (see [`reserve_backup_dest`]); a repeat edit on the same day gets a
/// time-suffixed sibling, and concurrent writers never overwrite each other's
/// pre-image. Returns `Ok(())` without writing when the source note does not
/// exist (nothing to back up). `.kimun` is hidden, so the indexer's walker skips
/// it and backups never appear in search.
pub(crate) async fn backup_note<P: AsRef<Path>>(
    workspace_path: P,
    path: &VaultPath,
) -> Result<(), FSError> {
    let workspace_path = workspace_path.as_ref();
    let src = resolve_path_on_disk(workspace_path, path).await;
    // Fail closed: only skip the backup when the source is genuinely absent.
    // A probe error (FS unhealthy) must abort the edit, not silently proceed
    // without a pre-image.
    match tokio::fs::try_exists(&src).await {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(e) => return Err(FSError::ReadFileError(e)),
    }

    let rel = src
        .strip_prefix(workspace_path)
        .map_err(|_| FSError::InvalidPath {
            path: src.to_string_lossy().into_owned(),
            message: "note path escapes the workspace".to_string(),
        })?;
    let backups_root = workspace_path.join(".kimun").join("backups");
    purge_old_backups(&backups_root).await;
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let base = backups_root.join(date).join(rel);
    if let Some(parent) = base.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Reserve a unique name, then stream the source into it — no full read into
    // memory, and the reserved name can't be clobbered by a concurrent backup.
    let dest = reserve_backup_dest(&base).await?;
    tokio::fs::copy(&src, &dest).await?;
    Ok(())
}
