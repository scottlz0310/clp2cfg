//! AST merger for various AI client configuration formats.

use crate::domain::{ClientTargetKind, McpServersMap, ServerChange};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;

/// Error types occurring during target AST merge.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    #[error("Failed to parse existing configuration JSON: {0}")]
    InvalidExistingJson(String),

    #[error("Configuration root must be a JSON object")]
    RootNotObject,

    #[error("Failed to serialize merged configuration: {0}")]
    SerializationFailed(String),

    #[error("Target is applied via its CLI, not by file merge")]
    NotFileTarget,
}

/// Parse existing config content. Some clients' files are JSONC (e.g. Zed's `settings.json`), so
/// fall back to JSON5 for them (comments are not preserved on rewrite; the `.bak` copy keeps the original).
fn parse_existing(kind: ClientTargetKind, content: &str) -> Result<Value, MergeError> {
    match serde_json::from_str(content) {
        Ok(v) => Ok(v),
        Err(e) if kind.file_spec().is_some_and(|file| file.allows_jsonc) => {
            json5::from_str(content).map_err(|_| MergeError::InvalidExistingJson(e.to_string()))
        }
        Err(e) => Err(MergeError::InvalidExistingJson(e.to_string())),
    }
}

/// The JSON key that holds the MCP servers for a file target.
fn servers_key(kind: ClientTargetKind) -> Result<&'static str, MergeError> {
    kind.file_spec()
        .map(|file| file.servers_key)
        .ok_or(MergeError::NotFileTarget)
}

/// The JSON value written for one server (shared by merging and change detection so they agree).
fn server_value(
    kind: ClientTargetKind,
    config: &crate::domain::NormalizedServerConfig,
) -> Result<Value, MergeError> {
    let mut server_val =
        serde_json::to_value(config).map_err(|e| MergeError::SerializationFailed(e.to_string()))?;

    // Some clients (Claude) require/prefer "type": "stdio"
    if kind.file_spec().is_some_and(|file| file.adds_stdio_type) {
        if let Some(obj) = server_val.as_object_mut() {
            if !obj.contains_key("type") {
                obj.insert("type".to_string(), Value::String("stdio".to_string()));
            }
        }
    }
    Ok(server_val)
}

/// Classify each server in `servers` against the target's existing file content.
pub fn classify_server_changes(
    kind: ClientTargetKind,
    existing_json: Option<&str>,
    servers: &McpServersMap,
) -> Result<BTreeMap<String, ServerChange>, MergeError> {
    let target_key = servers_key(kind)?;
    let root: Value = match existing_json {
        Some(content) if !content.trim().is_empty() => parse_existing(kind, content)?,
        _ => Value::Object(Map::new()),
    };
    let root_map = root.as_object().ok_or(MergeError::RootNotObject)?;
    let existing_servers = match root_map.get(target_key) {
        None => None,
        Some(value) => Some(value.as_object().ok_or_else(|| {
            MergeError::InvalidExistingJson(format!(
                "'{}' field exists but is not an object",
                target_key
            ))
        })?),
    };

    servers
        .iter()
        .map(|(name, config)| {
            let change = match existing_servers.and_then(|m| m.get(name)) {
                None => ServerChange::New,
                Some(existing) if *existing == server_value(kind, config)? => {
                    ServerChange::Unchanged
                }
                Some(_) => ServerChange::Updated,
            };
            Ok((name.clone(), change))
        })
        .collect()
}

