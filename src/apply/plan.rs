use super::{ApplyError, TargetIo};
use crate::delegate::plan::{build_cli_invocations, preview_invocations, CliInvocation};
use crate::domain::{ClientTarget, McpServersMap, ServerChange, TargetAction};
use crate::merge::{classify_server_changes, merge_mcp_servers};
use std::collections::BTreeMap;

/// サーバー名ごとの判定
pub type ServerChanges = BTreeMap<String, ServerChange>;

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
            Ok(TargetPlan::File {
                merged,
                changes,
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
}
