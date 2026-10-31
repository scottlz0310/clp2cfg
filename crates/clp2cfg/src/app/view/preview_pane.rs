use crate::app::TargetEntry;
use clp2cfg_core::apply::{count_changes, ServerChanges};
use clp2cfg_core::domain::{ServerChange, TargetAction};

/// 選択中ターゲットのマージ差分（CLI 経由なら実行予定コマンド）
pub fn preview_pane(ui: &mut egui::Ui, entry: &TargetEntry) {
    let is_cli = entry.target.kind.action() == TargetAction::CliCommand;

    ui.horizontal(|ui| {
        let title = if is_cli {
            "実行予定コマンド"
        } else {
            "マージプレビュー"
        };
        ui.heading(format!("{}: {}", title, entry.target.name));
        ui.label(
            egui::RichText::new(format!("({})", entry.target.path.display()))
                .small()
                .color(egui::Color32::GRAY),
        );
    });

    if let Some(changes) = &entry.changes {
        ui.label(egui::RichText::new(change_summary(changes)).small());
        ui.horizontal_wrapped(|ui| {
            for (name, change) in changes {
                let (label, color) = change_badge(*change);
                ui.label(egui::RichText::new(format!("[{}]", label)).color(color));
                ui.label(name);
                ui.add_space(8.0);
            }
        });
    } else if is_cli && entry.diff_preview.is_some() {
        ui.label(
            egui::RichText::new(
                "CLI 経由のため、既存の登録の有無は事前に確認できません（同名のサーバーは上書きされます）",
            )
            .small()
            .color(egui::Color32::GRAY),
        );
    }

    if let Some(error) = &entry.error {
        ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
    } else if let Some(preview) = &entry.diff_preview {
        diff_preview(ui, preview, is_cli);
    } else {
        ui.label(
            egui::RichText::new(
                "左ペインに有効なMCPスニペットを入力すると、ここにマージ結果が表示されます",
            )
            .color(egui::Color32::GRAY),
        );
    }
}

fn diff_preview(ui: &mut egui::Ui, preview: &str, is_cli: bool) {
    egui::ScrollArea::vertical()
        .id_salt("diff_preview_scroll")
        .show(ui, |ui| {
            if is_cli {
                // 1 コマンドが長くなりがちなため、TextEdit ではなく折り返すラベルで全文を見せる
                for command in preview.lines() {
                    ui.add(egui::Label::new(egui::RichText::new(command).monospace()).wrap());
                    ui.add_space(6.0);
                }
            } else {
                ui.add(
                    egui::TextEdit::multiline(&mut &*preview)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            }
        });
}

/// 判定の表示ラベルと色
fn change_badge(change: ServerChange) -> (&'static str, egui::Color32) {
    let color = match change {
        ServerChange::New => egui::Color32::LIGHT_GREEN,
        ServerChange::Updated => egui::Color32::YELLOW,
        ServerChange::Unchanged => egui::Color32::GRAY,
    };
    (change.label(), color)
}

/// 判定ごとの件数の要約（例: `新規追加 1 件 / 上書き 2 件 / 変更なし 0 件`）
fn change_summary(changes: &ServerChanges) -> String {
    format!(
        "新規追加 {} 件 / 上書き {} 件 / 変更なし {} 件",
        count_changes(changes, ServerChange::New),
        count_changes(changes, ServerChange::Updated),
        count_changes(changes, ServerChange::Unchanged)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_badge_distinguishes_new_and_updated() {
        let (new_label, new_color) = change_badge(ServerChange::New);
        let (updated_label, updated_color) = change_badge(ServerChange::Updated);
        let (unchanged_label, _) = change_badge(ServerChange::Unchanged);

        assert_eq!(
            [new_label, updated_label, unchanged_label],
            ["新規追加", "上書き", "変更なし"]
        );
        assert_ne!(new_color, updated_color);
    }
}
