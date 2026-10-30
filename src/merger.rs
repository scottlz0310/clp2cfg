//! AST merger for various AI client configuration formats.

use crate::cli::find_in_path;
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

    // 3. VS Code Copilot settings.json
    let vscode_user = vscode_user_dir(home.as_deref());
    if let Some(ref dir) = vscode_user {
        targets.push(ClientTarget::new(
            ClientTargetKind::VsCode,
            dir.join("settings.json"),
        ));
    }

    // 4. Project Local (.mcp.json or mcp.json)
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

    // 6. Windsurf
    if let Some(ref h) = home {
        targets.push(ClientTarget::new(
            ClientTargetKind::Windsurf,
            h.join(".codeium").join("windsurf").join("mcp_config.json"),
        ));
    }

    // 7. Cline / 8. Roo Code (VS Code extensions, stored in globalStorage)
    if let Some(ref dir) = vscode_user {
        let storage = dir.join("globalStorage");
        targets.push(ClientTarget::new(
            ClientTargetKind::Cline,
            storage
                .join("saoudrizwan.claude-dev")
                .join("settings")
                .join("cline_mcp_settings.json"),
        ));
        targets.push(ClientTarget::new(
            ClientTargetKind::RooCode,
            storage
                .join("rooveterinaryinc.roo-cline")
                .join("settings")
                .join("mcp_settings.json"),
        ));
    }

    // 9. Zed
    let zed_dir = if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("Zed"))
    } else {
        home.as_ref().map(|h| h.join(".config").join("zed"))
    };
    if let Some(dir) = zed_dir {
        targets.push(ClientTarget::new(
            ClientTargetKind::Zed,
            dir.join("settings.json"),
        ));
    }

    // 10. CLI delegation targets (detected on PATH)
    for kind in [
        ClientTargetKind::ClaudeCli,
        ClientTargetKind::CodexCli,
        ClientTargetKind::CopilotCli,
        ClientTargetKind::AntigravityCli,
    ] {
        let resolved = kind.cli_program().and_then(find_in_path);
        targets.push(ClientTarget::new_cli(kind, resolved));
    }

    targets
}

/// VS Code `User` directory (holds `settings.json` and `globalStorage`).
fn vscode_user_dir(home: Option<&Path>) -> Option<std::path::PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::data_dir().map(|d| d.join("Code").join("User"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
        })
    } else {
        dirs::config_dir().map(|c| c.join("Code").join("User"))
    }
}

/// Parse existing config content. Zed's `settings.json` is JSONC, so fall back to JSON5
/// (comments are not preserved on rewrite; the `.bak` copy keeps the original).
fn parse_existing(kind: ClientTargetKind, content: &str) -> Result<Value, MergeError> {
    match serde_json::from_str(content) {
        Ok(v) => Ok(v),
        Err(e) if kind == ClientTargetKind::Zed => {
            json5::from_str(content).map_err(|_| MergeError::InvalidExistingJson(e.to_string()))
        }
        Err(e) => Err(MergeError::InvalidExistingJson(e.to_string())),
    }
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
    let target_key = match kind {
        ClientTargetKind::Claude
        | ClientTargetKind::Cursor
        | ClientTargetKind::Project
        | ClientTargetKind::Windsurf
        | ClientTargetKind::Cline
        | ClientTargetKind::RooCode => "mcpServers",
        ClientTargetKind::VsCode => "github.copilot.chat.mcp",
        ClientTargetKind::Zed => "context_servers",
        ClientTargetKind::ClaudeCli
        | ClientTargetKind::CodexCli
        | ClientTargetKind::CopilotCli
        | ClientTargetKind::AntigravityCli => return Err(MergeError::NotFileTarget),
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
    fn test_detect_targets_include_new_clients() {
        let kinds: Vec<_> = detect_client_targets(None).iter().map(|t| t.kind).collect();
        for k in [
            ClientTargetKind::Windsurf,
            ClientTargetKind::Cline,
            ClientTargetKind::RooCode,
            ClientTargetKind::Zed,
            ClientTargetKind::ClaudeCli,
            ClientTargetKind::AntigravityCli,
        ] {
            assert!(kinds.contains(&k), "{k:?} missing");
        }
    }

    #[test]
    fn test_detect_targets_non_empty() {
        let targets = detect_client_targets(None);
        assert!(!targets.is_empty());
    }
}
