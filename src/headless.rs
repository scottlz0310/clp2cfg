//! Headless (no window) apply / dry-run, shared by `clp2cfg --apply` and `clp2cfg-cli`.

use crate::args::{Args, HeadlessAction, Mode};
use crate::delegate::exec::run_invocations;
use crate::delegate::plan::{build_cli_invocations, CliInvocation};
use crate::delegate::CliError;
use crate::discovery::detect_client_targets;
use crate::domain::{ClientTarget, ClientTargetKind, McpServersMap, ServerChange, TargetAction};
use crate::preview::{build_preview_with_changes, ServerChanges};
use crate::snippet::filter::{apply_filters, FilterOptions};
use crate::snippet::parser::parse_mcp_snippet;
use crate::snippet::placeholder::find_placeholders;
use crate::writer::safe_atomic_write;
use std::fmt::Write as _;
use std::path::Path;

/// Everything succeeded.
pub const EXIT_OK: u8 = 0;
/// The run was valid but at least one target could not be applied.
pub const EXIT_FAILED: u8 = 1;
/// Bad usage or input (nothing was written).
pub const EXIT_USAGE: u8 = 2;

/// Replacement for env values in the `--dry-run` output.
const MASK: &str = "***";

/// What the headless run needs from the outside world (injectable for tests).
pub trait Environment {
    fn clipboard_text(&self) -> Option<String>;
    fn stdin_text(&self) -> std::io::Result<String>;
    /// Every known target (existing or not), as the GUI would list them.
    fn targets(&self) -> Vec<ClientTarget>;
    fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError>;
}

/// The real environment: OS clipboard, standard input, detected clients, real CLI processes.
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn stdin_text(&self) -> std::io::Result<String> {
        std::io::read_to_string(std::io::stdin())
    }

    fn targets(&self) -> Vec<ClientTarget> {
        detect_client_targets(None)
    }

    fn run_cli(&self, invocations: &[CliInvocation]) -> Result<(), CliError> {
        run_invocations(invocations)
    }
}

/// Result of a headless run: what to print and the process exit code.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl Report {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_USAGE,
            stdout: String::new(),
            stderr: format!("{}\n", message.into()),
        }
    }
}

/// Where the snippet text came from (decides the wording of a parse error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    Clipboard,
    Stdin,
    File,
    Literal,
}

/// Resolve the snippet text: `None` = clipboard, `-` = stdin, an existing file = its content,
/// anything else = the JSON text itself.
pub fn read_input(
    input: Option<&str>,
    env: &dyn Environment,
) -> Result<(String, InputSource), String> {
    match input {
        None => env
            .clipboard_text()
            .filter(|text| !text.trim().is_empty())
            .map(|text| (text, InputSource::Clipboard))
            .ok_or_else(|| "クリップボードに文字列がありません".to_string()),
        Some("-") => env
            .stdin_text()
            .map(|text| (text, InputSource::Stdin))
            .map_err(|e| format!("標準入力を読み取れません: {e}")),
        Some(value) if Path::new(value).is_file() => std::fs::read_to_string(value)
            .map(|text| (text, InputSource::File))
            .map_err(|e| format!("ファイルを読み取れません: {value}: {e}")),
        Some(value) => Ok((value.to_string(), InputSource::Literal)),
    }
}

