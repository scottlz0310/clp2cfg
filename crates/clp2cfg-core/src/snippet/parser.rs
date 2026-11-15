//! Loose JSON parser for MCP server snippets.

use crate::domain::{McpServersMap, NormalizedServerConfig, ServerTransport};
use serde_json::Value;
use thiserror::Error;

/// Error types occurring during snippet parsing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("Empty input provided")]
    EmptyInput,

    #[error("No valid JSON structure found in input")]
    NoJsonFound,

    #[error("Failed to parse JSON/JSON5 snippet: {0}")]
    SyntaxError(String),

    #[error("Invalid MCP configuration format: {0}")]
    InvalidFormat(String),
}

/// Extracts JSON substring from raw text, removing Markdown code fences and external text.
pub fn extract_json_candidate(input: &str) -> Result<&str, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::EmptyInput);
    }

    // 1. Try extracting from markdown code fence (```json ... ``` or just ``` ... ```)
    if let Some(start_idx) = trimmed.find("```") {
        let after_opening = &trimmed[start_idx + 3..];
        // Skip optional language identifier on the same line
        let content_start = match after_opening.find('\n') {
            Some(newline_pos) => start_idx + 3 + newline_pos + 1,
            None => start_idx + 3,
        };

        if let Some(closing_idx) = trimmed[content_start..].rfind("```") {
            let candidate = trimmed[content_start..content_start + closing_idx].trim();
            if !candidate.is_empty() {
                return Ok(candidate);
            }
        }
    }

    // 2. Look for outer JSON object delimiters '{' and '}'
    if let (Some(first_brace), Some(last_brace)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if first_brace <= last_brace {
            return Ok(&trimmed[first_brace..=last_brace]);
        }
    }

    Err(ParseError::NoJsonFound)
}

/// Parse arbitrary MCP snippet strings into a normalized servers map.
pub fn parse_mcp_snippet(input: &str) -> Result<McpServersMap, ParseError> {
    let candidate = extract_json_candidate(input)?;

    // Parse with serde_json first, fallback to json5
    let value: Value = serde_json::from_str(candidate).or_else(|_| {
        json5::from_str(candidate).map_err(|e| ParseError::SyntaxError(describe_json5_error(&e)))
    })?;

    normalize_parsed_value(value)
}

/// json5 のエラー文には入力の抜粋が含まれ得るため、種別と位置だけを返す。
/// 入力の値（API キー等）を、画面・標準エラー・ログへ残さないため。
fn describe_json5_error(e: &json5::Error) -> String {
    let location = e.position().map(|p| format!(" at {p}")).unwrap_or_default();
    match e.code() {
        Some(code) => format!("{code}{location}"),
        None => format!("invalid JSON5{location}"),
    }
}

/// serde のデータエラーは入力値を含まない固定文言にする。
fn describe_config_error(e: &serde_json::Error) -> String {
    let message = e.to_string();
    // `NormalizedServerConfig::deserialize` のカスタムエラーは ` at line ...` 等が付かないためそのまま使える。
    // 念のため、既知の固定メッセージ接頭辞以外は汎用メッセージに丸めて秘密値の露出を防ぐ。
    if message.starts_with("missing field ")
        || message.starts_with("invalid field type ")
        || message.starts_with("conflicting fields:")
        || message == "server config must be an object"
    {
        message
    } else {
        "invalid server configuration".to_string()
    }
}

