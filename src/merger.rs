//! AST merger for various AI client configuration formats.

use crate::model::{ClientTarget, ClientTargetKind, McpServersMap, MergeError};
use serde_json::{Map, Value};
use std::path::Path;

/// Detect standard client configuration targets on the host system.
pub fn detect_client_targets(project_dir: Option<&Path>) -> Vec<ClientTarget> {
    let mut targets = Vec::new();
    let home = dirs::home_dir();

    // 1. Claude Code / Desktop
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Claude,
            h.join(".claude.json"),
        ));
    }

    // 2. Cursor
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Cursor,
            h.join(".cursor").join("mcp.json"),
        ));
    }

    // 3. Codex CLI
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Codex,
            h.join(".codex").join("config.json"),
        ));
    }

    // 4. VS Code Copilot settings.json
    let vscode_path = if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("Code").join("User").join("settings.json"))
    } else if cfg!(target_os = "macos") {
        home.as_ref().map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
                .join("settings.json")
        })
    } else {
        dirs::config_dir().map(|c| c.join("Code").join("User").join("settings.json"))
    };

    if let Some(path) = vscode_path {
        targets.push(ClientTarget::new(ClientTargetKind::VsCode, path));
    }

    // 5. Project Local (.mcp.json or mcp.json)
    let cwd = std::env::current_dir().ok();
    let base_dir = project_dir.or(cwd.as_deref());

    if let Some(dir) = base_dir {
        let dot_mcp = dir.join(".mcp.json");
        let plain_mcp = dir.join("mcp.json");
        let project_mcp_path = if plain_mcp.is_file() {
            plain_mcp
        } else {
            dot_mcp
        };
        targets.push(ClientTarget::new(
            ClientTargetKind::Project,
            project_mcp_path,
        ));
    }

    targets
}

/// Merges MCP servers into an existing configuration JSON according to target semantics.
pub fn merge_mcp_servers(
    kind: ClientTargetKind,
    existing_json: Option<&str>,
    servers: &McpServersMap,
) -> Result<String, MergeError> {
    // 1. Parse existing JSON or create empty root object
    let mut root: Value = match existing_json {
        Some(content) if !content.trim().is_empty() => serde_json::from_str(content)
            .map_err(|e| MergeError::InvalidExistingJson(e.to_string()))?,
        _ => Value::Object(Map::new()),
    };

    let root_map = root.as_object_mut().ok_or(MergeError::RootNotObject)?;

    // 2. Select the key path according to client target
    let target_key = match kind {
        ClientTargetKind::Claude | ClientTargetKind::Cursor | ClientTargetKind::Project => {
            "mcpServers"
        }
        ClientTargetKind::Codex => "mcp",
        ClientTargetKind::VsCode => "github.copilot.chat.mcp",
    };

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
        let mut server_val = serde_json::to_value(config)
            .map_err(|e| MergeError::SerializationFailed(e.to_string()))?;

        // Claude requires/prefers "type": "stdio"
        if kind == ClientTargetKind::Claude {
            if let Some(obj) = server_val.as_object_mut() {
                if !obj.contains_key("type") {
                    obj.insert("type".to_string(), Value::String("stdio".to_string()));
                }
            }
        }

        servers_container.insert(name.clone(), server_val);
    }

    // 4. Serialize with pretty printing (2 spaces)
    serde_json::to_string_pretty(&root).map_err(|e| MergeError::SerializationFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NormalizedServerConfig;
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
    fn test_merge_codex_target_uses_mcp_key() {
        let mut servers = McpServersMap::new();
        servers.insert("codex-server".to_string(), create_dummy_server("docker"));

        let merged = merge_mcp_servers(ClientTargetKind::Codex, None, &servers).unwrap();
        let val: Value = serde_json::from_str(&merged).unwrap();

        assert!(val.get("mcp").is_some());
        assert_eq!(val["mcp"]["codex-server"]["command"], "docker");
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
    fn test_detect_targets_non_empty() {
        let targets = detect_client_targets(None);
        assert!(!targets.is_empty());
    }
}
