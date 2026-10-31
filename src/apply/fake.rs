//! An in-memory [`TargetIo`] for tests.

use super::TargetIo;
use crate::delegate::plan::CliInvocation;
use crate::delegate::CliError;
use crate::writer::{backup_path_for, WriteError};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct FakeIo {
    pub files: RefCell<BTreeMap<PathBuf, String>>,
    /// The content a path had before its last write (what `restore_backup` puts back).
    pub backups: RefCell<BTreeMap<PathBuf, String>>,
    pub unreadable: RefCell<BTreeSet<PathBuf>>,
    pub unwritable: RefCell<BTreeSet<PathBuf>>,
    pub written: RefCell<Vec<PathBuf>>,
    pub cli_calls: RefCell<Vec<Vec<String>>>,
    pub cli_fails: Cell<bool>,
}

impl FakeIo {
    pub fn with_file(self, path: &str, content: &str) -> Self {
        self.files
            .borrow_mut()
            .insert(PathBuf::from(path), content.to_string());
        self
    }

    pub fn content(&self, path: &str) -> Option<String> {
        self.files.borrow().get(Path::new(path)).cloned()
    }
}

impl TargetIo for FakeIo {
    fn read_config(&self, path: &Path) -> std::io::Result<Option<String>> {
        if self.unreadable.borrow().contains(path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            ));
        }
        Ok(self.files.borrow().get(path).cloned())
    }

    fn write_config(&self, path: &Path, content: &str) -> Result<(), WriteError> {
        if self.unwritable.borrow().contains(path) {
            return Err(WriteError::Io(std::io::Error::other("denied")));
        }
        if let Some(previous) = self.files.borrow().get(path) {
            self.backups
                .borrow_mut()
                .insert(path.to_path_buf(), previous.clone());
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), content.to_string());
        self.written.borrow_mut().push(path.to_path_buf());
        Ok(())
    }

    fn restore_backup(&self, path: &Path) -> Result<(), WriteError> {
        let Some(backup) = self.backups.borrow().get(path).cloned() else {
            return Err(WriteError::BackupMissing(backup_path_for(path)));
        };
        self.files.borrow_mut().insert(path.to_path_buf(), backup);
        Ok(())
    }

    fn remove_config(&self, path: &Path) -> std::io::Result<()> {
        self.files
            .borrow_mut()
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn config_exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError> {
        self.cli_calls
            .borrow_mut()
            .push(invocations.iter().map(CliInvocation::display).collect());
        if self.cli_fails.get() {
            return Err(CliError::CommandFailed {
                command: "fake".to_string(),
                message: "boom".to_string(),
            });
        }
        Ok(())
    }
}