/// Normalizes various JSON AST structures into an McpServersMap.
fn normalize_parsed_value(value: Value) -> Result<McpServersMap, ParseError> {
    let obj = match value {
        Value::Object(map) => map,
        _ => {
            return Err(ParseError::InvalidFormat(
                "Top-level value must be an object".to_string(),
            ))
        }
    };

    // Case 1: Wrapped in "mcpServers"
    if let Some(servers) = obj.get("mcpServers") {
        return parse_servers_object(servers);
    }

    // Case 2: Wrapped in "mcp" (Codex style)
    if let Some(servers) = obj.get("mcp") {
        return parse_servers_object(servers);
    }

    // Case 3: Single server definition at top-level ("command" or "url" / "serverUrl")
    if is_single_server_object(&obj) {
        let config: NormalizedServerConfig =
            serde_json::from_value(Value::Object(obj)).map_err(|e| {
                ParseError::InvalidFormat(format!(
                    "Failed to parse server config: {}",
                    describe_config_error(&e)
                ))
            })?;
        let name = infer_server_name(&config);
        let mut map = McpServersMap::new();
        map.insert(name, config);
        return Ok(map);
    }

    // Case 4: Map of server names to server definitions
    parse_servers_object(&Value::Object(obj))
}

fn is_single_server_object(obj: &serde_json::Map<String, Value>) -> bool {
    ["command", "url", "serverUrl"]
        .iter()
        .any(|key| obj.get(*key).is_some_and(|v| !v.is_object()))
}

/// Parses an object whose values are individual server definitions.
fn parse_servers_object(val: &Value) -> Result<McpServersMap, ParseError> {
    let map = match val {
        Value::Object(map) => map,
        _ => {
            return Err(ParseError::InvalidFormat(
                "Expected an object containing servers".to_string(),
            ))
        }
    };

    let mut result = McpServersMap::new();
    for (name, server_val) in map {
        let config: NormalizedServerConfig =
            serde_json::from_value(server_val.clone()).map_err(|e| {
                ParseError::InvalidFormat(format!(
                    "Server '{}' is not a valid server config: {}",
                    name,
                    describe_config_error(&e)
                ))
            })?;
        result.insert(name.clone(), config);
    }

    if result.is_empty() {
        return Err(ParseError::InvalidFormat(
            "No servers found in snippet".to_string(),
        ));
    }

    Ok(result)
}

/// Automatically infers a reasonable server name from a server configuration.
pub fn infer_server_name(config: &NormalizedServerConfig) -> String {
    match &config.transport {
        ServerTransport::Stdio { command, args, .. } => infer_stdio_server_name(command, args),
        ServerTransport::Remote { url, .. } => infer_remote_server_name(url),
    }
}

fn infer_stdio_server_name(cmd: &str, args: &[String]) -> String {
    let cmd_stem = command_stem(cmd);
    let runner = is_package_runner(cmd_stem);

    if runner {
        // Look for the package name in arguments (skip flags like -y, --yes)
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            // Candidate package name
            let name = extract_package_base_name(arg);
            if !name.is_empty() {
                return name;
            }
        }
    }

    if cmd_stem.eq_ignore_ascii_case("docker") {
        // Find image name in docker run args
        if let Some(pos) = args.iter().position(|a| a == "run") {
            for arg in &args[pos + 1..] {
                if !arg.starts_with('-') && !arg.contains('=') {
                    let image_base = extract_package_base_name(arg);
                    if !image_base.is_empty() {
                        return image_base;
                    }
                }
            }
        }
    }

    if !cmd_stem.is_empty() && !runner {
        return strip_redundant_mcp_affixes(cmd_stem).to_string();
    }

    "mcp-server".to_string()
}

/// Extracts the command file stem regardless of Windows/Unix path separators.
fn command_stem(cmd: &str) -> &str {
    let file_name = cmd.rsplit(['/', '\\']).next().unwrap_or(cmd);
    std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name)
}

fn is_package_runner(cmd_stem: &str) -> bool {
    matches!(
        cmd_stem.to_ascii_lowercase().as_str(),
        "npx" | "bunx" | "uvx" | "pnpx" | "yarn"
    )
}

