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

    /// Human-readable, shell-quoted command line for previews and error messages.
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
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
/// Each server is registered as `remove` (failure ignored) followed by `add`, because
/// the CLIs do not support overwriting a same-named server in place.
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
        invocations.push(CliInvocation::new(program, remove_args(kind, name), true));
        invocations.push(CliInvocation::new(
            program,
            add_args(kind, name, config),
            false,
        ));
    }
    Ok(invocations)
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
    // `claude mcp add` の `-e` は可変長引数のため、name を先に置かないと name が env として解釈される
    args.push(name.to_string());
    for (k, v) in &config.env {
        args.push(
            if kind == ClientTargetKind::ClaudeCli {
                "-e"
            } else {
                "--env"
            }
            .into(),
        );
        args.push(format!("{k}={v}"));
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
        command: inv.display(),
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
    fn test_build_codex_remove_then_add() {
        let inv = build_cli_invocations(ClientTargetKind::CodexCli, "codex", &servers()).unwrap();
        assert_eq!(inv.len(), 2);
        assert_eq!(inv[0].display(), "codex mcp remove my-srv");
        assert!(inv[0].ignore_failure);
        assert_eq!(
            inv[1].display(),
            "codex mcp add my-srv --env 'API_KEY=a b' -- npx -y pkg"
        );
        assert!(!inv[1].ignore_failure);
    }

    #[test]
    fn test_build_claude_uses_user_scope_and_short_env_flag() {
        let inv = build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &servers()).unwrap();
        assert_eq!(inv[0].display(), "claude mcp remove --scope user my-srv");
        assert_eq!(
            inv[1].display(),
            "claude mcp add --scope user my-srv -e 'API_KEY=a b' -- npx -y pkg"
        );
    }

    #[test]
    fn test_build_copilot_and_agy_programs() {
        let c = build_cli_invocations(ClientTargetKind::CopilotCli, "copilot", &servers()).unwrap();
        assert!(c[1].display().starts_with("copilot mcp add "));
        let a = build_cli_invocations(ClientTargetKind::AntigravityCli, "agy", &servers()).unwrap();
        assert!(a[1].display().starts_with("agy mcp add "));
    }

    #[test]
    fn test_shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_run_ignores_remove_failure_but_stops_on_add_failure() {
        let inv = build_cli_invocations(ClientTargetKind::CodexCli, "codex", &servers()).unwrap();
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
}
