//! Undo of the most recent apply: restore files from their `.bak`, or remove files the apply created.

use crate::model::WriteError;
use crate::writer::restore_from_backup;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

/// A file written by an apply, with what is needed to undo it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedFile {
    pub target_name: String,
    pub path: PathBuf,
    /// Exact content written; undo refuses if the file no longer matches it.
    pub written: String,
    /// Whether the file existed before the apply (so a `.bak` was made) or was newly created.
    pub had_original: bool,
}

/// What the most recent (non dry-run) apply changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastApply {
    pub files: Vec<AppliedFile>,
    /// CLI-delegated targets that were run. Their `mcp add` cannot be undone from here.
    pub cli_targets: Vec<String>,
}

/// What undoing one file did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoAction {
    /// Content was replaced by the `.bak` taken before the apply.
    Restored,
    /// The file was created by the apply and has been removed.
    Removed,
    /// The file was created by the apply and is already gone.
    AlreadyGone,
}

#[derive(Debug, Error)]
pub enum UndoError {
    #[error("適用後に内容が変更されているため取り消しません（他の変更を失わないため）")]
    ModifiedSinceApply,

    #[error("バックアップからの復元に失敗: {0}")]
    Restore(#[from] WriteError),

    #[error("ファイル操作に失敗: {0}")]
    Io(#[from] std::io::Error),
}

/// Undo a single applied file.
///
/// The file is only touched when its current content is exactly what the apply wrote,
/// so changes made by anything else after the apply are never discarded.
pub fn undo_file(file: &AppliedFile) -> Result<UndoAction, UndoError> {
    let current = match fs::read_to_string(&file.path) {
        Ok(content) => Some(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    if current.as_deref().is_some_and(|c| c != file.written) {
        return Err(UndoError::ModifiedSinceApply);
    }

    match (file.had_original, current) {
        (true, _) => {
            restore_from_backup(&file.path)?;
            Ok(UndoAction::Restored)
        }
        (false, Some(_)) => {
            fs::remove_file(&file.path)?;
            Ok(UndoAction::Removed)
        }
        (false, None) => Ok(UndoAction::AlreadyGone),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::safe_atomic_write;
    use tempfile::tempdir;

    /// Apply `content` over `original` (if any) the way the app does, and describe it.
    fn apply(dir: &std::path::Path, original: Option<&str>, content: &str) -> AppliedFile {
        let path = dir.join("mcp.json");
        if let Some(original) = original {
            fs::write(&path, original).unwrap();
        }
        let had_original = path.is_file();
        safe_atomic_write(&path, content).unwrap();
        AppliedFile {
            target_name: "Test".to_string(),
            path,
            written: content.to_string(),
            had_original,
        }
    }

    #[test]
    fn test_undo_restores_original_and_removes_created() {
        struct Case {
            name: &'static str,
            original: Option<&'static str>,
            /// 適用後にファイルをこの内容へ変更する（None なら変更しない、削除は別ケース）
            tamper: Option<&'static str>,
            expected: Result<UndoAction, &'static str>,
            /// 取り消し後のファイル内容（None なら存在しない）
            after: Option<&'static str>,
        }

        let cases = [
            Case {
                name: "existing file is restored from .bak",
                original: Some("original"),
                tamper: None,
                expected: Ok(UndoAction::Restored),
                after: Some("original"),
            },
            Case {
                name: "file created by the apply is removed",
                original: None,
                tamper: None,
                expected: Ok(UndoAction::Removed),
                after: None,
            },
            Case {
                name: "existing file modified after the apply is left alone",
                original: Some("original"),
                tamper: Some("edited by someone else"),
                expected: Err("modified"),
                after: Some("edited by someone else"),
            },
            Case {
                name: "created file modified after the apply is left alone",
                original: None,
                tamper: Some("edited by someone else"),
                expected: Err("modified"),
                after: Some("edited by someone else"),
            },
        ];

        for case in cases {
            let dir = tempdir().unwrap();
            let applied = apply(dir.path(), case.original, "applied");
            if let Some(tampered) = case.tamper {
                fs::write(&applied.path, tampered).unwrap();
            }

            let result = undo_file(&applied);

            match (&result, &case.expected) {
                (Ok(actual), Ok(expected)) => assert_eq!(actual, expected, "{}", case.name),
                (Err(UndoError::ModifiedSinceApply), Err("modified")) => {}
                _ => panic!("{}: unexpected result {result:?}", case.name),
            }
            let after = fs::read_to_string(&applied.path).ok();
            assert_eq!(after.as_deref(), case.after, "{}", case.name);
        }
    }

    #[test]
    fn test_undo_when_file_was_deleted_after_apply() {
        // 新規作成したファイルが既に無いなら何もしない
        let dir = tempdir().unwrap();
        let created = apply(dir.path(), None, "applied");
        fs::remove_file(&created.path).unwrap();
        assert_eq!(undo_file(&created).unwrap(), UndoAction::AlreadyGone);

        // 既存ファイルが消されていても、.bak から復元できる（失うものがない）
        let dir = tempdir().unwrap();
        let existing = apply(dir.path(), Some("original"), "applied");
        fs::remove_file(&existing.path).unwrap();
        assert_eq!(undo_file(&existing).unwrap(), UndoAction::Restored);
        assert_eq!(fs::read_to_string(&existing.path).unwrap(), "original");
    }

    #[test]
    fn test_undo_fails_when_backup_is_missing() {
        let dir = tempdir().unwrap();
        let applied = apply(dir.path(), Some("original"), "applied");
        fs::remove_file(dir.path().join("mcp.json.bak")).unwrap();

        let err = undo_file(&applied).unwrap_err();

        assert!(matches!(
            err,
            UndoError::Restore(WriteError::BackupMissing(_))
        ));
        // 失敗しても現在のファイルは触らない
        assert_eq!(fs::read_to_string(&applied.path).unwrap(), "applied");
    }
}
