//! AST merger for various AI client configuration formats.

use crate::domain::{ClientTargetKind, McpServersMap, ServerChange};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// 0-based inclusive line range (`start..=end`) in a pretty-printed JSON document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

/// Snapshot of an existing configuration file for inspection in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingConfigSnapshot {
    /// Registered MCP server names under the target's `servers_key`.
    pub server_names: Vec<String>,
    /// Pretty-printed JSON with `env` / `headers` values masked as `***`.
    pub masked_json: String,
    /// Pretty-printed JSON with unmasked values.
    pub raw_json: String,
    /// 0-based inclusive line range for each server in `masked_json`.
    pub masked_server_lines: BTreeMap<String, LineRange>,
    /// 0-based inclusive line range for each server in `raw_json`.
    pub raw_server_lines: BTreeMap<String, LineRange>,
}

impl ExistingConfigSnapshot {
    /// Return the JSON text and matching per-server line ranges for the requested visibility mode.
    pub fn display(&self, reveal_secrets: bool) -> (&str, &BTreeMap<String, LineRange>) {
        if reveal_secrets {
            (&self.raw_json, &self.raw_server_lines)
        } else {
            (&self.masked_json, &self.masked_server_lines)
        }
    }
}

/// Result of removing selected MCP servers from an existing configuration JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveResult {
    /// Pretty-printed JSON after removal (raw values, for writing).
    pub raw_json: String,
    /// Pretty-printed JSON after removal with `env` / `headers` values masked as `***`.
    pub masked_json: String,
    /// Server names that existed and were removed, in deterministic order.
    pub removed: Vec<String>,
}

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

    #[error(
        "Target '{target}' does not support remote transport '{transport}' for server '{server}'"
    )]
    UnsupportedRemoteTransport {
        target: &'static str,
        server: String,
        transport: String,
    },
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
    name: &str,
    config: &crate::domain::NormalizedServerConfig,
) -> Result<Value, MergeError> {
    let file_spec = kind.file_spec().ok_or(MergeError::NotFileTarget)?;
    let mut server_val =
        serde_json::to_value(config).map_err(|e| MergeError::SerializationFailed(e.to_string()))?;

    if let Some(obj) = server_val.as_object_mut() {
        // Some clients (Claude) require/prefer "type": "stdio" for stdio servers
        if config.is_stdio() && file_spec.adds_stdio_type && !obj.contains_key("type") {
            obj.insert("type".to_string(), Value::String("stdio".to_string()));
        } else if config.is_remote() {
            let raw_type = config.transport_type();
            match file_spec.remote_type.resolve_type(raw_type) {
                Some(Some(mapped)) => {
                    obj.insert("type".to_string(), Value::String(mapped.to_string()));
                }
                Some(None) => {
                    obj.remove("type");
                }
                None => {
                    return Err(MergeError::UnsupportedRemoteTransport {
                        target: kind.cli_name(),
                        server: name.to_string(),
                        transport: raw_type.unwrap_or_default().to_string(),
                    });
                }
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
            let desired = server_value(kind, name, config)?;
            let change = match existing_servers.and_then(|m| m.get(name)) {
                None => ServerChange::New,
                Some(existing) if *existing == desired => ServerChange::Unchanged,
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
        servers_container.insert(name.clone(), server_value(kind, name, config)?);
    }

    // 4. Serialize with pretty printing (2 spaces)
    serde_json::to_string_pretty(&root).map_err(|e| MergeError::SerializationFailed(e.to_string()))
}

/// Locate the 0-based inclusive line range for each server entry under the target's `servers_key`
/// in a serialized JSON document (such as the output of [`merge_mcp_servers`]).
pub fn locate_server_line_ranges(
    kind: ClientTargetKind,
    json: &str,
) -> Result<BTreeMap<String, LineRange>, MergeError> {
    let target_key = servers_key(kind)?;
    Ok(scan_server_line_ranges(json, target_key))
}

/// Parse an existing configuration file and return registered MCP server names along with
/// masked (`env` / `headers` values replaced with `***`) and raw pretty-printed JSON previews.
pub fn inspect_existing_json(
    kind: ClientTargetKind,
    existing_json: &str,
) -> Result<ExistingConfigSnapshot, MergeError> {
    let target_key = servers_key(kind)?;
    if existing_json.trim().is_empty() {
        return Ok(ExistingConfigSnapshot {
            server_names: Vec::new(),
            masked_json: "{}".to_string(),
            raw_json: "{}".to_string(),
            masked_server_lines: BTreeMap::new(),
            raw_server_lines: BTreeMap::new(),
        });
    }

    let root = parse_existing(kind, existing_json)?;
    let root_map = root.as_object().ok_or(MergeError::RootNotObject)?;
    let server_names = match root_map.get(target_key) {
        None => Vec::new(),
        Some(value) => {
            let obj = value.as_object().ok_or_else(|| {
                MergeError::InvalidExistingJson(format!(
                    "'{}' field exists but is not an object",
                    target_key
                ))
            })?;
            obj.keys().cloned().collect()
        }
    };

    let raw_json = serde_json::to_string_pretty(&root)
        .map_err(|e| MergeError::SerializationFailed(e.to_string()))?;
    let raw_server_lines = scan_server_line_ranges(&raw_json, target_key);

    let mut masked_root = root;
    mask_secret_fields(&mut masked_root, target_key);
    let masked_json = serde_json::to_string_pretty(&masked_root)
        .map_err(|e| MergeError::SerializationFailed(e.to_string()))?;
    let masked_server_lines = scan_server_line_ranges(&masked_json, target_key);

    Ok(ExistingConfigSnapshot {
        server_names,
        masked_json,
        raw_json,
        masked_server_lines,
        raw_server_lines,
    })
}

/// Remove the specified MCP servers from an existing configuration JSON while preserving
/// all other MCP servers and top-level properties.
///
/// When all servers under `servers_key` are removed, the `servers_key` container is kept as
/// an empty JSON object (`{}`) so the file structure remains stable across clients.
pub fn remove_mcp_servers(
    kind: ClientTargetKind,
    existing_json: &str,
    to_remove: &BTreeSet<String>,
) -> Result<RemoveResult, MergeError> {
    let target_key = servers_key(kind)?;
    let mut root = parse_existing(kind, existing_json)?;
    let root_map = root.as_object_mut().ok_or(MergeError::RootNotObject)?;

    let mut removed = Vec::new();
    if let Some(value) = root_map.get_mut(target_key) {
        let servers_container = value.as_object_mut().ok_or_else(|| {
            MergeError::InvalidExistingJson(format!(
                "'{}' field exists but is not an object",
                target_key
            ))
        })?;
        for name in to_remove {
            if servers_container.remove(name).is_some() {
                removed.push(name.clone());
            }
        }
    }

    let raw_json = serde_json::to_string_pretty(&root)
        .map_err(|e| MergeError::SerializationFailed(e.to_string()))?;

    let mut masked_root = root;
    mask_secret_fields(&mut masked_root, target_key);
    let masked_json = serde_json::to_string_pretty(&masked_root)
        .map_err(|e| MergeError::SerializationFailed(e.to_string()))?;

    Ok(RemoveResult {
        raw_json,
        masked_json,
        removed,
    })
}

fn mask_secret_fields(root: &mut Value, target_key: &str) {
    let Some(servers) = root
        .as_object_mut()
        .and_then(|map| map.get_mut(target_key))
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    for server in servers.values_mut() {
        let Some(server_obj) = server.as_object_mut() else {
            continue;
        };
        for field in ["env", "headers"] {
            if let Some(secret_map) = server_obj.get_mut(field).and_then(Value::as_object_mut) {
                for val in secret_map.values_mut() {
                    *val = Value::String("***".to_string());
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerState {
    Object {
        is_root: bool,
        is_target_servers: bool,
        expecting_key: bool,
        awaiting_target_value: bool,
    },
    Array,
}

fn scan_server_line_ranges(json: &str, target_key: &str) -> BTreeMap<String, LineRange> {
    let mut ranges = BTreeMap::new();
    let bytes = json.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut line = 0usize;
    let mut stack: Vec<ContainerState> = Vec::new();
    let mut current_server: Option<(String, usize, usize)> = None;

    while i < len {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'"' => {
                let start = i;
                let start_line = line;
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' => {
                            i = (i + 2).min(len);
                        }
                        b'"' => {
                            i += 1;
                            break;
                        }
                        b'\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
                let raw = &json[start..i];
                if let Some(ContainerState::Object {
                    is_root,
                    is_target_servers,
                    expecting_key,
                    awaiting_target_value,
                }) = stack.last_mut()
                {
                    if *expecting_key {
                        *expecting_key = false;
                        if *is_root {
                            if let Ok(decoded) = serde_json::from_str::<String>(raw) {
                                *awaiting_target_value = decoded == target_key;
                            }
                        } else if *is_target_servers {
                            if let Ok(decoded) = serde_json::from_str::<String>(raw) {
                                current_server = Some((decoded, start_line, line));
                            }
                        }
                    } else {
                        *awaiting_target_value = false;
                        if *is_target_servers {
                            if let Some((_, _, end_line)) = current_server.as_mut() {
                                *end_line = line;
                            }
                        }
                    }
                }
            }
            b'{' => {
                let is_root = stack.is_empty();
                let is_target_servers = matches!(
                    stack.last(),
                    Some(ContainerState::Object {
                        is_root: true,
                        awaiting_target_value: true,
                        ..
                    })
                );
                if let Some(ContainerState::Object {
                    awaiting_target_value,
                    ..
                }) = stack.last_mut()
                {
                    *awaiting_target_value = false;
                }
                stack.push(ContainerState::Object {
                    is_root,
                    is_target_servers,
                    expecting_key: true,
                    awaiting_target_value: false,
                });
                i += 1;
            }
            b'}' => {
                let popped = stack.pop();
                if matches!(
                    popped,
                    Some(ContainerState::Object {
                        is_target_servers: true,
                        ..
                    })
                ) {
                    if let Some((name, start, end)) = current_server.take() {
                        ranges.insert(name, LineRange { start, end });
                    }
                } else if matches!(
                    stack.last(),
                    Some(ContainerState::Object {
                        is_target_servers: true,
                        ..
                    })
                ) {
                    if let Some((_, _, end_line)) = current_server.as_mut() {
                        *end_line = line;
                    }
                }
                i += 1;
            }
            b'[' => {
                if let Some(ContainerState::Object {
                    awaiting_target_value,
                    ..
                }) = stack.last_mut()
                {
                    *awaiting_target_value = false;
                }
                stack.push(ContainerState::Array);
                i += 1;
            }
            b']' => {
                stack.pop();
                if matches!(
                    stack.last(),
                    Some(ContainerState::Object {
                        is_target_servers: true,
                        ..
                    })
                ) {
                    if let Some((_, _, end_line)) = current_server.as_mut() {
                        *end_line = line;
                    }
                }
                i += 1;
            }
            b',' => {
                if let Some(ContainerState::Object {
                    is_target_servers,
                    expecting_key,
                    awaiting_target_value,
                    ..
                }) = stack.last_mut()
                {
                    if *is_target_servers {
                        if let Some((name, start, end)) = current_server.take() {
                            ranges.insert(name, LineRange { start, end });
                        }
                    }
                    *expecting_key = true;
                    *awaiting_target_value = false;
                }
                i += 1;
            }
            b':' => {
                i += 1;
            }
            _ => {
                // Primitive token (number, true, false, null)
                while i < len
                    && !matches!(
                        bytes[i],
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']' | b':'
                    )
                {
                    i += 1;
                }
                if let Some(ContainerState::Object {
                    is_target_servers,
                    awaiting_target_value,
                    ..
                }) = stack.last_mut()
                {
                    *awaiting_target_value = false;
                    if *is_target_servers {
                        if let Some((_, _, end_line)) = current_server.as_mut() {
                            *end_line = line;
                        }
                    }
                }
            }
        }
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{NormalizedServerConfig, ServerTransport};
    use std::collections::BTreeMap;

    fn create_dummy_server(command: &str) -> NormalizedServerConfig {
        NormalizedServerConfig {
            transport: ServerTransport::Stdio {
                command: command.to_string(),
                args: vec!["run".to_string()],
                env: BTreeMap::new(),
            },
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

    #[test]
    fn test_merge_remote_server_preserves_type_and_does_not_add_stdio_to_claude() {
        let http_srv = NormalizedServerConfig::remote("https://mcp.cloudflare.com/mcp")
            .with_transport_type("http");
        let typeless_srv = NormalizedServerConfig::remote("https://example.com/mcp");
        let servers = McpServersMap::from([
            ("cloudflare-api".to_string(), http_srv),
            ("plain".to_string(), typeless_srv),
        ]);

        let merged = merge_mcp_servers(ClientTargetKind::Claude, None, &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(val["mcpServers"]["cloudflare-api"]["type"], "http");
        assert_eq!(
            val["mcpServers"]["cloudflare-api"]["url"],
            "https://mcp.cloudflare.com/mcp"
        );
        assert!(val["mcpServers"]["plain"].get("type").is_none());
        assert_eq!(val["mcpServers"]["plain"]["url"], "https://example.com/mcp");

        let changes =
            classify_server_changes(ClientTargetKind::Claude, Some(&merged), &servers).unwrap();
        assert_eq!(changes["cloudflare-api"], ServerChange::Unchanged);
        assert_eq!(changes["plain"], ServerChange::Unchanged);
    }

    #[test]
    fn test_merge_remote_server_maps_http_type_per_target() {
        let cases = [
            // (kind, servers_key, input_type, expected_type)
            (
                ClientTargetKind::Cline,
                "mcpServers",
                Some("http"),
                Some("streamableHttp"),
            ),
            (
                ClientTargetKind::Cline,
                "mcpServers",
                Some("streamable-http"),
                Some("streamableHttp"),
            ),
            (
                ClientTargetKind::Cline,
                "mcpServers",
                None,
                Some("streamableHttp"),
            ),
            (
                ClientTargetKind::Cline,
                "mcpServers",
                Some("sse"),
                Some("sse"),
            ),
            (
                ClientTargetKind::RooCode,
                "mcpServers",
                Some("http"),
                Some("streamable-http"),
            ),
            (
                ClientTargetKind::RooCode,
                "mcpServers",
                Some("streamableHttp"),
                Some("streamable-http"),
            ),
            (
                ClientTargetKind::RooCode,
                "mcpServers",
                None,
                Some("streamable-http"),
            ),
            (
                ClientTargetKind::RooCode,
                "mcpServers",
                Some("sse"),
                Some("sse"),
            ),
            (
                ClientTargetKind::Claude,
                "mcpServers",
                Some("streamableHttp"),
                Some("http"),
            ),
            (
                ClientTargetKind::Cursor,
                "mcpServers",
                Some("streamable-http"),
                Some("http"),
            ),
            (ClientTargetKind::Cursor, "mcpServers", None, None),
            (
                ClientTargetKind::VsCode,
                "github.copilot.chat.mcp",
                Some("HTTP"),
                Some("http"),
            ),
            (ClientTargetKind::Zed, "context_servers", None, None),
        ];

        for (kind, key, input_type, expected_type) in cases {
            let mut srv = NormalizedServerConfig::remote("https://mcp.cloudflare.com/mcp");
            if let Some(t) = input_type {
                srv = srv.with_transport_type(t);
            }
            let servers = McpServersMap::from([("cloudflare-api".to_string(), srv)]);

            let merged = merge_mcp_servers(kind, None, &servers).unwrap();
            let val: Value = serde_json::from_str(&merged).unwrap();
            let actual_type = val[key]["cloudflare-api"]
                .get("type")
                .and_then(Value::as_str);
            assert_eq!(actual_type, expected_type, "{kind:?} input={input_type:?}");

            // マージ後の設定は変更分類でも Unchanged と一致する
            let changes = classify_server_changes(kind, Some(&merged), &servers).unwrap();
            assert_eq!(
                changes["cloudflare-api"],
                ServerChange::Unchanged,
                "{kind:?} input={input_type:?}"
            );
        }
    }

    #[test]
    fn test_merge_rejects_unsupported_remote_transport_type() {
        let srv = NormalizedServerConfig::remote("wss://example.com/mcp")
            .with_transport_type("websocket");
        let servers = McpServersMap::from([("ws".to_string(), srv)]);

        let err = merge_mcp_servers(ClientTargetKind::Cline, None, &servers).unwrap_err();
        assert_eq!(
            err,
            MergeError::UnsupportedRemoteTransport {
                target: "cline",
                server: "ws".to_string(),
                transport: "websocket".to_string(),
            }
        );

        let err_classify =
            classify_server_changes(ClientTargetKind::Claude, Some("{}"), &servers).unwrap_err();
        assert_eq!(
            err_classify,
            MergeError::UnsupportedRemoteTransport {
                target: "claude",
                server: "ws".to_string(),
                transport: "websocket".to_string(),
            }
        );
    }

    #[test]
    fn test_locate_server_line_ranges_handles_nested_and_special_keys() {
        let existing = r#"{
            "nested": {
                "mcpServers": {
                    "decoy": { "command": "fake" }
                }
            },
            "mcpServers": {
                "empty-srv": {},
                "escaped\"key": {
                    "command": "node",
                    "args": ["{not a block}", "line\\n"]
                }
            }
        }"#;
        let servers = McpServersMap::from([
            ("日本語-srv".to_string(), create_dummy_server("uvx")),
            ("new-srv".to_string(), create_dummy_server("bunx")),
        ]);

        let merged = merge_mcp_servers(ClientTargetKind::Cursor, Some(existing), &servers).unwrap();
        let ranges = locate_server_line_ranges(ClientTargetKind::Cursor, &merged).unwrap();
        let lines: Vec<&str> = merged.lines().collect();

        // ネストした別オブジェクト内の decoy は含まれない
        assert!(!ranges.contains_key("decoy"));

        for key in ["empty-srv", "escaped\"key", "日本語-srv", "new-srv"] {
            let range = ranges
                .get(key)
                .unwrap_or_else(|| panic!("missing line range for {key}"));
            let encoded_key = serde_json::to_string(key).unwrap();
            assert!(
                lines[range.start].trim_start().starts_with(&encoded_key),
                "start line mismatch for {key}: {}",
                lines[range.start]
            );
            let end_trimmed = lines[range.end].trim();
            assert!(
                end_trimmed.ends_with('}') || end_trimmed.ends_with("},"),
                "end line mismatch for {key}: {end_trimmed}"
            );
        }

        // 1 行の空オブジェクトは start == end
        let empty_range = ranges["empty-srv"];
        assert_eq!(empty_range.start, empty_range.end);
    }

    #[test]
    fn test_locate_server_line_ranges_per_target_key_and_rejects_cli() {
        let servers = McpServersMap::from([("srv".to_string(), create_dummy_server("node"))]);
        for kind in [ClientTargetKind::VsCode, ClientTargetKind::Zed] {
            let merged = merge_mcp_servers(kind, None, &servers).unwrap();
            let ranges = locate_server_line_ranges(kind, &merged).unwrap();
            let range = ranges["srv"];
            let lines: Vec<&str> = merged.lines().collect();
            assert!(lines[range.start].contains("\"srv\""), "{kind:?}");
            assert!(range.end > range.start, "{kind:?}");
        }

        assert_eq!(
            locate_server_line_ranges(ClientTargetKind::ClaudeCli, "{}"),
            Err(MergeError::NotFileTarget)
        );
    }

    #[test]
    fn test_inspect_existing_json_masks_env_and_headers_and_preserves_other_fields() {
        let existing = r#"{
            "theme": "dark",
            "mcpServers": {
                "local-srv": {
                    "command": "node",
                    "args": ["index.js"],
                    "env": {
                        "API_KEY": "super-secret-env",
                        "EMPTY": ""
                    }
                },
                "remote-srv": {
                    "type": "http",
                    "url": "https://example.com/mcp",
                    "headers": {
                        "Authorization": "Bearer secret-token"
                    }
                }
            }
        }"#;

        let snap = inspect_existing_json(ClientTargetKind::Cursor, existing).unwrap();
        assert_eq!(snap.server_names, vec!["local-srv", "remote-srv"]);
        assert!(snap.masked_server_lines.contains_key("local-srv"));
        assert!(snap.masked_server_lines.contains_key("remote-srv"));
        assert!(snap.raw_server_lines.contains_key("local-srv"));
        assert!(snap.raw_server_lines.contains_key("remote-srv"));

        // masked_json は env / headers を *** にし、他の値は保持する
        assert!(!snap.masked_json.contains("super-secret-env"));
        assert!(!snap.masked_json.contains("secret-token"));
        assert!(snap.masked_json.contains(r#""API_KEY": "***""#));
        assert!(snap.masked_json.contains(r#""Authorization": "***""#));
        assert!(snap.masked_json.contains(r#""theme": "dark""#));
        assert!(snap.masked_json.contains(r#""command": "node""#));

        // raw_json は実値を保持する
        assert!(snap.raw_json.contains("super-secret-env"));
        assert!(snap.raw_json.contains("secret-token"));
    }

    #[test]
    fn test_inspect_existing_json_computes_separate_line_ranges_for_masked_and_raw() {
        // 先行サーバーの env に複数行オブジェクトがあると、マスク版と平文版で後続サーバーの行位置が異なる
        let existing = r#"{
            "mcpServers": {
                "first": {
                    "env": {
                        "NESTED": { "k": "v" }
                    }
                },
                "second": {
                    "command": "node"
                }
            }
        }"#;

        let snap = inspect_existing_json(ClientTargetKind::Cursor, existing).unwrap();
        let (masked_text, masked_lines) = snap.display(false);
        let (raw_text, raw_lines) = snap.display(true);

        assert_ne!(masked_lines["second"], raw_lines["second"]);

        let masked_vec: Vec<&str> = masked_text.lines().collect();
        let raw_vec: Vec<&str> = raw_text.lines().collect();
        assert!(masked_vec[masked_lines["second"].start].contains("\"second\""));
        assert!(raw_vec[raw_lines["second"].start].contains("\"second\""));
    }

    #[test]
    fn test_inspect_existing_json_supports_zed_jsonc_and_empty_and_errors() {
        let zed = "{\n  // comment\n  \"context_servers\": {\n    \"z\": { \"command\": \"uvx\", },\n  },\n}";
        let snap = inspect_existing_json(ClientTargetKind::Zed, zed).unwrap();
        assert_eq!(snap.server_names, vec!["z"]);

        let empty = inspect_existing_json(ClientTargetKind::Cursor, "   ").unwrap();
        assert!(empty.server_names.is_empty());
        assert_eq!(empty.masked_json, "{}");

        assert!(matches!(
            inspect_existing_json(ClientTargetKind::Cursor, "{ broken"),
            Err(MergeError::InvalidExistingJson(_))
        ));
        assert!(matches!(
            inspect_existing_json(ClientTargetKind::Cursor, "[]"),
            Err(MergeError::RootNotObject)
        ));
        assert_eq!(
            inspect_existing_json(ClientTargetKind::CodexCli, "{}"),
            Err(MergeError::NotFileTarget)
        );
    }

    #[test]
    fn test_remove_mcp_servers_preserves_other_servers_and_keeps_empty_container() {
        let existing = r#"{
            "theme": "dark",
            "mcpServers": {
                "keep": {
                    "command": "node",
                    "env": { "SECRET": "keep-secret" }
                },
                "drop-a": { "command": "uvx" },
                "drop-b": { "command": "bunx" }
            }
        }"#;

        let partial = remove_mcp_servers(
            ClientTargetKind::Cursor,
            existing,
            &BTreeSet::from(["drop-a".to_string(), "missing".to_string()]),
        )
        .unwrap();
        assert_eq!(partial.removed, vec!["drop-a"]);
        assert!(partial.raw_json.contains("keep-secret"));
        assert!(!partial.masked_json.contains("keep-secret"));
        assert!(partial.masked_json.contains(r#""SECRET": "***""#));
        let val: Value = serde_json::from_str(&partial.raw_json).unwrap();
        assert_eq!(val["theme"], "dark");
        assert!(val["mcpServers"].get("keep").is_some());
        assert!(val["mcpServers"].get("drop-a").is_none());
        assert!(val["mcpServers"].get("drop-b").is_some());

        // 全サーバーを削除しても servers_key は空オブジェクト {} として残す
        let all = remove_mcp_servers(
            ClientTargetKind::Cursor,
            existing,
            &BTreeSet::from([
                "keep".to_string(),
                "drop-a".to_string(),
                "drop-b".to_string(),
            ]),
        )
        .unwrap();
        assert_eq!(all.removed, vec!["drop-a", "drop-b", "keep"]);
        let empty_val: Value = serde_json::from_str(&all.raw_json).unwrap();
        assert_eq!(empty_val["theme"], "dark");
        assert_eq!(empty_val["mcpServers"], Value::Object(Map::new()));

        // Zed JSONC 対応
        let zed = "{\n  // comment\n  \"theme\": \"One Dark\",\n  \"context_servers\": {\n    \"a\": { \"command\": \"x\", },\n    \"b\": { \"command\": \"y\", },\n  },\n}";
        let zed_res = remove_mcp_servers(
            ClientTargetKind::Zed,
            zed,
            &BTreeSet::from(["a".to_string()]),
        )
        .unwrap();
        let zed_val: Value = serde_json::from_str(&zed_res.raw_json).unwrap();
        assert_eq!(zed_val["theme"], "One Dark");
        assert!(zed_val["context_servers"].get("a").is_none());
        assert!(zed_val["context_servers"].get("b").is_some());

        assert_eq!(
            remove_mcp_servers(ClientTargetKind::ClaudeCli, "{}", &BTreeSet::new()),
            Err(MergeError::NotFileTarget)
        );
    }
}
