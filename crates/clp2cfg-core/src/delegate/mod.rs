//! CLI delegation to the clients' official CLIs (`<cli> mcp add ...`).
//!
//! `plan` builds the commands (pure); `exec` runs them (spawns processes). `exec` is deliberately
//! not re-exported here: this module is pure in the layering rules (`tests/architecture_test.rs`).

use thiserror::Error;

pub mod exec;
pub mod plan;

/// Error types occurring while building or running official CLI commands.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CliError {
    #[error("Target kind does not use CLI delegation")]
    NotCliTarget,

    #[error("Failed to launch '{program}': {message}")]
    LaunchFailed { program: String, message: String },

    #[error("'{command}' exited with failure: {message}")]
    CommandFailed { command: String, message: String },
}
