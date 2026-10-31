//! The output of a headless run: exit codes, the dry-run and apply listings, secret masking.

use crate::apply::{
    apply_targets, count_changes, plan_target, Applied, Mode as ApplyMode, ServerChanges, TargetIo,
    TargetPlan,
};
use crate::domain::{ClientTarget, McpServersMap, ServerChange, TargetAction};
use std::fmt::Write as _;

/// Everything succeeded.
pub const EXIT_OK: u8 = 0;
/// The run was valid but at least one target could not be applied.
pub const EXIT_FAILED: u8 = 1;
/// Bad usage or input (nothing was written).
pub const EXIT_USAGE: u8 = 2;

/// Replacement for env values in the `--dry-run` output.
const MASK: &str = "***";

/// Result of a headless run: what to print and the process exit code.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl Report {
    pub(super) fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_USAGE,
            stdout: String::new(),
            stderr: format!("{}\n", message.into()),
        }
    }
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
    format!(
        "新規追加 {} / 上書き {} / 変更なし {}",
        count_changes(changes, ServerChange::New),
        count_changes(changes, ServerChange::Updated),
        count_changes(changes, ServerChange::Unchanged)
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

pub(super) fn dry_run(
    servers: &McpServersMap,
    targets: &[ClientTarget],
    show_secrets: bool,
    io: &dyn TargetIo,
) -> Report {
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
        // 表示するコマンドは env の値を伏せたサーバーで作る。ファイルの判定は実際の値で行う
        let planned = match target.kind.action() {
            TargetAction::FileMerge => plan_target(target, servers, io),
            TargetAction::CliCommand => plan_target(target, &shown, io),
        };
        match planned {
            Ok(TargetPlan::File { changes, .. }) => {
                ok += 1;
                for (name, change) in &changes {
                    let _ = writeln!(
                        out,
                        "  {} {}  {}",
                        change_symbol(*change),
                        name,
                        change.label()
                    );
                }
            }
            Ok(TargetPlan::Cli { invocations }) => {
                ok += 1;
                for invocation in invocations {
                    let _ = writeln!(out, "  $ {}", invocation.display());
                }
            }
            Err(e) => errors.push(format!("[{}] {e}", target.name)),
        }
    }
    let _ = writeln!(out, "\n書き込みはしていません（dry-run）");
    finish(out, errors, "検証OK", ok)
}

pub(super) fn apply(
    servers: &McpServersMap,
    targets: &[ClientTarget],
    io: &dyn TargetIo,
) -> Report {
    let outcome = apply_targets(targets, servers, ApplyMode::Apply, io);

    let mut out = String::new();
    let mut errors = Vec::new();
    let mut ok = 0;
    for result in &outcome.results {
        match &result.result {
            Ok(Applied::File { changes, .. }) => {
                ok += 1;
                let _ = writeln!(
                    out,
                    "[{}] 適用: {} ({})",
                    result.name,
                    result.path.display(),
                    change_summary(changes)
                );
            }
            Ok(Applied::Cli { commands, .. }) => {
                ok += 1;
                let _ = writeln!(out, "[{}] 実行: {commands} コマンド", result.name);
            }
            Err(e) => errors.push(format!("[{}] {e}", result.name)),
        }
    }
    finish(out, errors, "適用完了", ok)
}
