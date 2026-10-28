//! Loose JSON parser for MCP server snippets.

use crate::model::{McpServersMap, NormalizedServerConfig, ParseError};
use serde_json::Value;

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
        json5::from_str(candidate).map_err(|e| ParseError::SyntaxError(e.to_string()))
    })?;

    normalize_parsed_value(value)
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

    // Case 3: Single server definition with "command" at top-level
    if obj.contains_key("command") {
        let config: NormalizedServerConfig =
            serde_json::from_value(Value::Object(obj)).map_err(|e| {
                ParseError::InvalidFormat(format!("Failed to parse server config: {}", e))
            })?;
        let name = infer_server_name(&config);
        let mut map = McpServersMap::new();
        map.insert(name, config);
        return Ok(map);
    }

    // Case 4: Map of server names to server definitions
    parse_servers_object(&Value::Object(obj))
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
                    name, e
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

/// Automatically infers a reasonable server name from a command and its arguments.
pub fn infer_server_name(config: &NormalizedServerConfig) -> String {
    let cmd = config.command.as_str();

    // Check if command is a package runner (npx, bunx, uvx, pnpm)
    let is_runner = matches!(
        cmd,
        "npx" | "bunx" | "uvx" | "pnpx" | "yarn" | "npx.cmd" | "bunx.cmd"
    );

    if is_runner {
        // Look for the package name in arguments (skip flags like -y, --yes)
        for arg in &config.args {
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

    if cmd == "docker" {
        // Find image name in docker run args
        if let Some(pos) = config.args.iter().position(|a| a == "run") {
            for arg in &config.args[pos + 1..] {
                if !arg.starts_with('-') && !arg.contains('=') {
                    let image_base = extract_package_base_name(arg);
                    if !image_base.is_empty() {
                        return image_base;
                    }
                }
            }
        }
    }

    // Fallback to command name without extension or path
    let cmd_name = std::path::Path::new(cmd)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(cmd);

    if !cmd_name.is_empty() && !is_runner {
        return cmd_name.to_string();
    }

    "mcp-server".to_string()
}

/// Strips npm/docker scope prefixes and common prefixes like "mcp-server-".
fn extract_package_base_name(pkg: &str) -> String {
    // 1. Remove npm scope (e.g., @modelcontextprotocol/server-filesystem -> server-filesystem)
    let without_scope = if let Some(slash_idx) = pkg.rfind('/') {
        &pkg[slash_idx + 1..]
    } else {
        pkg
    };

    // 2. Remove version tag (e.g., server-filesystem@latest -> server-filesystem)
    let without_version = if let Some(at_idx) = without_scope.find('@') {
        &without_scope[..at_idx]
    } else {
        without_scope
    };

    // 3. Remove common redundant prefixes if they leave a clear name
    let clean_name = without_version
        .strip_prefix("mcp-server-")
        .or_else(|| without_version.strip_prefix("mcp-"))
        .unwrap_or(without_version);

    if clean_name.is_empty() {
        without_version.to_string()
    } else {
        clean_name.to_string()
    }
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
        assert_eq!(server.command, "uvx");
        assert_eq!(
            server.args,
            vec!["mcp-server-sqlite", "--db-path", "test.db"]
        );
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
        assert_eq!(server.command, "npx");
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
        assert_eq!(server.command, "npx");
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
}
