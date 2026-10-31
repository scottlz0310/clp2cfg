use super::undo::AppliedFile;
use super::{plan_target, ApplyError, ServerChanges, TargetIo, TargetPlan};
use crate::domain::{ClientTarget, McpServersMap};
use std::path::PathBuf;

/// Whether to really change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Apply,
    /// Plan and check everything, write and run nothing.
    DryRun,
}

/// What happened to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    File {
        changes: ServerChanges,
        /// What was written, for undo. `None` in a dry run.
        written: Option<AppliedFile>,
    },
    Cli {
        commands: usize,
        /// The commands were run (`false` in a dry run).
        executed: bool,
    },
}

/// The result for one target, in the order the targets were given.
#[derive(Debug)]
pub struct TargetOutcome {
    pub name: String,
    pub path: PathBuf,
    pub result: Result<Applied, ApplyError>,
}

/// The results for all targets. One target failing does not stop the others.
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    pub results: Vec<TargetOutcome>,
}

impl ApplyOutcome {
    /// The files that were written (empty in a dry run), for undo.
    pub fn applied_files(&self) -> Vec<AppliedFile> {
        self.results
            .iter()
            .filter_map(|r| match &r.result {
                Ok(Applied::File { written, .. }) => written.clone(),
                _ => None,
            })
            .collect()
    }

    /// The CLI-delegated targets whose commands were run.
    pub fn executed_cli_names(&self) -> Vec<String> {
        self.results
            .iter()
            .filter(|r| matches!(r.result, Ok(Applied::Cli { executed: true, .. })))
            .map(|r| r.name.clone())
            .collect()
    }
}

/// Apply `servers` to each target (or only check, in a dry run).
/// The wording of the results is left to the frontends.
pub fn apply_targets<'a>(
    targets: impl IntoIterator<Item = &'a ClientTarget>,
    servers: &McpServersMap,
    mode: Mode,
    io: &dyn TargetIo,
) -> ApplyOutcome {
    let results = targets
        .into_iter()
        .map(|target| TargetOutcome {
            name: target.name.clone(),
            path: target.path.clone(),
            result: apply_target(target, servers, mode, io),
        })
        .collect();
    ApplyOutcome { results }
}

