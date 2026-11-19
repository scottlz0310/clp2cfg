//! Builds the official-CLI commands that register servers, and renders them for display.
//! Pure: nothing here runs a process (see `exec`).

use crate::delegate::CliError;
use crate::domain::{
    CliSpec, ClientTargetKind, McpServersMap, NormalizedServerConfig, RemoteCliStyle,
    RemoteTransportKind, ServerTransport,
};

/// Kind of CLI operation represented by a [`CliInvocation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOperation {
    Remove,
    Add,
}

/// A single subprocess invocation of a client's official CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInvocation {
    /// Target MCP server name this invocation acts on.
    pub server_name: String,
    /// Whether this command removes or adds the server.
    pub operation: CliOperation,
    pub program: String,
    pub args: Vec<String>,
    /// Failure is tolerated (e.g. `remove` of a server that does not exist yet).
    pub ignore_failure: bool,
}

/// Minimum character count for standalone secret values to avoid redacting common short
/// tokens like `"1"` or `"ok"` in CLI error output; `KEY=VALUE` / `Header: Value` pairs
/// are always redacted regardless of value length.
const MIN_STANDALONE_SECRET_LEN: usize = 4;

#[derive(Clone, Copy)]
enum SecretArg {
    None,
    Env,
    Header,
}

impl SecretArg {
    fn from_flag(arg: &str) -> Self {
        match arg {
            "-e" | "--env" => Self::Env,
            "-H" | "--header" => Self::Header,
            _ => Self::None,
        }
    }

    fn redact_arg(self, arg: &str) -> String {
        match self {
            Self::Env => match arg.split_once('=') {
                Some((key, _)) => format!("{key}=***"),
                None => "***".to_string(),
            },
            Self::Header => match arg.split_once(':') {
                Some((key, _)) => format!("{}: ***", key.trim_end()),
                None => "***".to_string(),
            },
            Self::None => arg.to_string(),
        }
    }
}

