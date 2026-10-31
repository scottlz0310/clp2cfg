use super::preview_pane::preview_pane;
use crate::app::{AppState, TargetEntry, UiAction};
use clp2cfg_core::domain::TargetAction;

/// ターゲット一覧が右ペインの高さに占める上限の割合（残りは差分プレビューに使う）
const TARGET_LIST_HEIGHT_RATIO: f32 = 0.5;

/// 右ペイン: ターゲットクライアントの一覧と、選択中のターゲットのマージプレビュー
pub fn target_pane(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("ターゲットクライアント");
        ui.label(
            egui::RichText::new("設定ファイルを更新する対象を選択してください")
                .small()
                .color(egui::Color32::GRAY),
        );

        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .id_salt("target_list")
            .max_height(ui.available_height() * TARGET_LIST_HEIGHT_RATIO)
            .show(ui, |ui| {
                for (idx, entry) in state.target_entries.iter().enumerate() {
                    let is_active = idx == state.selected_target_idx;
                    actions.extend(target_row(ui, idx, entry, is_active));
                }
            });

        ui.add_space(8.0);
        ui.separator();

        if let Some(entry) = state.target_entries.get(state.selected_target_idx) {
            preview_pane(ui, entry);
        }
    });
    actions
}

/// 1 ターゲットの行: チェック、種別バッジ、名前、上書き件数、パス
fn target_row(
    ui: &mut egui::Ui,
    idx: usize,
    entry: &TargetEntry,
    is_active: bool,
) -> Vec<UiAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        let mut selected = entry.selected;
        if ui.checkbox(&mut selected, "").changed() {
            actions.push(UiAction::SetTargetSelected { idx, selected });
        }

        ui.label(target_badge(entry));

        let text = egui::RichText::new(&entry.target.name);
        let text = if is_active { text.strong() } else { text };
        if ui.selectable_label(is_active, text).clicked() {
            actions.push(UiAction::SelectTarget(idx));
        }

        let overwrites = entry.overwrite_count();
        if overwrites > 0 {
            ui.label(
                egui::RichText::new(format!("⚠ 上書き {} 件", overwrites))
                    .small()
                    .color(egui::Color32::YELLOW),
            );
        }

        // 長いパスが右ペインの幅を押し広げると、下のプレビューの折り返し幅も
        // 見えている幅より広くなって見切れるため、残り幅で省略する
        let path = entry.target.path.to_string_lossy();
        ui.add(
            egui::Label::new(
                egui::RichText::new(path.as_ref())
                    .small()
                    .color(egui::Color32::GRAY),
            )
            .truncate(),
        )
        .on_hover_text(path.as_ref());
    });
    actions
}

/// 種別と存在の表示（`[既存]` / `[新規]` / `[CLI検出]` / `[CLI未検出]`）
fn target_badge(entry: &TargetEntry) -> egui::RichText {
    let exists = entry.target.exists;
    let is_cli = entry.target.kind.action() == TargetAction::CliCommand;
    if is_cli && exists {
        egui::RichText::new("[CLI検出]").color(egui::Color32::LIGHT_GREEN)
    } else if is_cli {
        egui::RichText::new("[CLI未検出]").color(egui::Color32::GRAY)
    } else if exists {
        egui::RichText::new("[既存]").color(egui::Color32::LIGHT_GREEN)
    } else {
        egui::RichText::new("[新規]").color(egui::Color32::GRAY)
    }
}
