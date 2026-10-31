use crate::delegate::CliError;
use crate::merge::MergeError;
use crate::writer::WriteError;
use std::path::PathBuf;
use thiserror::Error;

/// Why planning or applying to one target failed. The inner errors keep their own messages.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// An existing config file that cannot be read must not be treated as empty
    /// (merging into "nothing" and writing it back would drop the other settings).
    #[error("設定ファイルを読み取れません: {}: {source}", .path.display())]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(transparent)]
    Merge(#[from] MergeError),

    #[error(transparent)]
    Cli(#[from] CliError),

    #[error(transparent)]
    Write(#[from] WriteError),
}
