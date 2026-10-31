//! Choosing which targets a headless run applies to.

use crate::domain::{ClientTarget, ClientTargetKind, TargetAction};

/// Without `--target`: every existing file target (CLI-delegated clients run only when named).
pub(super) fn select_targets(
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
