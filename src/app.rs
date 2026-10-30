use crate::cli::{build_cli_invocations, preview_invocations, run_invocations};
use crate::filter::{apply_filters, FilterOptions};
use crate::merger::{detect_client_targets, merge_mcp_servers};
use crate::model::{ClientTarget, McpServersMap, TargetAction};
use crate::parser::parse_mcp_snippet;
use crate::writer::safe_atomic_write;

/// 各クライアント設定ターゲットの GUI 表示・状態
#[derive(Debug, Clone)]
pub struct TargetEntry {
    pub target: ClientTarget,
    pub diff_preview: Option<String>,
    pub error: Option<String>,
}

impl TargetEntry {
    pub fn new(target: ClientTarget) -> Self {
        Self {
            target,
            diff_preview: None,
            error: None,
        }
    }
}

/// ターゲットへの適用予定内容（ファイルマージ結果 or 実行予定 CLI コマンド）を生成する
fn build_preview(target: &ClientTarget, servers: &McpServersMap) -> Result<String, String> {
    match target.kind.action() {
        TargetAction::FileMerge => {
            let existing = if target.path.is_file() {
                std::fs::read_to_string(&target.path).ok()
            } else {
                None
            };
            merge_mcp_servers(target.kind, existing.as_deref(), servers).map_err(|e| e.to_string())
        }
        TargetAction::CliCommand => {
            let program = target.path.to_string_lossy();
            build_cli_invocations(target.kind, &program, servers)
                .map(|inv| preview_invocations(&inv))
                .map_err(|e| e.to_string())
        }
    }
}

/// Windows 等で並行テスト実行時に OLE/COM クリップボードの同時初期化によるヒープ破損を防ぐためのロック
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// clp2cfg のメインアプリケーション状態
pub struct Clp2cfgApp {
    /// 入力スニペット文字列
    pub raw_input: String,
    /// パースの無駄な再実行を防ぐための入力キャッシュ
    last_parsed_input: String,
    /// パースされた MCP サーバー定義
    pub parsed_config: Result<McpServersMap, String>,
    /// npx を bunx に自動置換するフィルタトグル
    pub replace_npx_with_bunx: bool,
    /// ターゲットクライアント一覧
    pub target_entries: Vec<TargetEntry>,
    /// 右ペインで差分詳細を表示するターゲットのインデックス
    pub selected_target_idx: usize,
    /// 操作結果ステータスメッセージ (メッセージ, is_error)
    pub status_message: Option<(String, bool)>,
    /// 起動時のクリップボード読み取りが完了したか
    initial_clipboard_read: bool,
    /// 自動クリップボード読み取りトグル
    pub auto_read_clipboard: bool,
    /// 前回フレームでのウィンドウフォーカス状態
    pub was_focused: bool,
}

impl Default for Clp2cfgApp {
    fn default() -> Self {
        Self::new_with_targets(detect_client_targets(None))
    }
}

impl Clp2cfgApp {
    /// 指定されたターゲット一覧でアプリケーションを初期化
    pub fn new_with_targets(targets: Vec<ClientTarget>) -> Self {
        let target_entries = targets.into_iter().map(TargetEntry::new).collect();
        Self {
            raw_input: String::new(),
            last_parsed_input: String::new(),
            parsed_config: Err("入力が空です".to_string()),
            replace_npx_with_bunx: true,
            target_entries,
            selected_target_idx: 0,
            status_message: None,
            initial_clipboard_read: false,
            auto_read_clipboard: true,
            was_focused: false,
        }
    }

