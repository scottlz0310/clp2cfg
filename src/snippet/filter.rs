//! Command and runtime filters (e.g., npx to bunx, pipx run to uvx replacement).

use crate::domain::NormalizedServerConfig;

/// Options controlling command filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterOptions {
    pub replace_npx_with_bunx: bool,
    pub replace_pipx_with_uvx: bool,
}

impl Default for FilterOptions {
    fn default() -> Self {
        Self {
            replace_npx_with_bunx: true,
            replace_pipx_with_uvx: true,
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
    if options.replace_pipx_with_uvx {
        replace_pipx_run_with_uvx(&mut config);
    }
    config
}

/// `pipx run PKG [args...]` → `uvx PKG [args...]`.
///
/// 同じ意味に変換できる素直な形だけを対象にする。`pipx run` 固有のオプション
/// （`--spec` / `--python` 等）や、URL・パス・バージョン指定つきの引数は `uvx` と構文が
/// 異なり、機械的に書き換えると別物になるため、そのまま残す。
fn replace_pipx_run_with_uvx(config: &mut NormalizedServerConfig) {
    let is_plain_run = config.command == "pipx"
        && config.args.first().map(String::as_str) == Some("run")
        && config
            .args
            .get(1)
            .is_some_and(|app| is_plain_package_name(app));
    if is_plain_run {
        config.command = "uvx".to_string();
        config.args.remove(0);
    }
}

/// PyPI のパッケージ名として素直な文字列（英数字・`.`・`_`・`-`）か。
fn is_plain_package_name(value: &str) -> bool {
    value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !value.ends_with(".py")
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
                replace_pipx_with_uvx: false,
            };
            let filtered = apply_filters(config, &options);
            assert_eq!(
                filtered.command, case.expected_command,
                "Failed case: {}",
                case.name
            );
        }
    }

    #[test]
    fn test_apply_filters_pipx_run_to_uvx() {
        struct TestCase {
            name: &'static str,
            command: &'static str,
            args: &'static [&'static str],
            enabled: bool,
            expected_command: &'static str,
            expected_args: &'static [&'static str],
        }

        let cases = [
            TestCase {
                name: "plain pipx run is rewritten",
                command: "pipx",
                args: &["run", "mcp-server-git", "--repository", "/tmp/repo"],
                enabled: true,
                expected_command: "uvx",
                expected_args: &["mcp-server-git", "--repository", "/tmp/repo"],
            },
            TestCase {
                name: "package name with dots and underscores is rewritten",
                command: "pipx",
                args: &["run", "my_pkg.tool"],
                enabled: true,
                expected_command: "uvx",
                expected_args: &["my_pkg.tool"],
            },
            TestCase {
                name: "disabled filter leaves pipx run untouched",
                command: "pipx",
                args: &["run", "mcp-server-git"],
                enabled: false,
                expected_command: "pipx",
                expected_args: &["run", "mcp-server-git"],
            },
            TestCase {
                name: "pipx run options (--spec) are not rewritten",
                command: "pipx",
                args: &["run", "--spec", "pkg", "app"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run", "--spec", "pkg", "app"],
            },
            TestCase {
                name: "version-pinned package is not rewritten",
                command: "pipx",
                args: &["run", "pkg==1.0"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run", "pkg==1.0"],
            },
            TestCase {
                name: "URL is not rewritten",
                command: "pipx",
                args: &["run", "git+https://example.com/pkg.git"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run", "git+https://example.com/pkg.git"],
            },
            TestCase {
                name: "script path is not rewritten",
                command: "pipx",
                args: &["run", "./tool.py"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run", "./tool.py"],
            },
            TestCase {
                name: "script file name is not rewritten",
                command: "pipx",
                args: &["run", "tool.py"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run", "tool.py"],
            },
            TestCase {
                name: "pipx subcommands other than run are not rewritten",
                command: "pipx",
                args: &["install", "pkg"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["install", "pkg"],
            },
            TestCase {
                name: "pipx run without an app is not rewritten",
                command: "pipx",
                args: &["run"],
                enabled: true,
                expected_command: "pipx",
                expected_args: &["run"],
            },
            TestCase {
                name: "python -m is intentionally not rewritten",
                command: "python",
                args: &["-m", "mcp_server_git"],
                enabled: true,
                expected_command: "python",
                expected_args: &["-m", "mcp_server_git"],
            },
        ];

        for case in cases {
            let mut config = NormalizedServerConfig::new(case.command);
            config.args = case.args.iter().map(|a| a.to_string()).collect();
            let options = FilterOptions {
                replace_npx_with_bunx: false,
                replace_pipx_with_uvx: case.enabled,
            };

            let filtered = apply_filters(config, &options);

            assert_eq!(
                filtered.command, case.expected_command,
                "command, case: {}",
                case.name
            );
            assert_eq!(
                filtered.args, case.expected_args,
                "args, case: {}",
                case.name
            );
        }
    }

    #[test]
    fn test_default_options_enable_both_filters() {
        let options = FilterOptions::default();
        assert!(options.replace_npx_with_bunx);
        assert!(options.replace_pipx_with_uvx);
    }
}
