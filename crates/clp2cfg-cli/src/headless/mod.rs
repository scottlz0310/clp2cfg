//! Headless (no window) apply / dry-run, shared by `clp2cfg --apply` and `clp2cfg-cli`.
//!
//! The run is described by a [`HeadlessRequest`]; the outside world comes in through
//! [`InputEnv`], [`TargetCatalog`] and [`TargetIo`], so nothing here knows about the command line.

mod input;
mod report;
mod request;
mod select;

use clp2cfg_core::apply::{TargetCatalog, TargetIo};
use clp2cfg_core::snippet::pipeline::{prepare, Prepared};
use clp2cfg_core::snippet::placeholder::EnvOverrides;
use report::{apply, dry_run};
use select::select_targets;

pub use input::{read_input, InputEnv, InputSource};
pub use report::{Report, EXIT_FAILED, EXIT_OK, EXIT_USAGE};
pub use request::{HeadlessAction, HeadlessRequest};

/// Run a headless apply or dry-run.
pub fn run(
    request: &HeadlessRequest,
    input: &dyn InputEnv,
    catalog: &dyn TargetCatalog,
    io: &dyn TargetIo,
) -> Report {
    let Prepared {
        servers,
        placeholders,
    } = match load_servers(request, input) {
        Ok(prepared) => prepared,
        Err(message) => return Report::usage(message),
    };

    if !request.allow_placeholders && !placeholders.is_empty() {
        let list = placeholders
            .iter()
            .map(|p| format!("{} / {}", p.server, p.key))
            .collect::<Vec<_>>()
            .join(", ");
        return Report::usage(format!(
            "未入力のプレースホルダーが {} 件残っています（{list}）。実値に置き換えるか、--allow-placeholders を指定してください",
            placeholders.len()
        ));
    }

    let targets = match select_targets(&request.targets, catalog.targets()) {
        Ok(targets) => targets,
        Err(message) => return Report::usage(message),
    };

    match request.action {
        HeadlessAction::DryRun => dry_run(&servers, &targets, request.show_secrets, io),
        HeadlessAction::Apply => apply(&servers, &targets, io),
    }
}

