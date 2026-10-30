//! CLI delegation adapter: builds and runs official `<cli> mcp add` commands.

use crate::model::{
    CliError, ClientTargetKind, McpServersMap, NormalizedServerConfig, TargetAction,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A single subprocess invocation of a client's official CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    pub program: String,
    pub args: Vec<String>,
    /// Failure is tolerated (e.g. `remove` of a server that does not exist yet).
    pub ignore_failure: bool,
}

impl CliInvocation {
    fn new(program: &str, args: Vec<String>, ignore_failure: bool) -> Self {
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
    if kind.action() != TargetAction::CliCommand {
        return Err(CliError::NotCliTarget);
    }

    let mut invocations = Vec::new();
    for (name, config) in servers {
        if needs_remove_before_add(kind) {
            invocations.push(CliInvocation::new(program, remove_args(kind, name), true));
        }
        invocations.push(CliInvocation::new(
            program,
            add_args(kind, name, config),
            false,
        ));
    }
    Ok(invocations)
}

/// Verified on real CLIs: `claude mcp add` and `copilot mcp add` reject an existing name;
/// `codex mcp add` and `agy mcp add` upsert.
fn needs_remove_before_add(kind: ClientTargetKind) -> bool {
    matches!(
        kind,
        ClientTargetKind::ClaudeCli | ClientTargetKind::CopilotCli
    )
}

fn remove_args(kind: ClientTargetKind, name: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["mcp".into(), "remove".into()];
    if kind == ClientTargetKind::ClaudeCli {
        args.extend(["--scope".into(), "user".into()]);
    }
    args.push(name.to_string());
    args
}

fn add_args(kind: ClientTargetKind, name: &str, config: &NormalizedServerConfig) -> Vec<String> {
    let mut args: Vec<String> = vec!["mcp".into(), "add".into()];
    if kind == ClientTargetKind::ClaudeCli {
        args.extend(["--scope".into(), "user".into()]);
    }
    // claude/codex/copilot: name は env フラグより前 (claude の `-e` は可変長で name を飲み込む)
    // agy: フラグは name より前に置く必要がある
    let env_flag = if kind == ClientTargetKind::ClaudeCli {
        "-e"
    } else {
        "--env"
    };
    let env_args = config
        .env
        .iter()
        .flat_map(|(k, v)| [env_flag.to_string(), format!("{k}={v}")]);
    if kind == ClientTargetKind::AntigravityCli {
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

/// Run invocations in order using `runner`; stops at the first non-ignorable failure.
pub fn run_invocations_with<F>(invocations: &[CliInvocation], mut runner: F) -> Result<(), CliError>
where
    F: FnMut(&CliInvocation) -> Result<(), CliError>,
{
    for inv in invocations {
        match runner(inv) {
            Ok(()) => {}
            Err(_) if inv.ignore_failure => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Run invocations as real subprocesses.
pub fn run_invocations(invocations: &[CliInvocation]) -> Result<(), CliError> {
    run_invocations_with(invocations, run_subprocess)
}

fn run_subprocess(inv: &CliInvocation) -> Result<(), CliError> {
    let output = Command::new(&inv.program)
        .args(&inv.args)
        .output()
        .map_err(|e| CliError::LaunchFailed {
            program: inv.program.clone(),
            message: e.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = if stderr.trim().is_empty() {
        output.status.to_string()
    } else {
        stderr.trim().to_string()
    };
    Err(CliError::CommandFailed {
        command: inv.display_redacted(),
        message,
    })
}

/// Locate an executable on `PATH`.
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    find_in_dirs(program, std::env::split_paths(&paths))
}

fn find_in_dirs(program: &str, dirs: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    for dir in dirs {
        for ext in exts {
            let candidate = if ext.is_empty() {
                dir.join(program)
            } else {
                dir.join(format!("{program}.{ext}"))
            };
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
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
    fn test_run_ignores_remove_failure_but_stops_on_add_failure() {
        let inv = build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &servers()).unwrap();
        let fail = |i: &CliInvocation| -> Result<(), CliError> {
            Err(CliError::CommandFailed {
                command: i.display(),
                message: "boom".into(),
            })
        };
        // remove failure tolerated, add failure reported
        let err = run_invocations_with(&inv, fail).unwrap_err();
        assert!(
            matches!(err, CliError::CommandFailed { ref command, .. } if command.contains("mcp add"))
        );

        let mut seen = Vec::new();
        run_invocations_with(&inv, |i| {
            seen.push(i.display());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn test_run_subprocess_launch_failure() {
        let inv = CliInvocation::new("clp2cfg-nonexistent-binary", vec![], false);
        assert!(matches!(
            run_invocations(&[inv]),
            Err(CliError::LaunchFailed { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_run_subprocess_real_exit_status() {
        let ok = CliInvocation::new("true", vec![], false);
        assert!(run_invocations(&[ok]).is_ok());
        let bad = CliInvocation::new("false", vec![], false);
        assert!(matches!(
            run_invocations(&[bad]),
            Err(CliError::CommandFailed { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_find_in_dirs_requires_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mytool");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        assert!(find_in_dirs("mytool", [dir.path().to_path_buf()].into_iter()).is_none());
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_in_dirs("mytool", [dir.path().to_path_buf()].into_iter()),
            Some(exe)
        );
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

    #[test]
    fn test_command_failed_error_does_not_leak_env_values() {
        let inv = CliInvocation::new(
            "definitely-not-a-real-program",
            vec![
                "mcp".into(),
                "add".into(),
                "-e".into(),
                "K=topsecret".into(),
            ],
            false,
        );
        let err = run_subprocess(&inv).unwrap_err().to_string();
        assert!(!err.contains("topsecret"), "{err}");
    }
}
