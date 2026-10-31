use crate::app::{AppState, UiAction};

/// クリップボード読み取り・クリア・コマンドの置換フィルタ・自動読み取りのトグル
pub fn top_bar(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    egui::Panel::top("top_panel").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.heading("clp2cfg");
            ui.label(
                egui::RichText::new("MCP Server Config Merger")
                    .small()
                    .color(egui::Color32::GRAY),
            );

            ui.separator();

            if ui
                .button("📋 読み取り (Ctrl+V)")
                .on_hover_text("クリップボードから設定スニペットを読み取ります (Ctrl+V)")
                .clicked()
            {
                actions.push(UiAction::ReadClipboard);
            }

            if ui
                .button("🗑 クリア")
                .on_hover_text("入力欄をクリアします")
                .clicked()
            {
                actions.push(UiAction::Clear);
            }

            ui.separator();

            let mut replace = state.replace_npx_with_bunx;
            if ui.checkbox(&mut replace, "npx → bunx 置換").changed() {
                actions.push(UiAction::SetReplaceNpxWithBunx(replace));
            }

            let mut replace_pipx = state.replace_pipx_with_uvx;
            if ui
                .checkbox(&mut replace_pipx, "pipx run → uvx 置換")
                .on_hover_text(
                    "`pipx run パッケージ名 …` を `uvx パッケージ名 …` に書き換えます（uv が必要）。\n\
                     --spec 等のオプション・URL・バージョン指定つきは書き換えません。",
                )
                .changed()
            {
                actions.push(UiAction::SetReplacePipxWithUvx(replace_pipx));
            }

            let mut auto_read = state.auto_read_clipboard;
            if ui.checkbox(&mut auto_read, "自動読み取り").changed() {
                actions.push(UiAction::SetAutoRead(auto_read));
            }
        });
    });
    actions
}