fn load_servers(request: &HeadlessRequest, input: &dyn InputEnv) -> Result<Prepared, String> {
    let (text, source) = read_input(request.input.as_deref(), input)?;
    prepare(&text, &request.filters, &EnvOverrides::new()).map_err(|e| match source {
        InputSource::Literal => {
            format!("入力を解釈できません（ファイルが存在せず、JSON としても無効です）: {e}")
        }
        _ => format!("入力を解釈できません: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Args;
    use clp2cfg_core::apply::SystemIo;
    use clp2cfg_core::delegate::plan::CliInvocation;
    use clp2cfg_core::delegate::CliError;
    use clp2cfg_core::domain::{ClientTarget, ClientTargetKind};
    use clp2cfg_core::writer::WriteError;
    use std::cell::RefCell;
    use std::path::Path;
    use tempfile::tempdir;

    struct FakeEnv {
        clipboard: Option<String>,
        stdin: String,
        targets: Vec<ClientTarget>,
        cli_calls: RefCell<Vec<Vec<String>>>,
        cli_fails: bool,
    }

    impl FakeEnv {
        fn new(targets: Vec<ClientTarget>) -> Self {
            Self {
                clipboard: None,
                stdin: String::new(),
                targets,
                cli_calls: RefCell::new(Vec::new()),
                cli_fails: false,
            }
        }
    }

    impl InputEnv for FakeEnv {
        fn clipboard_text(&self) -> Option<String> {
            self.clipboard.clone()
        }
        fn stdin_text(&self) -> std::io::Result<String> {
            Ok(self.stdin.clone())
        }
    }

    impl TargetCatalog for FakeEnv {
        fn targets(&self) -> Vec<ClientTarget> {
            self.targets.clone()
        }
    }

    /// Files are real (a temp dir); only the CLI is faked.
    impl TargetIo for FakeEnv {
        fn read_config(&self, path: &Path) -> std::io::Result<Option<String>> {
            SystemIo.read_config(path)
        }
        fn write_config(&self, path: &Path, content: &str) -> Result<(), WriteError> {
            SystemIo.write_config(path, content)
        }
        fn restore_backup(&self, path: &Path) -> Result<(), WriteError> {
            SystemIo.restore_backup(path)
        }
        fn remove_config(&self, path: &Path) -> std::io::Result<()> {
            SystemIo.remove_config(path)
        }
        fn config_exists(&self, path: &Path) -> bool {
            SystemIo.config_exists(path)
        }
        fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError> {
            self.cli_calls
                .borrow_mut()
                .push(invocations.iter().map(CliInvocation::display).collect());
            if self.cli_fails {
                return Err(CliError::CommandFailed {
                    command: "fake".to_string(),
                    message: "boom".to_string(),
                });
            }
            Ok(())
        }
    }

    fn file_target(
        dir: &Path,
        kind: ClientTargetKind,
        name: &str,
        content: Option<&str>,
    ) -> ClientTarget {
        let path = dir.join(name);
        if let Some(content) = content {
            std::fs::write(&path, content).unwrap();
        }
        clp2cfg_core::discovery::file_target(kind, path)
    }

    fn cli_target(kind: ClientTargetKind) -> ClientTarget {
        ClientTarget::new_cli(kind, Some(std::path::PathBuf::from("fake-cli")))
    }

    /// What the binaries do: the arguments become a request, then the run.
    fn run_args(args: &Args, env: &FakeEnv) -> Report {
        let request = HeadlessRequest::try_from(args).unwrap();
        run(&request, env, env, env)
    }

    fn args(extra: &[&str]) -> Args {
        use clap::Parser;
        Args::try_parse_from(std::iter::once("clp2cfg").chain(extra.iter().copied())).unwrap()
    }

    const SNIPPET: &str = r#"{"mcpServers":{"srv":{"command":"node","args":["a"]}}}"#;

    #[test]
    fn test_dry_run_reports_changes_without_writing() {
        let dir = tempdir().unwrap();
        let existing = r#"{"mcpServers":{"srv":{"command":"old"},"keep":{"command":"x"}}}"#;
        let target = file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "mcp.json",
            Some(existing),
        );
        let env = FakeEnv::new(vec![target.clone()]);

        let report = run_args(&args(&["--dry-run", SNIPPET]), &env);

        assert_eq!(report.exit_code, EXIT_OK, "{report:?}");
        assert!(report.stdout.contains("~ srv  上書き"), "{}", report.stdout);
        assert!(report.stdout.contains("書き込みはしていません"));
        assert_eq!(std::fs::read_to_string(&target.path).unwrap(), existing);
        assert!(!dir.path().join("mcp.json.bak").exists());
    }

    #[test]
    fn test_dry_run_masks_env_values_unless_asked() {
        let dir = tempdir().unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let env = FakeEnv::new(vec![target, cli_target(ClientTargetKind::CodexCli)]);
        let snippet = r#"{"srv":{"command":"node","env":{"API_KEY":"sk-secret-value"}}}"#;

        let masked = run_args(
            &args(&["--dry-run", "--target", "cursor,codex-cli", snippet]),
            &env,
        );
        assert_eq!(masked.exit_code, EXIT_OK, "{masked:?}");
        assert!(
            !masked.stdout.contains("sk-secret-value"),
            "{}",
            masked.stdout
        );
        assert!(masked.stdout.contains("***"));
        // CLI のコマンドも伏せる
        assert!(masked.stdout.contains("API_KEY=***"), "{}", masked.stdout);

        let shown = run_args(
            &args(&[
                "--dry-run",
                "--show-secrets",
                "--target",
                "cursor,codex-cli",
                snippet,
            ]),
            &env,
        );
        assert!(shown.stdout.contains("sk-secret-value"), "{}", shown.stdout);
        assert!(
            env.cli_calls.borrow().is_empty(),
            "dry-run は CLI を実行しない"
        );
    }

    #[test]
    fn test_apply_defaults_to_existing_file_targets_only() {
        let dir = tempdir().unwrap();
        let existing = file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "cursor.json",
            Some("{}"),
        );
        let missing = file_target(
            dir.path(),
            ClientTargetKind::Windsurf,
            "windsurf.json",
            None,
        );
        let env = FakeEnv::new(vec![
            existing.clone(),
            missing.clone(),
            cli_target(ClientTargetKind::ClaudeCli),
        ]);

        let report = run_args(&args(&["--apply", SNIPPET]), &env);

        assert_eq!(report.exit_code, EXIT_OK, "{report:?}");
        assert!(std::fs::read_to_string(&existing.path)
            .unwrap()
            .contains("srv"));
        assert!(
            !missing.path.exists(),
            "存在しないファイルは --target なしでは作らない"
        );
        assert!(
            env.cli_calls.borrow().is_empty(),
            "CLI は --target なしでは実行しない"
        );
        assert!(
            report.stdout.contains("適用完了: 1 件"),
            "{}",
            report.stdout
        );
    }

    #[test]
    fn test_unreadable_existing_file_fails_without_touching_it() {
        // UTF-16 LE（BOM 付き）の既存ファイルを空として上書きしない
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(r#"{"other":"precious"}"#.encode_utf16().flat_map(u16::to_le_bytes));

        for flag in ["--dry-run", "--apply"] {
            let dir = tempdir().unwrap();
            let target = file_target(dir.path(), ClientTargetKind::Cursor, "cursor.json", None);
            std::fs::write(&target.path, &bytes).unwrap();
            let target =
                clp2cfg_core::discovery::file_target(ClientTargetKind::Cursor, target.path);
            let env = FakeEnv::new(vec![target.clone()]);

            let report = run_args(&args(&[flag, SNIPPET]), &env);

            assert_eq!(report.exit_code, EXIT_FAILED, "{flag}: {report:?}");
            assert!(
                report.stderr.contains("設定ファイルを読み取れません"),
                "{flag}: {}",
                report.stderr
            );
            assert_eq!(std::fs::read(&target.path).unwrap(), bytes, "{flag}");
            assert!(
                !clp2cfg_core::writer::backup_path_for(&target.path).exists(),
                "{flag}: .bak を作らない"
            );
        }
    }

    /// dry-run の CLI 計画は、env の値だけを伏せ、コマンド側の引数（`-e` など）は伏せない。
    /// （`display_redacted` で代用するとコマンドの `-e KEY=val` まで伏せてしまうため、方式を固定する）
    #[test]
    fn test_dry_run_masks_env_values_but_not_command_arguments() {
        let env = FakeEnv::new(vec![cli_target(ClientTargetKind::ClaudeCli)]);
        let snippet =
            r#"{"srv":{"command":"node","args":["-e","X=1"],"env":{"K":"sk-secret-value"}}}"#;

        let report = run_args(
            &args(&["--dry-run", "--target", "claude-cli", snippet]),
            &env,
        );

        assert_eq!(report.exit_code, EXIT_OK, "{report:?}");
        assert!(
            report
                .stdout
                .contains("  $ fake-cli mcp add --scope user srv -e 'K=***' -- node -e X=1\n"),
            "{}",
            report.stdout
        );
        assert!(!report.stdout.contains("sk-secret-value"));
    }

    /// 適用の出力行（ファイルは判定の要約つき、CLI はコマンド数）を固定する
    #[test]
    fn test_apply_output_lines() {
        let dir = tempdir().unwrap();
        let existing = file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "cursor.json",
            Some(r#"{"mcpServers":{"srv":{"command":"old"},"other":{"command":"x"}}}"#),
        );
        let env = FakeEnv::new(vec![
            existing.clone(),
            cli_target(ClientTargetKind::CodexCli),
        ]);

        let report = run_args(
            &args(&["--apply", "--target", "cursor,codex-cli", SNIPPET]),
            &env,
        );

        assert_eq!(report.exit_code, EXIT_OK, "{report:?}");
        assert_eq!(
            report.stdout,
            format!(
                "[Cursor] 適用: {} (新規追加 0 / 上書き 1 / 変更なし 0)\n[Codex CLI] 実行: 1 コマンド\n\n適用完了: 2 件\n",
                existing.path.display()
            )
        );
        assert_eq!(report.stderr, "");
    }

    #[test]
    fn test_apply_with_explicit_targets_creates_files_and_runs_cli() {
        let dir = tempdir().unwrap();
        let missing = file_target(
            dir.path(),
            ClientTargetKind::Windsurf,
            "windsurf.json",
            None,
        );
        let env = FakeEnv::new(vec![
            missing.clone(),
            file_target(dir.path(), ClientTargetKind::Cursor, "cursor.json", None),
            cli_target(ClientTargetKind::ClaudeCli),
        ]);

        let report = run_args(
            &args(&[
                "--apply",
                "--target",
                "windsurf,claude-cli,windsurf",
                SNIPPET,
            ]),
            &env,
        );

        assert_eq!(report.exit_code, EXIT_OK, "{report:?}");
        assert!(missing.path.is_file());
        assert!(
            !dir.path().join("cursor.json").exists(),
            "指定していないターゲットは触らない"
        );
        let calls = env.cli_calls.borrow();
        assert_eq!(calls.len(), 1, "重複指定は 1 回だけ");
        assert!(calls[0].iter().any(|c| c.contains("mcp add")), "{calls:?}");
    }

    #[test]
    fn test_placeholders_block_apply_unless_allowed() {
        let dir = tempdir().unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let env = FakeEnv::new(vec![target.clone()]);
        let snippet =
            r#"{"srv":{"command":"node","env":{"TOKEN":"YOUR_API_KEY","REGION":"us-east-1"}}}"#;

        for mode in ["--apply", "--dry-run"] {
            let blocked = run_args(&args(&[mode, snippet]), &env);
            assert_eq!(blocked.exit_code, EXIT_USAGE, "{mode}: {blocked:?}");
            assert!(blocked.stderr.contains("srv / TOKEN"), "{}", blocked.stderr);
            assert!(
                !blocked.stderr.contains("YOUR_API_KEY"),
                "値は出さない: {}",
                blocked.stderr
            );
        }
        assert_eq!(std::fs::read_to_string(&target.path).unwrap(), "{}");

        let allowed = run_args(&args(&["--apply", "--allow-placeholders", snippet]), &env);
        assert_eq!(allowed.exit_code, EXIT_OK, "{allowed:?}");
        assert!(std::fs::read_to_string(&target.path)
            .unwrap()
            .contains("YOUR_API_KEY"));
    }

    #[test]
    fn test_input_sources() {
        let dir = tempdir().unwrap();
        let snippet_file = dir.path().join("snippet.json");
        std::fs::write(&snippet_file, SNIPPET).unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let mut env = FakeEnv::new(vec![target]);
        env.clipboard = Some(SNIPPET.to_string());
        env.stdin = SNIPPET.to_string();
        let file_arg = snippet_file.to_string_lossy().to_string();

        let cases: [(&str, Vec<&str>); 4] = [
            ("file path", vec!["--dry-run", &file_arg]),
            ("JSON string", vec!["--dry-run", SNIPPET]),
            ("stdin", vec!["--dry-run", "-"]),
            ("clipboard", vec!["--dry-run"]),
        ];
        for (name, argv) in cases {
            let report = run_args(&args(&argv), &env);
            assert_eq!(report.exit_code, EXIT_OK, "{name}: {report:?}");
            assert!(report.stdout.contains("srv"), "{name}: {}", report.stdout);
        }
    }

    #[test]
    fn test_usage_errors_exit_with_2_and_write_nothing() {
        let dir = tempdir().unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let no_targets = FakeEnv::new(vec![file_target(
            dir.path(),
            ClientTargetKind::Windsurf,
            "missing.json",
            None,
        )]);
        let env = FakeEnv::new(vec![target.clone()]); // クリップボードは空

        struct Case {
            name: &'static str,
            argv: Vec<&'static str>,
            use_env_without_existing_target: bool,
            expected_stderr: &'static str,
        }
        let cases = [
            Case {
                name: "clipboard is empty",
                argv: vec!["--apply"],
                use_env_without_existing_target: false,
                expected_stderr: "クリップボードに文字列がありません",
            },
            Case {
                name: "not a file and not JSON",
                argv: vec!["--apply", "no-such-file.json"],
                use_env_without_existing_target: false,
                expected_stderr: "ファイルが存在せず、JSON としても無効",
            },
            Case {
                name: "no existing config file and no --target",
                argv: vec!["--apply", SNIPPET],
                use_env_without_existing_target: true,
                expected_stderr: "--target で指定してください",
            },
        ];

        for case in cases {
            let used = if case.use_env_without_existing_target {
                &no_targets
            } else {
                &env
            };
            let report = run_args(&args(&case.argv), used);
            assert_eq!(report.exit_code, EXIT_USAGE, "{}: {report:?}", case.name);
            assert!(
                report.stderr.contains(case.expected_stderr),
                "{}: {}",
                case.name,
                report.stderr
            );
            assert!(report.stdout.is_empty(), "{}", case.name);
        }
        assert_eq!(std::fs::read_to_string(&target.path).unwrap(), "{}");
    }

    #[test]
    fn test_unknown_target_name_is_a_usage_error() {
        // clap が先に弾くが、直接 Args を組み立てても安全であること
        let dir = tempdir().unwrap();
        let env = FakeEnv::new(vec![file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "m.json",
            Some("{}"),
        )]);
        let mut argv = args(&["--apply", SNIPPET]);
        argv.targets = vec!["notepad".to_string()];

        let report = run_args(&argv, &env);

        assert_eq!(report.exit_code, EXIT_USAGE);
        assert!(
            report.stderr.contains("不明なターゲット: notepad"),
            "{}",
            report.stderr
        );
    }

    #[test]
    fn test_partial_failure_exits_with_1_but_applies_the_rest() {
        let dir = tempdir().unwrap();
        let good = file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "good.json",
            Some("{}"),
        );
        let broken = file_target(
            dir.path(),
            ClientTargetKind::Windsurf,
            "broken.json",
            Some("{ not json"),
        );
        let env = FakeEnv::new(vec![broken.clone(), good.clone()]);

        let report = run_args(&args(&["--apply", SNIPPET]), &env);

        assert_eq!(report.exit_code, EXIT_FAILED, "{report:?}");
        assert!(report.stderr.contains("Windsurf"), "{}", report.stderr);
        assert!(std::fs::read_to_string(&good.path).unwrap().contains("srv"));
        assert_eq!(std::fs::read_to_string(&broken.path).unwrap(), "{ not json");
        assert!(report.stdout.contains("失敗: 1 件"), "{}", report.stdout);
    }

    #[test]
    fn test_cli_failure_exits_with_1() {
        let mut env = FakeEnv::new(vec![cli_target(ClientTargetKind::CodexCli)]);
        env.cli_fails = true;

        let report = run_args(&args(&["--apply", "--target", "codex-cli", SNIPPET]), &env);

        assert_eq!(report.exit_code, EXIT_FAILED, "{report:?}");
        assert!(report.stderr.contains("Codex CLI"), "{}", report.stderr);
    }

    #[test]
    fn test_filters_follow_flags() {
        let dir = tempdir().unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let env = FakeEnv::new(vec![target.clone()]);
        let snippet = r#"{"a":{"command":"npx","args":["-y","pkg"]},"b":{"command":"pipx","args":["run","tool"]}}"#;

        run_args(&args(&["--apply", snippet]), &env);
        let content = std::fs::read_to_string(&target.path).unwrap();
        assert!(
            content.contains("\"bunx\"") && content.contains("\"uvx\""),
            "{content}"
        );

        std::fs::write(&target.path, "{}").unwrap();
        run_args(
            &args(&["--apply", "--no-npx-to-bunx", "--no-pipx-to-uvx", snippet]),
            &env,
        );
        let content = std::fs::read_to_string(&target.path).unwrap();
        assert!(
            content.contains("\"npx\"") && content.contains("\"pipx\""),
            "{content}"
        );
    }

    #[test]
    fn test_invalid_input_never_echoes_secret_values() {
        let dir = tempdir().unwrap();
        let env = FakeEnv::new(vec![file_target(
            dir.path(),
            ClientTargetKind::Cursor,
            "mcp.json",
            Some("{}"),
        )]);

        // 引用符なしの数値の認証情報、構文エラーの行に混ざった値、型違いの args
        let inputs = [
            (
                r#"{"srv":{"command":"node","env":{"TOKEN":123456789}}}"#,
                "123456789",
            ),
            (
                "{srv: {command: 'node', env: {TOKEN: 'sk-secret-xyz' oops}}}",
                "sk-secret-xyz",
            ),
            (
                r#"{"srv":{"command":"node","args":"sk-secret-args"}}"#,
                "sk-secret-args",
            ),
        ];

        for (input, secret) in inputs {
            for mode in ["--dry-run", "--apply"] {
                let mut stdin_env = FakeEnv::new(env.targets.clone());
                stdin_env.stdin = input.to_string();
                for report in [
                    run_args(&args(&[mode, input]), &env),
                    run_args(&args(&[mode, "-"]), &stdin_env),
                ] {
                    assert_eq!(report.exit_code, EXIT_USAGE, "{mode}: {report:?}");
                    assert!(
                        !report.stdout.contains(secret),
                        "{mode} stdout: {}",
                        report.stdout
                    );
                    assert!(
                        !report.stderr.contains(secret),
                        "{mode} stderr: {}",
                        report.stderr
                    );
                }
            }
        }
        assert_eq!(std::fs::read_to_string(&env.targets[0].path).unwrap(), "{}");
    }
}
