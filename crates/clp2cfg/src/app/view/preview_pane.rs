use crate::app::{AppState, PreviewTab, TargetEntry, UiAction};
use clp2cfg_core::apply::{count_changes, ExistingTargetState, ServerChanges, ServerLineRanges};
use clp2cfg_core::delegate::plan::{CliInvocation, CliOperation};
use clp2cfg_core::domain::{ServerChange, TargetAction};
use clp2cfg_core::merge::{ExistingConfigSnapshot, LineRange};
use std::collections::HashMap;

/// CLI サーバー識別用のアクセント色パレット（暗い背景でも視認しやすい色）
const CLI_SERVER_COLORS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(120, 210, 255),
    egui::Color32::from_rgb(150, 235, 160),
    egui::Color32::from_rgb(255, 215, 120),
    egui::Color32::from_rgb(215, 165, 255),
    egui::Color32::from_rgb(255, 165, 165),
    egui::Color32::from_rgb(130, 235, 220),
];

/// 選択中ターゲットのマージ差分（CLI 経由なら実行予定コマンド）
pub fn preview_pane(
    ui: &mut egui::Ui,
    state: &AppState,
    target_idx: usize,
    entry: &TargetEntry,
) -> Vec<UiAction> {
    let mut actions = Vec::new();
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

    let active_jump = state
        .preview_jump_request
        .as_ref()
        .filter(|req| req.target_idx == target_idx);
    let highlighted_server = active_jump.map(|req| req.server.as_str());

    if !is_cli {
        ui.horizontal(|ui| {
            let merge_selected = state.preview_tab == PreviewTab::MergePreview;
            if ui
                .selectable_label(merge_selected, "マージプレビュー")
                .clicked()
            {
                actions.push(UiAction::SetPreviewTab(PreviewTab::MergePreview));
            }
            let existing_label = match &entry.existing_config {
                Some(ExistingTargetState::Loaded(snap)) => {
                    format!("既存設定 ({}件)", snap.server_names.len())
                }
                Some(ExistingTargetState::Error(_)) => "既存設定 (エラー)".to_string(),
                _ => "既存設定".to_string(),
            };
            let existing_selected = state.preview_tab == PreviewTab::ExistingConfig;
            if ui
                .selectable_label(existing_selected, existing_label)
                .clicked()
            {
                actions.push(UiAction::SetPreviewTab(PreviewTab::ExistingConfig));
            }
        });
        ui.add_space(4.0);
    }

    let jump_state_id = ui.make_persistent_id(("preview_jump_handled_seq", target_idx));
    let last_handled_seq: u64 = ui.data(|d| d.get_temp(jump_state_id).unwrap_or(0));
    let pending_jump = active_jump.filter(|req| req.seq != last_handled_seq);

    if !is_cli && state.preview_tab == PreviewTab::ExistingConfig {
        let (tab_actions, jumped_seq) = render_existing_config_pane(
            ui,
            state.reveal_env_inputs,
            entry.existing_config.as_ref(),
            highlighted_server,
            pending_jump.map(|req| (req.server.as_str(), req.seq)),
        );
        actions.extend(tab_actions);
        if let Some(seq) = jumped_seq {
            ui.data_mut(|d| d.insert_temp(jump_state_id, seq));
        }
        return actions;
    }

    if let Some(changes) = &entry.changes {
        ui.label(egui::RichText::new(change_summary(changes)).small());
        ui.horizontal_wrapped(|ui| {
            for (name, change) in changes {
                let (label, color) = change_badge(*change);
                let is_focused = highlighted_server == Some(name.as_str());
                let button = jump_badge_button(ui, label, color, name, is_focused);
                if button
                    .on_hover_text("クリックしてプレビュー内の該当箇所へジャンプ")
                    .clicked()
                {
                    actions.push(UiAction::JumpToPreviewServer(name.clone()));
                }
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
        if let Some(invocations) = entry.cli_invocations.as_deref() {
            let groups = group_cli_invocations_by_server(invocations);
            if !groups.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (idx, (server_name, cmds)) in groups.iter().enumerate() {
                        let color = cli_server_color(idx);
                        let is_focused = highlighted_server == Some(*server_name);
                        let badge = format!("CLI {}件", cmds.len());
                        let button = jump_badge_button(ui, &badge, color, server_name, is_focused);
                        if button
                            .on_hover_text("クリックして実行予定コマンドの該当箇所へジャンプ")
                            .clicked()
                        {
                            actions.push(UiAction::JumpToPreviewServer((*server_name).to_string()));
                        }
                    }
                });
            }
        }
    }

    if let Some(error) = &entry.error {
        ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
    } else if let Some(preview) = &entry.diff_preview {
        let mut jumped_seq = None;
        egui::ScrollArea::vertical()
            .id_salt("diff_preview_scroll")
            .show(ui, |ui| {
                if is_cli {
                    if let Some(invocations) = entry.cli_invocations.as_deref() {
                        jumped_seq = render_cli_preview(
                            ui,
                            invocations,
                            highlighted_server,
                            pending_jump.map(|req| (req.server.as_str(), req.seq)),
                        );
                    } else {
                        for command in preview.lines() {
                            ui.add(
                                egui::Label::new(egui::RichText::new(command).monospace())
                                    .wrap()
                                    .selectable(true),
                            );
                            ui.add_space(6.0);
                        }
                    }
                } else {
                    jumped_seq = render_file_preview(
                        ui,
                        preview,
                        entry.changes.as_ref(),
                        entry.server_lines.as_ref(),
                        highlighted_server,
                        pending_jump.map(|req| (req.server.as_str(), req.seq)),
                    );
                }
            });

        if let Some(seq) = jumped_seq {
            ui.data_mut(|d| d.insert_temp(jump_state_id, seq));
        }
    } else {
        ui.label(
            egui::RichText::new(
                "左ペインに有効なMCPスニペットを入力すると、ここにマージ結果が表示されます",
            )
            .color(egui::Color32::GRAY),
        );
    }

    actions
}

