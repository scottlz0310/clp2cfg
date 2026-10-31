use crate::app::{AppState, UiAction};

/// 左ペイン: 入力スニペット、プレースホルダーの実値入力、パース結果
pub fn input_pane(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    egui::Panel::left("left_pane")
        .resizable(true)
        .default_size(450.0)
        .min_size(300.0)
        .show(ui, |ui| {
            ui.heading("入力スニペット");
            ui.label(
                egui::RichText::new("JSON / Markdownコードブロックを貼り付け")
                    .small()
                    .color(egui::Color32::GRAY),
            );

            ui.add_space(4.0);

            let mut input = state.raw_input.clone();
            let response = ui.add(
                egui::TextEdit::multiline(&mut input)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16),
            );
            if response.changed() {
                actions.push(UiAction::SetInput(input));
            }

            actions.extend(placeholder_inputs(ui, state));

            ui.add_space(8.0);
            ui.separator();
            ui.heading("パース結果プレビュー");
            parsed_servers(ui, state);
        });
    actions
}

/// 未入力プレースホルダーの警告と、実値を入力するミニ入力欄
fn placeholder_inputs(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    if state.placeholder_fields().is_empty() {
        return actions;
    }

    ui.add_space(8.0);
    ui.separator();
    let unresolved = state.unresolved_placeholder_count();
    ui.horizontal(|ui| {
        if unresolved > 0 {
            ui.label(
                egui::RichText::new(format!("⚠ 未入力のプレースホルダー: {} 件", unresolved))
                    .color(egui::Color32::YELLOW),
            );
        } else {
            ui.label(
                egui::RichText::new("✓ プレースホルダーはすべて入力済み")
                    .color(egui::Color32::LIGHT_GREEN),
            );
        }
        let mut reveal = state.reveal_env_inputs;
        if ui.checkbox(&mut reveal, "値を表示").changed() {
            actions.push(UiAction::SetRevealEnv(reveal));
        }
    });

    egui::ScrollArea::vertical()
        .id_salt("placeholder_inputs")
        .max_height(120.0)
        .show(ui, |ui| {
            for field in state.placeholder_fields() {
                let mut value = state.placeholder_value(field);
                ui.horizontal(|ui| {
                    ui.label(format!("{} / {}", field.server, field.key));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut value)
                            .password(!state.reveal_env_inputs)
                            .hint_text("実値を入力")
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        actions.push(UiAction::SetPlaceholder {
                            field: field.clone(),
                            value,
                        });
                    }
                });
            }
        });
    actions
}

/// 認識したサーバー定義の一覧（パースできなければエラー）
fn parsed_servers(ui: &mut egui::Ui, state: &AppState) {
    match &state.parsed_config {
        Ok(servers) => {
            ui.label(
                egui::RichText::new(format!("✓ {} 件のサーバー定義を認識", servers.len()))
                    .color(egui::Color32::LIGHT_GREEN),
            );
            egui::ScrollArea::vertical()
                .id_salt("parsed_servers")
                .show(ui, |ui| {
                    for (name, config) in servers {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.strong(name);
                                ui.monospace(&config.command);
                            });
                            if !config.args.is_empty() {
                                ui.label(format!("引数: {}", config.args.join(" ")));
                            }
                            if !config.env.is_empty() {
                                ui.label(format!("環境変数: {} 件", config.env.len()));
                            }
                        });
                    }
                });
        }
        Err(err) => {
            ui.label(egui::RichText::new(err).color(egui::Color32::LIGHT_RED));
        }
    }
}
