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
            message: inv.redact_text(&e.to_string()),
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(build_command_failed_error(
        inv,
        &output.status.to_string(),
        &output.stderr,
    ))
}

fn build_command_failed_error(
    inv: &CliInvocation,
    status_text: &str,
    stderr_bytes: &[u8],
) -> CliError {
    let stderr = String::from_utf8_lossy(stderr_bytes);
    let raw_message = if stderr.trim().is_empty() {
        status_text.to_string()
    } else {
        stderr.trim().to_string()
    };
    CliError::CommandFailed {
        command: inv.display_redacted(),
        message: inv.redact_text(&raw_message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegate::plan::build_cli_invocations;
    use crate::domain::{ClientTargetKind, McpServersMap, NormalizedServerConfig, ServerTransport};
    use std::collections::BTreeMap;

    fn servers() -> McpServersMap {
        let mut cfg = NormalizedServerConfig::new("npx");
        if let ServerTransport::Stdio { args, env, .. } = &mut cfg.transport {
            *args = vec!["-y".into(), "pkg".into()];
            *env = BTreeMap::from([("API_KEY".to_string(), "a b".to_string())]);
        }
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

    #[test]
    fn test_build_command_failed_error_redacts_stderr_secrets() {
        let inv = CliInvocation::new(
            "claude",
            vec![
                "mcp".into(),
                "add".into(),
                "-e".into(),
                "API_KEY=super-secret-env".into(),
                "-e".into(),
                "STAR_KEY=alpha***omega".into(),
                "-H".into(),
                "Authorization: Bearer super-secret-token".into(),
                "-H".into(),
                "X-Star: Bearer beta***gamma".into(),
            ],
            false,
        );

        let cases = [
            (
                "env value leaked in stderr",
                b"error: invalid K/V API_KEY=super-secret-env (super-secret-env)".as_slice(),
                "API_KEY=***",
                &["super-secret-env"][..],
            ),
            (
                "env value containing *** leaked in stderr",
                b"error: STAR_KEY=alpha***omega (alpha***omega)".as_slice(),
                "STAR_KEY=*** (***)",
                &["alpha***omega"][..],
            ),
            (
                "header bearer token leaked in stderr",
                b"http 401 for Authorization: Bearer super-secret-token (super-secret-token)"
                    .as_slice(),
                "Authorization: ***",
                &["super-secret-token"][..],
            ),
            (
                "header bearer token containing *** leaked in stderr",
                b"http 401 for X-Star: Bearer beta***gamma (beta***gamma)".as_slice(),
                "X-Star: *** (***)",
                &["beta***gamma"][..],
            ),
            (
                "empty stderr falls back to status text",
                b"   \n".as_slice(),
                "exit status: 1",
                &[
                    "super-secret-env",
                    "super-secret-token",
                    "alpha***omega",
                    "beta***gamma",
                ][..],
            ),
        ];

        for (name, stderr_bytes, expected_substring, forbidden) in cases {
            let err = build_command_failed_error(&inv, "exit status: 1", stderr_bytes);
            let rendered = err.to_string();
            assert!(rendered.contains(expected_substring), "{name}: {rendered}");
            for secret in forbidden {
                assert!(
                    !rendered.contains(secret),
                    "{name}: leaked {secret} in {rendered}"
                );
            }
        }
    }
}
