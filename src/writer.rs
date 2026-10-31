//! Safe atomic writer with automatic backup.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Error types occurring during safe atomic writing.
#[derive(Debug, Error)]
pub enum WriteError {
    #[error("I/O error during writing: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to create backup at {path}: {source}")]
    BackupFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to persist temporary file to {path}: {source}")]
    PersistFailed {
        path: PathBuf,
        source: tempfile::PersistError,
    },

    #[error("Backup file not found: {0}")]
    BackupMissing(PathBuf),
}

/// Read a config file; `Ok(None)` when it does not exist (any other failure is an error).
pub fn read_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Remove a config file (used to undo a file the apply created).
pub fn remove_file(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)
}

/// The path of the backup made before overwriting `target_path` (e.g. `mcp.json` -> `mcp.json.bak`).
pub fn backup_path_for(target_path: &Path) -> PathBuf {
    target_path.with_extension(format!(
        "{}.bak",
        target_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("json")
    ))
}

/// Creates a backup of the target file if it already exists.
pub fn create_backup_if_exists(target_path: &Path) -> Result<Option<PathBuf>, WriteError> {
    if !target_path.is_file() {
        return Ok(None);
    }

    let backup_path = backup_path_for(target_path);

    fs::copy(target_path, &backup_path).map_err(|source| WriteError::BackupFailed {
        path: backup_path.clone(),
        source,
    })?;

    Ok(Some(backup_path))
}

/// Safely writes content to the target file atomically, creating parent dirs and backup.
pub fn safe_atomic_write(target_path: &Path, content: &str) -> Result<(), WriteError> {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));

    // 1. Ensure parent directory exists
    if !parent.is_dir() {
        fs::create_dir_all(parent)?;
    }

    // 2. Create backup if target file exists
    create_backup_if_exists(target_path)?;

    // 3. Create a temporary file in the same parent directory
    let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
    temp_file.write_all(content.as_bytes())?;
    temp_file.flush()?;

    // 4. Atomically replace target path
    // On Windows, rename over an existing file fails, so we handle it with a safe swap.
    replace_atomically(temp_file, target_path)
}

/// Atomically restores `target_path` from its `.bak`.
///
/// The `.bak` is left in place and no new backup of the current content is made
/// (unlike [`safe_atomic_write`], which would overwrite the `.bak` being restored).
pub fn restore_from_backup(target_path: &Path) -> Result<(), WriteError> {
    let backup_path = backup_path_for(target_path);
    if !backup_path.is_file() {
        return Err(WriteError::BackupMissing(backup_path));
    }
    let content = fs::read(&backup_path)?;

    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
    temp_file.write_all(&content)?;
    temp_file.flush()?;

    replace_atomically(temp_file, target_path)
}

/// Atomically replaces target_path with the contents of temp_file across platforms.
fn replace_atomically(
    temp_file: tempfile::NamedTempFile,
    target_path: &Path,
) -> Result<(), WriteError> {
    if !target_path.exists() {
        // Target doesn't exist yet, simple persist works directly
        temp_file
            .persist(target_path)
            .map_err(|e| WriteError::PersistFailed {
                path: target_path.to_path_buf(),
                source: e,
            })?;
        return Ok(());
    }

    // When target already exists on Windows:
    // Move existing to .old -> persist temp to target -> remove .old
    let old_path = target_path.with_extension("replace.old");
    if old_path.exists() {
        let _ = fs::remove_file(&old_path);
    }

    fs::rename(target_path, &old_path)?;

    match temp_file.persist(target_path) {
        Ok(_) => {
            let _ = fs::remove_file(&old_path);
            Ok(())
        }
        Err(e) => {
            // Rollback on failure
            let _ = fs::rename(&old_path, target_path);
            Err(WriteError::PersistFailed {
                path: target_path.to_path_buf(),
                source: e,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_read_if_exists_distinguishes_missing_from_unreadable() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.json");

        assert_eq!(read_if_exists(&path).unwrap(), None);

        fs::write(&path, "{}").unwrap();
        assert_eq!(read_if_exists(&path).unwrap(), Some("{}".to_string()));

        // UTF-8 でない内容は「無い」ではなく読み取り失敗
        fs::write(&path, [0xFF, 0xFE, 0x00]).unwrap();
        assert!(read_if_exists(&path).is_err());

        // ディレクトリも「無い」ではなく読み取り失敗
        assert!(read_if_exists(dir.path()).is_err());
    }

    #[test]
    fn test_remove_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "{}").unwrap();

        remove_file(&path).unwrap();

        assert!(!path.exists());
        assert!(remove_file(&path).is_err());
    }

    #[test]
    fn test_safe_atomic_write_new_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("sub").join("config.json");

        safe_atomic_write(&file_path, "{\"hello\":\"world\"}").unwrap();

        assert!(file_path.is_file());
        let read = fs::read_to_string(&file_path).unwrap();
        assert_eq!(read, "{\"hello\":\"world\"}");
    }

    #[test]
    fn test_safe_atomic_write_creates_backup() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.json");

        // Write initial
        safe_atomic_write(&file_path, "initial content").unwrap();

        // Write update
        safe_atomic_write(&file_path, "updated content").unwrap();

        // Check content
        let read = fs::read_to_string(&file_path).unwrap();
        assert_eq!(read, "updated content");

        // Check backup
        let backup_path = dir.path().join("config.json.bak");
        assert!(backup_path.is_file());
        let backup_read = fs::read_to_string(&backup_path).unwrap();
        assert_eq!(backup_read, "initial content");
    }

    #[test]
    fn test_backup_path_for() {
        let cases = [
            ("/a/mcp.json", "/a/mcp.json.bak"),
            ("/a/settings.jsonc", "/a/settings.jsonc.bak"),
            ("/a/noext", "/a/noext.json.bak"),
        ];
        for (target, expected) in cases {
            assert_eq!(backup_path_for(Path::new(target)), Path::new(expected));
        }
    }

    #[test]
    fn test_restore_from_backup_replaces_content_and_keeps_backup() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.json");
        safe_atomic_write(&file_path, "original").unwrap();
        safe_atomic_write(&file_path, "updated").unwrap();

        restore_from_backup(&file_path).unwrap();

        assert_eq!(fs::read_to_string(&file_path).unwrap(), "original");
        // .bak は残り、復元によって上書きされない
        let backup_path = dir.path().join("config.json.bak");
        assert_eq!(fs::read_to_string(backup_path).unwrap(), "original");
    }

    #[test]
    fn test_restore_from_backup_without_backup_fails() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.json");
        fs::write(&file_path, "current").unwrap();

        let err = restore_from_backup(&file_path).unwrap_err();

        assert!(matches!(err, WriteError::BackupMissing(_)));
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "current");
    }
}
