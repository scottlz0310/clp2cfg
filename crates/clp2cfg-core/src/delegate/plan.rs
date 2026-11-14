//! Builds the official-CLI commands that register servers, and renders them for display.
//! Pure: nothing here runs a process (see `exec`).

use crate::delegate::CliError;
use crate::domain::{
    CliSpec, ClientTargetKind, McpServersMap, NormalizedServerConfig, RemoteCliStyle,
    ServerTransport,
};

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

    /// Same as [`display`](Self::display) but with `-e` / `--env` and `-H` / `--header` values
    /// replaced by `***`, so secrets never reach status messages or logs.
    pub fn display_redacted(&self) -> String {
        self.render(true)
    }

    fn render(&self, redact_secrets: bool) -> String {
        #[derive(Clone, Copy)]
        enum SecretArg {
            None,
            Env,
            Header,
        }

        let mut after = SecretArg::None;
        let mut past_double_dash = false;
        let mut parts = vec![shell_quote(&self.program)];
        for arg in &self.args {
            let value = if redact_secrets && !past_double_dash {
                match after {
                    SecretArg::Env => match arg.split_once('=') {
                        Some((key, _)) => format!("{key}=***"),
                        None => "***".to_string(),
                    },
                    SecretArg::Header => match arg.split_once(':') {
                        Some((key, _)) => format!("{}: ***", key.trim_end()),
                        None => "***".to_string(),
                    },
                    SecretArg::None => arg.clone(),
                }
            } else {
                arg.clone()
            };

            if past_double_dash {
                after = SecretArg::None;
            } else if arg == "--" {
                past_double_dash = true;
                after = SecretArg::None;
            } else {
                after = match arg.as_str() {
                    "-e" | "--env" => SecretArg::Env,
                    "-H" | "--header" => SecretArg::Header,
                    _ => SecretArg::None,
                };
            }
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
/// Extra properties outside the normalized transport fields cannot be expressed through the CLIs.
pub fn build_cli_invocations(
    kind: ClientTargetKind,
    program: &str,
    servers: &McpServersMap,
) -> Result<Vec<CliInvocation>, CliError> {
    let spec = kind.cli_spec().ok_or(CliError::NotCliTarget)?;

    let mut invocations = Vec::new();
    for (name, config) in servers {
        let add = add_args(spec, name, config)?;
        if spec.remove_before_add {
            invocations.push(CliInvocation::new(program, remove_args(spec, name), true));
        }
        invocations.push(CliInvocation::new(program, add, false));
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

fn add_args(
    spec: &CliSpec,
    name: &str,
    config: &NormalizedServerConfig,
) -> Result<Vec<String>, CliError> {
    let mut args: Vec<String> = vec!["mcp".into(), "add".into()];
    if let Some(scope) = spec.scope {
        args.extend(["--scope".into(), scope.into()]);
    }

    match &config.transport {
        ServerTransport::Stdio {
            command,
            args: cmd_args,
            env,
        } => {
            let env_args = env
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
            args.push(command.clone());
            args.extend(cmd_args.iter().cloned());
            Ok(args)
        }
        ServerTransport::Remote {
            transport_type,
            url,
            headers,
        } => {
            let transport = resolve_remote_transport(spec, name, transport_type.as_deref())?;
            match spec.remote {
                RemoteCliStyle::TransportAndPositionalUrl {
                    transport_flag,
                    supports_sse,
                } => {
                    if transport == "sse" && !supports_sse {
                        return Err(CliError::UnsupportedServer {
                            program: spec.program.to_string(),
                            server: name.to_string(),
                            reason: "SSE transport".to_string(),
                        });
                    }
                    args.extend([transport_flag.to_string(), transport.to_string()]);
                    let header_args: Vec<String> = if headers.is_empty() {
                        Vec::new()
                    } else if let Some(flag) = spec.header_flag {
                        headers
                            .iter()
                            .flat_map(|(k, v)| [flag.to_string(), format!("{k}: {v}")])
                            .collect()
                    } else {
                        return Err(CliError::UnsupportedServer {
                            program: spec.program.to_string(),
                            server: name.to_string(),
                            reason: "HTTP headers".to_string(),
                        });
                    };

                    if spec.headers_before_name {
                        args.extend(header_args);
                        args.push(name.to_string());
                        args.push(url.clone());
                    } else {
                        args.push(name.to_string());
                        args.push(url.clone());
                        args.extend(header_args);
                    }
                    Ok(args)
                }
                RemoteCliStyle::UrlFlag => {
                    if transport == "sse" {
                        return Err(CliError::UnsupportedServer {
                            program: spec.program.to_string(),
                            server: name.to_string(),
                            reason: "SSE transport".to_string(),
                        });
                    }
                    if !headers.is_empty() {
                        return Err(CliError::UnsupportedServer {
                            program: spec.program.to_string(),
                            server: name.to_string(),
                            reason: "HTTP headers".to_string(),
                        });
                    }
                    args.push(name.to_string());
                    args.push("--url".into());
                    args.push(url.clone());
                    Ok(args)
                }
            }
        }
    }
}

fn resolve_remote_transport(
    spec: &CliSpec,
    server: &str,
    transport_type: Option<&str>,
) -> Result<&'static str, CliError> {
    match transport_type.map(str::to_ascii_lowercase).as_deref() {
        None | Some("http" | "streamable-http" | "streamablehttp") => Ok("http"),
        Some("sse") => Ok("sse"),
        Some(_) => Err(CliError::UnsupportedServer {
            program: spec.program.to_string(),
            server: server.to_string(),
            reason: "this transport type".to_string(),
        }),
    }
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
        if let ServerTransport::Stdio { args, env, .. } = &mut cfg.transport {
            *args = vec!["-y".into(), "pkg".into()];
            *env = BTreeMap::from([("API_KEY".to_string(), "a b".to_string())]);
        }
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

    #[test]
    fn test_display_redacted_masks_headers_too() {
        let inv = CliInvocation::new(
            "claude",
            vec![
                "mcp".into(),
                "add".into(),
                "--transport".into(),
                "http".into(),
                "api".into(),
                "https://example.com/mcp".into(),
                "-H".into(),
                "Authorization: Bearer top-secret".into(),
            ],
            false,
        );
        assert_eq!(
            inv.display_redacted(),
            "claude mcp add --transport http api https://example.com/mcp -H 'Authorization: ***'"
        );
    }

    fn remote_servers(transport: Option<&str>, headers: &[(&str, &str)]) -> McpServersMap {
        let mut cfg = NormalizedServerConfig::remote("https://mcp.cloudflare.com/mcp");
        if let Some(t) = transport {
            cfg = cfg.with_transport_type(t);
        }
        *cfg.headers_mut().unwrap() = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        McpServersMap::from([("cloudflare-api".to_string(), cfg)])
    }

    #[test]
    fn test_build_remote_http_commands_for_each_cli() {
        let plain = remote_servers(Some("http"), &[]);
        let with_headers = remote_servers(Some("http"), &[("Authorization", "Bearer tok")]);

        let claude =
            build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &with_headers).unwrap();
        assert_eq!(
            claude[0].display(),
            "claude mcp remove --scope user cloudflare-api"
        );
        assert_eq!(
            claude[1].display(),
            "claude mcp add --scope user --transport http cloudflare-api https://mcp.cloudflare.com/mcp -H 'Authorization: Bearer tok'"
        );

        let copilot =
            build_cli_invocations(ClientTargetKind::CopilotCli, "copilot", &with_headers).unwrap();
        assert_eq!(
            copilot[1].display(),
            "copilot mcp add --transport http --header 'Authorization: Bearer tok' cloudflare-api https://mcp.cloudflare.com/mcp"
        );

        let agy =
            build_cli_invocations(ClientTargetKind::AntigravityCli, "agy", &with_headers).unwrap();
        assert_eq!(
            agy[0].display(),
            "agy mcp add --type http --header 'Authorization: Bearer tok' cloudflare-api https://mcp.cloudflare.com/mcp"
        );

        let codex = build_cli_invocations(ClientTargetKind::CodexCli, "codex", &plain).unwrap();
        assert_eq!(
            codex[0].display(),
            "codex mcp add cloudflare-api --url https://mcp.cloudflare.com/mcp"
        );
    }

    #[test]
    fn test_build_remote_rejects_unsupported_combinations() {
        let with_headers = remote_servers(Some("http"), &[("Authorization", "Bearer tok")]);
        let sse = remote_servers(Some("sse"), &[]);
        let unknown = remote_servers(Some("websocket"), &[]);

        assert!(matches!(
            build_cli_invocations(ClientTargetKind::CodexCli, "codex", &with_headers),
            Err(CliError::UnsupportedServer { .. })
        ));
        assert!(matches!(
            build_cli_invocations(ClientTargetKind::CodexCli, "codex", &sse),
            Err(CliError::UnsupportedServer { .. })
        ));
        assert!(matches!(
            build_cli_invocations(ClientTargetKind::AntigravityCli, "agy", &sse),
            Err(CliError::UnsupportedServer { .. })
        ));
        assert!(matches!(
            build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &unknown),
            Err(CliError::UnsupportedServer { .. })
        ));

        // claude & copilot support sse
        let claude_sse =
            build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &sse).unwrap();
        assert!(claude_sse[1].display().contains("--transport sse"));
    }
}
