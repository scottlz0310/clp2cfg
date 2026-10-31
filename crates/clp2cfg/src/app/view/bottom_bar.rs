use crate::app::{AppState, UiAction};

/// 適用対象の要約・ステータス・適用／検証／取り消しボタン。
/// 長いステータス（警告つき等）がボタンに隠れないよう、左側だけを縮める。
pub fn bottom_bar(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let (selection, cli_count) = state.selection_summary();
    let undo_hint = state.undo_hint();
    egui::Panel::bottom("bottom_panel").show(ui, |ui| {
        // CLI（サブプロセス実行）を含むときは目を引く色にする
        let selection_color = if cli_count > 0 {
            egui::Color32::LIGHT_YELLOW
        } else {
            egui::Color32::GRAY
        };
        ui.add(egui::Label::new(egui::RichText::new(&selection).color(selection_color)).truncate())
            .on_hover_text(&selection);
        egui::Sides::new().shrink_left().truncate().show(
            ui,
            |ui| status(ui, state),
            |ui| actions.extend(buttons(ui, state, undo_hint.as_deref())),
        );
    });
    actions
}

fn status(ui: &mut egui::Ui, state: &AppState) {
    if let Some((msg, is_err)) = &state.status_message {
        let color = if *is_err {
            egui::Color32::LIGHT_RED
        } else {
            egui::Color32::LIGHT_GREEN
        };
        ui.label(egui::RichText::new(msg).color(color))
            .on_hover_text(msg);
    } else {
        ui.label(
            egui::RichText::new("スニペットを貼り付けてターゲットを選択してください")
                .color(egui::Color32::GRAY),
        );
    }
}

fn buttons(ui: &mut egui::Ui, state: &AppState, undo_hint: Option<&str>) -> Vec<UiAction> {
    let mut actions = Vec::new();
    let can_apply = state.can_apply();

    if ui
        .add_enabled(can_apply, egui::Button::new("💾 適用 (Ctrl+Enter)"))
        .on_hover_text("選択したターゲットにマージを適用します (Ctrl+Enter)")
        .clicked()
    {
        actions.push(UiAction::Apply);
    }

    if ui
        .add_enabled(can_apply, egui::Button::new("🔍 検証 (Dry-Run)"))
        .on_hover_text("ファイル書き込みを行わずにマージ結果を検証します")
        .clicked()
    {
        actions.push(UiAction::DryRun);
    }

    let undo_button = ui.add_enabled(
        undo_hint.is_some(),
        egui::Button::new("↩ 直前の適用を取り消す"),
    );
    if undo_button.clicked() {
        actions.push(UiAction::Undo);
    }
    undo_button.on_hover_text(
        undo_hint.unwrap_or("取り消せる適用がありません（適用すると有効になります）"),
    );

    ui.label(
        egui::RichText::new("[Esc] 閉じる")
            .small()
            .color(egui::Color32::GRAY),
    );
    actions
}
