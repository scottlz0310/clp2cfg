mod action;
mod clipboard;
mod ports;
mod state;
mod system;

pub use action::UiAction;
pub use clipboard::ArboardClipboard;
pub use ports::ClipboardSource;
pub use state::{AppState, StartupOptions, TargetEntry};
pub use system::load_app_icon;

use crate::apply::{
    count_changes, ServerChanges, SystemCatalog, SystemIo, TargetCatalog, TargetIo,
};
use crate::domain::{ServerChange, TargetAction};
use system::{setup_system_fonts, AUTO_READ_STORAGE_KEY};

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

/// ターゲット一覧が右ペインの高さに占める上限の割合（残りは差分プレビューに使う）
const TARGET_LIST_HEIGHT_RATIO: f32 = 0.5;

/// clp2cfg のウィンドウ。状態は [`AppState`]、ここは egui への接続（描画・ショートカット・保存）を担う
pub struct Clp2cfgApp {
    pub state: AppState,
    /// 前回フレームでのウィンドウフォーカス状態
    pub was_focused: bool,
}

impl Default for Clp2cfgApp {
    fn default() -> Self {
        Self::with_ports(
            SystemCatalog.targets(),
            Box::new(SystemIo),
            Box::new(ArboardClipboard),
        )
    }
}

impl Clp2cfgApp {
    /// 指定されたターゲット一覧とクリップボード・ファイル I/O で初期化
    pub fn with_ports(
        targets: Vec<crate::domain::ClientTarget>,
        io: Box<dyn TargetIo>,
        clipboard: Box<dyn ClipboardSource>,
    ) -> Self {
        Self {
            state: AppState::new(targets, io, clipboard),
            was_focused: false,
        }
    }

    /// eframe 用のコンストラクタ（フォント設定、保存済みトグルの復元、起動引数の反映、起動時のクリップボード自動読み取り）
    pub fn new(cc: &eframe::CreationContext<'_>, startup: StartupOptions) -> Self {
        setup_system_fonts(&cc.egui_ctx);
        let mut app = Self::default();
        if let Some(enabled) = cc
            .storage
            .and_then(|s| eframe::get_value(s, AUTO_READ_STORAGE_KEY))
        {
            app.state.auto_read_clipboard = enabled;
        }
        app.state.apply_startup(startup);
        app
    }