/// 既存設定ファイルと登録済み MCP サーバー一覧の表示（ファイル系ターゲットのみ）
fn render_existing_config_pane(
    ui: &mut egui::Ui,
    reveal_env_inputs: bool,
    existing: Option<&ExistingTargetState>,
    highlighted_server: Option<&str>,
    pending_jump: Option<(&str, u64)>,
) -> (Vec<UiAction>, Option<u64>) {
    let mut actions = Vec::new();
    match existing {
        Some(ExistingTargetState::Loaded(snapshot)) => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "登録済み MCP サーバー: {} 件",
                        snapshot.server_names.len()
                    ))
                    .small(),
                );
                let mut reveal = reveal_env_inputs;
                if ui
                    .checkbox(&mut reveal, "機密値（env / headers）を表示")
                    .changed()
                {
                    actions.push(UiAction::SetRevealEnv(reveal));
                }
            });

            if snapshot.server_names.is_empty() {
                ui.label(
                    egui::RichText::new("この設定ファイルには MCP サーバーが登録されていません。")
                        .small()
                        .color(egui::Color32::GRAY),
                );
            } else {
                ui.horizontal_wrapped(|ui| {
                    for name in &snapshot.server_names {
                        let is_focused = highlighted_server == Some(name.as_str());
                        let button = jump_badge_button(
                            ui,
                            "登録済",
                            egui::Color32::LIGHT_BLUE,
                            name,
                            is_focused,
                        );
                        if button
                            .on_hover_text("クリックして既存設定内の該当箇所へジャンプ")
                            .clicked()
                        {
                            actions.push(UiAction::JumpToPreviewServer(name.clone()));
                        }
                    }
                });
            }

            let mut jumped_seq = None;
            egui::ScrollArea::vertical()
                .id_salt("existing_config_scroll")
                .show(ui, |ui| {
                    jumped_seq = render_existing_json_view(
                        ui,
                        snapshot,
                        reveal_env_inputs,
                        highlighted_server,
                        pending_jump,
                    );
                });
            (actions, jumped_seq)
        }
        Some(ExistingTargetState::Missing) => {
            ui.label(
                egui::RichText::new("設定ファイルはまだ作成されていません。")
                    .color(egui::Color32::GRAY),
            );
            (actions, pending_jump.map(|(_, seq)| seq))
        }
        Some(ExistingTargetState::Error(err)) => {
            ui.label(
                egui::RichText::new(format!("既存設定の読み込みに失敗しました: {err}"))
                    .color(egui::Color32::LIGHT_RED),
            );
            (actions, pending_jump.map(|(_, seq)| seq))
        }
        None => {
            ui.label(
                egui::RichText::new("CLI 経由のため、既存の登録の有無は事前に確認できません。")
                    .color(egui::Color32::GRAY),
            );
            (actions, pending_jump.map(|(_, seq)| seq))
        }
    }
}