impl CliInvocation {
    pub(super) fn new(
        server_name: &str,
        operation: CliOperation,
        program: &str,
        args: Vec<String>,
        ignore_failure: bool,
    ) -> Self {
        Self {
            server_name: server_name.to_string(),
            operation,
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

    /// Replaces secret values passed via `-e` / `--env` and `-H` / `--header` in `text`
    /// (such as subprocess `stderr`) with `***`.
    pub fn redact_text(&self, text: &str) -> String {
        let rules = self.secret_redaction_rules();
        if rules.is_empty() || text.is_empty() {
            return text.to_string();
        }
        // Single-pass left-to-right longest match so already-redacted `***` spans are never
        // re-matched and real secrets containing `***` are still redacted safely.
        let mut redacted = String::with_capacity(text.len());
        let mut pos = 0;
        while pos < text.len() {
            let rest = &text[pos..];
            if let Some((target, replacement)) =
                rules.iter().find(|(target, _)| rest.starts_with(target))
            {
                redacted.push_str(replacement);
                pos += target.len();
            } else if let Some(ch) = rest.chars().next() {
                redacted.push(ch);
                pos += ch.len_utf8();
            } else {
                break;
            }
        }
        redacted
    }

    fn arg_entries(&self) -> impl Iterator<Item = (&str, SecretArg)> {
        let mut after = SecretArg::None;
        let mut past_double_dash = false;
        self.args.iter().map(move |arg| {
            let kind = if past_double_dash {
                SecretArg::None
            } else {
                after
            };

            if past_double_dash {
                after = SecretArg::None;
            } else if arg == "--" {
                past_double_dash = true;
                after = SecretArg::None;
            } else {
                after = SecretArg::from_flag(arg);
            }

            (arg.as_str(), kind)
        })
    }

    fn secret_redaction_rules(&self) -> Vec<(String, String)> {
        let mut rules = Vec::new();
        for (arg, kind) in self.arg_entries() {
            collect_secret_rules(kind, arg, &mut rules);
        }
        // Sort longest-first so longest match wins at each byte offset.
        rules.retain(|(target, _)| !target.is_empty());
        rules.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        rules.dedup_by(|a, b| a.0 == b.0);
        rules
    }

    fn render(&self, redact_secrets: bool) -> String {
        let mut parts = vec![shell_quote(&self.program)];
        for (arg, secret_kind) in self.arg_entries() {
            let value = if redact_secrets && !matches!(secret_kind, SecretArg::None) {
                secret_kind.redact_arg(arg)
            } else {
                arg.to_string()
            };
            parts.push(shell_quote(&value));
        }
        parts.join(" ")
    }
}

fn push_standalone_secret(value: &str, rules: &mut Vec<(String, String)>) {
    if value.chars().count() >= MIN_STANDALONE_SECRET_LEN {
        rules.push((value.to_string(), "***".to_string()));
    }
}

fn collect_secret_rules(kind: SecretArg, arg: &str, rules: &mut Vec<(String, String)>) {
    match kind {
        SecretArg::None => {}
        SecretArg::Env => match arg.split_once('=') {
            Some((key, val)) if !val.is_empty() => {
                rules.push((arg.to_string(), format!("{key}=***")));
                push_standalone_secret(val, rules);
            }
            Some(_) => {}
            None => push_standalone_secret(arg, rules),
        },
        SecretArg::Header => match arg.split_once(':') {
            Some((key, raw_val)) => {
                let val = raw_val.trim();
                if val.is_empty() {
                    return;
                }
                let masked_pair = format!("{}: ***", key.trim_end());
                rules.push((arg.to_string(), masked_pair.clone()));
                let normalized_pair = format!("{}: {val}", key.trim_end());
                if normalized_pair != arg {
                    rules.push((normalized_pair, masked_pair));
                }
                push_standalone_secret(val, rules);
                if let Some((_, rest)) = val.split_once(char::is_whitespace) {
                    let credential = rest.trim_start();
                    if !credential.is_empty() && !credential.contains(char::is_whitespace) {
                        push_standalone_secret(credential, rules);
                    }
                }
            }
            None => push_standalone_secret(arg, rules),
        },
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
            invocations.push(CliInvocation::new(
                name,
                CliOperation::Remove,
                program,
                remove_args(spec, name),
                true,
            ));
        }
        invocations.push(CliInvocation::new(
            name,
            CliOperation::Add,
            program,
            add,
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
    match RemoteTransportKind::parse(transport_type) {
        Some(RemoteTransportKind::Http { .. }) => Ok("http"),
        Some(RemoteTransportKind::Sse) => Ok("sse"),
        None => Err(CliError::UnsupportedServer {
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
        assert_eq!(inv[0].server_name, "my-srv");
        assert_eq!(inv[0].operation, CliOperation::Remove);
        assert!(inv[0].ignore_failure);
        assert_eq!(inv[0].display(), "claude mcp remove --scope user my-srv");
        assert_eq!(inv[1].server_name, "my-srv");
        assert_eq!(inv[1].operation, CliOperation::Add);
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
            let inv = CliInvocation::new(
                "n",
                CliOperation::Add,
                "p",
                args.iter().map(|a| a.to_string()).collect(),
                false,
            );
            assert_eq!(inv.display(), plain);
            assert_eq!(inv.display_redacted(), redacted);
        }
    }

    #[test]
    fn test_display_redacted_masks_headers_too() {
        let inv = CliInvocation::new(
            "api",
            CliOperation::Add,
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

    #[test]
    fn test_redact_text_masks_secrets_safely() {
        let inv = CliInvocation::new(
            "api",
            CliOperation::Add,
            "claude",
            vec![
                "mcp".into(),
                "add".into(),
                "-e".into(),
                "API_KEY=secret-value-123".into(),
                "--env".into(),
                "SHORT=ab".into(),
                "-e".into(),
                "SUB=secret-value".into(),
                "-e".into(),
                "EMPTY=".into(),
                "-H".into(),
                "Authorization: Bearer bearer-token-999".into(),
                "-e".into(),
                "ASTERISK_KEY=alpha***omega".into(),
                "-H".into(),
                "X-Api-Key: Bearer beta***gamma".into(),
                "--".into(),
                "node".into(),
                "ARG=post-double-dash-ok".into(),
            ],
            false,
        );

        let cases = [
            (
                "pair env leaked",
                "failed: API_KEY=secret-value-123",
                "failed: API_KEY=***",
            ),
            (
                "overlapping longer secret masked before shorter substring",
                "bad value secret-value-123 and secret-value",
                "bad value *** and ***",
            ),
            (
                "short value masked in pair form only, standalone not corrupted",
                "line ab failed with SHORT=ab",
                "line ab failed with SHORT=***",
            ),
            (
                "header full pair, header value, and bearer credential",
                "rejected Authorization: Bearer bearer-token-999 (token bearer-token-999)",
                "rejected Authorization: *** (token ***)",
            ),
            (
                "env secret containing *** is redacted in pair and standalone forms",
                "error: ASTERISK_KEY=alpha***omega (alpha***omega)",
                "error: ASTERISK_KEY=*** (***)",
            ),
            (
                "header secret containing *** is redacted in pair, value, and credential forms",
                "error: X-Api-Key: Bearer beta***gamma (Bearer beta***gamma / beta***gamma)",
                "error: X-Api-Key: *** (*** / ***)",
            ),
            (
                "post-double-dash args are not treated as secrets",
                "running ARG=post-double-dash-ok failed with code 1",
                "running ARG=post-double-dash-ok failed with code 1",
            ),
            ("empty stderr remains empty", "", ""),
        ];

        for (name, input, expected) in cases {
            assert_eq!(inv.redact_text(input), expected, "{name}");
        }
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
