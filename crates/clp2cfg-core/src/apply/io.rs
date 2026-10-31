use crate::delegate::exec::run_invocations;
use crate::delegate::plan::CliInvocation;
use crate::delegate::CliError;
use crate::discovery::detect_client_targets;
use crate::domain::ClientTarget;
use crate::writer::{self, WriteError};
use std::path::Path;

/// What applying needs from the outside world (injectable for tests).
pub trait TargetIo {
    /// The config file's content, `None` when it does not exist. Unreadable is an error.
    fn read_config(&self, path: &Path) -> std::io::Result<Option<String>>;
    /// Replace the config file atomically, keeping a `.bak` of the previous content.
    fn write_config(&self, path: &Path, content: &str) -> Result<(), WriteError>;
    /// Put the `.bak` taken before an apply back in place.
    fn restore_backup(&self, path: &Path) -> Result<(), WriteError>;
    fn remove_config(&self, path: &Path) -> std::io::Result<()>;
    /// Whether the config file exists as a regular file (also true when its content is unreadable).
    fn config_exists(&self, path: &Path) -> bool;
    /// Run the commands of a CLI-delegated target, in order.
    fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError>;
}

/// The real file system and real subprocesses.
pub struct SystemIo;

impl TargetIo for SystemIo {
    fn read_config(&self, path: &Path) -> std::io::Result<Option<String>> {
        writer::read_if_exists(path)
    }

    fn write_config(&self, path: &Path, content: &str) -> Result<(), WriteError> {
        writer::safe_atomic_write(path, content)
    }

    fn restore_backup(&self, path: &Path) -> Result<(), WriteError> {
        writer::restore_from_backup(path)
    }

    fn remove_config(&self, path: &Path) -> std::io::Result<()> {
        writer::remove_file(path)
    }

    fn config_exists(&self, path: &Path) -> bool {
        writer::is_file(path)
    }

    fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError> {
        run_invocations(invocations)
    }
}

/// Where the clients' config files are expected to be (injectable for tests).
pub trait TargetCatalog {
    /// Every known target, existing or not, as the GUI lists them.
    fn targets(&self) -> Vec<ClientTarget>;
}

/// The clients detected on this machine.
pub struct SystemCatalog;

impl TargetCatalog for SystemCatalog {
    fn targets(&self) -> Vec<ClientTarget> {
        detect_client_targets(None)
    }
}