fn infer_remote_server_name(url: &str) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = without_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_port.starts_with('[') {
        host_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };

    let without_prefix = host
        .strip_prefix("mcp.")
        .or_else(|| host.strip_prefix("www."))
        .filter(|rest| rest.contains('.'))
        .unwrap_or(host);

    let is_ipv4 = !without_prefix.is_empty()
        && without_prefix
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.');
    let without_tld = if !is_ipv4 && !host_port.starts_with('[') {
        match without_prefix.rsplit_once('.') {
            Some((head, tld))
                if !head.is_empty() && tld.chars().any(|c| c.is_ascii_alphabetic()) =>
            {
                head
            }
            _ => without_prefix,
        }
    } else {
        without_prefix
    };

    let sanitized: String = without_tld
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');
    if trimmed.is_empty() {
        "mcp-server".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Strips npm/docker scope prefixes and redundant "mcp" affixes (both prefix and postfix).
fn extract_package_base_name(pkg: &str) -> String {
    // 1. Separate npm scope (e.g., @playwright/mcp -> scope="playwright", rest="mcp")
    //    or registry/org path (e.g., ghcr.io/org/tool-mcp -> rest="tool-mcp").
    let (npm_scope, without_scope) = match pkg.rsplit_once(['/', '\\']) {
        Some((prefix, rest)) => {
            let scope = prefix
                .strip_prefix('@')
                .filter(|s| !s.is_empty() && !s.contains(['/', '\\']));
            (scope, rest)
        }
        None => (None, pkg),
    };

    // 2. Remove version or container tag (e.g., server-filesystem@latest, tool-mcp:latest)
    let without_version = match without_scope.find('@') {
        Some(at_idx) => &without_scope[..at_idx],
        None => match without_scope.rsplit_once(':') {
            // Avoid splitting Windows drive letters like "C:"
            Some((head, _)) if head.len() > 1 => head,
            _ => without_scope,
        },
    };

    // 3. Remove redundant MCP prefixes and postfixes (leaving non-empty core name)
    let clean_name = strip_redundant_mcp_affixes(without_version);

    // If the package basename itself is generic ("mcp" / "mcp-server") under an npm scope
    // such as "@playwright/mcp@latest", prefer the normalized scope name ("playwright").
    if is_generic_mcp_name(clean_name) {
        if let Some(scope) = npm_scope {
            let clean_scope = strip_redundant_mcp_affixes(scope);
            if !clean_scope.is_empty() {
                return clean_scope.to_string();
            }
        }
    }

    if clean_name.is_empty() {
        without_version.to_string()
    } else {
        clean_name.to_string()
    }
}

fn is_generic_mcp_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "mcp" | "mcp-server" | "mcp_server" | "server-mcp" | "server_mcp"
    )
}

