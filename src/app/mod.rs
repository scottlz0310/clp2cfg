mod action;
mod clipboard;
mod ports;
mod state;
mod system;
mod view;

pub use action::UiAction;
pub use clipboard::ArboardClipboard;
pub use ports::ClipboardSource;
pub use state::{AppState, StartupOptions, TargetEntry};
pub use system::load_app_icon;

use crate::apply::{SystemCatalog, SystemIo, TargetCatalog, TargetIo};
use system::{setup_system_fonts, AUTO_READ_STORAGE_KEY};

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

    /// ルート Ui を受け取り、画面 UI を描画する。
    /// 各 view は状態を読むだけで操作を返し、状態の更新はフレームの終わりにまとめて行う。
    pub fn render_ui(&mut self, ui: &mut egui::Ui) {
        let mut actions = view::shortcuts(ui, &self.state);

        // ウィンドウフォーカス獲得時の自動クリップボード同期
        let is_focused = ui.input(|i| i.focused);
        if is_focused && !self.was_focused && self.state.auto_read_clipboard {
            actions.push(UiAction::AutoReadClipboard);
        }
        self.was_focused = is_focused;

        actions.extend(view::top_bar(ui, &self.state));
        actions.extend(view::bottom_bar(ui, &self.state));
        actions.extend(view::input_pane(ui, &self.state));
        actions.extend(view::target_pane(ui, &self.state));

        // 更新後の状態を、次の入力を待たずに描き直す
        if !actions.is_empty() {
            ui.ctx().request_repaint();
        }
        for action in actions {
            self.state.update(action);
        }
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
    use crate::domain::{ClientTarget, ClientTargetKind, ServerChange};
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
    fn test_shortcut_esc_closes_window() {
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
    fn test_shortcut_ctrl_enter_applies() {
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

        // 状態の更新はフレームの終わりに行うので、更新後の画面を次のフレームで描き直すよう求める
        assert!(
            output
                .viewport_output
                .values()
                .any(|vp| vp.repaint_delay.is_zero()),
            "操作があったフレームは再描画を要求する"
        );

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
    fn test_shortcut_ctrl_v_when_not_focused() {
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
