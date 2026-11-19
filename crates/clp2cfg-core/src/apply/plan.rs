use super::{ApplyError, TargetIo};
use crate::delegate::plan::{build_cli_invocations, preview_invocations, CliInvocation};
use crate::domain::{ClientTarget, McpServersMap, ServerChange, TargetAction};
use crate::merge::{
    classify_server_changes, inspect_existing_json, locate_server_line_ranges, merge_mcp_servers,
    ExistingConfigSnapshot, LineRange,
};
use std::collections::BTreeMap;

/// サーバー名ごとの判定
pub type ServerChanges = BTreeMap<String, ServerChange>;

/// サーバー名ごとのプレビュー JSON 内行範囲（0-based, inclusive）
pub type ServerLineRanges = BTreeMap<String, LineRange>;

/// How many servers got `change` (the frontends word the summary themselves).
pub fn count_changes(changes: &ServerChanges, change: ServerChange) -> usize {
    changes.values().filter(|c| **c == change).count()
}

/// What applying would do to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetPlan {
    /// Merge into the config file.
    File {
        merged: String,
        /// Per server: new / updated / unchanged.
        changes: ServerChanges,
        /// Per server: 0-based inclusive line range within `merged`.
        server_lines: ServerLineRanges,
        /// The file existed before (so a `.bak` will be made), as opposed to being created.
        had_original: bool,
    },
    /// Run the client's official CLI. Existing registrations cannot be checked beforehand.
    Cli { invocations: Vec<CliInvocation> },
}

impl TargetPlan {
    /// Text for the preview pane: the merged file, or the commands (one per line).
    pub fn preview(&self) -> String {
        match self {
            Self::File { merged, .. } => merged.clone(),
            Self::Cli { invocations } => preview_invocations(invocations),
        }
    }

    /// The per-server judgement, for file targets.
    pub fn changes(&self) -> Option<&ServerChanges> {
        match self {
            Self::File { changes, .. } => Some(changes),
            Self::Cli { .. } => None,
        }
    }

    /// The per-server line ranges in the merged preview, for file targets.
    pub fn server_lines(&self) -> Option<&ServerLineRanges> {
        match self {
            Self::File { server_lines, .. } => Some(server_lines),
            Self::Cli { .. } => None,
        }
    }

    /// The planned official-CLI invocations, for CLI targets.
    pub fn invocations(&self) -> Option<&[CliInvocation]> {
        match self {
            Self::File { .. } => None,
            Self::Cli { invocations } => Some(invocations),
        }
    }
}

/// Inspection state of an existing configuration file for a file-based target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExistingTargetState {
    /// Config file does not exist yet.
    Missing,
    /// Config file exists and was parsed successfully.
    Loaded(ExistingConfigSnapshot),
    /// Config file could not be read or parsed (e.g. UTF-16, invalid JSON).
    Error(String),
}

impl ExistingTargetState {
    /// Number of registered MCP servers if loaded successfully.
    pub fn server_count(&self) -> Option<usize> {
        match self {
            Self::Loaded(snapshot) => Some(snapshot.server_names.len()),
            Self::Missing | Self::Error(_) => None,
        }
    }
}

/// Inspect the existing configuration file of `target` via `io`.
/// Returns `None` for CLI-delegated targets (which cannot be inspected beforehand).
pub fn inspect_target(target: &ClientTarget, io: &dyn TargetIo) -> Option<ExistingTargetState> {
    match target.kind.action() {
        TargetAction::CliCommand => None,
        TargetAction::FileMerge => match io.read_config(&target.path) {
            Ok(None) => Some(ExistingTargetState::Missing),
            Ok(Some(content)) => match inspect_existing_json(target.kind, &content) {
                Ok(snapshot) => Some(ExistingTargetState::Loaded(snapshot)),
                Err(err) => Some(ExistingTargetState::Error(err.to_string())),
            },
            Err(source) => {
                let err = ApplyError::ReadConfig {
                    path: target.path.clone(),
                    source,
                };
                Some(ExistingTargetState::Error(err.to_string()))
            }
        },
    }
}

