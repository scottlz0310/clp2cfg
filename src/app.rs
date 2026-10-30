use crate::cli::{build_cli_invocations, preview_invocations, run_invocations};
use crate::filter::{apply_filters, FilterOptions};
use crate::merger::{
    classify_server_changes, detect_client_targets, merge_mcp_servers, ServerChange,
};
use crate::model::{ClientTarget, McpServersMap, TargetAction};
use crate::parser::parse_mcp_snippet;
use crate::placeholder::{
    fill_placeholders, find_placeholders, is_placeholder, EnvOverrides, EnvRef,
};
use crate::writer::safe_atomic_write;
use std::collections::BTreeMap;

/// 各クライアント設定ターゲットの GUI 表示・状態
#[derive(Debug, Clone)]
pub struct TargetEntry {
    pub target: ClientTarget,
    pub diff_preview: Option<String>,
    /// サーバーごとの「新規追加 / 上書き / 変更なし」（ファイル系ターゲットのみ）
    pub changes: Option<ServerChanges>,
    pub error: Option<String>,
}

impl TargetEntry {
    pub fn new(target: ClientTarget) -> Self {
        Self {
            target,
            diff_preview: None,
            changes: None,
            error: None,
        }
    }

    /// 既存の定義を置き換えるサーバーの数
    pub fn overwrite_count(&self) -> usize {
        self.changes.as_ref().map_or(0, |changes| {
            changes
                .values()
                .filter(|c| **c == ServerChange::Updated)
                .count()
        })
    }
}

/// サーバー名ごとの判定
pub type ServerChanges = BTreeMap<String, ServerChange>;

/// 判定の表示ラベルと色
fn change_badge(change: ServerChange) -> (&'static str, egui::Color32) {
    match change {
        ServerChange::New => ("新規追加", egui::Color32::LIGHT_GREEN),
        ServerChange::Updated => ("上書き", egui::Color32::YELLOW),
        ServerChange::Unchanged => ("変更なし", egui::Color32::GRAY),
    }
}

/// 判定ごとの件数の要約（例: `新規追加 1 件 / 上書き 2 件 / 変更なし 0 件`）
fn change_summary(changes: &ServerChanges) -> String {
    let count = |target: ServerChange| changes.values().filter(|c| **c == target).count();
    format!(
        "新規追加 {} 件 / 上書き {} 件 / 変更なし {} 件",
        count(ServerChange::New),
        count(ServerChange::Updated),
        count(ServerChange::Unchanged)
    )
}

/// ターゲットへの適用予定内容（ファイルマージ結果 or 実行予定 CLI コマンド）と、
/// ファイル系ターゲットでのサーバーごとの判定（新規追加 / 上書き / 変更なし）を生成する。
/// CLI 委譲ターゲットは既存の登録を事前に確認できないため、判定は `None`。
fn build_preview_with_changes(
    target: &ClientTarget,
    servers: &McpServersMap,
) -> Result<(String, Option<ServerChanges>), String> {
    match target.kind.action() {
        TargetAction::FileMerge => {
            let existing = if target.path.is_file() {
                std::fs::read_to_string(&target.path).ok()
            } else {
                None
            };
            let merged = merge_mcp_servers(target.kind, existing.as_deref(), servers)
                .map_err(|e| e.to_string())?;
            let changes = classify_server_changes(target.kind, existing.as_deref(), servers)
                .map_err(|e| e.to_string())?;
            Ok((merged, Some(changes)))
        }
        TargetAction::CliCommand => {
            let program = target.path.to_string_lossy();
            build_cli_invocations(target.kind, &program, servers)
                .map(|inv| (preview_invocations(&inv), None))
                .map_err(|e| e.to_string())
        }
    }
}

/// ターゲットへの適用予定内容だけが必要な場合（適用時）
fn build_preview(target: &ClientTarget, servers: &McpServersMap) -> Result<String, String> {
    build_preview_with_changes(target, servers).map(|(preview, _)| preview)
}

/// ターゲット一覧が右ペインの高さに占める上限の割合（残りは差分プレビューに使う）
const TARGET_LIST_HEIGHT_RATIO: f32 = 0.5;