/// Strips redundant `mcp-` / `mcp-server-` prefixes and `-mcp` / `-mcp-server` postfixes
/// without reducing generic names (`mcp`, `mcp-server`) to bare `server` or an empty string.
fn strip_redundant_mcp_affixes(name: &str) -> &str {
    const PREFIXES: &[&str] = &["mcp-server-", "mcp_server_", "mcp-", "mcp_"];
    const SUFFIXES: &[&str] = &[
        "-mcp-server",
        "_mcp_server",
        "-server-mcp",
        "_server_mcp",
        "-mcp",
        "_mcp",
    ];

    if is_generic_mcp_name(name) {
        return name;
    }

    let mut current = name;
    for prefix in PREFIXES {
        if current.len() > prefix.len()
            && current
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            current = &current[prefix.len()..];
            break;
        }
    }

    if is_generic_mcp_name(current) {
        return current;
    }

    for suffix in SUFFIXES {
        if current.len() > suffix.len()
            && current
                .get(current.len() - suffix.len()..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            current = &current[..current.len() - suffix.len()];
            break;
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_candidate_markdown() {
        let input = "Here is the config:\n```json\n{\n  \"mcpServers\": {}\n}\n```\nEnjoy!";
        let extracted = extract_json_candidate(input).unwrap();
        assert_eq!(extracted, "{\n  \"mcpServers\": {}\n}");
    }

    #[test]
    fn test_extract_json_candidate_surrounding_text() {
        let input = "Configure it with: { \"command\": \"git\" } in your settings.";
        let extracted = extract_json_candidate(input).unwrap();
        assert_eq!(extracted, "{ \"command\": \"git\" }");
    }

    #[test]
    fn test_parse_wrapped_mcp_servers() {
        let snippet = r#"{
            "mcpServers": {
                "sqlite": {
                    "command": "uvx",
                    "args": ["mcp-server-sqlite", "--db-path", "test.db"]
                }
            }
        }"#;

        let parsed = parse_mcp_snippet(snippet).unwrap();
        assert_eq!(parsed.len(), 1);
        let server = parsed.get("sqlite").unwrap();
        assert_eq!(server.command(), Some("uvx"));
        assert_eq!(server.args(), ["mcp-server-sqlite", "--db-path", "test.db"]);
    }

    #[test]
    fn test_parse_codex_style_mcp() {
        let snippet = r#"{
            "mcp": {
                "fetch": {
                    "command": "uvx",
                    "args": ["mcp-server-fetch"]
                }
            }
        }"#;

        let parsed = parse_mcp_snippet(snippet).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parsed.contains_key("fetch"));
    }

    #[test]
    fn test_parse_single_unnamed_server_with_inferred_name() {
        let snippet = r#"{
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\data"]
        }"#;

        let parsed = parse_mcp_snippet(snippet).unwrap();
        assert_eq!(parsed.len(), 1);
        let (name, server) = parsed.into_iter().next().unwrap();
        assert_eq!(name, "server-filesystem");
        assert_eq!(server.command(), Some("npx"));
    }

    #[test]
    fn test_parse_json5_with_comments_and_trailing_commas() {
        let snippet = r#"{
            // Configuration for Memory MCP
            "mcpServers": {
                "memory": {
                    "command": "npx",
                    args: ["-y", "@modelcontextprotocol/server-memory",],
                    /* extra property */
                    type: 'stdio',
                },
            }
        }"#;

        let parsed = parse_mcp_snippet(snippet).unwrap();
        assert_eq!(parsed.len(), 1);
        let server = parsed.get("memory").unwrap();
        assert_eq!(server.command(), Some("npx"));
        assert_eq!(server.extra.get("type").unwrap(), "stdio");
    }

    #[test]
    fn test_preserve_extra_fields() {
        let snippet = r#"{
            "command": "uvx",
            "args": ["server-test"],
            "disabled": false,
            "autoApprove": ["read"]
        }"#;

        let parsed = parse_mcp_snippet(snippet).unwrap();
        let server = parsed.values().next().unwrap();
        assert_eq!(server.extra.get("disabled").unwrap(), false);
        assert!(server.extra.contains_key("autoApprove"));
    }

    #[test]
    fn test_parse_invalid_input() {
        assert_eq!(parse_mcp_snippet(""), Err(ParseError::EmptyInput));
        assert_eq!(
            parse_mcp_snippet("not a json"),
            Err(ParseError::NoJsonFound)
        );
        assert_eq!(
            parse_mcp_snippet("{ without closing"),
            Err(ParseError::NoJsonFound)
        );
        assert!(matches!(
            parse_mcp_snippet("{ invalid: }"),
            Err(ParseError::SyntaxError(_))
        ));
    }

    #[test]
    fn test_parse_errors_never_contain_input_values() {
        struct Case {
            name: &'static str,
            input: &'static str,
            secret: &'static str,
            /// エラー文に残っていてほしい手がかり
            hint: &'static str,
        }

        let cases = [
            Case {
                name: "unquoted number as env value (map of servers)",
                input: r#"{"srv":{"command":"node","env":{"TOKEN":123456789}}}"#,
                secret: "123456789",
                hint: "invalid field type",
            },
            Case {
                name: "unquoted number as env value (single server)",
                input: r#"{"command":"node","env":{"TOKEN":987654321}}"#,
                secret: "987654321",
                hint: "invalid field type",
            },
            Case {
                name: "string where an args array is expected",
                input: r#"{"srv":{"command":"node","args":"sk-secret-args"}}"#,
                secret: "sk-secret-args",
                hint: "invalid field type",
            },
            Case {
                name: "missing command keeps only the field name",
                input: r#"{"srv":{"args":["sk-secret-args"]}}"#,
                secret: "sk-secret-args",
                hint: "missing field `command`",
            },
            Case {
                name: "json5 syntax error does not echo the source line",
                input: "{srv: {command: 'node', env: {TOKEN: 'sk-secret-xyz' oops}}}",
                secret: "sk-secret-xyz",
                hint: "line 1",
            },
            Case {
                name: "invalid headers value does not echo the secret",
                input: r#"{"srv":{"type":"http","url":"https://example.com","headers":{"Authorization":99887766}}}"#,
                secret: "99887766",
                hint: "invalid field type",
            },
        ];

        for case in cases {
            let err = parse_mcp_snippet(case.input).unwrap_err().to_string();
            assert!(!err.contains(case.secret), "{}: {err}", case.name);
            assert!(err.contains(case.hint), "{}: {err}", case.name);
        }
    }

    #[test]
    fn test_parse_remote_mcp_servers() {
        let cases = [
            (
                "http with type and url",
                r#"{"mcpServers":{"cloudflare-api":{"type":"http","url":"https://mcp.cloudflare.com/mcp"}}}"#,
                "cloudflare-api",
                Some("http"),
                "https://mcp.cloudflare.com/mcp",
            ),
            (
                "url only without type",
                r#"{"cursor-remote":{"url":"https://example.com/mcp"}}"#,
                "cursor-remote",
                None,
                "https://example.com/mcp",
            ),
            (
                "windsurf serverUrl normalized to url",
                r#"{"mcpServers":{"ws":{"serverUrl":"https://windsurf.example/mcp"}}}"#,
                "ws",
                None,
                "https://windsurf.example/mcp",
            ),
            (
                "sse with headers",
                r#"{"mcp":{"legacy":{"type":"sse","url":"https://example.com/sse","headers":{"Authorization":"Bearer tok"}}}}"#,
                "legacy",
                Some("sse"),
                "https://example.com/sse",
            ),
        ];

        for (label, input, name, expected_type, expected_url) in cases {
            let parsed = parse_mcp_snippet(input).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(parsed.len(), 1, "{label}");
            let cfg = parsed
                .get(name)
                .unwrap_or_else(|| panic!("{label}: missing {name}"));
            assert!(cfg.is_remote(), "{label}");
            assert_eq!(cfg.transport_type(), expected_type, "{label}");
            assert_eq!(cfg.url(), Some(expected_url), "{label}");
        }
    }

    #[test]
    fn test_parse_single_unnamed_remote_server_infers_name() {
        let cases = [
            (
                r#"{"type":"http","url":"https://mcp.cloudflare.com/mcp"}"#,
                "cloudflare",
            ),
            (r#"{"url":"https://mcp.dev/mcp"}"#, "mcp"),
            (r#"{"url":"http://localhost:3000/mcp"}"#, "localhost"),
            (r#"{"serverUrl":"http://127.0.0.1:8080/mcp"}"#, "127-0-0-1"),
            (
                r#"{"url":"https://api.githubcopilot.com/mcp/"}"#,
                "api-githubcopilot",
            ),
        ];

        for (input, expected_name) in cases {
            let parsed = parse_mcp_snippet(input).unwrap();
            assert_eq!(parsed.len(), 1);
            let (name, server) = parsed.into_iter().next().unwrap();
            assert_eq!(name, expected_name, "input={input}");
            assert!(server.is_remote(), "input={input}");
        }
    }

    #[test]
    fn test_server_named_url_in_map_is_not_mistaken_for_single_server() {
        let input = r#"{"url":{"command":"node"}}"#;
        let parsed = parse_mcp_snippet(input).unwrap();
        assert!(parsed["url"].is_stdio());
        assert_eq!(parsed["url"].command(), Some("node"));
    }

    #[test]
    fn test_parse_single_unnamed_stdio_server_normalizes_mcp_affixes() {
        let cases = [
            // Local binaries with Windows / Unix paths and -mcp postfix
            (
                "windows local exe with -mcp postfix",
                r#"{"command":"C:\\Users\\alice\\.bun\\bin\\desktop-touch-mcp.exe","args":[]}"#,
                "desktop-touch",
            ),
            (
                "windows local exe playwright-mcp",
                r#"{"command":"C:\\Users\\alice\\.bun\\bin\\playwright-mcp.exe","args":[]}"#,
                "playwright",
            ),
            (
                "unix local binary with -mcp-server postfix",
                r#"{"command":"/usr/local/bin/foo-mcp-server"}"#,
                "foo",
            ),
            (
                "local binary with -server-mcp postfix",
                r#"{"command":"bar-server-mcp.cmd"}"#,
                "bar",
            ),
            (
                "local binary with mcp-server- prefix and uppercase MCP",
                r#"{"command":"C:\\bin\\MCP-Server-Git.exe"}"#,
                "Git",
            ),
            (
                "local binary with underscore _mcp postfix",
                r#"{"command":"my_tool_mcp.exe"}"#,
                "my_tool",
            ),
            (
                "both prefix and postfix affixes are stripped",
                r#"{"command":"mcp-custom-mcp.exe"}"#,
                "custom",
            ),
            // Package runners (including full Windows path to bunx.exe)
            (
                "full-path bunx.exe runner with scoped -mcp package",
                r#"{"command":"C:\\Users\\alice\\.bun\\bin\\bunx.exe","args":["@harusame64/desktop-touch-mcp"]}"#,
                "desktop-touch",
            ),
            (
                "npx with scoped generic @playwright/mcp@latest",
                r#"{"command":"npx","args":["@playwright/mcp@latest"]}"#,
                "playwright",
            ),
            (
                "npx with scoped generic @playwright/mcp-server",
                r#"{"command":"npx","args":["@playwright/mcp-server"]}"#,
                "playwright",
            ),
            (
                "uvx with mcp-server- prefix",
                r#"{"command":"uvx","args":["mcp-server-sqlite","--db-path","test.db"]}"#,
                "sqlite",
            ),
            (
                "docker run with image tag and -mcp postfix",
                r#"{"command":"docker","args":["run","-i","--rm","ghcr.io/org/my-tool-mcp:latest"]}"#,
                "my-tool",
            ),
            // Edge cases & non-empty guards
            (
                "bare mcp.exe does not reduce to empty",
                r#"{"command":"C:\\bin\\mcp.exe"}"#,
                "mcp",
            ),
            (
                "bare mcp-server.exe does not reduce to empty",
                r#"{"command":"mcp-server.exe"}"#,
                "mcp-server",
            ),
            (
                "existing server- prefix without mcp is preserved",
                r#"{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem"]}"#,
                "server-filesystem",
            ),
            (
                "non-ascii prefix/name with -mcp postfix does not panic on char boundaries",
                r#"{"command":"C:\\tools\\日本語-mcp.exe"}"#,
                "日本語",
            ),
            (
                "non-ascii name without hyphen separator is safe on char boundaries",
                r#"{"command":"あmcp"}"#,
                "あmcp",
            ),
        ];

        for (label, input, expected_name) in cases {
            let parsed = parse_mcp_snippet(input).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(parsed.len(), 1, "{label}");
            let (name, server) = parsed.into_iter().next().unwrap();
            assert_eq!(name, expected_name, "{label}: input={input}");
            assert!(server.is_stdio(), "{label}");
        }
    }
}
