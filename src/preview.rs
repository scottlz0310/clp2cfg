//! Preview of what applying a snippet to a target would do (file merge result or CLI commands).

use crate::cli::{build_cli_invocations, preview_invocations};
use crate::merger::{classify_server_changes, merge_mcp_servers, ServerChange};
use crate::model::{ClientTarget, McpServersMap, TargetAction};
use std::collections::BTreeMap;

/// サーバー名ごとの判定
pub type ServerChanges = BTreeMap<String, ServerChange>;

/// ターゲットへの適用予定内容（ファイルマージ結果 or 実行予定 CLI コマンド）と、
/// ファイル系ターゲットでのサーバーごとの判定（新規追加 / 上書き / 変更なし）を生成する。
/// CLI 委譲ターゲットは既存の登録を事前に確認できないため、判定は `None`。
pub fn build_preview_with_changes(
    target: &ClientTarget,
    servers: &McpServersMap,
) -> Result<(String, Option<ServerChanges>), String> {
    match target.kind.action() {
        TargetAction::FileMerge => {
            let existing = if target.path.is_file() {
                std::fs::read_to_string(&target.path).ok()
            } else {
                None
            };
            let merged = merge_mcp_servers(target.kind, existing.as_deref(), servers)
                .map_err(|e| e.to_string())?;
            let changes = classify_server_changes(target.kind, existing.as_deref(), servers)
                .map_err(|e| e.to_string())?;
            Ok((merged, Some(changes)))
        }
        TargetAction::CliCommand => {
            let program = target.path.to_string_lossy();
            build_cli_invocations(target.kind, &program, servers)
                .map(|inv| (preview_invocations(&inv), None))
                .map_err(|e| e.to_string())
        }
    }
}

/// ターゲットへの適用予定内容だけが必要な場合（適用時）
pub fn build_preview(target: &ClientTarget, servers: &McpServersMap) -> Result<String, String> {
    build_preview_with_changes(target, servers).map(|(preview, _)| preview)
}
