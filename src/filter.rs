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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config(command: &str) -> NormalizedServerConfig {
        let mut config = NormalizedServerConfig::new(command);
        config.args = vec!["arg1".to_string()];
        config
    }

    #[test]
    fn test_apply_filters_command_replacement() {
        struct TestCase {
            name: &'static str,
            input_command: &'static str,
            replace_npx: bool,
            expected_command: &'static str,
        }

        let cases = [
            TestCase {
                name: "npx is replaced with bunx when filter enabled",
                input_command: "npx",
                replace_npx: true,
                expected_command: "bunx",
            },
            TestCase {
                name: "npx is preserved when filter disabled",
                input_command: "npx",
                replace_npx: false,
                expected_command: "npx",
            },
            TestCase {
                name: "uvx command is preserved regardless of filter",
                input_command: "uvx",
                replace_npx: true,
                expected_command: "uvx",
            },
            TestCase {
                name: "node command is preserved regardless of filter",
                input_command: "node",
                replace_npx: true,
                expected_command: "node",
            },
        ];

        for case in cases {
            let config = create_test_config(case.input_command);
            let options = FilterOptions {
                replace_npx_with_bunx: case.replace_npx,
            };
            let filtered = apply_filters(config, &options);
            assert_eq!(
                filtered.command, case.expected_command,
                "Failed case: {}",
                case.name
            );
        }
    }
}
