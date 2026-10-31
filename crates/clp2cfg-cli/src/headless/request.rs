//! What a headless run was asked to do, independent of how it was asked (command-line flags).

use clp2cfg_core::snippet::filter::FilterOptions;

/// What a headless run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessAction {
    Apply,
    DryRun,
}

/// A fully specified headless run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessRequest {
    /// The snippet: `None` = clipboard, `-` = stdin, a file path, or the JSON text itself.
    pub input: Option<String>,
    pub action: HeadlessAction,
    /// `ClientTargetKind::cli_name`s. Empty = every existing file target.
    pub targets: Vec<String>,
    /// Show env values in the dry-run output instead of masking them.
    pub show_secrets: bool,
    /// Apply even when env values are still placeholders.
    pub allow_placeholders: bool,
    pub filters: FilterOptions,
}
