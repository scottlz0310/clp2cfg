//! The command line of clp2cfg: argument parsing and the headless apply / dry-run.
//! The GUI binary reuses this for `--apply` / `--dry-run`.

pub mod args;
pub mod entry;
pub mod headless;