fn render_existing_json_view(
    ui: &mut egui::Ui,
    snapshot: &ExistingConfigSnapshot,
    reveal_env_inputs: bool,
    highlighted_server: Option<&str>,
    pending_jump: Option<(&str, u64)>,
) -> Option<u64> {
    let text = if reveal_env_inputs {
        snapshot.raw_json.as_str()
    } else {
        snapshot.masked_json.as_str()
    };
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let default_color = ui.visuals().text_color();

    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = build_existing_config_layout_job(
            buf.as_str(),
            &snapshot.server_lines,
            highlighted_server,
            font_id.clone(),
            default_color,
            wrap_width,
        );
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let output = egui::TextEdit::multiline(&mut &*text)
        .font(egui::TextStyle::Monospace)
        .code_editor()
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter)
        .show(ui);

    if let Some((server, seq)) = pending_jump {
        if let Some(range) = snapshot.server_lines.get(server).copied() {
            if let Some((start_char, end_char)) = line_range_char_span(text, range) {
                let start_rect = output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(start_char));
                let end_rect = output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(end_char));
                let local_rect = start_rect.union(end_rect);
                let target_rect = local_rect.translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(target_rect, Some(egui::Align::TOP));
            }
        }
        return Some(seq);
    }

    None
}

fn build_existing_config_layout_job(
    text: &str,
    server_lines: &ServerLineRanges,
    highlighted_server: Option<&str>,
    font_id: egui::FontId,
    default_color: egui::Color32,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let focused_range = highlighted_server.and_then(|name| server_lines.get(name).copied());

    for (line_idx, segment) in text.split_inclusive('\n').enumerate() {
        let is_focused = focused_range.is_some_and(|r| line_idx >= r.start && line_idx <= r.end);
        let (color, background) = if is_focused {
            (
                egui::Color32::LIGHT_BLUE,
                egui::Color32::from_rgba_unmultiplied(120, 200, 255, 38),
            )
        } else {
            (default_color, egui::Color32::TRANSPARENT)
        };
        job.append(
            segment,
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                background,
                ..Default::default()
            },
        );
    }

    job
}

fn jump_badge_button(
    ui: &mut egui::Ui,
    badge: &str,
    badge_color: egui::Color32,
    server_name: &str,
    is_focused: bool,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    job.append(
        &format!("[{badge}] "),
        0.0,
        egui::TextFormat {
            font_id: font_id.clone(),
            color: badge_color,
            ..Default::default()
        },
    );
    job.append(
        server_name,
        0.0,
        egui::TextFormat {
            font_id,
            color: ui.visuals().text_color(),
            ..Default::default()
        },
    );
    let mut btn = egui::Button::new(job);
    if is_focused {
        btn = btn.stroke(egui::Stroke::new(1.5, badge_color));
    }
    ui.add(btn)
}

fn render_file_preview(
    ui: &mut egui::Ui,
    preview: &str,
    changes: Option<&ServerChanges>,
    server_lines: Option<&ServerLineRanges>,
    highlighted_server: Option<&str>,
    pending_jump: Option<(&str, u64)>,
) -> Option<u64> {
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let default_color = ui.visuals().text_color();

    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let job = build_file_preview_layout_job(
            buf.as_str(),
            changes,
            server_lines,
            highlighted_server,
            font_id.clone(),
            default_color,
            wrap_width,
        );
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let output = egui::TextEdit::multiline(&mut &*preview)
        .font(egui::TextStyle::Monospace)
        .code_editor()
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter)
        .show(ui);

    if let Some((server, seq)) = pending_jump {
        if let Some(range) = server_lines.and_then(|map| map.get(server)).copied() {
            if let Some((start_char, end_char)) = line_range_char_span(preview, range) {
                let start_rect = output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(start_char));
                let end_rect = output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(end_char));
                let local_rect = start_rect.union(end_rect);
                let target_rect = local_rect.translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(target_rect, Some(egui::Align::TOP));
            }
        }
        return Some(seq);
    }

    None
}

