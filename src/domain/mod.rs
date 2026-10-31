//! Domain types: no I/O, no OS access, no UI. The layering rules live in `tests/architecture_test.rs`.

mod change;
mod client;
mod server;

pub use change::ServerChange;
pub use client::{ClientTarget, ClientTargetKind, TargetAction};
pub use server::{McpServersMap, NormalizedServerConfig};