/// Work out what applying `servers` to `target` would do, without changing anything.
pub fn plan_target(
    target: &ClientTarget,
    servers: &McpServersMap,
    io: &dyn TargetIo,
) -> Result<TargetPlan, ApplyError> {
    match target.kind.action() {
        TargetAction::FileMerge => {
            let existing =
                io.read_config(&target.path)
                    .map_err(|source| ApplyError::ReadConfig {
                        path: target.path.clone(),
                        source,
                    })?;
            let merged = merge_mcp_servers(target.kind, existing.as_deref(), servers)?;
            let changes = classify_server_changes(target.kind, existing.as_deref(), servers)?;
            let server_lines = locate_server_line_ranges(target.kind, &merged)?;
            Ok(TargetPlan::File {
                merged,
                changes,
                server_lines,
                had_original: existing.is_some(),
            })
        }
        TargetAction::CliCommand => {
            let program = target.path.to_string_lossy();
            let invocations = build_cli_invocations(target.kind, &program, servers)?;
            Ok(TargetPlan::Cli { invocations })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeIo;
    use super::super::SystemIo;
    use super::*;
    use crate::domain::{ClientTargetKind, NormalizedServerConfig};
    use crate::merge::MergeError;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn servers() -> McpServersMap {
        McpServersMap::from([("new-one".to_string(), NormalizedServerConfig::new("node"))])
    }

    /// UTF-16 LE（BOM 付き。Windows PowerShell 5.1 の `>` / `Out-File` の既定）
    fn utf16le_with_bom(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn test_unreadable_existing_file_is_an_error_not_an_empty_config() {
        let original = r#"{"mcpServers":{"keep-me":{"command":"node"}},"other":"precious"}"#;
        let cases: [(&str, Vec<u8>); 2] = [
            ("UTF-16 LE (BOM)", utf16le_with_bom(original)),
            ("invalid UTF-8", vec![b'{', 0xC3, 0x28, b'}']),
        ];

        for (label, bytes) in cases {
            let dir = tempdir().unwrap();
            let path = dir.path().join(".mcp.json");
            std::fs::write(&path, &bytes).unwrap();
            let target = ClientTarget::new(ClientTargetKind::Project, path.clone(), true);

            let err = plan_target(&target, &servers(), &SystemIo).unwrap_err();

            assert!(matches!(err, ApplyError::ReadConfig { .. }), "{label}");
            let message = err.to_string();
            assert!(
                message.starts_with("設定ファイルを読み取れません: ")
                    && message.contains(".mcp.json"),
                "{label}: {message}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{label}");
        }
    }

    #[test]
    fn test_missing_file_is_planned_as_new() {
        let dir = tempdir().unwrap();
        let target = ClientTarget::new(
            ClientTargetKind::Project,
            dir.path().join(".mcp.json"),
            false,
        );

        let plan = plan_target(&target, &servers(), &SystemIo).unwrap();

        let TargetPlan::File {
            changes,
            had_original,
            ..
        } = &plan
        else {
            panic!("{plan:?}");
        };
        assert!(!had_original);
        assert_eq!(changes["new-one"], ServerChange::New);
        assert!(plan.preview().contains("new-one"));
    }

    #[test]
    fn test_readable_existing_file_is_merged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"keep-me":{"command":"node"}}}"#).unwrap();
        let target = ClientTarget::new(ClientTargetKind::Project, path, true);

        let plan = plan_target(&target, &servers(), &SystemIo).unwrap();

        let preview = plan.preview();
        assert!(preview.contains("keep-me") && preview.contains("new-one"));
        assert!(matches!(
            plan,
            TargetPlan::File {
                had_original: true,
                ..
            }
        ));
    }

    #[test]
    fn test_plan_with_a_fake_port_classifies_each_server() {
        let existing = r#"{"mcpServers":{"same":{"command":"node"},"changed":{"command":"old"}}}"#;
        let io = FakeIo::default().with_file("/cfg/a.json", existing);
        let target =
            ClientTarget::new(ClientTargetKind::Cursor, PathBuf::from("/cfg/a.json"), true);
        let servers = McpServersMap::from([
            ("same".to_string(), NormalizedServerConfig::new("node")),
            ("changed".to_string(), NormalizedServerConfig::new("new")),
            ("added".to_string(), NormalizedServerConfig::new("node")),
        ]);

        let plan = plan_target(&target, &servers, &io).unwrap();

        let changes = plan.changes().unwrap();
        assert_eq!(changes["same"], ServerChange::Unchanged);
        assert_eq!(changes["changed"], ServerChange::Updated);
        assert_eq!(changes["added"], ServerChange::New);
        assert_eq!(count_changes(changes, ServerChange::New), 1);
        assert_eq!(count_changes(changes, ServerChange::Updated), 1);
        assert_eq!(count_changes(changes, ServerChange::Unchanged), 1);
        // 計画は何も書かない
        assert_eq!(io.content("/cfg/a.json").unwrap(), existing);
        assert!(io.written.borrow().is_empty());
    }

    #[test]
    fn test_cli_target_plans_commands_and_has_no_changes() {
        let target =
            ClientTarget::new_cli(ClientTargetKind::CodexCli, Some(PathBuf::from("fake-cli")));

        let plan = plan_target(&target, &servers(), &FakeIo::default()).unwrap();

        assert_eq!(plan.changes(), None);
        assert_eq!(plan.preview(), "fake-cli mcp add new-one -- node");
        assert!(matches!(&plan, TargetPlan::Cli { invocations } if invocations.len() == 1));
    }

    #[test]
    fn test_merge_errors_keep_their_message() {
        let io = FakeIo::default().with_file("/cfg/a.json", "[1, 2]");
        let target =
            ClientTarget::new(ClientTargetKind::Cursor, PathBuf::from("/cfg/a.json"), true);

        let err = plan_target(&target, &servers(), &io).unwrap_err();

        assert!(matches!(err, ApplyError::Merge(_)));
        assert_eq!(
            err.to_string(),
            MergeError::RootNotObject.to_string(),
            "transparent なので元のメッセージのまま"
        );
    }

    #[test]
    fn test_local_binary_snippet_updates_existing_server_without_duplicate() {
        let existing = r#"{
            "mcpServers": {
                "desktop-touch": {
                    "type": "stdio",
                    "command": "bunx",
                    "args": ["@harusame64/desktop-touch-mcp"]
                }
            }
        }"#;
        let io = FakeIo::default().with_file("/cfg/.claude.json", existing);
        let target = ClientTarget::new(
            ClientTargetKind::Claude,
            PathBuf::from("/cfg/.claude.json"),
            true,
        );
        let snippet =
            r#"{"command":"C:\\Users\\alice\\.bun\\bin\\desktop-touch-mcp.exe","args":[]}"#;
        let prepared = crate::snippet::pipeline::prepare(
            snippet,
            &crate::snippet::filter::FilterOptions::default(),
            &crate::snippet::placeholder::EnvOverrides::new(),
        )
        .unwrap();

        let plan = plan_target(&target, &prepared.servers, &io).unwrap();
        let changes = plan.changes().unwrap();

        assert_eq!(changes.get("desktop-touch"), Some(&ServerChange::Updated));
        assert!(!changes.contains_key("desktop-touch-mcp"));
        let TargetPlan::File { merged, .. } = plan else {
            panic!("expected file plan");
        };
        let merged_json: serde_json::Value = serde_json::from_str(&merged).unwrap();
        let servers_obj = merged_json["mcpServers"].as_object().unwrap();
        assert_eq!(servers_obj.len(), 1);
        assert_eq!(
            servers_obj["desktop-touch"]["command"],
            r"C:\Users\alice\.bun\bin\desktop-touch-mcp.exe"
        );
    }

    #[test]
    fn test_inspect_target_handles_missing_loaded_error_and_cli() {
        let missing = ClientTarget::new(
            ClientTargetKind::Cursor,
            PathBuf::from("/cfg/missing.json"),
            false,
        );
        assert_eq!(
            inspect_target(&missing, &FakeIo::default()),
            Some(ExistingTargetState::Missing)
        );

        let io = FakeIo::default()
            .with_file(
                "/cfg/ok.json",
                r#"{"mcpServers":{"a":{"command":"node","env":{"K":"sec"}}}}"#,
            )
            .with_file("/cfg/bad.json", "{ broken");
        let ok_target = ClientTarget::new(
            ClientTargetKind::Cursor,
            PathBuf::from("/cfg/ok.json"),
            true,
        );
        let Some(ExistingTargetState::Loaded(snap)) = inspect_target(&ok_target, &io) else {
            panic!("expected Loaded");
        };
        assert_eq!(snap.server_names, vec!["a"]);
        assert!(!snap.masked_json.contains("sec"));
        assert!(snap.raw_json.contains("sec"));

        let bad_target = ClientTarget::new(
            ClientTargetKind::Cursor,
            PathBuf::from("/cfg/bad.json"),
            true,
        );
        assert!(matches!(
            inspect_target(&bad_target, &io),
            Some(ExistingTargetState::Error(_))
        ));

        let cli = ClientTarget::new_cli(ClientTargetKind::ClaudeCli, Some(PathBuf::from("claude")));
        assert_eq!(inspect_target(&cli, &io), None);
    }
}