fn render_cli_preview(
    ui: &mut egui::Ui,
    invocations: &[CliInvocation],
    highlighted_server: Option<&str>,
    pending_jump: Option<(&str, u64)>,
) -> Option<u64> {
    let groups = group_cli_invocations_by_server(invocations);
    let mut jumped_seq = None;

    for (idx, (server_name, cmds)) in groups.iter().enumerate() {
        let accent = cli_server_color(idx);
        let is_focused = highlighted_server == Some(*server_name);
        let fill = egui::Color32::from_rgba_unmultiplied(
            accent.r(),
            accent.g(),
            accent.b(),
            if is_focused { 36 } else { 18 },
        );
        let stroke_width = if is_focused { 1.5 } else { 1.0 };
        let frame = egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(stroke_width, accent.linear_multiply(0.6)));

        let response = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(*server_name).strong().color(accent));
                    ui.label(
                        egui::RichText::new(format!("({} コマンド)", cmds.len()))
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                });
                ui.add_space(4.0);
                for inv in cmds {
                    let (op_label, op_color) = cli_operation_badge(inv.operation);
                    ui.label(
                        egui::RichText::new(format!("[{op_label}]"))
                            .small()
                            .color(op_color),
                    );
                    // 1 コマンドが長くなりがちなため、TextEdit ではなく折り返すラベルで全文を見せる
                    ui.add(
                        egui::Label::new(egui::RichText::new(inv.display()).monospace())
                            .wrap()
                            .selectable(true),
                    );
                    ui.add_space(4.0);
                }
            })
            .response;

        if let Some((target_server, seq)) = pending_jump {
            if target_server == *server_name {
                ui.scroll_to_rect(response.rect, Some(egui::Align::TOP));
                jumped_seq = Some(seq);
            }
        }

        ui.add_space(6.0);
    }

    // ジャンプ対象が見つからなくても再試行し続けないよう消費する
    jumped_seq.or_else(|| pending_jump.map(|(_, seq)| seq))
}

fn build_file_preview_layout_job(
    text: &str,
    changes: Option<&ServerChanges>,
    server_lines: Option<&ServerLineRanges>,
    highlighted_server: Option<&str>,
    font_id: egui::FontId,
    default_color: egui::Color32,
    wrap_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let line_styles = build_line_styles(text, changes, server_lines, highlighted_server);

    for (line_idx, segment) in text.split_inclusive('\n').enumerate() {
        let (color, background) = match line_styles.get(line_idx).copied().flatten() {
            Some((change, is_focused)) => preview_line_colors(change, is_focused),
            None => (default_color, egui::Color32::TRANSPARENT),
        };
        job.append(
            segment,
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color,
                background,
                ..Default::default()
            },
        );
    }

    job
}

/// 各行（0-based）に対応する `(ServerChange, is_focused)` を前計算する
fn build_line_styles(
    text: &str,
    changes: Option<&ServerChanges>,
    server_lines: Option<&ServerLineRanges>,
    highlighted_server: Option<&str>,
) -> Vec<Option<(ServerChange, bool)>> {
    let line_count = text.split_inclusive('\n').count();
    let mut styles = vec![None; line_count];
    let (Some(changes), Some(server_lines)) = (changes, server_lines) else {
        return styles;
    };

    for (name, change) in changes {
        let Some(range) = server_lines.get(name) else {
            continue;
        };
        let is_focused = highlighted_server == Some(name.as_str());
        let end = range.end.min(line_count.saturating_sub(1));
        for style in styles.iter_mut().take(end + 1).skip(range.start) {
            *style = Some((*change, is_focused));
        }
    }

    styles
}

fn preview_line_colors(change: ServerChange, is_focused: bool) -> (egui::Color32, egui::Color32) {
    let alpha = if is_focused { 52 } else { 24 };
    match change {
        ServerChange::New => (
            egui::Color32::LIGHT_GREEN,
            egui::Color32::from_rgba_unmultiplied(80, 200, 120, alpha),
        ),
        ServerChange::Updated => (
            egui::Color32::YELLOW,
            egui::Color32::from_rgba_unmultiplied(230, 190, 60, alpha),
        ),
        ServerChange::Unchanged => (
            egui::Color32::GRAY,
            if is_focused {
                egui::Color32::from_rgba_unmultiplied(160, 160, 160, 36)
            } else {
                egui::Color32::TRANSPARENT
            },
        ),
    }
}

/// 指定行範囲（0-based inclusive）の開始・終了文字インデックス（`char` 単位）を返す
fn line_range_char_span(text: &str, range: LineRange) -> Option<(usize, usize)> {
    let mut char_offset = 0usize;
    let mut start_char = None;
    let mut end_char = None;

    for (line_idx, segment) in text.split_inclusive('\n').enumerate() {
        let seg_chars = segment.chars().count();
        let line_content_chars = segment.trim_end_matches(['\r', '\n']).chars().count();
        if line_idx == range.start {
            start_char = Some(char_offset);
        }
        if line_idx == range.end {
            end_char = Some(char_offset + line_content_chars);
            break;
        }
        char_offset += seg_chars;
    }

    start_char.zip(end_char)
}

