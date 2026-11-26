//! Installing a small set of files into one folder, safely.

use std::path::{Component, Path};

use super::{path_to_string, replace_atomically, SystemError, SystemPath};

/// What [`install_files`] did to the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderInstall {
    /// Files were written and none of them replaced an existing file.
    /// If the folder already existed and only gained new files, this is also the result.
    Created,
    /// Every file was already there with the same bytes. Nothing was written.
    Unchanged,
    /// These files existed with other bytes and were replaced.
    Replaced(Vec<String>),
}

/// A file name must be exactly one plain path component.
fn check_name(name: &str) -> Result<(), SystemError> {
    let mut parts = Path::new(name).components();
    let ok = matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !name.contains(['/', '\\']);
    if ok {
        Ok(())
    } else {
        Err(SystemError::io(
            "install",
            Path::new(name),
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a file name must be one plain path component",
            ),
        ))
    }
}

/// Writes `files` (name and bytes) into the folder `dir`.
///
/// The folder is created with its parents when missing. A folder that is a
/// symbolic link is refused in every case, even with `force`. Files that
/// already hold the same bytes are left alone. A file with different bytes is
/// an [`SystemError::AlreadyExists`] error unless `force` is set, and the check
/// runs before any write, so a refusal changes nothing. With `force`, those
/// files are replaced atomically and named in [`FolderInstall::Replaced`].
/// Other files in the folder are never touched.
pub fn install_files(
    dir: &SystemPath,
    files: &[(&str, &[u8])],
    force: bool,
) -> Result<FolderInstall, SystemError> {
    for (name, _) in files {
        check_name(name)?;
    }
    match std::fs::symlink_metadata(dir.as_path()) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(SystemError::Symlink {
                path: path_to_string(dir.as_path()),
            });
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(SystemError::io("inspect", dir.as_path(), e)),
    }

    // Decide what to write before writing anything.
    let mut missing = Vec::new();
    let mut differing = Vec::new();
    for (name, bytes) in files {
        let path = dir.join(name);
        match std::fs::read(path.as_path()) {
            Ok(existing) if existing == *bytes => {}
            Ok(_) => differing.push((*name, *bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push((*name, *bytes)),
            Err(e) => return Err(SystemError::io("read", path.as_path(), e)),
        }
    }
    if !force {
        if let Some((name, _)) = differing.first() {
            return Err(SystemError::AlreadyExists {
                path: path_to_string(dir.join(name).as_path()),
            });
        }
    }

    for (name, bytes) in missing.iter().chain(differing.iter()) {
        replace_atomically(dir.join(name).as_path(), bytes)?;
    }
    if !differing.is_empty() {
        Ok(FolderInstall::Replaced(
            differing.iter().map(|(n, _)| n.to_string()).collect(),
        ))
    } else if !missing.is_empty() {
        Ok(FolderInstall::Created)
    } else {
        Ok(FolderInstall::Unchanged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn target(tmp: &TempDir, rel: &str) -> SystemPath {
        SystemPath::try_absolute(tmp.path().join(rel)).unwrap()
    }

    #[test]
    fn creates_the_folder_and_files() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "a/b/skill");
        let r = install_files(&dir, &[("one.md", b"1"), ("two.md", b"2")], false).unwrap();
        assert!(matches!(r, FolderInstall::Created));
        assert_eq!(std::fs::read(dir.join("one.md")).unwrap(), b"1");
        assert_eq!(std::fs::read(dir.join("two.md")).unwrap(), b"2");
    }

    #[test]
    fn identical_content_is_unchanged() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "skill");
        install_files(&dir, &[("one.md", b"1")], false).unwrap();
        let r = install_files(&dir, &[("one.md", b"1")], false).unwrap();
        assert!(matches!(r, FolderInstall::Unchanged));
    }

    #[test]
    fn different_content_without_force_is_already_exists() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "skill");
        install_files(&dir, &[("one.md", b"1")], false).unwrap();
        let err = install_files(&dir, &[("one.md", b"changed")], false).unwrap_err();
        assert!(matches!(err, SystemError::AlreadyExists { .. }));
        assert_eq!(std::fs::read(dir.join("one.md")).unwrap(), b"1");
    }

    #[test]
    fn force_replaces_only_the_differing_files_and_names_them() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "skill");
        install_files(&dir, &[("one.md", b"1"), ("two.md", b"2")], false).unwrap();
        let r = install_files(&dir, &[("one.md", b"1"), ("two.md", b"new")], true).unwrap();
        match r {
            FolderInstall::Replaced(names) => assert_eq!(names, vec!["two.md".to_string()]),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(std::fs::read(dir.join("two.md")).unwrap(), b"new");
    }

    #[test]
    fn extra_files_stay() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "skill");
        install_files(&dir, &[("one.md", b"1")], false).unwrap();
        std::fs::write(dir.join("mine.txt"), b"keep").unwrap();
        install_files(&dir, &[("one.md", b"2")], true).unwrap();
        assert_eq!(std::fs::read(dir.join("mine.txt")).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_is_refused_even_with_force() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let dir = SystemPath::try_absolute(&link).unwrap();
        for force in [false, true] {
            let err = install_files(&dir, &[("one.md", b"1")], force).unwrap_err();
            assert!(matches!(err, SystemError::Symlink { .. }));
        }
        assert!(!real.join("one.md").exists());
    }

    #[test]
    fn a_name_with_a_separator_or_dots_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let dir = target(&tmp, "skill");
        for bad in ["a/b.md", "../x.md", "..", ".", "", "a\\b.md", "/abs.md"] {
            let err = install_files(&dir, &[(bad, b"x")], false).unwrap_err();
            match err {
                SystemError::Io { source, .. } => {
                    assert_eq!(source.kind(), std::io::ErrorKind::InvalidInput, "{bad}")
                }
                other => panic!("{bad}: unexpected {other:?}"),
            }
        }
        assert!(!tmp.path().join("x.md").exists());
    }
}