    /// eframe 用のコンストラクタ（起動時にフォント設定およびクリップボード自動読み取りを試みる）
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_system_fonts(&cc.egui_ctx);
        let mut app = Self::default();
        if app.auto_read_clipboard {
            app.read_from_clipboard();
            app.initial_clipboard_read = true;
        }
        app
    }

    /// OS クリップボードから文字列を読み取り、入力欄に展開してパースする
    pub fn read_from_clipboard(&mut self) -> bool {
        if let Some(text) = Self::get_clipboard_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() && trimmed != self.raw_input.trim() {
                self.raw_input = text;
                self.reparse();
                self.status_message = Some(("クリップボードから読み取りました".to_string(), false));
                return true;
            }
        }
        false
    }

    /// OS クリップボードの文字列を取得
    fn get_clipboard_text() -> Option<String> {
        let _guard = CLIPBOARD_LOCK.lock().ok()?;
        let mut clipboard = arboard::Clipboard::new().ok()?;
        clipboard.get_text().ok()
    }

    /// 入力文字列をパースし、差分プレビューを更新する
    pub fn reparse(&mut self) {
        self.last_parsed_input = self.raw_input.clone();
        let trimmed = self.raw_input.trim();

        if trimmed.is_empty() {
            self.parsed_config = Err("入力が空です".to_string());
            for entry in &mut self.target_entries {
                entry.diff_preview = None;
                entry.error = None;
            }
            return;
        }

        match parse_mcp_snippet(&self.raw_input) {
            Ok(servers) => {
                let options = FilterOptions {
                    replace_npx_with_bunx: self.replace_npx_with_bunx,
                };

                let filtered_servers: McpServersMap = servers
                    .into_iter()
                    .map(|(k, v)| (k, apply_filters(v, &options)))
                    .collect();

                // 各ターゲットに対するマージプレビューを計算
                for entry in &mut self.target_entries {
                    match build_preview(&entry.target, &filtered_servers) {
                        Ok(preview) => {
                            entry.diff_preview = Some(preview);
                            entry.error = None;
                        }
                        Err(err) => {
                            entry.diff_preview = None;
                            entry.error = Some(format!("マージプレビュー生成失敗: {}", err));
                        }
                    }
                }

                let count = filtered_servers.len();
                self.parsed_config = Ok(filtered_servers);
                self.status_message = Some((
                    format!("パース成功: {} 件のサーバー設定を検出", count),
                    false,
                ));
            }
            Err(err) => {
                self.parsed_config = Err(format!("パースエラー: {}", err));
                for entry in &mut self.target_entries {
                    entry.diff_preview = None;
                    entry.error = None;
                }
                self.status_message = Some((format!("パース失敗: {}", err), true));
            }
        }
    }

    /// 選択されたターゲットにマージを適用（または Dry-Run シミュレーション）する
    pub fn apply_all(&mut self, dry_run: bool) {
        let servers = match &self.parsed_config {
            Ok(s) => s.clone(),
            Err(_) => {
                self.status_message =
                    Some(("有効なMCP設定がパースされていません".to_string(), true));
                return;
            }
        };

        let selected_count = self
            .target_entries
            .iter()
            .filter(|e| e.target.selected)
            .count();
        if selected_count == 0 {
            self.status_message =
                Some(("適用対象のターゲットが選択されていません".to_string(), true));
            return;
        }

        let mut success_targets = Vec::new();
        let mut fail_targets = Vec::new();

        for entry in &mut self.target_entries {
            if !entry.target.selected {
                continue;
            }

            let result = match entry.target.kind.action() {
                TargetAction::FileMerge => {
                    build_preview(&entry.target, &servers).and_then(|merged| {
                        if dry_run {
                            return Ok(());
                        }
                        safe_atomic_write(&entry.target.path, &merged)
                            .map_err(|e| e.to_string())
                            .map(|_| entry.target.exists = true)
                    })
                }
                TargetAction::CliCommand => {
                    let program = entry.target.path.to_string_lossy();
                    build_cli_invocations(entry.target.kind, &program, &servers)
                        .and_then(|inv| {
                            if dry_run {
                                Ok(())
                            } else {
                                run_invocations(&inv)
                            }
                        })
                        .map_err(|e| e.to_string())
                }
            };

            match result {
                Ok(()) if dry_run => success_targets.push(format!("{}(検証OK)", entry.target.name)),
                Ok(()) => success_targets.push(entry.target.name.clone()),
                Err(err) => fail_targets.push(format!("{}: {}", entry.target.name, err)),
            }
        }

        let prefix = if dry_run { "[Dry-Run] " } else { "" };
        if fail_targets.is_empty() {
            self.status_message = Some((
                format!(
                    "{}適用完了: {} 件 ({})",
                    prefix,
                    success_targets.len(),
                    success_targets.join(", ")
                ),
                false,
            ));
        } else {
            self.status_message = Some((
                format!(
                    "{}一部失敗: 成功 {} 件, 失敗 {} 件 ({})",
                    prefix,
                    success_targets.len(),
                    fail_targets.len(),
                    fail_targets.join("; ")
                ),
                true,
            ));
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
        if ctrl_enter_pressed {
            let can_apply =
                self.parsed_config.is_ok() && self.target_entries.iter().any(|e| e.target.selected);
            if can_apply {
                self.apply_all(false);
            }
        }

        // Ctrl+V: テキスト入力ウィジェットにフォーカスがない場合にクリップボードから読み取り
        // （テキスト入力中の通常の貼り付けは TextEdit 側で自動処理される）
        if ctrl_v_pressed {
            let has_text_focus = ui.memory(|m| m.focused().is_some());
            if !has_text_focus {
                self.read_from_clipboard();
            }
        }
    }

    /// ルート Ui を受け取り、画面 UI を構築・描画する
    pub fn render_ui(&mut self, ui: &mut egui::Ui) {
        // キーボードショートカットの処理
        self.handle_shortcuts(ui);

        // ウィンドウフォーカス獲得時の自動クリップボード同期
        let is_focused = ui.input(|i| i.focused);
        if is_focused && !self.was_focused && self.auto_read_clipboard {
            self.read_from_clipboard();
        }
        self.was_focused = is_focused;

        // トップバー
        egui::Panel::top("top_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
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
                    self.read_from_clipboard();
                }

                if ui
                    .button("🗑 クリア")
                    .on_hover_text("入力欄をクリアします")
                    .clicked()
                {
                    self.raw_input.clear();
                    self.reparse();
                }

                ui.separator();

                let mut replace = self.replace_npx_with_bunx;
                if ui.checkbox(&mut replace, "npx → bunx 置換").changed() {
                    self.replace_npx_with_bunx = replace;
                    self.reparse();
                }

                ui.checkbox(&mut self.auto_read_clipboard, "自動読み取り");
            });
        });

        // ボトムバー（ステータス表示 & アクションボタン）
        egui::Panel::bottom("bottom_panel").show(ui, |ui| {
            ui.horizontal(|ui| {
                // 左側: ステータスメッセージ
                if let Some((msg, is_err)) = &self.status_message {
                    let color = if *is_err {
                        egui::Color32::LIGHT_RED
                    } else {
                        egui::Color32::LIGHT_GREEN
                    };
                    ui.label(egui::RichText::new(msg).color(color));
                } else {
                    ui.label(
                        egui::RichText::new("スニペットを貼り付けてターゲットを選択してください")
                            .color(egui::Color32::GRAY),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can_apply = self.parsed_config.is_ok()
                        && self.target_entries.iter().any(|e| e.target.selected);

                    if ui
                        .add_enabled(can_apply, egui::Button::new("💾 適用 (Ctrl+Enter)"))
                        .on_hover_text("選択したターゲットにマージを適用します (Ctrl+Enter)")
                        .clicked()
                    {
                        self.apply_all(false);
                    }

                    if ui
                        .add_enabled(can_apply, egui::Button::new("🔍 検証 (Dry-Run)"))
                        .on_hover_text("ファイル書き込みを行わずにマージ結果を検証します")
                        .clicked()
                    {
                        self.apply_all(true);
                    }

                    ui.label(
                        egui::RichText::new("[Esc] 閉じる")
                            .small()
                            .color(egui::Color32::GRAY),
                    );
                });
            });
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
                let text_edit = egui::TextEdit::multiline(&mut self.raw_input)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16);

                let response = ui.add(text_edit);
                if response.changed() {
                    self.reparse();
                }

                ui.add_space(8.0);
                ui.separator();
                ui.heading("パース結果プレビュー");

                match &self.parsed_config {
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
                .max_height(140.0)
                .show(ui, |ui| {
                    for (idx, entry) in self.target_entries.iter_mut().enumerate() {
                        let is_active = idx == self.selected_target_idx;
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut entry.target.selected, "");

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
                                self.selected_target_idx = idx;
                            }

                            ui.label(
                                egui::RichText::new(entry.target.path.to_string_lossy())
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    }
                });

            ui.add_space(8.0);
            ui.separator();

            // 選択中ターゲットのマージ差分プレビュー
            if let Some(entry) = self.target_entries.get(self.selected_target_idx) {
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

                if let Some(error) = &entry.error {
                    ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
                } else if let Some(preview) = &entry.diff_preview {
                    egui::ScrollArea::vertical()
                        .id_salt("diff_preview_scroll")
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut preview.as_str())
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY),
                            );
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
}

