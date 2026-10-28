//! Safe atomic writer with automatic backup.

use crate::model::WriteError;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Creates a backup of the target file if it already exists.
pub fn create_backup_if_exists(target_path: &Path) -> Result<Option<PathBuf>, WriteError> {
    if !target_path.is_file() {
        return Ok(None);
    }

    let backup_path = target_path.with_extension(format!(
        "{}.bak",
        target_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("json")
    ));

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
}