/// CLI 実行予定コマンドをサーバー単位にグルーピングする（登場順を維持）
fn group_cli_invocations_by_server(
    invocations: &[CliInvocation],
) -> Vec<(&str, Vec<&CliInvocation>)> {
    let mut groups: Vec<(&str, Vec<&CliInvocation>)> = Vec::new();
    let mut index_by_name: HashMap<&str, usize> = HashMap::new();

    for inv in invocations {
        let name = inv.server_name.as_str();
        if let Some(&idx) = index_by_name.get(name) {
            groups[idx].1.push(inv);
        } else {
            let idx = groups.len();
            index_by_name.insert(name, idx);
            groups.push((name, vec![inv]));
        }
    }

    groups
}

fn cli_server_color(index: usize) -> egui::Color32 {
    CLI_SERVER_COLORS[index % CLI_SERVER_COLORS.len()]
}

fn cli_operation_badge(op: CliOperation) -> (&'static str, egui::Color32) {
    match op {
        CliOperation::Remove => ("remove", egui::Color32::from_rgb(255, 160, 120)),
        CliOperation::Add => ("add", egui::Color32::LIGHT_GREEN),
    }
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
    use clp2cfg_core::delegate::plan::build_cli_invocations;
    use clp2cfg_core::domain::{ClientTargetKind, McpServersMap, NormalizedServerConfig};

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

    #[test]
    fn test_build_file_preview_layout_job_highlights_each_change_kind() {
        let json = "{\n  \"mcpServers\": {\n    \"new-srv\": {\n      \"command\": \"node\"\n    },\n    \"upd-srv\": {\n      \"command\": \"uvx\"\n    },\n    \"same-srv\": {\n      \"command\": \"echo\"\n    },\n    \"keep-srv\": {\n      \"command\": \"python\"\n    }\n  }\n}";
        let changes = ServerChanges::from([
            ("new-srv".to_string(), ServerChange::New),
            ("upd-srv".to_string(), ServerChange::Updated),
            ("same-srv".to_string(), ServerChange::Unchanged),
        ]);
        let server_lines = ServerLineRanges::from([
            ("new-srv".to_string(), LineRange { start: 2, end: 4 }),
            ("upd-srv".to_string(), LineRange { start: 5, end: 7 }),
            ("same-srv".to_string(), LineRange { start: 8, end: 10 }),
            ("keep-srv".to_string(), LineRange { start: 11, end: 13 }),
        ]);

        let job = build_file_preview_layout_job(
            json,
            Some(&changes),
            Some(&server_lines),
            Some("upd-srv"),
            egui::FontId::monospace(14.0),
            egui::Color32::WHITE,
            600.0,
        );

        assert_eq!(job.text, json);

        let format_at = |needle: &str| {
            let offset = egui::text::ByteIndex(json.find(needle).unwrap());
            &job.sections
                .iter()
                .find(|s| s.byte_range.contains(&offset))
                .unwrap()
                .format
        };

        assert_eq!(format_at("\"mcpServers\"").color, egui::Color32::WHITE);
        assert_eq!(format_at("\"new-srv\"").color, egui::Color32::LIGHT_GREEN);
        assert_eq!(format_at("\"upd-srv\"").color, egui::Color32::YELLOW);
        assert!(format_at("\"upd-srv\"").background.a() > format_at("\"new-srv\"").background.a());
        assert_eq!(format_at("\"same-srv\"").color, egui::Color32::GRAY);
        // 既存のまま残る対象外サーバーは通常色
        assert_eq!(format_at("\"keep-srv\"").color, egui::Color32::WHITE);
    }

    #[test]
    fn test_line_range_char_span_handles_multibyte_characters() {
        let text = "{\n  \"日本語\": {\n    \"command\": \"node\"\n  }\n}";
        let (start, end) = line_range_char_span(text, LineRange { start: 1, end: 3 }).unwrap();
        let chars: Vec<char> = text.chars().collect();
        let slice: String = chars[start..end].iter().collect();
        assert!(slice.starts_with("  \"日本語\": {"));
        assert!(slice.ends_with("  }"));
    }

    #[test]
    fn test_group_cli_invocations_by_server_preserves_order() {
        let servers = McpServersMap::from([
            ("alpha".to_string(), NormalizedServerConfig::new("node")),
            ("beta".to_string(), NormalizedServerConfig::new("uvx")),
        ]);
        let invs = build_cli_invocations(ClientTargetKind::ClaudeCli, "claude", &servers).unwrap();
        let groups = group_cli_invocations_by_server(&invs);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "alpha");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[0].1[0].operation, CliOperation::Remove);
        assert_eq!(groups[0].1[1].operation, CliOperation::Add);
        assert_eq!(groups[1].0, "beta");
        assert_eq!(groups[1].1.len(), 2);
    }
}
