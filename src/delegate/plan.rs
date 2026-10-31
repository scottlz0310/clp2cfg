//! Builds the official-CLI commands that register servers, and renders them for display.
//! Pure: nothing here runs a process (see `exec`).

use crate::delegate::CliError;
use crate::domain::{CliSpec, ClientTargetKind, McpServersMap, NormalizedServerConfig};

/// A single subprocess invocation of a client's official CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    pub program: String,
    pub args: Vec<String>,
    /// Failure is tolerated (e.g. `remove` of a server that does not exist yet).
    pub ignore_failure: bool,
}

impl CliInvocation {
    pub(super) fn new(program: &str, args: Vec<String>, ignore_failure: bool) -> Self {
        Self {
            program: program.to_string(),
            args,
            ignore_failure,
        }
    }

    /// Human-readable, shell-quoted command line for previews.
    pub fn display(&self) -> String {
        self.render(false)
    }

    /// Same as [`display`](Self::display) but with `-e` / `--env` values replaced by `***`,
    /// so secrets never reach status messages or logs.
    pub fn display_redacted(&self) -> String {
        self.render(true)
    }

    fn render(&self, redact_env: bool) -> String {
        let mut after_env_flag = false;
        let mut parts = vec![shell_quote(&self.program)];
        for arg in &self.args {
            let value = match arg.split_once('=') {
                Some((key, _)) if redact_env && after_env_flag => format!("{key}=***"),
                _ => arg.clone(),
            };
            after_env_flag = matches!(arg.as_str(), "-e" | "--env");
            parts.push(shell_quote(&value));
        }
        parts.join(" ")
    }
}

fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Build the commands that register `servers` through the client's official CLI.
///
/// CLIs whose `mcp add` rejects an existing name get a `remove` (failure ignored) first;
/// CLIs that upsert natively (Codex, Antigravity) run `add` alone, so a failed `add`
/// never leaves the previous definition removed.
/// Properties other than `command` / `args` / `env` cannot be expressed through the CLIs.
pub fn build_cli_invocations(
    kind: ClientTargetKind,
    program: &str,
    servers: &McpServersMap,
) -> Result<Vec<CliInvocation>, CliError> {
    let spec = kind.cli_spec().ok_or(CliError::NotCliTarget)?;

    let mut invocations = Vec::new();
    for (name, config) in servers {
        if spec.remove_before_add {
            invocations.push(CliInvocation::new(program, remove_args(spec, name), true));
        }
        invocations.push(CliInvocation::new(
            program,
            add_args(spec, name, config),
            false,
        ));
    }
    Ok(invocations)
}

fn remove_args(spec: &CliSpec, name: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["mcp".into(), "remove".into()];
    if let Some(scope) = spec.scope {
        args.extend(["--scope".into(), scope.into()]);
    }
    args.push(name.to_string());
    args
}

fn add_args(spec: &CliSpec, name: &str, config: &NormalizedServerConfig) -> Vec<String> {
    let mut args: Vec<String> = vec!["mcp".into(), "add".into()];
    if let Some(scope) = spec.scope {
        args.extend(["--scope".into(), scope.into()]);
    }
    let env_args = config
        .env
        .iter()
        .flat_map(|(k, v)| [spec.env_flag.to_string(), format!("{k}={v}")]);
    if spec.flags_before_name {
        args.extend(env_args);
        args.push(name.to_string());
    } else {
        args.push(name.to_string());
        args.extend(env_args);
    }
    args.push("--".into());
    args.push(config.command.clone());
    args.extend(config.args.iter().cloned());
    args
}

/// Render invocations as a multi-line preview (one command per line).
pub fn preview_invocations(invocations: &[CliInvocation]) -> String {
    invocations
        .iter()
        .map(CliInvocation::display)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn servers() -> McpServersMap {
        let mut cfg = NormalizedServerConfig::new("npx");
        cfg.args = vec!["-y".into(), "pkg".into()];
        cfg.env = BTreeMap::from([("API_KEY".to_string(), "a b".to_string())]);
        McpServersMap::from([("my-srv".to_string(), cfg)])
    }

    #[test]
    fn test_build_rejects_file_target() {
        let err = build_cli_invocations(ClientTargetKind::Cursor, "x", &servers()).unwrap_err();
        assert_eq!(err, CliError::NotCliTarget);
    }

    #[test]
    fn test_build_codex_upserts_without_remove() {
        let inv = build_cli_invocations(ClientTargetKind::CodexCli, "codex", &servers()).unwrap();
        assert_eq!(inv.len(), 1);
        assert_eq!(
            inv[0].display(),
            "codex mcp add my-srv --env 'API_KEY=a b' -- npx -y pkg"
        );
        assert!(!inv[0].ignore_failure);
    }

    #[test]
    fn test_build_agy_puts_flags_before_name_without_remove() {
        let inv =
            build_cli_invocations(ClientTargetKind::AntigravityCli, "agy", &servers()).unwrap();
        assert_eq!(inv.len(), 1);
        assert_eq!(
            inv[0].display(),
            "agy mcp add --env 'API_KEY=a b' my-srv -- npx -y pkg"
        );
    }

    #[test]
    fn test_build_claude_uses_user_scope_and_short_env_flag() {
        let inv = build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &servers()).unwrap();
        assert_eq!(inv.len(), 2);
        assert!(inv[0].ignore_failure);
        assert_eq!(inv[0].display(), "claude mcp remove --scope user my-srv");
        assert_eq!(
            inv[1].display(),
            "claude mcp add --scope user my-srv -e 'API_KEY=a b' -- npx -y pkg"
        );
    }

    #[test]
    fn test_build_copilot_removes_then_adds() {
        let c = build_cli_invocations(ClientTargetKind::CopilotCli, "copilot", &servers()).unwrap();
        assert_eq!(c[0].display(), "copilot mcp remove my-srv");
        assert_eq!(
            c[1].display(),
            "copilot mcp add my-srv --env 'API_KEY=a b' -- npx -y pkg"
        );
    }

    #[test]
    fn test_shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_display_redacted_masks_only_env_values() {
        let cases = [
            // (args, display, display_redacted)
            (
                vec!["mcp", "add", "n", "-e", "K=secret", "--", "cmd", "A=b"],
                "p mcp add n -e K=secret -- cmd A=b",
                "p mcp add n -e 'K=***' -- cmd A=b",
            ),
            (
                vec!["mcp", "add", "--env", "K=with space", "n", "--", "cmd"],
                "p mcp add --env 'K=with space' n -- cmd",
                "p mcp add --env 'K=***' n -- cmd",
            ),
            (
                vec!["mcp", "add", "n", "--", "cmd"],
                "p mcp add n -- cmd",
                "p mcp add n -- cmd",
            ),
        ];

        for (args, plain, redacted) in cases {
            let inv = CliInvocation::new("p", args.iter().map(|a| a.to_string()).collect(), false);
            assert_eq!(inv.display(), plain);
            assert_eq!(inv.display_redacted(), redacted);
        }
    }
}