/// OS 標準の日本語/CJKフォントを探索して egui のフォント定義に登録
fn setup_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_candidates = [
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ];

    for path in font_candidates {
        if let Ok(font_data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk_fallback".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_data)),
            );
            if let Some(prop) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                prop.push("cjk_fallback".to_owned());
            }
            if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                mono.push("cjk_fallback".to_owned());
            }
            break;
        }
    }

    ctx.set_fonts(fonts);
}

/// アプリケーションアイコン（`assets/icon.png`）を読み込み、`egui::IconData` を生成する。
/// 読み込みに失敗した場合は警告ログを出力して `None` を返す。
pub fn load_app_icon() -> Option<egui::IconData> {
    const ICON_BYTES: &[u8] = include_bytes!("../assets/icon.png");
    match image::load_from_memory(ICON_BYTES) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            Some(egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(err) => {
            tracing::warn!("Failed to decode application icon: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClientTarget, ClientTargetKind};
    use tempfile::tempdir;

    fn make_test_target(dir: &std::path::Path, kind: ClientTargetKind, file: &str) -> ClientTarget {
        ClientTarget::new(kind, dir.join(file))
    }

    #[test]
    fn test_app_reparse_valid_snippet() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);

        app.raw_input = r#"{
            "mcpServers": {
                "fetch": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-fetch"]
                }
            }
        }"#
        .to_string();

        app.reparse();

        assert!(app.parsed_config.is_ok());
        let servers = app.parsed_config.unwrap();
        assert_eq!(servers.len(), 1);
        let fetch = servers.get("fetch").unwrap();
        // npx -> bunx フィルタが適用されていること
        assert_eq!(fetch.command, "bunx");

        // プレビューが生成されていること
        assert!(app.target_entries[0].diff_preview.is_some());
        let preview = app.target_entries[0].diff_preview.as_ref().unwrap();
        assert!(preview.contains("bunx"));
        assert!(preview.contains("fetch"));
    }

    #[test]
    fn test_app_reparse_empty_input() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);

        app.raw_input = "   ".to_string();
        app.reparse();

        assert!(app.parsed_config.is_err());
        assert!(app.target_entries[0].diff_preview.is_none());
    }

    #[test]
    fn test_app_apply_dry_run_and_actual_write() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let target_path = target.path.clone();

        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        // ターゲットを選択状態にする
        app.target_entries[0].target.selected = true;

        app.raw_input = r#"{
            "test-server": {
                "command": "python",
                "args": ["server.py"]
            }
        }"#
        .to_string();
        app.reparse();

        // Dry-Run
        app.apply_all(true);
        assert!(!target_path.exists());
        assert!(app.status_message.as_ref().unwrap().0.contains("[Dry-Run]"));

        // Actual Apply
        app.apply_all(false);
        assert!(target_path.exists());
        let content = std::fs::read_to_string(&target_path).unwrap();
        assert!(content.contains("test-server"));
        assert!(content.contains("server.py"));
    }

    #[test]
    fn test_app_default_initialization() {
        let app = Clp2cfgApp::default();
        assert!(app.parsed_config.is_err());
        assert!(app.replace_npx_with_bunx);
        assert!(app.auto_read_clipboard);
        assert_eq!(app.selected_target_idx, 0);
    }

    #[test]
    fn test_app_reparse_syntax_error() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);

        app.raw_input = "{ invalid json content }".to_string();
        app.reparse();

        assert!(app.parsed_config.is_err());
        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
        assert!(app.target_entries[0].diff_preview.is_none());
    }

    #[test]
    fn test_app_apply_without_parsed_config() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.target_entries[0].target.selected = true;

        app.apply_all(false);
        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
        assert!(app
            .status_message
            .as_ref()
            .unwrap()
            .0
            .contains("有効なMCP設定がパースされていません"));
    }

    #[test]
    fn test_app_apply_write_failure_reports_error() {
        let temp = tempdir().unwrap();
        // 親パスをファイルにしておくことで、ディレクトリ作成・ファイル書き込みを確実に失敗させる
        let blocking_file = temp.path().join("blocking_file");
        std::fs::write(&blocking_file, "blocking content").unwrap();

        let target = make_test_target(&blocking_file, ClientTargetKind::Cursor, "mcp.json");

        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.target_entries[0].target.selected = true;

        app.raw_input = r#"{"test": {"command": "echo"}}"#.to_string();
        app.reparse();

        app.apply_all(false);
        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
        assert!(app.status_message.as_ref().unwrap().0.contains("一部失敗"));
    }

    #[test]
    fn test_app_apply_no_targets_selected() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.target_entries[0].target.selected = false;

        app.raw_input = r#"{"s": {"command": "cmd"}}"#.to_string();
        app.reparse();

        app.apply_all(false);
        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
    }

    #[test]
    fn test_app_cli_target_previews_command_and_dry_run_does_not_execute() {
        let mut target = ClientTarget::new_cli(ClientTargetKind::CodexCli, None);
        // 存在しないバイナリ: 実実行なら失敗し、Dry-Run なら実行されない
        target.path = std::path::PathBuf::from("clp2cfg-nonexistent-binary");
        target.selected = true;
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.raw_input =
            r#"{"mcpServers":{"fetch":{"command":"uvx","args":["mcp-server-fetch"]}}}"#.to_string();
        app.reparse();

        let preview = app.target_entries[0].diff_preview.as_ref().unwrap();
        assert!(preview.contains("clp2cfg-nonexistent-binary mcp remove fetch"));
        assert!(preview.contains("mcp add fetch -- uvx mcp-server-fetch"));

        app.apply_all(true);
        assert!(!app.status_message.as_ref().unwrap().1);

        app.apply_all(false);
        let (msg, is_err) = app.status_message.as_ref().unwrap();
        assert!(*is_err, "{msg}");
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
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.raw_input = r#"{"fetch": {"command": "npx", "args": ["fetch"]}}"#.to_string();
        app.reparse();

        let ctx = egui::Context::default();

        // 1. 通常レンダリング（パース成功状態）
        run_headless(&ctx, &mut app);

        // 2. パースエラー状態でのレンダリング
        app.raw_input = "{ invalid json }".to_string();
        app.reparse();
        run_headless(&ctx, &mut app);

        // 3. エラーメッセージあり状態でのレンダリング
        app.status_message = Some(("エラー発生".to_string(), true));
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
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
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
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.auto_read_clipboard = false;
        app.raw_input = r#"{"test": {"command": "echo"}}"#.to_string();
        app.reparse();
        app.target_entries[0].target.selected = true;

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
        assert!(app.target_entries[0].target.path.exists());
        assert!(app.status_message.as_ref().unwrap().0.contains("適用完了"));
    }

    #[test]
    fn test_handle_shortcuts_ctrl_v_when_not_focused() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
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
}