    /// 未入力プレースホルダーの警告と、実値を入力するミニ入力欄を描画する
    fn render_placeholder_inputs(&mut self, ui: &mut egui::Ui) {
        if self.state.placeholder_fields().is_empty() {
            return;
        }

        ui.add_space(8.0);
        ui.separator();
        let unresolved = self.state.unresolved_placeholder_count();
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
            let mut reveal = self.state.reveal_env_inputs;
            if ui.checkbox(&mut reveal, "値を表示").changed() {
                self.state.update(UiAction::SetRevealEnv(reveal));
            }
        });

        let mut edits = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("placeholder_inputs")
            .max_height(120.0)
            .show(ui, |ui| {
                for field in self.state.placeholder_fields() {
                    let mut value = self.state.placeholder_value(field);
                    ui.horizontal(|ui| {
                        ui.label(format!("{} / {}", field.server, field.key));
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut value)
                                .password(!self.state.reveal_env_inputs)
                                .hint_text("実値を入力")
                                .desired_width(f32::INFINITY),
                        );
                        if response.changed() {
                            edits.push(UiAction::SetPlaceholder {
                                field: field.clone(),
                                value,
                            });
                        }
                    });
                }
            });
        for edit in edits {
            self.state.update(edit);
        }
    }

    /// キーボードショートカット操作を処理する (Esc, Ctrl+Enter, Ctrl+V)
    pub fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let esc_pressed = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let ctrl_enter_pressed = ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
        });
        let ctrl_v_pressed = ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::V)
                || i.consume_key(egui::Modifiers::CTRL, egui::Key::V)
        });

        // Esc: ウィンドウを安全に閉じる
        if esc_pressed {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Ctrl+Enter: 選択ターゲットにマージを適用 (Apply)
        if ctrl_enter_pressed && self.state.can_apply() {
            self.state.update(UiAction::Apply);
        }

        // Ctrl+V: テキスト入力ウィジェットにフォーカスがない場合にクリップボードから読み取り
        // （テキスト入力中の通常の貼り付けは TextEdit 側で自動処理される）
        if ctrl_v_pressed {
            let has_text_focus = ui.memory(|m| m.focused().is_some());
            if !has_text_focus {
                self.state.update(UiAction::ReadClipboard);
            }
        }
    }

    /// ルート Ui を受け取り、画面 UI を構築・描画する
    pub fn render_ui(&mut self, ui: &mut egui::Ui) {
        // キーボードショートカットの処理
        self.handle_shortcuts(ui);

        // ウィンドウフォーカス獲得時の自動クリップボード同期
        let is_focused = ui.input(|i| i.focused);
        if is_focused && !self.was_focused && self.state.auto_read_clipboard {
            self.state.update(UiAction::AutoReadClipboard);
        }
        self.was_focused = is_focused;

        // トップバー
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
                    self.state.update(UiAction::ReadClipboard);
                }

                if ui
                    .button("🗑 クリア")
                    .on_hover_text("入力欄をクリアします")
                    .clicked()
                {
                    self.state.update(UiAction::Clear);
                }

                ui.separator();

                let mut replace = self.state.replace_npx_with_bunx;
                if ui.checkbox(&mut replace, "npx → bunx 置換").changed() {
                    self.state.update(UiAction::SetReplaceNpxWithBunx(replace));
                }

                let mut replace_pipx = self.state.replace_pipx_with_uvx;
                if ui
                    .checkbox(&mut replace_pipx, "pipx run → uvx 置換")
                    .on_hover_text(
                        "`pipx run パッケージ名 …` を `uvx パッケージ名 …` に書き換えます（uv が必要）。\n\
                         --spec 等のオプション・URL・バージョン指定つきは書き換えません。",
                    )
                    .changed()
                {
                    self.state.update(UiAction::SetReplacePipxWithUvx(replace_pipx));
                }

                let mut auto_read = self.state.auto_read_clipboard;
                if ui.checkbox(&mut auto_read, "自動読み取り").changed() {
                    self.state.update(UiAction::SetAutoRead(auto_read));
                }
            });
        });

        // ボトムバー（ステータス表示 & アクションボタン）
        // 長いステータス（警告つき等）がボタンに隠れないよう、左側だけを縮める
        let status = self.state.status_message.clone();
        let (selection, cli_count) = self.state.selection_summary();
        let undo_hint = self.state.undo_hint();
        egui::Panel::bottom("bottom_panel").show(ui, |ui| {
            // CLI（サブプロセス実行）を含むときは目を引く色にする
            let selection_color = if cli_count > 0 {
                egui::Color32::LIGHT_YELLOW
            } else {
                egui::Color32::GRAY
            };
            ui.add(
                egui::Label::new(egui::RichText::new(&selection).color(selection_color)).truncate(),
            )
            .on_hover_text(&selection);
            egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
                    // 左側: ステータスメッセージ
                    if let Some((msg, is_err)) = &status {
                        let color = if *is_err {
                            egui::Color32::LIGHT_RED
                        } else {
                            egui::Color32::LIGHT_GREEN
                        };
                        ui.label(egui::RichText::new(msg).color(color))
                            .on_hover_text(msg);
                    } else {
                        ui.label(
                            egui::RichText::new(
                                "スニペットを貼り付けてターゲットを選択してください",
                            )
                            .color(egui::Color32::GRAY),
                        );
                    }
                },
                |ui| {
                    let can_apply = self.state.can_apply();

                    if ui
                        .add_enabled(can_apply, egui::Button::new("💾 適用 (Ctrl+Enter)"))
                        .on_hover_text("選択したターゲットにマージを適用します (Ctrl+Enter)")
                        .clicked()
                    {
                        self.state.update(UiAction::Apply);
                    }

                    if ui
                        .add_enabled(can_apply, egui::Button::new("🔍 検証 (Dry-Run)"))
                        .on_hover_text("ファイル書き込みを行わずにマージ結果を検証します")
                        .clicked()
                    {
                        self.state.update(UiAction::DryRun);
                    }

                    let undo_button = ui.add_enabled(
                        undo_hint.is_some(),
                        egui::Button::new("↩ 直前の適用を取り消す"),
                    );
                    let undo_clicked = undo_button.clicked();
                    undo_button.on_hover_text(
                        undo_hint
                            .as_deref()
                            .unwrap_or("取り消せる適用がありません（適用すると有効になります）"),
                    );
                    if undo_clicked {
                        self.state.update(UiAction::Undo);
                    }

                    ui.label(
                        egui::RichText::new("[Esc] 閉じる")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                },
            );
        });

        // 左ペイン: 入力スニペット & パース概要
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

                // テキスト入力エリア
                let mut input = self.state.raw_input.clone();
                let text_edit = egui::TextEdit::multiline(&mut input)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16);

                let response = ui.add(text_edit);
                if response.changed() {
                    self.state.update(UiAction::SetInput(input));
                }

                self.render_placeholder_inputs(ui);

                ui.add_space(8.0);
                ui.separator();
                ui.heading("パース結果プレビュー");

                match &self.state.parsed_config {
                    Ok(servers) => {
                        ui.label(
                            egui::RichText::new(format!(
                                "✓ {} 件のサーバー定義を認識",
                                servers.len()
                            ))
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
            });

        // 右ペイン: ターゲットクライアント一覧 & 差分プレビュー
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("ターゲットクライアント");
            ui.label(
                egui::RichText::new("設定ファイルを更新する対象を選択してください")
                    .small()
                    .color(egui::Color32::GRAY),
            );

            ui.add_space(4.0);

            // ターゲット一覧
            egui::ScrollArea::vertical()
                .id_salt("target_list")
                .max_height(ui.available_height() * TARGET_LIST_HEIGHT_RATIO)
                .show(ui, |ui| {
                    let mut actions = Vec::new();
                    for (idx, entry) in self.state.target_entries.iter().enumerate() {
                        let is_active = idx == self.state.selected_target_idx;
                        ui.horizontal(|ui| {
                            let mut selected = entry.selected;
                            if ui.checkbox(&mut selected, "").changed() {
                                actions.push(UiAction::SetTargetSelected { idx, selected });
                            }

                            let exists = entry.target.exists;
                            let is_cli = entry.target.kind.action() == TargetAction::CliCommand;
                            let badge = if is_cli && exists {
                                egui::RichText::new("[CLI検出]").color(egui::Color32::LIGHT_GREEN)
                            } else if is_cli {
                                egui::RichText::new("[CLI未検出]").color(egui::Color32::GRAY)
                            } else if exists {
                                egui::RichText::new("[既存]").color(egui::Color32::LIGHT_GREEN)
                            } else {
                                egui::RichText::new("[新規]").color(egui::Color32::GRAY)
                            };
                            ui.label(badge);

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
                    }
                    for action in actions {
                        self.state.update(action);
                    }
                });

            ui.add_space(8.0);
            ui.separator();

            // 選択中ターゲットのマージ差分プレビュー
            if let Some(entry) = self.state.target_entries.get(self.state.selected_target_idx) {
                ui.horizontal(|ui| {
                    let title = match entry.target.kind.action() {
                        TargetAction::FileMerge => "マージプレビュー",
                        TargetAction::CliCommand => "実行予定コマンド",
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
                } else if entry.target.kind.action() == TargetAction::CliCommand
                    && entry.diff_preview.is_some()
                {
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
                    let is_cli = entry.target.kind.action() == TargetAction::CliCommand;
                    egui::ScrollArea::vertical()
                        .id_salt("diff_preview_scroll")
                        .show(ui, |ui| {
                            if is_cli {
                                // 1 コマンドが長くなりがちなため、TextEdit ではなく折り返すラベルで全文を見せる
                                for command in preview.lines() {
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(command).monospace())
                                            .wrap(),
                                    );
                                    ui.add_space(6.0);
                                }
                            } else {
                                ui.add(
                                    egui::TextEdit::multiline(&mut preview.as_str())
                                        .font(egui::TextStyle::Monospace)
                                        .code_editor()
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        });
                } else {
                    ui.label(
                        egui::RichText::new(
                            "左ペインに有効なMCPスニペットを入力すると、ここにマージ結果が表示されます",
                        )
                        .color(egui::Color32::GRAY),
                    );
                }
            }
        });
    }
}

impl eframe::App for Clp2cfgApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render_ui(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            AUTO_READ_STORAGE_KEY,
            &self.state.auto_read_clipboard,
        );
    }

    /// スクロール位置等の egui 内部状態は復元不要。保存対象は自動読み取りトグルのみとする
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ports::FakeClipboard;
    use crate::domain::{ClientTarget, ClientTargetKind};
    use tempfile::tempdir;

    /// 実ファイルを使い、クリップボードはフェイクの状態
    fn new_app(targets: Vec<ClientTarget>) -> Clp2cfgApp {
        Clp2cfgApp::with_ports(
            targets,
            Box::new(SystemIo),
            Box::new(FakeClipboard::default()),
        )
    }

    fn make_test_target(dir: &std::path::Path, kind: ClientTargetKind, file: &str) -> ClientTarget {
        crate::discovery::file_target(kind, dir.join(file))
    }

    /// 描画結果は検証しないため、egui 0.37 の未適用 TexturesDelta 検査を避けて破棄する
    fn run_headless(ctx: &egui::Context, app: &mut Clp2cfgApp) {
        let mut output = ctx.run_ui(Default::default(), |ui| app.render_ui(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn test_app_render_ui_headless() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = new_app(vec![target]);
        app.state.update(UiAction::SetInput(
            r#"{"fetch": {"command": "npx", "args": ["fetch"]}}"#.to_string(),
        ));

        let ctx = egui::Context::default();

        // 1. 通常レンダリング（パース成功状態）
        run_headless(&ctx, &mut app);

        // 2. パースエラー状態でのレンダリング
        app.state
            .update(UiAction::SetInput("{ invalid json }".to_string()));
        run_headless(&ctx, &mut app);

        // 3. エラーメッセージあり状態でのレンダリング
        app.state.status_message = Some(("エラー発生".to_string(), true));
        run_headless(&ctx, &mut app);
    }

    #[test]
    fn test_setup_system_fonts_runs_without_panic() {
        let ctx = egui::Context::default();
        setup_system_fonts(&ctx);
    }

    #[test]
    fn test_load_app_icon_loads_valid_icon() {
        let icon = load_app_icon();
        assert!(icon.is_some(), "load_app_icon should succeed");
        let icon = icon.unwrap();
        assert!(icon.width > 0);
        assert!(icon.height > 0);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }

    #[test]
    fn test_handle_shortcuts_esc_closes_window() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = new_app(vec![target]);
        let ctx = egui::Context::default();

        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });

        let mut output = ctx.run_ui(raw_input, |ui| app.render_ui(ui));
        output.textures_delta.clear();

        let has_close_cmd = output.viewport_output.values().any(|vp| {
            vp.commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::Close))
        });
        assert!(
            has_close_cmd,
            "Escape key should send ViewportCommand::Close"
        );
    }

    #[test]
    fn test_handle_shortcuts_ctrl_enter_applies() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = new_app(vec![target]);
        app.state.auto_read_clipboard = false;
        app.state.update(UiAction::SetInput(
            r#"{"test": {"command": "echo"}}"#.to_string(),
        ));
        app.state.target_entries[0].selected = true;

        let ctx = egui::Context::default();
        let mut raw_input = egui::RawInput::default();
        let modifiers = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });

        let mut output = ctx.run_ui(raw_input, |ui| app.render_ui(ui));
        output.textures_delta.clear();

        // 適用処理が実行され、ファイルが作成されたことを確認
        assert!(app.state.target_entries[0].target.path.exists());
        assert!(app
            .state
            .status_message
            .as_ref()
            .unwrap()
            .0
            .contains("適用完了"));
    }

    #[test]
    fn test_handle_shortcuts_ctrl_v_when_not_focused() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = new_app(vec![target]);
        let ctx = egui::Context::default();

        let mut raw_input = egui::RawInput::default();
        let modifiers = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::V,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });

        // テキストボックスフォーカスなしでの描画実行でパニックせず完了することを確認
        let mut output = ctx.run_ui(raw_input, |ui| app.render_ui(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn test_save_persists_auto_read_toggle() {
        #[derive(Default)]
        struct MemStorage(std::collections::HashMap<String, String>);
        impl eframe::Storage for MemStorage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.to_string(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }

        for enabled in [true, false] {
            let mut app = new_app(Vec::new());
            app.state.auto_read_clipboard = enabled;
            let mut storage = MemStorage::default();
            eframe::App::save(&mut app, &mut storage);
            assert_eq!(
                eframe::get_value::<bool>(&storage, AUTO_READ_STORAGE_KEY),
                Some(enabled)
            );
        }
    }

    /// 描画された文字列のうち `needle` を含むものの右端（画面座標）の最大値
    fn max_text_right_edge(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<f32> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.pos.x + text.galley.rect.max.x)
                }
                _ => None,
            })
            .reduce(f32::max)
    }

    #[test]
    fn test_cli_command_preview_wraps_within_screen() {
        const SCREEN_WIDTH: f32 = 960.0;

        // 非常に長いパスのターゲット行が右ペインの幅を押し広げても、
        // 実行予定コマンドは画面幅の中で折り返されること
        let temp = tempdir().unwrap();
        let long_name = format!("{}.json", "d".repeat(300));
        let file_target = make_test_target(temp.path(), ClientTargetKind::Cursor, &long_name);
        let mut cli_target = ClientTarget::new_cli(ClientTargetKind::ClaudeCli, None);
        cli_target.path = std::path::PathBuf::from("claude");

        let mut app = new_app(vec![file_target, cli_target]);
        app.state.auto_read_clipboard = false;
        app.state.selected_target_idx = 1;
        app.state.update(UiAction::SetInput(r#"{"srv":{"command":"uvx","args":["mcp-server-everything-and-more"],"env":{"E2E_KEY":"value with space"}}}"#.to_string()));

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SCREEN_WIDTH, 640.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw_input, |ui| app.render_ui(ui));
        output.textures_delta.clear();

        let right = max_text_right_edge(&output.shapes, "mcp add")
            .expect("コマンドのプレビューが描画されていること");
        assert!(
            right <= SCREEN_WIDTH,
            "コマンドが画面幅 {SCREEN_WIDTH} を超えて描画された: {right}"
        );
    }

    #[test]
    fn test_target_entry_changes_follow_existing_file() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"same":{"command":"node","args":["a"]},"diff":{"command":"old"}}}"#,
        )
        .unwrap();
        let mut cli = ClientTarget::new_cli(ClientTargetKind::CodexCli, None);
        cli.path = std::path::PathBuf::from("codex");

        let mut app = new_app(vec![
            crate::discovery::file_target(ClientTargetKind::Cursor, path),
            cli,
        ]);
        app.state.auto_read_clipboard = false;
        app.state.replace_npx_with_bunx = false;
        app.state.update(UiAction::SetInput(r#"{"same":{"command":"node","args":["a"]},"diff":{"command":"new"},"fresh":{"command":"x"}}"#.to_string()));

        let entry = &app.state.target_entries[0];
        let changes = entry.changes.as_ref().unwrap();
        assert_eq!(changes["same"], ServerChange::Unchanged);
        assert_eq!(changes["diff"], ServerChange::Updated);
        assert_eq!(changes["fresh"], ServerChange::New);
        assert_eq!(entry.overwrite_count(), 1);
        assert_eq!(
            change_summary(changes),
            "新規追加 1 件 / 上書き 1 件 / 変更なし 1 件"
        );

        // CLI 委譲ターゲットは既存の登録を確認できないので判定なし
        assert!(app.state.target_entries[1].changes.is_none());
        assert_eq!(app.state.target_entries[1].overwrite_count(), 0);

        // 表示（一覧の警告・見出しのバッジ・CLI の注記）で panic しない
        let ctx = egui::Context::default();
        run_headless(&ctx, &mut app);
        app.state.selected_target_idx = 1;
        run_headless(&ctx, &mut app);

        // 入力が無効になれば判定も消える
        app.state
            .update(UiAction::SetInput("{ invalid".to_string()));
        assert!(app.state.target_entries.iter().all(|e| e.changes.is_none()));
        app.state.update(UiAction::SetInput(String::new()));
        assert!(app.state.target_entries.iter().all(|e| e.changes.is_none()));
    }

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
    fn test_render_with_an_undoable_apply_does_not_panic() {
        // 取り消しボタンと説明を描画する（適用後は「取り消し可能」になる）
        let temp = tempdir().unwrap();
        let path = temp.path().join("mcp.json");
        let mut app = new_app(vec![crate::discovery::file_target(
            ClientTargetKind::Cursor,
            path,
        )]);
        app.state.selected_target_idx = 0;
        app.state.target_entries[0].selected = true;
        app.state.update(UiAction::SetInput(
            r#"{"srv":{"command":"node"}}"#.to_string(),
        ));
        app.state.update(UiAction::Apply);
        assert!(app.state.undo_hint().is_some());

        let ctx = egui::Context::default();
        run_headless(&ctx, &mut app);
    }
}
