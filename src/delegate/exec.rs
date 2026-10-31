//! Runs the commands built by `plan` as real subprocesses.

use crate::delegate::plan::CliInvocation;
use crate::delegate::CliError;
use std::process::Command;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::plan::build_cli_invocations;
    use crate::domain::{ClientTargetKind, McpServersMap, NormalizedServerConfig};
    use std::collections::BTreeMap;

    fn servers() -> McpServersMap {
        let mut cfg = NormalizedServerConfig::new("npx");
        cfg.args = vec!["-y".into(), "pkg".into()];
        cfg.env = BTreeMap::from([("API_KEY".to_string(), "a b".to_string())]);
        McpServersMap::from([("my-srv".to_string(), cfg)])
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