fn apply_target(
    target: &ClientTarget,
    servers: &McpServersMap,
    mode: Mode,
    io: &dyn TargetIo,
) -> Result<Applied, ApplyError> {
    match plan_target(target, servers, io)? {
        TargetPlan::File {
            merged,
            changes,
            had_original,
        } => {
            let written = match mode {
                Mode::DryRun => None,
                Mode::Apply => {
                    io.write_config(&target.path, &merged)?;
                    Some(AppliedFile {
                        target_name: target.name.clone(),
                        path: target.path.clone(),
                        written: merged,
                        had_original,
                    })
                }
            };
            Ok(Applied::File { changes, written })
        }
        TargetPlan::Cli { invocations } => {
            let executed = mode == Mode::Apply;
            if executed {
                io.run_cli(&invocations)?;
            }
            Ok(Applied::Cli {
                commands: invocations.len(),
                executed,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::FakeIo;
    use super::*;
    use crate::domain::{ClientTargetKind, NormalizedServerConfig, ServerChange};
    use std::path::Path;

    fn servers() -> McpServersMap {
        McpServersMap::from([("srv".to_string(), NormalizedServerConfig::new("node"))])
    }

    fn file_target(kind: ClientTargetKind, path: &str, exists: bool) -> ClientTarget {
        ClientTarget::new(kind, PathBuf::from(path), exists)
    }

    fn cli_target(kind: ClientTargetKind) -> ClientTarget {
        ClientTarget::new_cli(kind, Some(PathBuf::from("fake-cli")))
    }

    fn written_json(io: &FakeIo, path: &str) -> serde_json::Value {
        serde_json::from_str(&io.content(path).unwrap()).unwrap()
    }

    #[test]
    fn test_dry_run_changes_nothing() {
        let io = FakeIo::default().with_file("/cfg/a.json", r#"{"keep":1}"#);
        let targets = [
            file_target(ClientTargetKind::Cursor, "/cfg/a.json", true),
            file_target(ClientTargetKind::Windsurf, "/cfg/new.json", false),
            cli_target(ClientTargetKind::CodexCli),
        ];

        let outcome = apply_targets(&targets, &servers(), Mode::DryRun, &io);

        assert!(outcome.results.iter().all(|r| r.result.is_ok()));
        assert_eq!(io.content("/cfg/a.json").unwrap(), r#"{"keep":1}"#);
        assert_eq!(io.content("/cfg/new.json"), None);
        assert!(io.written.borrow().is_empty() && io.cli_calls.borrow().is_empty());
        assert!(outcome.applied_files().is_empty());
        assert!(outcome.executed_cli_names().is_empty());
        // 判定は dry-run でも得られる
        let Ok(Applied::File { changes, written }) = &outcome.results[0].result else {
            panic!("{:?}", outcome.results[0]);
        };
        assert_eq!(changes["srv"], ServerChange::New);
        assert!(written.is_none());
        assert!(matches!(
            outcome.results[2].result,
            Ok(Applied::Cli {
                commands: 1,
                executed: false
            })
        ));
    }

    #[test]
    fn test_apply_writes_files_and_runs_cli_in_order() {
        let io = FakeIo::default().with_file("/cfg/a.json", r#"{"keep":1}"#);
        let targets = [
            file_target(ClientTargetKind::Cursor, "/cfg/a.json", true),
            cli_target(ClientTargetKind::ClaudeCli),
            file_target(ClientTargetKind::Windsurf, "/cfg/new.json", false),
        ];

        let outcome = apply_targets(&targets, &servers(), Mode::Apply, &io);

        assert_eq!(
            outcome
                .results
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["Cursor", "Claude Code CLI", "Windsurf"]
        );
        // 既存の設定を残してマージされ、新規ファイルも作られる
        let existing = written_json(&io, "/cfg/a.json");
        assert_eq!(existing["keep"], 1);
        assert_eq!(existing["mcpServers"]["srv"]["command"], "node");
        assert_eq!(
            written_json(&io, "/cfg/new.json")["mcpServers"]["srv"]["command"],
            "node"
        );
        // 取り消し用の記録: 対象・パス・書いた内容・既存だったか
        let files = outcome.applied_files();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].target_name, "Cursor");
        assert_eq!(files[0].path, Path::new("/cfg/a.json"));
        assert_eq!(files[0].written, io.content("/cfg/a.json").unwrap());
        assert!(files[0].had_original);
        assert!(!files[1].had_original);
        // CLI は remove → add の 2 コマンドを 1 回で実行する
        assert_eq!(io.cli_calls.borrow().len(), 1);
        assert_eq!(io.cli_calls.borrow()[0].len(), 2);
        assert_eq!(outcome.executed_cli_names(), ["Claude Code CLI"]);
    }

    #[test]
    fn test_a_failing_target_does_not_stop_the_others() {
        let io = FakeIo::default();
        io.unwritable
            .borrow_mut()
            .insert(PathBuf::from("/cfg/bad.json"));
        io.cli_fails.set(true);
        let targets = [
            file_target(ClientTargetKind::Cursor, "/cfg/bad.json", false),
            cli_target(ClientTargetKind::CodexCli),
            file_target(ClientTargetKind::Windsurf, "/cfg/ok.json", false),
        ];

        let outcome = apply_targets(&targets, &servers(), Mode::Apply, &io);

        assert!(matches!(
            outcome.results[0].result,
            Err(ApplyError::Write(_))
        ));
        assert!(matches!(outcome.results[1].result, Err(ApplyError::Cli(_))));
        assert!(outcome.results[2].result.is_ok());
        // 失敗したものは記録に残らない
        assert_eq!(outcome.applied_files().len(), 1);
        assert!(outcome.executed_cli_names().is_empty());
        assert_eq!(io.cli_calls.borrow().len(), 1, "失敗しても実行は試みた");
    }

    #[test]
    fn test_an_unreadable_config_is_an_error_and_nothing_is_written() {
        let io = FakeIo::default().with_file("/cfg/a.json", "{}");
        io.unreadable
            .borrow_mut()
            .insert(PathBuf::from("/cfg/a.json"));
        let targets = [file_target(ClientTargetKind::Cursor, "/cfg/a.json", true)];

        for mode in [Mode::DryRun, Mode::Apply] {
            let outcome = apply_targets(&targets, &servers(), mode, &io);

            let message = outcome.results[0].result.as_ref().unwrap_err().to_string();
            assert!(
                message.starts_with("設定ファイルを読み取れません: ") && message.contains("a.json"),
                "{message}"
            );
        }
        assert!(io.written.borrow().is_empty());
    }

    #[test]
    fn test_invalid_existing_json_fails_that_target() {
        let io = FakeIo::default().with_file("/cfg/a.json", "{ not json");
        let targets = [file_target(ClientTargetKind::Cursor, "/cfg/a.json", true)];

        let outcome = apply_targets(&targets, &servers(), Mode::Apply, &io);

        assert!(matches!(
            outcome.results[0].result,
            Err(ApplyError::Merge(_))
        ));
        assert_eq!(io.content("/cfg/a.json").unwrap(), "{ not json");
    }
}