/// Run a headless apply or dry-run described by `args`.
pub fn run(args: &Args, env: &dyn Environment) -> Report {
    let Mode::Headless(action) = args.mode() else {
        return Report::usage("--apply または --dry-run を指定してください");
    };

    let servers = match load_servers(args, env) {
        Ok(servers) => servers,
        Err(message) => return Report::usage(message),
    };

    if !args.allow_placeholders {
        let placeholders = find_placeholders(&servers);
        if !placeholders.is_empty() {
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
    }

    let targets = match select_targets(&args.targets, env.targets()) {
        Ok(targets) => targets,
        Err(message) => return Report::usage(message),
    };

    match action {
        HeadlessAction::DryRun => dry_run(&servers, &targets, args.show_secrets),
        HeadlessAction::Apply => apply(&servers, &targets, env),
    }
}

fn load_servers(args: &Args, env: &dyn Environment) -> Result<McpServersMap, String> {
    let (text, source) = read_input(args.input.as_deref(), env)?;
    let servers = parse_mcp_snippet(&text).map_err(|e| match source {
        InputSource::Literal => {
            format!("入力を解釈できません（ファイルが存在せず、JSON としても無効です）: {e}")
        }
        _ => format!("入力を解釈できません: {e}"),
    })?;

    let options = FilterOptions {
        replace_npx_with_bunx: !args.no_npx_to_bunx,
        replace_pipx_with_uvx: !args.no_pipx_to_uvx,
    };
    Ok(servers
        .into_iter()
        .map(|(name, config)| (name, apply_filters(config, &options)))
        .collect())
}

/// Without `--target`: every existing file target (CLI-delegated clients run only when named).
fn select_targets(
    names: &[String],
    detected: Vec<ClientTarget>,
) -> Result<Vec<ClientTarget>, String> {
    if names.is_empty() {
        let existing: Vec<_> = detected
            .into_iter()
            .filter(|t| t.kind.action() == TargetAction::FileMerge && t.exists)
            .collect();
        if existing.is_empty() {
            return Err(
                "適用先がありません: 既存の設定ファイルが見つかりません。--target で指定してください"
                    .to_string(),
            );
        }
        return Ok(existing);
    }

    let mut selected: Vec<ClientTarget> = Vec::new();
    for name in names {
        let kind = ClientTargetKind::from_cli_name(name)
            .ok_or_else(|| format!("不明なターゲット: {name}"))?;
        if selected.iter().any(|t| t.kind == kind) {
            continue;
        }
        let target = detected
            .iter()
            .find(|t| t.kind == kind)
            .cloned()
            .ok_or_else(|| format!("ターゲットの場所を特定できません: {name}"))?;
        selected.push(target);
    }
    Ok(selected)
}

/// A copy of `servers` with every env value replaced by [`MASK`].
fn mask_env(servers: &McpServersMap) -> McpServersMap {
    servers
        .iter()
        .map(|(name, config)| {
            let mut masked = config.clone();
            for value in masked.env.values_mut() {
                *value = MASK.to_string();
            }
            (name.clone(), masked)
        })
        .collect()
}

fn change_symbol(change: ServerChange) -> char {
    match change {
        ServerChange::New => '+',
        ServerChange::Updated => '~',
        ServerChange::Unchanged => '=',
    }
}

fn change_summary(changes: &ServerChanges) -> String {
    let count = |target: ServerChange| changes.values().filter(|c| **c == target).count();
    format!(
        "新規追加 {} / 上書き {} / 変更なし {}",
        count(ServerChange::New),
        count(ServerChange::Updated),
        count(ServerChange::Unchanged)
    )
}

fn finish(stdout: String, errors: Vec<String>, done_label: &str, done: usize) -> Report {
    let mut stdout = stdout;
    let _ = writeln!(stdout, "\n{done_label}: {done} 件");
    if errors.is_empty() {
        return Report {
            exit_code: EXIT_OK,
            stdout,
            stderr: String::new(),
        };
    }
    let _ = writeln!(stdout, "失敗: {} 件", errors.len());
    Report {
        exit_code: EXIT_FAILED,
        stdout,
        stderr: errors.iter().map(|e| format!("{e}\n")).collect(),
    }
}

fn dry_run(servers: &McpServersMap, targets: &[ClientTarget], show_secrets: bool) -> Report {
    // env の値（API キー等）をログやターミナル履歴に残さないよう、表示用は伏せる
    let shown = if show_secrets {
        servers.clone()
    } else {
        mask_env(servers)
    };

    let mut out = String::new();
    let _ = writeln!(out, "[dry-run] サーバー定義 {} 件:", servers.len());
    let _ = writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&shown).unwrap_or_default()
    );

    let mut errors = Vec::new();
    let mut ok = 0;
    for target in targets {
        let _ = writeln!(out, "\n[{}] {}", target.name, target.path.display());
        match target.kind.action() {
            TargetAction::FileMerge => match build_preview_with_changes(target, servers) {
                Ok((_, changes)) => {
                    ok += 1;
                    for (name, change) in changes.iter().flatten() {
                        let _ = writeln!(
                            out,
                            "  {} {}  {}",
                            change_symbol(*change),
                            name,
                            change.label()
                        );
                    }
                }
                Err(e) => errors.push(format!("[{}] {e}", target.name)),
            },
            TargetAction::CliCommand => {
                let program = target.path.to_string_lossy();
                match build_cli_invocations(target.kind, &program, &shown) {
                    Ok(invocations) => {
                        ok += 1;
                        for invocation in invocations {
                            let _ = writeln!(out, "  $ {}", invocation.display());
                        }
                    }
                    Err(e) => errors.push(format!("[{}] {e}", target.name)),
                }
            }
        }
    }
    let _ = writeln!(out, "\n書き込みはしていません（dry-run）");
    finish(out, errors, "検証OK", ok)
}

