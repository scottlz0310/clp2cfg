//! Command and runtime filters (e.g., npx to bunx replacement).

use crate::model::NormalizedServerConfig;

/// Options controlling command filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterOptions {
    pub replace_npx_with_bunx: bool,
}

impl Default for FilterOptions {
    fn default() -> Self {
        Self {
            replace_npx_with_bunx: true,
        }
    }
}

/// Apply configured filters to a server config.
pub fn apply_filters(
    mut config: NormalizedServerConfig,
    options: &FilterOptions,
) -> NormalizedServerConfig {
    if options.replace_npx_with_bunx && config.command == "npx" {
        config.command = "bunx".to_string();
    }
    config
}
