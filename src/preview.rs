//! Preview of what applying a snippet to a target would do (file merge result or CLI commands).

use crate::delegate::plan::{build_cli_invocations, preview_invocations};
use crate::domain::{ClientTarget, McpServersMap, ServerChange, TargetAction};
use crate::merge::{classify_server_changes, merge_mcp_servers};
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
            // 読めない既存ファイルを「無い」扱いにすると、他の設定を消して上書きしてしまう
            let existing = if target.path.is_file() {
                Some(std::fs::read_to_string(&target.path).map_err(|e| {
                    format!(
                        "設定ファイルを読み取れません: {}: {e}",
                        target.path.display()
                    )
                })?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ClientTargetKind, NormalizedServerConfig};
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
            let target = crate::discovery::file_target(ClientTargetKind::Project, path.clone());

            let err = build_preview_with_changes(&target, &servers()).unwrap_err();

            assert!(
                err.contains("設定ファイルを読み取れません") && err.contains(".mcp.json"),
                "{label}: {err}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "{label}");
        }
    }

    #[test]
    fn test_missing_file_is_previewed_as_new() {
        let dir = tempdir().unwrap();
        let target =
            crate::discovery::file_target(ClientTargetKind::Project, dir.path().join(".mcp.json"));

        let (preview, changes) = build_preview_with_changes(&target, &servers()).unwrap();

        assert!(preview.contains("new-one"));
        assert_eq!(changes.unwrap().get("new-one"), Some(&ServerChange::New));
    }

    #[test]
    fn test_readable_existing_file_is_merged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"keep-me":{"command":"node"}}}"#).unwrap();
        let target = crate::discovery::file_target(ClientTargetKind::Project, path);

        let (preview, _) = build_preview_with_changes(&target, &servers()).unwrap();

        assert!(preview.contains("keep-me") && preview.contains("new-one"));
    }
}