fn apply(servers: &McpServersMap, targets: &[ClientTarget], env: &dyn Environment) -> Report {
    let mut out = String::new();
    let mut errors = Vec::new();
    let mut ok = 0;

    for target in targets {
        match target.kind.action() {
            TargetAction::FileMerge => {
                let result =
                    build_preview_with_changes(target, servers).and_then(|(merged, changes)| {
                        safe_atomic_write(&target.path, &merged).map_err(|e| e.to_string())?;
                        Ok(changes)
                    });
                match result {
                    Ok(changes) => {
                        ok += 1;
                        let summary = changes.as_ref().map(change_summary).unwrap_or_default();
                        let _ = writeln!(
                            out,
                            "[{}] 適用: {} ({summary})",
                            target.name,
                            target.path.display()
                        );
                    }
                    Err(e) => errors.push(format!("[{}] {e}", target.name)),
                }
            }
            TargetAction::CliCommand => {
                let program = target.path.to_string_lossy();
                let result = build_cli_invocations(target.kind, &program, servers)
                    .and_then(|invocations| env.run_cli(&invocations).map(|()| invocations.len()));
                match result {
                    Ok(count) => {
                        ok += 1;
                        let _ = writeln!(out, "[{}] 実行: {count} コマンド", target.name);
                    }
                    Err(e) => errors.push(format!("[{}] {e}", target.name)),
                }
            }
        }
    }
    finish(out, errors, "適用完了", ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
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

    impl Environment for FakeEnv {
        fn clipboard_text(&self) -> Option<String> {
            self.clipboard.clone()
        }
        fn stdin_text(&self) -> std::io::Result<String> {
            Ok(self.stdin.clone())
        }
        fn targets(&self) -> Vec<ClientTarget> {
            self.targets.clone()
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
        crate::discovery::file_target(kind, path)
    }

    fn cli_target(kind: ClientTargetKind) -> ClientTarget {
        ClientTarget::new_cli(kind, Some(std::path::PathBuf::from("fake-cli")))
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

        let report = run(&args(&["--dry-run", SNIPPET]), &env);

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

        let masked = run(
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

        let shown = run(
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

        let report = run(&args(&["--apply", SNIPPET]), &env);

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
            let target = crate::discovery::file_target(ClientTargetKind::Cursor, target.path);
            let env = FakeEnv::new(vec![target.clone()]);

            let report = run(&args(&[flag, SNIPPET]), &env);

            assert_eq!(report.exit_code, EXIT_FAILED, "{flag}: {report:?}");
            assert!(
                report.stderr.contains("設定ファイルを読み取れません"),
                "{flag}: {}",
                report.stderr
            );
            assert_eq!(std::fs::read(&target.path).unwrap(), bytes, "{flag}");
            assert!(
                !crate::writer::backup_path_for(&target.path).exists(),
                "{flag}: .bak を作らない"
            );
        }
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

        let report = run(
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
            let blocked = run(&args(&[mode, snippet]), &env);
            assert_eq!(blocked.exit_code, EXIT_USAGE, "{mode}: {blocked:?}");
            assert!(blocked.stderr.contains("srv / TOKEN"), "{}", blocked.stderr);
            assert!(
                !blocked.stderr.contains("YOUR_API_KEY"),
                "値は出さない: {}",
                blocked.stderr
            );
        }
        assert_eq!(std::fs::read_to_string(&target.path).unwrap(), "{}");

        let allowed = run(&args(&["--apply", "--allow-placeholders", snippet]), &env);
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
            let report = run(&args(&argv), &env);
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
            Case {
                name: "GUI mode is not handled here",
                argv: vec![SNIPPET],
                use_env_without_existing_target: false,
                expected_stderr: "--apply または --dry-run",
            },
        ];

        for case in cases {
            let used = if case.use_env_without_existing_target {
                &no_targets
            } else {
                &env
            };
            let report = run(&args(&case.argv), used);
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

        let report = run(&argv, &env);

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

        let report = run(&args(&["--apply", SNIPPET]), &env);

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

        let report = run(&args(&["--apply", "--target", "codex-cli", SNIPPET]), &env);

        assert_eq!(report.exit_code, EXIT_FAILED, "{report:?}");
        assert!(report.stderr.contains("Codex CLI"), "{}", report.stderr);
    }

    #[test]
    fn test_filters_follow_flags() {
        let dir = tempdir().unwrap();
        let target = file_target(dir.path(), ClientTargetKind::Cursor, "mcp.json", Some("{}"));
        let env = FakeEnv::new(vec![target.clone()]);
        let snippet = r#"{"a":{"command":"npx","args":["-y","pkg"]},"b":{"command":"pipx","args":["run","tool"]}}"#;

        run(&args(&["--apply", snippet]), &env);
        let content = std::fs::read_to_string(&target.path).unwrap();
        assert!(
            content.contains("\"bunx\"") && content.contains("\"uvx\""),
            "{content}"
        );

        std::fs::write(&target.path, "{}").unwrap();
        run(
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
                    run(&args(&[mode, input]), &env),
                    run(&args(&[mode, "-"]), &stdin_env),
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