/// 自動読み取りトグルの永続化キー
const AUTO_READ_STORAGE_KEY: &str = "auto_read_clipboard";

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
    /// `pipx run` を uvx に自動置換するフィルタトグル
    pub replace_pipx_with_uvx: bool,
    /// ターゲットクライアント一覧
    pub target_entries: Vec<TargetEntry>,
    /// 右ペインで差分詳細を表示するターゲットのインデックス
    pub selected_target_idx: usize,
    /// 操作結果ステータスメッセージ (メッセージ, is_error)
    pub status_message: Option<(String, bool)>,
    /// 自動クリップボード読み取りトグル（次回起動へ永続化される）
    pub auto_read_clipboard: bool,
    /// スニペット内のプレースホルダー env（入力済みのものも含む。入力欄を消さないため）
    placeholder_fields: Vec<EnvRef>,
    /// ミニ入力欄で入力された実値（シークレットを含むためログ・ステータスには出さない）
    env_overrides: EnvOverrides,
    /// 入力欄の値を平文で表示するか
    reveal_env_inputs: bool,
    /// 前回クリップボードから取り込んだ（または判定した）内容。同一内容の再取込で編集中の入力を上書きしないために保持する
    last_clipboard_seen: Option<String>,
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
            replace_pipx_with_uvx: true,
            target_entries,
            selected_target_idx: 0,
            status_message: None,
            auto_read_clipboard: true,
            placeholder_fields: Vec::new(),
            env_overrides: EnvOverrides::new(),
            reveal_env_inputs: false,
            last_clipboard_seen: None,
            was_focused: false,
        }
    }

    /// eframe 用のコンストラクタ（フォント設定、保存済みトグルの復元、起動時のクリップボード自動読み取り）
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_system_fonts(&cc.egui_ctx);
        let mut app = Self::default();
        if let Some(enabled) = cc
            .storage
            .and_then(|s| eframe::get_value(s, AUTO_READ_STORAGE_KEY))
        {
            app.auto_read_clipboard = enabled;
        }
        if app.auto_read_clipboard {
            app.auto_read_from_clipboard();
        }
        app
    }

    /// 手動操作（ボタン・Ctrl+V）用: OS クリップボードの文字列を内容を問わず入力欄に展開してパースする
    pub fn read_from_clipboard(&mut self) -> bool {
        match Self::get_clipboard_text() {
            Some(text) => self.load_clipboard_text(&text, false),
            None => false,
        }
    }

    /// 自動読み取り用（起動時・フォーカス獲得時）
    pub fn auto_read_from_clipboard(&mut self) -> bool {
        match Self::get_clipboard_text() {
            Some(text) => self.load_clipboard_text(&text, true),
            None => false,
        }
    }

    /// クリップボード文字列を入力欄へ取り込む。
    ///
    /// `only_valid_mcp` が真（自動読み取り）の場合、前回判定済みの内容と同一、または
    /// MCP 設定としてパースできない内容は取り込まない。意図しない内容の表示と、
    /// 編集中の入力の上書きを避けるため。
    fn load_clipboard_text(&mut self, text: &str, only_valid_mcp: bool) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }
        if only_valid_mcp && self.last_clipboard_seen.as_deref() == Some(trimmed) {
            return false;
        }
        self.last_clipboard_seen = Some(trimmed.to_string());
        if only_valid_mcp && parse_mcp_snippet(text).is_err() {
            return false;
        }
        if trimmed == self.raw_input.trim() {
            return false;
        }
        self.raw_input = text.to_string();
        self.env_overrides.clear();
        self.reparse();
        self.status_message = Some(("クリップボードから読み取りました".to_string(), false));
        true
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
            self.placeholder_fields.clear();
            self.parsed_config = Err("入力が空です".to_string());
            for entry in &mut self.target_entries {
                entry.diff_preview = None;
                entry.changes = None;
                entry.error = None;
            }
            return;
        }

        match parse_mcp_snippet(&self.raw_input) {
            Ok(servers) => {
                let options = FilterOptions {
                    replace_npx_with_bunx: self.replace_npx_with_bunx,
                    replace_pipx_with_uvx: self.replace_pipx_with_uvx,
                };

                let mut filtered_servers: McpServersMap = servers
                    .into_iter()
                    .map(|(k, v)| (k, apply_filters(v, &options)))
                    .collect();

                self.placeholder_fields = find_placeholders(&filtered_servers);
                fill_placeholders(
                    &mut filtered_servers,
                    &self.placeholder_fields,
                    &self.env_overrides,
                );

                // 各ターゲットに対するマージプレビューを計算
                for entry in &mut self.target_entries {
                    match build_preview_with_changes(&entry.target, &filtered_servers) {
                        Ok((preview, changes)) => {
                            entry.diff_preview = Some(preview);
                            entry.changes = changes;
                            entry.error = None;
                        }
                        Err(err) => {
                            entry.diff_preview = None;
                            entry.changes = None;
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
                self.placeholder_fields.clear();
                self.parsed_config = Err(format!("パースエラー: {}", err));
                for entry in &mut self.target_entries {
                    entry.diff_preview = None;
                    entry.changes = None;
                    entry.error = None;
                }
                self.status_message = Some((format!("パース失敗: {}", err), true));
            }
        }
    }

    /// 「適用」で更新される対象の要約と、そのうち CLI を実行する件数。
    /// 一覧の画面外にあるチェック済みターゲットにも気付けるよう、ボトムバーに常時表示する。
    pub fn selection_summary(&self) -> (String, usize) {
        let selected: Vec<_> = self
            .target_entries
            .iter()
            .filter(|e| e.target.selected)
            .collect();
        if selected.is_empty() {
            return ("適用対象: なし".to_string(), 0);
        }

        let cli_count = selected
            .iter()
            .filter(|e| e.target.kind.action() == TargetAction::CliCommand)
            .count();
        let names = selected
            .iter()
            .map(|e| e.target.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let cli_note = if cli_count > 0 {
            format!("（うち CLI 実行 {} 件）", cli_count)
        } else {
            String::new()
        };
        (
            format!("適用対象: {} 件{}: {}", selected.len(), cli_note, names),
            cli_count,
        )
    }

    /// 現在の設定（入力済みの実値を反映後）でもプレースホルダーのままの env の件数
    pub fn unresolved_placeholder_count(&self) -> usize {
        let Ok(servers) = &self.parsed_config else {
            return 0;
        };
        self.placeholder_fields
            .iter()
            .filter(|r| {
                servers
                    .get(&r.server)
                    .and_then(|c| c.env.get(&r.key))
                    .is_some_and(|v| is_placeholder(v))
            })
            .count()
    }

    /// 未入力プレースホルダーの警告と、実値を入力するミニ入力欄を描画する
    fn render_placeholder_inputs(&mut self, ui: &mut egui::Ui) {
        if self.placeholder_fields.is_empty() {
            return;
        }

        ui.add_space(8.0);
        ui.separator();
        let unresolved = self.unresolved_placeholder_count();
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
            ui.checkbox(&mut self.reveal_env_inputs, "値を表示");
        });

        let mut changed = false;
        egui::ScrollArea::vertical()
            .id_salt("placeholder_inputs")
            .max_height(120.0)
            .show(ui, |ui| {
                for field in &self.placeholder_fields {
                    let mut value = self.env_overrides.get(field).cloned().unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(format!("{} / {}", field.server, field.key));
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut value)
                                .password(!self.reveal_env_inputs)
                                .hint_text("実値を入力")
                                .desired_width(f32::INFINITY),
                        );
                        if response.changed() {
                            if value.is_empty() {
                                self.env_overrides.remove(field);
                            } else {
                                self.env_overrides.insert(field.clone(), value);
                            }
                            changed = true;
                        }
                    });
                }
            });
        if changed {
            self.reparse();
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
            let unresolved = self.unresolved_placeholder_count();
            let warning = if unresolved > 0 {
                format!(
                    " ⚠ 未入力のプレースホルダーが {} 件残っています",
                    unresolved
                )
            } else {
                String::new()
            };
            self.status_message = Some((
                format!(
                    "{}適用完了: {} 件{} ({})",
                    prefix,
                    success_targets.len(),
                    warning,
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
            self.auto_read_from_clipboard();
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
                    self.read_from_clipboard();
                }

                if ui
                    .button("🗑 クリア")
                    .on_hover_text("入力欄をクリアします")
                    .clicked()
                {
                    self.raw_input.clear();
                    self.env_overrides.clear();
                    self.reparse();
                }

                ui.separator();

                let mut replace = self.replace_npx_with_bunx;
                if ui.checkbox(&mut replace, "npx → bunx 置換").changed() {
                    self.replace_npx_with_bunx = replace;
                    self.reparse();
                }

                let mut replace_pipx = self.replace_pipx_with_uvx;
                if ui
                    .checkbox(&mut replace_pipx, "pipx run → uvx 置換")
                    .on_hover_text(
                        "`pipx run パッケージ名 …` を `uvx パッケージ名 …` に書き換えます（uv が必要）。\n\
                         --spec 等のオプション・URL・バージョン指定つきは書き換えません。",
                    )
                    .changed()
                {
                    self.replace_pipx_with_uvx = replace_pipx;
                    self.reparse();
                }

                ui.checkbox(&mut self.auto_read_clipboard, "自動読み取り");
            });
        });

        // ボトムバー（ステータス表示 & アクションボタン）
        // 長いステータス（警告つき等）がボタンに隠れないよう、左側だけを縮める
        let status = self.status_message.clone();
        let (selection, cli_count) = self.selection_summary();
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
                let text_edit = egui::TextEdit::multiline(&mut self.raw_input)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16);

                let response = ui.add(text_edit);
                if response.changed() {
                    self.reparse();
                }

                self.render_placeholder_inputs(ui);

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
                .max_height(ui.available_height() * TARGET_LIST_HEIGHT_RATIO)
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
        eframe::set_value(storage, AUTO_READ_STORAGE_KEY, &self.auto_read_clipboard);
    }

    /// スクロール位置等の egui 内部状態は復元不要。保存対象は自動読み取りトグルのみとする
    fn persist_egui_memory(&self) -> bool {
        false
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
        assert!(app.replace_pipx_with_uvx);
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
        assert!(!preview.contains("mcp remove"), "codex upserts natively");
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

    #[test]
    fn test_load_clipboard_text_policy() {
        const VALID: &str = r#"{"mcpServers":{"a":{"command":"echo"}}}"#;
        const VALID_OTHER: &str = r#"{"mcpServers":{"b":{"command":"echo"}}}"#;

        struct Case {
            clip: &'static str,
            only_valid: bool,
            seen: Option<&'static str>,
            raw: &'static str,
            expected: bool,
            after: &'static str,
        }
        let case = |clip, only_valid, seen, raw, expected, after| Case {
            clip,
            only_valid,
            seen,
            raw,
            expected,
            after,
        };

        let cases = [
            // 自動: 有効な MCP 設定は取り込む
            case(VALID, true, None, "", true, VALID),
            // 自動: 非 MCP テキスト（パスワード等）は取り込まない
            case("hunter2", true, None, "", false, ""),
            // 自動: 空白のみは取り込まない
            case(
                "  
", true, None, "", false, "",
            ),
            // 自動: 前回と同一内容なら編集中の入力を上書きしない
            case(VALID, true, Some(VALID), "edited", false, "edited"),
            // 自動: 別の有効な内容なら取り込む
            case(VALID_OTHER, true, Some(VALID), "edited", true, VALID_OTHER),
            // 手動: 非 MCP テキストでも取り込む
            case("hunter2", false, None, "", true, "hunter2"),
            // 手動: 前回と同一内容でも取り込む
            case(VALID, false, Some(VALID), "edited", true, VALID),
            // 手動: 入力欄と同一内容なら何もしない
            case(VALID, false, None, VALID, false, VALID),
        ];

        for c in cases {
            let mut app = Clp2cfgApp::new_with_targets(Vec::new());
            app.last_clipboard_seen = c.seen.map(str::to_string);
            app.raw_input = c.raw.to_string();

            assert_eq!(
                app.load_clipboard_text(c.clip, c.only_valid),
                c.expected,
                "clip={:?} only_valid={} seen={:?} raw={:?}",
                c.clip,
                c.only_valid,
                c.seen,
                c.raw
            );
            assert_eq!(app.raw_input, c.after, "clip={:?}", c.clip);
        }
    }

    #[test]
    fn test_auto_read_skips_non_mcp_then_does_not_retry_same_content() {
        let mut app = Clp2cfgApp::new_with_targets(Vec::new());
        assert!(!app.load_clipboard_text("not json", true));
        // 一度判定した内容は記録され、手動読み取りでは取り込める
        assert_eq!(app.last_clipboard_seen.as_deref(), Some("not json"));
        assert!(app.load_clipboard_text("not json", false));
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
            let mut app = Clp2cfgApp::new_with_targets(Vec::new());
            app.auto_read_clipboard = enabled;
            let mut storage = MemStorage::default();
            eframe::App::save(&mut app, &mut storage);
            assert_eq!(
                eframe::get_value::<bool>(&storage, AUTO_READ_STORAGE_KEY),
                Some(enabled)
            );
        }
    }

    #[test]
    fn test_placeholder_override_flow() {
        const SNIPPET: &str = r#"{"mcpServers":{"gh":{"command":"npx","env":{"GITHUB_TOKEN":"YOUR_TOKEN","REGION":"us-east-1"}}}}"#;
        let token = EnvRef {
            server: "gh".to_string(),
            key: "GITHUB_TOKEN".to_string(),
        };
        let token_value = |app: &Clp2cfgApp| {
            app.parsed_config.as_ref().unwrap()["gh"].env["GITHUB_TOKEN"].clone()
        };

        let mut app = Clp2cfgApp::new_with_targets(Vec::new());
        app.raw_input = SNIPPET.to_string();
        app.reparse();

        // 検知: プレースホルダーのみが対象
        assert_eq!(app.placeholder_fields, vec![token.clone()]);
        assert_eq!(app.unresolved_placeholder_count(), 1);
        assert_eq!(token_value(&app), "YOUR_TOKEN");

        // 実値の入力が parse 結果へ反映され、未入力件数が減る（入力欄は残る）
        app.env_overrides
            .insert(token.clone(), "ghp_real".to_string());
        app.reparse();
        assert_eq!(token_value(&app), "ghp_real");
        assert_eq!(app.unresolved_placeholder_count(), 0);
        assert_eq!(app.placeholder_fields, vec![token.clone()]);

        // npx → bunx 切替などの再パースでも入力値は保持される
        app.replace_npx_with_bunx = false;
        app.reparse();
        assert_eq!(token_value(&app), "ghp_real");

        // クリップボードからの取り込みでは前回の入力値を引き継がない
        let other = SNIPPET.replace("YOUR_TOKEN", "<token>");
        assert!(app.load_clipboard_text(&other, false));
        assert!(app.env_overrides.is_empty());
        assert_eq!(token_value(&app), "<token>");
        assert_eq!(app.unresolved_placeholder_count(), 1);
    }

    #[test]
    fn test_placeholder_override_is_ignored_once_snippet_has_real_value() {
        let token = EnvRef {
            server: "gh".to_string(),
            key: "T".to_string(),
        };
        let mut app = Clp2cfgApp::new_with_targets(Vec::new());
        app.env_overrides.insert(token, "stale".to_string());
        app.raw_input = r#"{"gh":{"command":"x","env":{"T":"real-value"}}}"#.to_string();
        app.reparse();

        assert!(app.placeholder_fields.is_empty());
        assert_eq!(
            app.parsed_config.as_ref().unwrap()["gh"].env["T"],
            "real-value"
        );
    }

    #[test]
    fn test_apply_with_unresolved_placeholder_warns_without_leaking_values() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = Clp2cfgApp::new_with_targets(vec![target]);
        app.raw_input =
            r#"{"gh":{"command":"x","env":{"A":"YOUR_KEY","B":"sk-secret-value"}}}"#.to_string();
        app.reparse();
        app.target_entries[0].target.selected = true;

        app.apply_all(false);

        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err);
        assert!(msg.contains("適用完了"), "{msg}");
        assert!(msg.contains("未入力のプレースホルダーが 1 件"), "{msg}");
        assert!(!msg.contains("sk-secret-value"), "{msg}");
    }

    #[test]
    fn test_selection_summary() {
        let temp = tempdir().unwrap();
        let build = |selected: &[bool]| {
            let kinds = [
                ClientTargetKind::Cursor,
                ClientTargetKind::ClaudeCli,
                ClientTargetKind::CodexCli,
            ];
            let targets = kinds
                .iter()
                .zip(selected)
                .map(|(kind, on)| {
                    let mut t = make_test_target(temp.path(), *kind, "x");
                    t.selected = *on;
                    t
                })
                .collect();
            Clp2cfgApp::new_with_targets(targets)
        };

        struct Case {
            /// 選択状態 [Cursor, Claude CLI, Codex CLI]
            selected: &'static [bool],
            expected_cli: usize,
            contains: &'static [&'static str],
            excludes: &'static [&'static str],
        }

        let cases = [
            Case {
                selected: &[false, false, false],
                expected_cli: 0,
                contains: &["適用対象: なし"],
                excludes: &[],
            },
            Case {
                selected: &[true, false, false],
                expected_cli: 0,
                contains: &["1 件", "Cursor"],
                excludes: &["CLI 実行"],
            },
            Case {
                selected: &[true, true, true],
                expected_cli: 2,
                contains: &["3 件", "うち CLI 実行 2 件", "Claude Code CLI", "Codex CLI"],
                excludes: &[],
            },
            Case {
                selected: &[false, true, false],
                expected_cli: 1,
                contains: &["1 件", "うち CLI 実行 1 件"],
                excludes: &["Cursor"],
            },
        ];

        for case in cases {
            let (text, cli_count) = build(case.selected).selection_summary();
            assert_eq!(cli_count, case.expected_cli, "text={text}");
            for needle in case.contains {
                assert!(text.contains(needle), "text={text}");
            }
            for needle in case.excludes {
                assert!(!text.contains(needle), "text={text}");
            }
        }
    }

    #[test]
    fn test_pipx_run_replacement_follows_toggle() {
        const SNIPPET: &str =
            r#"{"git":{"command":"pipx","args":["run","mcp-server-git","--repository","/r"]}}"#;

        let mut app = Clp2cfgApp::new_with_targets(Vec::new());
        app.raw_input = SNIPPET.to_string();
        app.reparse();
        let config = &app.parsed_config.as_ref().unwrap()["git"];
        assert_eq!(config.command, "uvx");
        assert_eq!(config.args, ["mcp-server-git", "--repository", "/r"]);

        app.replace_pipx_with_uvx = false;
        app.reparse();
        let config = &app.parsed_config.as_ref().unwrap()["git"];
        assert_eq!(config.command, "pipx");
        assert_eq!(config.args[0], "run");
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

        let mut app = Clp2cfgApp::new_with_targets(vec![file_target, cli_target]);
        app.auto_read_clipboard = false;
        app.selected_target_idx = 1;
        app.raw_input = r#"{"srv":{"command":"uvx","args":["mcp-server-everything-and-more"],"env":{"E2E_KEY":"value with space"}}}"#.to_string();
        app.reparse();

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

        let mut app = Clp2cfgApp::new_with_targets(vec![
            ClientTarget::new(ClientTargetKind::Cursor, path),
            cli,
        ]);
        app.auto_read_clipboard = false;
        app.replace_npx_with_bunx = false;
        app.raw_input = r#"{"same":{"command":"node","args":["a"]},"diff":{"command":"new"},"fresh":{"command":"x"}}"#.to_string();
        app.reparse();

        let entry = &app.target_entries[0];
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
        assert!(app.target_entries[1].changes.is_none());
        assert_eq!(app.target_entries[1].overwrite_count(), 0);

        // 表示（一覧の警告・見出しのバッジ・CLI の注記）で panic しない
        let ctx = egui::Context::default();
        run_headless(&ctx, &mut app);
        app.selected_target_idx = 1;
        run_headless(&ctx, &mut app);

        // 入力が無効になれば判定も消える
        app.raw_input = "{ invalid".to_string();
        app.reparse();
        assert!(app.target_entries.iter().all(|e| e.changes.is_none()));
        app.raw_input.clear();
        app.reparse();
        assert!(app.target_entries.iter().all(|e| e.changes.is_none()));
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
}
