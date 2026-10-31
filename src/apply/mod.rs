//! Applying a snippet to the clients: plan what would happen, do it, undo it.
//!
//! Both frontends (GUI and headless) go through here. Files and subprocesses are reached only
//! through [`TargetIo`], so the logic can be tested with a fake and frontends never call the
//! writer or the CLI runner directly.

mod error;
#[cfg(test)]
mod fake;
mod io;
mod plan;
mod run;
mod undo;

pub use error::ApplyError;
pub use io::{SystemCatalog, SystemIo, TargetCatalog, TargetIo};
pub use plan::{count_changes, plan_target, ServerChanges, TargetPlan};
pub use run::{apply_targets, Applied, ApplyOutcome, Mode, TargetOutcome};
pub use undo::{undo_file, AppliedFile, LastApply, UndoAction, UndoError};