/// Merges MCP servers into an existing configuration JSON according to target semantics.
pub fn merge_mcp_servers(
    kind: ClientTargetKind,
    existing_json: Option<&str>,
    servers: &McpServersMap,
) -> Result<String, MergeError> {
    // 1. Parse existing JSON or create empty root object
    let mut root: Value = match existing_json {
        Some(content) if !content.trim().is_empty() => parse_existing(kind, content)?,
        _ => Value::Object(Map::new()),
    };

    let root_map = root.as_object_mut().ok_or(MergeError::RootNotObject)?;

    // 2. Select the key path according to client target
    let target_key = servers_key(kind)?;

    // Ensure the container object exists
    if !root_map.contains_key(target_key) {
        root_map.insert(target_key.to_string(), Value::Object(Map::new()));
    }

    let servers_container = root_map
        .get_mut(target_key)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            MergeError::InvalidExistingJson(format!(
                "'{}' field exists but is not an object",
                target_key
            ))
        })?;

    // 3. Upsert each server
    for (name, config) in servers {
        servers_container.insert(name.clone(), server_value(kind, config)?);
    }

    // 4. Serialize with pretty printing (2 spaces)
    serde_json::to_string_pretty(&root).map_err(|e| MergeError::SerializationFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NormalizedServerConfig;
    use std::collections::BTreeMap;

    fn create_dummy_server(command: &str) -> NormalizedServerConfig {
        NormalizedServerConfig {
            command: command.to_string(),
            args: vec!["run".to_string()],
            env: BTreeMap::new(),
            extra: Map::new(),
        }
    }

    #[test]
    fn test_merge_into_empty_claude() {
        let mut servers = McpServersMap::new();
        servers.insert("test-srv".to_string(), create_dummy_server("node"));

        let merged = merge_mcp_servers(ClientTargetKind::Claude, None, &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(val["mcpServers"]["test-srv"]["command"], "node");
        assert_eq!(val["mcpServers"]["test-srv"]["type"], "stdio");
    }

    #[test]
    fn test_merge_preserves_existing_properties() {
        let existing = r#"{
            "theme": "dark",
            "mcpServers": {
                "existing-srv": {
                    "command": "python",
                    "args": ["main.py"]
                }
            }
        }"#;

        let mut servers = McpServersMap::new();
        servers.insert("new-srv".to_string(), create_dummy_server("uvx"));

        let merged = merge_mcp_servers(ClientTargetKind::Cursor, Some(existing), &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(val["theme"], "dark");
        assert_eq!(val["mcpServers"]["existing-srv"]["command"], "python");
        assert_eq!(val["mcpServers"]["new-srv"]["command"], "uvx");
    }

    #[test]
    fn test_merge_updates_existing_server_config() {
        let existing = r#"{
            "mcpServers": {
                "my-server": {
                    "command": "old-cmd",
                    "args": ["old-arg"]
                }
            }
        }"#;

        let mut servers = McpServersMap::new();
        servers.insert("my-server".to_string(), create_dummy_server("new-cmd"));

        let merged =
            merge_mcp_servers(ClientTargetKind::Project, Some(existing), &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(val["mcpServers"]["my-server"]["command"], "new-cmd");
    }

    #[test]
    fn test_merge_vscode_target_uses_copilot_key() {
        let mut servers = McpServersMap::new();
        servers.insert("vscode-server".to_string(), create_dummy_server("npx"));

        let merged = merge_mcp_servers(ClientTargetKind::VsCode, None, &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert!(val.get("github.copilot.chat.mcp").is_some());
        assert_eq!(
            val["github.copilot.chat.mcp"]["vscode-server"]["command"],
            "npx"
        );
    }

    #[test]
    fn test_merge_windsurf_cline_roo_use_mcp_servers_key() {
        let mut servers = McpServersMap::new();
        servers.insert("srv".to_string(), create_dummy_server("npx"));

        for kind in [
            ClientTargetKind::Windsurf,
            ClientTargetKind::Cline,
            ClientTargetKind::RooCode,
        ] {
            let merged = merge_mcp_servers(kind, None, &servers).unwrap();
            let val: Value = serde_json::from_str(&merged).unwrap();
            assert_eq!(val["mcpServers"]["srv"]["command"], "npx", "{kind:?}");
            assert!(val["mcpServers"]["srv"].get("type").is_none());
        }
    }

    #[test]
    fn test_merge_zed_uses_context_servers_and_preserves_jsonc() {
        let existing = r#"{
            // editor settings
            "theme": "One Dark",
            "context_servers": {
                "old": { "command": "x", },
            },
        }"#;
        let mut servers = McpServersMap::new();
        servers.insert("srv".to_string(), create_dummy_server("uvx"));

        let merged = merge_mcp_servers(ClientTargetKind::Zed, Some(existing), &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(val["theme"], "One Dark");
        assert_eq!(val["context_servers"]["old"]["command"], "x");
        assert_eq!(val["context_servers"]["srv"]["command"], "uvx");
    }

    /// ファイル種ごとの違い（設定キー / JSONC を許すか / `type: stdio` を補うか）を全種類で固定する
    #[test]
    fn test_file_targets_keep_their_servers_key_and_quirks() {
        // (種類, 設定キー, 既存の JSONC を許す, "type": "stdio" を補う)
        let cases = [
            (ClientTargetKind::Claude, "mcpServers", false, true),
            (ClientTargetKind::Cursor, "mcpServers", false, false),
            (
                ClientTargetKind::VsCode,
                "github.copilot.chat.mcp",
                false,
                false,
            ),
            (ClientTargetKind::Project, "mcpServers", false, false),
            (ClientTargetKind::Windsurf, "mcpServers", false, false),
            (ClientTargetKind::Cline, "mcpServers", false, false),
            (ClientTargetKind::RooCode, "mcpServers", false, false),
            (ClientTargetKind::Zed, "context_servers", true, false),
        ];
        let mut servers = McpServersMap::new();
        servers.insert("srv".to_string(), create_dummy_server("npx"));

        for (kind, key, allows_jsonc, adds_stdio_type) in cases {
            let merged: Value =
                serde_json::from_str(&merge_mcp_servers(kind, None, &servers).unwrap()).unwrap();
            assert_eq!(merged[key]["srv"]["command"], "npx", "{kind:?}");
            assert_eq!(
                merged[key]["srv"].get("type").cloned(),
                adds_stdio_type.then(|| Value::String("stdio".to_string())),
                "{kind:?}"
            );

            // 同じ定義が既にあれば「変更なし」と判定される（マージと判定が同じキーを見る）
            let existing = merged.to_string();
            let changes = classify_server_changes(kind, Some(&existing), &servers).unwrap();
            assert_eq!(changes["srv"], ServerChange::Unchanged, "{kind:?}");

            let jsonc = "{ // comment\n \"keep\": 1 }";
            let result = merge_mcp_servers(kind, Some(jsonc), &servers);
            if allows_jsonc {
                let value: Value = serde_json::from_str(&result.unwrap()).unwrap();
                assert_eq!(value["keep"], 1, "{kind:?}");
            } else {
                assert!(
                    matches!(result, Err(MergeError::InvalidExistingJson(_))),
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn test_non_zed_rejects_jsonc() {
        let servers = McpServersMap::new();
        let err = merge_mcp_servers(
            ClientTargetKind::Cursor,
            Some("{ // c\n \"a\": 1 }"),
            &servers,
        )
        .unwrap_err();
        assert!(matches!(err, MergeError::InvalidExistingJson(_)));
    }

    #[test]
    fn test_merge_rejects_cli_targets() {
        let servers = McpServersMap::new();
        let err = merge_mcp_servers(ClientTargetKind::CodexCli, None, &servers).unwrap_err();
        assert_eq!(err, MergeError::NotFileTarget);
    }

    #[test]
    fn test_classify_server_changes() {
        struct Case {
            name: &'static str,
            kind: ClientTargetKind,
            existing: Option<&'static str>,
            expected: &'static [(&'static str, ServerChange)],
        }

        // snippet 側: a = node run, b = uvx run
        let mut servers = McpServersMap::new();
        servers.insert("a".to_string(), create_dummy_server("node"));
        servers.insert("b".to_string(), create_dummy_server("uvx"));

        let cases = [
            Case {
                name: "no existing file: everything is new",
                kind: ClientTargetKind::Cursor,
                existing: None,
                expected: &[("a", ServerChange::New), ("b", ServerChange::New)],
            },
            Case {
                name: "empty file: everything is new",
                kind: ClientTargetKind::Cursor,
                existing: Some("  "),
                expected: &[("a", ServerChange::New), ("b", ServerChange::New)],
            },
            Case {
                name: "file without the servers key: everything is new",
                kind: ClientTargetKind::Cursor,
                existing: Some(r#"{"theme":"dark"}"#),
                expected: &[("a", ServerChange::New), ("b", ServerChange::New)],
            },
            Case {
                name: "identical definition is unchanged, different one is updated",
                kind: ClientTargetKind::Cursor,
                existing: Some(
                    r#"{"mcpServers":{"a":{"command":"node","args":["run"]},"b":{"command":"old"}}}"#,
                ),
                expected: &[("a", ServerChange::Unchanged), ("b", ServerChange::Updated)],
            },
            Case {
                name: "extra fields in the existing server count as updated (they would be dropped)",
                kind: ClientTargetKind::Cursor,
                existing: Some(
                    r#"{"mcpServers":{"a":{"command":"node","args":["run"],"disabled":true}}}"#,
                ),
                expected: &[("a", ServerChange::Updated), ("b", ServerChange::New)],
            },
            Case {
                name: "claude: type stdio is part of the written value",
                kind: ClientTargetKind::Claude,
                existing: Some(
                    r#"{"mcpServers":{"a":{"command":"node","args":["run"],"type":"stdio"}}}"#,
                ),
                expected: &[("a", ServerChange::Unchanged), ("b", ServerChange::New)],
            },
            Case {
                name: "claude: existing without type differs from what would be written",
                kind: ClientTargetKind::Claude,
                existing: Some(r#"{"mcpServers":{"a":{"command":"node","args":["run"]}}}"#),
                expected: &[("a", ServerChange::Updated), ("b", ServerChange::New)],
            },
            Case {
                name: "vscode uses its own key",
                kind: ClientTargetKind::VsCode,
                existing: Some(
                    r#"{"github.copilot.chat.mcp":{"b":{"command":"uvx","args":["run"]}}}"#,
                ),
                expected: &[("a", ServerChange::New), ("b", ServerChange::Unchanged)],
            },
            Case {
                name: "zed jsonc existing file",
                kind: ClientTargetKind::Zed,
                existing: Some(
                    "{\n  // comment\n  \"context_servers\": {\"a\": {\"command\": \"node\", \"args\": [\"run\"],}},\n}",
                ),
                expected: &[("a", ServerChange::Unchanged), ("b", ServerChange::New)],
            },
        ];

        for case in cases {
            let changes = classify_server_changes(case.kind, case.existing, &servers)
                .unwrap_or_else(|e| panic!("{}: {e}", case.name));
            let expected: BTreeMap<String, ServerChange> = case
                .expected
                .iter()
                .map(|(n, c)| (n.to_string(), *c))
                .collect();
            assert_eq!(changes, expected, "{}", case.name);
        }
    }

    #[test]
    fn test_classify_server_changes_errors() {
        let mut servers = McpServersMap::new();
        servers.insert("a".to_string(), create_dummy_server("node"));

        assert!(matches!(
            classify_server_changes(ClientTargetKind::Cursor, Some("{ not json"), &servers),
            Err(MergeError::InvalidExistingJson(_))
        ));
        assert!(matches!(
            classify_server_changes(ClientTargetKind::Cursor, Some("[]"), &servers),
            Err(MergeError::RootNotObject)
        ));
        assert!(matches!(
            classify_server_changes(
                ClientTargetKind::Cursor,
                Some(r#"{"mcpServers":[]}"#),
                &servers
            ),
            Err(MergeError::InvalidExistingJson(_))
        ));
        assert!(matches!(
            classify_server_changes(ClientTargetKind::ClaudeCli, None, &servers),
            Err(MergeError::NotFileTarget)
        ));
    }
}
