//! The window's state and how it changes. Nothing here draws or touches the OS: the clipboard and
//! the files come in through ports, so this is tested without a window.

use super::action::UiAction;
use super::ports::ClipboardSource;
use crate::apply::{
    apply_targets, plan_target, undo_file, Applied, LastApply, Mode, ServerChanges, TargetIo,
    UndoAction,
};
use crate::domain::{ClientTarget, McpServersMap, ServerChange, TargetAction};
use crate::snippet::filter::FilterOptions;
use crate::snippet::pipeline::{prepare, Prepared};
use crate::snippet::placeholder::{is_placeholder, EnvOverrides, EnvRef};

/// 各クライアント設定ターゲットの GUI 表示・状態
#[derive(Debug, Clone)]
pub struct TargetEntry {
    pub target: ClientTarget,
    /// 「適用」の対象にするか。存在するターゲットは、最初から選択済みにする
    pub selected: bool,
    pub diff_preview: Option<String>,
    /// サーバーごとの「新規追加 / 上書き / 変更なし」（ファイル系ターゲットのみ）
    pub changes: Option<ServerChanges>,
    pub error: Option<String>,
}

impl TargetEntry {
    pub fn new(target: ClientTarget) -> Self {
        Self {
            selected: target.exists,
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

/// 起動引数から渡される、GUI の初期状態
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupOptions {
    /// 起動引数の INPUT。`Ok` は流し込む内容、`Err` は読み込みに失敗したメッセージ
    pub initial_input: Option<Result<String, String>>,
    pub replace_npx_with_bunx: bool,
    pub replace_pipx_with_uvx: bool,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            initial_input: None,
            replace_npx_with_bunx: true,
            replace_pipx_with_uvx: true,
        }
    }
}

/// clp2cfg のウィンドウの状態
pub struct AppState {
    /// 入力スニペット文字列
    pub raw_input: String,
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
    /// 入力欄の値を平文で表示するか
    pub reveal_env_inputs: bool,
    /// スニペット内のプレースホルダー env（入力済みのものも含む。入力欄を消さないため）
    placeholder_fields: Vec<EnvRef>,
    /// ミニ入力欄で入力された実値（シークレットを含むためログ・ステータスには出さない）
    env_overrides: EnvOverrides,
    /// 直前の（Dry-Run でない）適用の記録。ファイル系を書き込めたときだけ持ち、メモリ上のみ
    last_apply: Option<LastApply>,
    /// 前回クリップボードから取り込んだ（または判定した）内容。同一内容の再取込で編集中の入力を上書きしないために保持する
    last_clipboard_seen: Option<String>,
    io: Box<dyn TargetIo>,
    clipboard: Box<dyn ClipboardSource>,
}

impl AppState {
    /// 指定されたターゲット一覧で初期化する
    pub fn new(
        targets: Vec<ClientTarget>,
        io: Box<dyn TargetIo>,
        clipboard: Box<dyn ClipboardSource>,
    ) -> Self {
        let target_entries = targets.into_iter().map(TargetEntry::new).collect();
        Self {
            raw_input: String::new(),
            parsed_config: Err("入力が空です".to_string()),
            replace_npx_with_bunx: true,
            replace_pipx_with_uvx: true,
            target_entries,
            selected_target_idx: 0,
            status_message: None,
            auto_read_clipboard: true,
            reveal_env_inputs: false,
            placeholder_fields: Vec::new(),
            env_overrides: EnvOverrides::new(),
            last_apply: None,
            last_clipboard_seen: None,
            io,
            clipboard,
        }
    }

    /// ユーザーの操作を反映する
    pub fn update(&mut self, action: UiAction) {
        match action {
            UiAction::ReadClipboard => {
                self.read_from_clipboard();
            }
            UiAction::AutoReadClipboard => {
                self.auto_read_from_clipboard();
            }
            UiAction::Clear => {
                self.raw_input.clear();
                self.env_overrides.clear();
                self.reparse();
            }
            UiAction::InputEdited => self.reparse(),
            UiAction::SetReplaceNpxWithBunx(on) => {
                self.replace_npx_with_bunx = on;
                self.reparse();
            }
            UiAction::SetReplacePipxWithUvx(on) => {
                self.replace_pipx_with_uvx = on;
                self.reparse();
            }
            UiAction::SetPlaceholder { field, value } => {
                if value.is_empty() {
                    self.env_overrides.remove(&field);
                } else {
                    self.env_overrides.insert(field, value);
                }
                self.reparse();
            }
            UiAction::Apply => self.apply_all(false),
            UiAction::DryRun => self.apply_all(true),
            UiAction::Undo => self.undo_last_apply(),
        }
    }

    /// 起動引数を反映する。INPUT があるときはそれを流し込み、起動時のクリップボード自動読み取りは行わない
    /// （引数で明示された入力を、クリップボードの内容で上書きしないため）。
    pub fn apply_startup(&mut self, startup: StartupOptions) {
        self.replace_npx_with_bunx = startup.replace_npx_with_bunx;
        self.replace_pipx_with_uvx = startup.replace_pipx_with_uvx;

        match startup.initial_input {
            Some(Ok(text)) => self.preload_input(&text),
            Some(Err(message)) => self.status_message = Some((message, true)),
            None if self.auto_read_clipboard => {
                self.auto_read_from_clipboard();
            }
            None => {}
        }
    }

    /// 入力欄へ内容を流し込み、パースとプレビューまで済ませる
    fn preload_input(&mut self, text: &str) {
        self.raw_input = text.to_string();
        self.env_overrides.clear();
        self.reparse();
        if self.parsed_config.is_ok() {
            self.status_message = Some(("起動引数の入力を読み込みました".to_string(), false));
        }
        // この時点のクリップボードは「判定済み」とし、後から別の内容がコピーされたときだけ自動読み取りの対象にする
        self.last_clipboard_seen = self.clipboard.text().map(|text| text.trim().to_string());
    }

    /// 手動操作（ボタン・Ctrl+V）用: OS クリップボードの文字列を内容を問わず入力欄に展開してパースする
    fn read_from_clipboard(&mut self) -> bool {
        match self.clipboard.text() {
            Some(text) => self.load_clipboard_text(&text, false),
            None => false,
        }
    }

    /// 自動読み取り用（起動時・フォーカス獲得時）
    fn auto_read_from_clipboard(&mut self) -> bool {
        match self.clipboard.text() {
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
        if only_valid_mcp && prepare(text, &FilterOptions::default(), &EnvOverrides::new()).is_err()
        {
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

    /// 入力文字列をパースし、差分プレビューを更新する
    fn reparse(&mut self) {
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

        let filters = FilterOptions {
            replace_npx_with_bunx: self.replace_npx_with_bunx,
            replace_pipx_with_uvx: self.replace_pipx_with_uvx,
        };
        match prepare(&self.raw_input, &filters, &self.env_overrides) {
            Ok(Prepared {
                servers: filtered_servers,
                placeholders,
            }) => {
                self.placeholder_fields = placeholders;

                // 各ターゲットに対するマージプレビューを計算
                for entry in &mut self.target_entries {
                    match plan_target(&entry.target, &filtered_servers, self.io.as_ref()) {
                        Ok(plan) => {
                            entry.diff_preview = Some(plan.preview());
                            entry.changes = plan.changes().cloned();
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

    /// 「適用」「検証」を押せるか（有効な設定があり、対象が 1 つ以上選ばれている）
    pub fn can_apply(&self) -> bool {
        self.parsed_config.is_ok() && self.target_entries.iter().any(|e| e.selected)
    }

    /// 「適用」で更新される対象の要約と、そのうち CLI を実行する件数。
    /// 一覧の画面外にあるチェック済みターゲットにも気付けるよう、ボトムバーに常時表示する。
    pub fn selection_summary(&self) -> (String, usize) {
        let selected: Vec<_> = self.target_entries.iter().filter(|e| e.selected).collect();
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

    /// スニペット内のプレースホルダー env（入力済みのものも含む）
    pub fn placeholder_fields(&self) -> &[EnvRef] {
        &self.placeholder_fields
    }

    /// ミニ入力欄に入力済みの実値（未入力なら空）
    pub fn placeholder_value(&self, field: &EnvRef) -> String {
        self.env_overrides.get(field).cloned().unwrap_or_default()
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

    /// 取り消しボタンの説明（取り消せる適用がなければ `None`）
    pub fn undo_hint(&self) -> Option<String> {
        let last = self.last_apply.as_ref()?;
        let names = last
            .files
            .iter()
            .map(|f| f.target_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let mut hint = format!(
            "直前の適用で書き込んだファイルを元に戻します（{}）。\n\
             既存ファイルは .bak から復元し、新規作成したファイルは削除します。\n\
             適用後に内容が変わっているファイルは取り消しません。",
            names
        );
        if !last.cli_targets.is_empty() {
            hint.push_str(&format!(
                "\nCLI 委譲（{}）は取り消せません。各 CLI の `mcp remove` で削除してください。",
                last.cli_targets.join(", ")
            ));
        }
        Some(hint)
    }

    /// 直前の適用で書き込んだファイルを元に戻す（ファイル系のみ）
    fn undo_last_apply(&mut self) {
        let Some(last) = self.last_apply.take() else {
            self.status_message = Some(("取り消せる適用がありません".to_string(), true));
            return;
        };

        let mut done = Vec::new();
        let mut failed = Vec::new();
        for file in &last.files {
            match undo_file(file, self.io.as_ref()) {
                Ok(action) => {
                    let label = match action {
                        UndoAction::Restored => "復元",
                        UndoAction::Removed => "削除",
                        UndoAction::AlreadyGone => "既に削除済み",
                    };
                    done.push(format!("{}({})", file.target_name, label));
                }
                Err(err) => failed.push(format!("{}: {}", file.target_name, err)),
            }
            if let Some(entry) = self
                .target_entries
                .iter_mut()
                .find(|e| e.target.path == file.path)
            {
                entry.target.exists = self.io.config_exists(&entry.target.path);
            }
        }

        // ファイルが変わったので、プレビューと判定を作り直す（ステータスを上書きするので先に行う）
        self.reparse();

        let mut message = if failed.is_empty() {
            format!("取り消し完了: {} 件 ({})", done.len(), done.join(", "))
        } else {
            format!(
                "取り消しに失敗: 成功 {} 件, 失敗 {} 件 ({})",
                done.len(),
                failed.len(),
                failed.join("; ")
            )
        };
        if !last.cli_targets.is_empty() {
            message.push_str(&format!(
                " ※ CLI 委譲（{}）は取り消せません",
                last.cli_targets.join(", ")
            ));
        }
        self.status_message = Some((message, !failed.is_empty()));
    }

    /// 選択されたターゲットにマージを適用（または Dry-Run シミュレーション）する
    fn apply_all(&mut self, dry_run: bool) {
        let servers = match &self.parsed_config {
            Ok(s) => s.clone(),
            Err(_) => {
                self.status_message =
                    Some(("有効なMCP設定がパースされていません".to_string(), true));
                return;
            }
        };

        let selected: Vec<usize> = self
            .target_entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.selected)
            .map(|(idx, _)| idx)
            .collect();
        if selected.is_empty() {
            self.status_message =
                Some(("適用対象のターゲットが選択されていません".to_string(), true));
            return;
        }

        let mode = if dry_run { Mode::DryRun } else { Mode::Apply };
        let outcome = apply_targets(
            selected.iter().map(|&idx| &self.target_entries[idx].target),
            &servers,
            mode,
            self.io.as_ref(),
        );

        let mut success_targets = Vec::new();
        let mut fail_targets = Vec::new();
        for (&idx, result) in selected.iter().zip(&outcome.results) {
            match &result.result {
                Ok(_) if dry_run => success_targets.push(format!("{}(検証OK)", result.name)),
                Ok(applied) => {
                    // 書き込めたファイルは「既存」になる
                    if matches!(applied, Applied::File { .. }) {
                        self.target_entries[idx].target.exists = true;
                    }
                    success_targets.push(result.name.clone());
                }
                Err(err) => fail_targets.push(format!("{}: {}", result.name, err)),
            }
        }
        let applied_files = outcome.applied_files();
        let applied_clis = outcome.executed_cli_names();

        // 何も変更できなかった適用では、直前の記録を残す。CLI だけを実行した適用は取り消せないので、
        // 古い記録が「直前の適用」と誤認されないよう破棄する
        if !dry_run && (!applied_files.is_empty() || !applied_clis.is_empty()) {
            self.last_apply = (!applied_files.is_empty()).then_some(LastApply {
                files: applied_files,
                cli_targets: applied_clis,
            });
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ports::FakeClipboard;
    use crate::apply::{AppliedFile, SystemIo};
    use crate::domain::{ClientTarget, ClientTargetKind};
    use tempfile::tempdir;

    /// 実ファイルを使い、クリップボードはフェイクの状態
    fn new_state(targets: Vec<ClientTarget>) -> AppState {
        AppState::new(
            targets,
            Box::new(SystemIo),
            Box::new(FakeClipboard::default()),
        )
    }

    /// クリップボードの内容を test から差し替えられる状態
    fn new_state_with_clipboard(targets: Vec<ClientTarget>) -> (AppState, FakeClipboard) {
        let clipboard = FakeClipboard::default();
        let state = AppState::new(targets, Box::new(SystemIo), Box::new(clipboard.clone()));
        (state, clipboard)
    }

    fn make_test_target(dir: &std::path::Path, kind: ClientTargetKind, file: &str) -> ClientTarget {
        crate::discovery::file_target(kind, dir.join(file))
    }

    #[test]
    fn test_app_reparse_valid_snippet() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Claude, "claude.json");
        let mut app = new_state(vec![target]);

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
        let mut app = new_state(vec![target]);

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

        let mut app = new_state(vec![target]);
        // ターゲットを選択状態にする
        app.target_entries[0].selected = true;

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
        let app = new_state(Vec::new());
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
        let mut app = new_state(vec![target]);

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
        let mut app = new_state(vec![target]);
        app.target_entries[0].selected = true;

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

        let mut app = new_state(vec![target]);
        app.target_entries[0].selected = true;

        app.raw_input = r#"{"test": {"command": "echo"}}"#.to_string();
        app.reparse();

        app.apply_all(false);
        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
        assert!(app.status_message.as_ref().unwrap().0.contains("一部失敗"));
    }

    #[test]
    fn test_app_unreadable_existing_file_is_an_error_and_is_not_overwritten() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let bytes = vec![b'{', 0xC3, 0x28, b'}']; // 不正な UTF-8
        std::fs::write(&target.path, &bytes).unwrap();
        let target = crate::discovery::file_target(ClientTargetKind::Cursor, target.path);
        let path = target.path.clone();

        let mut app = new_state(vec![target]);
        app.raw_input = r#"{"test": {"command": "echo"}}"#.to_string();
        app.reparse();

        let entry = &app.target_entries[0];
        assert!(entry.diff_preview.is_none());
        assert!(entry
            .error
            .as_ref()
            .is_some_and(|e| e.contains("設定ファイルを読み取れません")));

        app.apply_all(false);

        assert!(app.status_message.as_ref().unwrap().1); // is_error = true
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!crate::writer::backup_path_for(&path).exists());
    }

    #[test]
    fn test_app_apply_no_targets_selected() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let mut app = new_state(vec![target]);
        app.target_entries[0].selected = false;

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
        let mut app = new_state(vec![target]);
        app.target_entries[0].selected = true;
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
            let mut app = new_state(Vec::new());
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
        let mut app = new_state(Vec::new());
        assert!(!app.load_clipboard_text("not json", true));
        // 一度判定した内容は記録され、手動読み取りでは取り込める
        assert_eq!(app.last_clipboard_seen.as_deref(), Some("not json"));
        assert!(app.load_clipboard_text("not json", false));
    }

    #[test]
    fn test_placeholder_override_flow() {
        const SNIPPET: &str = r#"{"mcpServers":{"gh":{"command":"npx","env":{"GITHUB_TOKEN":"YOUR_TOKEN","REGION":"us-east-1"}}}}"#;
        let token = EnvRef {
            server: "gh".to_string(),
            key: "GITHUB_TOKEN".to_string(),
        };
        let token_value =
            |app: &AppState| app.parsed_config.as_ref().unwrap()["gh"].env["GITHUB_TOKEN"].clone();

        let mut app = new_state(Vec::new());
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
        let mut app = new_state(Vec::new());
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
        let mut app = new_state(vec![target]);
        app.raw_input =
            r#"{"gh":{"command":"x","env":{"A":"YOUR_KEY","B":"sk-secret-value"}}}"#.to_string();
        app.reparse();
        app.target_entries[0].selected = true;

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
                .map(|kind| make_test_target(temp.path(), *kind, "x"))
                .collect();
            let mut app = new_state(targets);
            for (entry, on) in app.target_entries.iter_mut().zip(selected) {
                entry.selected = *on;
            }
            app
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

        let mut app = new_state(Vec::new());
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

    #[test]
    fn test_target_entry_is_selected_by_default_only_when_the_target_exists() {
        let cases = [
            (
                ClientTarget::new(ClientTargetKind::Cursor, "a.json".into(), true),
                true,
            ),
            (
                ClientTarget::new(ClientTargetKind::Cursor, "a.json".into(), false),
                false,
            ),
            (
                ClientTarget::new_cli(ClientTargetKind::CodexCli, Some("codex".into())),
                true,
            ),
            (
                ClientTarget::new_cli(ClientTargetKind::CodexCli, None),
                false,
            ),
        ];
        for (target, expected) in cases {
            let name = format!("{:?} exists={}", target.kind, target.exists);
            assert_eq!(TargetEntry::new(target).selected, expected, "{name}");
        }
    }
    /// 新規 or 既存のファイルターゲット 1 件を選択したアプリと、そのファイルパス
    fn app_with_file_target(
        dir: &std::path::Path,
        existing: Option<&str>,
    ) -> (AppState, std::path::PathBuf) {
        let path = dir.join("mcp.json");
        if let Some(content) = existing {
            std::fs::write(&path, content).unwrap();
        }
        let target = crate::discovery::file_target(ClientTargetKind::Cursor, path.clone());
        let mut app = new_state(vec![target]);
        app.target_entries[0].selected = true;
        app.auto_read_clipboard = false;
        app.replace_npx_with_bunx = false;
        app.raw_input = r#"{"added":{"command":"node"}}"#.to_string();
        app.reparse();
        (app, path)
    }

    /// 適用のステータスメッセージの書式と、取り消し用の記録の中身を固定する
    #[test]
    fn test_apply_status_messages_and_last_apply_record() {
        let temp = tempdir().unwrap();
        let existing = temp.path().join("existing.json");
        std::fs::write(&existing, r#"{"keep":1}"#).unwrap();
        let ok_existing = crate::discovery::file_target(ClientTargetKind::Cursor, existing.clone());
        let ok_new =
            crate::discovery::file_target(ClientTargetKind::Windsurf, temp.path().join("new.json"));
        // 親パスがファイルのため、書き込みが必ず失敗する
        let blocking = temp.path().join("blocking_file");
        std::fs::write(&blocking, "x").unwrap();
        let bad = crate::discovery::file_target(ClientTargetKind::Cline, blocking.join("mcp.json"));

        let mut app = new_state(vec![ok_existing, ok_new, bad]);
        for entry in &mut app.target_entries {
            entry.selected = true;
        }
        app.auto_read_clipboard = false;
        app.raw_input = r#"{"test":{"command":"echo"}}"#.to_string();
        app.reparse();

        app.apply_all(true);
        let (dry, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err);
        assert_eq!(
            dry,
            "[Dry-Run] 適用完了: 3 件 (Cursor(検証OK), Windsurf(検証OK), Cline (VS Code)(検証OK))"
        );
        assert!(app.last_apply.is_none(), "Dry-Run は記録を作らない");

        app.apply_all(false);
        let (message, is_err) = app.status_message.clone().unwrap();
        assert!(is_err);
        assert!(
            message.starts_with("一部失敗: 成功 2 件, 失敗 1 件 (Cline (VS Code): "),
            "{message}"
        );

        // 取り消し用の記録: 書き込めた 2 件だけ。新規作成かどうか、書いた内容を持つ
        let last = app.last_apply.as_ref().unwrap();
        assert_eq!(last.files.len(), 2);
        let by_name = |name: &str| last.files.iter().find(|f| f.target_name == name).unwrap();
        let updated = by_name("Cursor");
        assert_eq!(updated.path, existing);
        assert!(updated.had_original);
        assert_eq!(updated.written, std::fs::read_to_string(&existing).unwrap());
        let created = by_name("Windsurf");
        assert!(!created.had_original);
        assert_eq!(
            created.written,
            std::fs::read_to_string(&created.path).unwrap()
        );
        assert!(last.cli_targets.is_empty());

        // 書き込めたターゲットは「既存」になる
        assert!(app.target_entries[1].target.exists);
        assert!(!app.target_entries[2].target.exists);
    }

    #[test]
    fn test_apply_success_message_lists_every_target() {
        let temp = tempdir().unwrap();
        let (mut app, _path) = app_with_file_target(temp.path(), None);

        app.apply_all(false);

        let (message, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err);
        assert_eq!(message, "適用完了: 1 件 (Cursor)");
    }

    #[test]
    fn test_undo_removes_file_created_by_apply() {
        let temp = tempdir().unwrap();
        let (mut app, path) = app_with_file_target(temp.path(), None);
        assert!(app.undo_hint().is_none(), "適用前は取り消せない");

        app.apply_all(false);
        assert!(path.is_file());
        assert!(app.undo_hint().is_some());

        app.undo_last_apply();

        assert!(!path.exists());
        assert!(!app.target_entries[0].target.exists);
        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err, "{msg}");
        assert!(
            msg.contains("取り消し完了") && msg.contains("削除"),
            "{msg}"
        );
        assert!(app.undo_hint().is_none(), "取り消しは 1 回限り");
    }

    #[test]
    fn test_undo_restores_existing_file_from_backup() {
        let temp = tempdir().unwrap();
        let original = r#"{"mcpServers":{"keep":{"command":"x"}}}"#;
        let (mut app, path) = app_with_file_target(temp.path(), Some(original));

        app.apply_all(false);
        assert!(std::fs::read_to_string(&path).unwrap().contains("added"));

        app.undo_last_apply();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err && msg.contains("復元"), "{msg}");
        // 取り消し後はプレビューが元のファイルから作り直される
        let changes = app.target_entries[0].changes.as_ref().unwrap();
        assert_eq!(changes["added"], ServerChange::New);
    }

    #[test]
    fn test_undo_refuses_when_file_changed_after_apply() {
        let temp = tempdir().unwrap();
        let (mut app, path) = app_with_file_target(temp.path(), Some(r#"{"a":1}"#));
        app.apply_all(false);
        std::fs::write(&path, "edited after apply").unwrap();

        app.undo_last_apply();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "edited after apply"
        );
        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(is_err && msg.contains("取り消しに失敗"), "{msg}");
    }

    #[test]
    fn test_dry_run_and_failed_apply_do_not_replace_last_apply() {
        let temp = tempdir().unwrap();
        let (mut app, _path) = app_with_file_target(temp.path(), None);

        // Dry-Run は記録しない
        app.apply_all(true);
        assert!(app.undo_hint().is_none());

        // 適用すると記録される。その後、何も変更できなかった適用（存在しない CLI）では記録を残す
        app.apply_all(false);
        let recorded = app.last_apply.clone();
        assert!(recorded.is_some());

        let mut cli = ClientTarget::new_cli(ClientTargetKind::CodexCli, None);
        cli.path = std::path::PathBuf::from("clp2cfg-nonexistent-binary");
        app.target_entries[0].selected = false;
        let mut cli_entry = TargetEntry::new(cli);
        cli_entry.selected = true;
        app.target_entries.push(cli_entry);
        app.apply_all(false);
        assert!(
            app.status_message.as_ref().unwrap().1,
            "CLI の実行は失敗する"
        );
        assert_eq!(app.last_apply, recorded);
    }

    #[test]
    fn test_undo_hint_mentions_cli_targets_cannot_be_undone() {
        let mut app = new_state(Vec::new());
        app.last_apply = Some(LastApply {
            files: vec![AppliedFile {
                target_name: "Cursor".to_string(),
                path: std::path::PathBuf::from("x"),
                written: String::new(),
                had_original: false,
            }],
            cli_targets: vec!["Codex CLI".to_string()],
        });

        let hint = app.undo_hint().unwrap();

        assert!(hint.contains("Cursor"), "{hint}");
        assert!(
            hint.contains("Codex CLI") && hint.contains("取り消せません"),
            "{hint}"
        );
    }
    #[test]
    fn test_apply_startup_preloads_input_and_filter_flags() {
        let mut app = new_state(Vec::new());
        app.auto_read_clipboard = true;

        app.apply_startup(StartupOptions {
            initial_input: Some(Ok(
                r#"{"srv":{"command":"npx","args":["-y","pkg"]},"py":{"command":"pipx","args":["run","tool"]}}"#
                    .to_string(),
            )),
            replace_npx_with_bunx: false,
            replace_pipx_with_uvx: false,
        });

        let servers = app.parsed_config.as_ref().unwrap();
        assert_eq!(
            servers["srv"].command, "npx",
            "--no-npx-to-bunx が反映される"
        );
        assert_eq!(
            servers["py"].command, "pipx",
            "--no-pipx-to-uvx が反映される"
        );
        assert!(!app.replace_npx_with_bunx && !app.replace_pipx_with_uvx);
        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(!is_err && msg.contains("起動引数"), "{msg}");
        // 永続化されるトグルは起動引数で書き換えない
        assert!(app.auto_read_clipboard);
    }

    #[test]
    fn test_apply_startup_reports_input_errors_and_keeps_the_error_visible() {
        let mut app = new_state(Vec::new());
        app.auto_read_clipboard = true;

        app.apply_startup(StartupOptions {
            initial_input: Some(Err("ファイルを読み取れません: x".to_string())),
            ..StartupOptions::default()
        });

        let (msg, is_err) = app.status_message.clone().unwrap();
        assert!(is_err && msg.contains("ファイルを読み取れません"), "{msg}");
        assert!(app.raw_input.is_empty());
    }

    #[test]
    fn test_apply_startup_with_invalid_input_shows_the_parse_error() {
        let mut app = new_state(Vec::new());

        app.apply_startup(StartupOptions {
            initial_input: Some(Ok("{ not json".to_string())),
            ..StartupOptions::default()
        });

        assert!(app.parsed_config.is_err());
        assert!(app.status_message.as_ref().unwrap().1);
    }

    const SNIPPET: &str = r#"{"mcpServers":{"a":{"command":"echo"}}}"#;
    const OTHER_SNIPPET: &str = r#"{"mcpServers":{"b":{"command":"echo"}}}"#;

    #[test]
    fn test_clipboard_actions_read_through_the_port() {
        let (mut state, clipboard) = new_state_with_clipboard(Vec::new());

        // クリップボードが空なら何も起きない
        state.update(UiAction::ReadClipboard);
        state.update(UiAction::AutoReadClipboard);
        assert!(state.raw_input.is_empty() && state.status_message.is_none());

        // 手動は内容を問わず取り込む
        clipboard.set(Some("not json"));
        state.update(UiAction::ReadClipboard);
        assert_eq!(state.raw_input, "not json");
        assert_eq!(
            state.status_message,
            Some(("クリップボードから読み取りました".to_string(), false))
        );

        // 自動は MCP 設定として有効なものだけ。判定済みの内容は繰り返さない
        clipboard.set(Some("still not json"));
        state.update(UiAction::AutoReadClipboard);
        assert_eq!(state.raw_input, "not json");
        clipboard.set(Some(SNIPPET));
        state.update(UiAction::AutoReadClipboard);
        assert_eq!(state.raw_input, SNIPPET);
        assert!(state.parsed_config.is_ok());
        state.raw_input = "edited".to_string();
        state.update(UiAction::AutoReadClipboard);
        assert_eq!(
            state.raw_input, "edited",
            "同じ内容で編集中の入力を上書きしない"
        );
    }

    #[test]
    fn test_startup_input_marks_the_current_clipboard_as_seen() {
        let (mut state, clipboard) = new_state_with_clipboard(Vec::new());
        clipboard.set(Some(SNIPPET));

        state.apply_startup(StartupOptions {
            initial_input: Some(Ok(OTHER_SNIPPET.to_string())),
            ..Default::default()
        });
        assert_eq!(state.raw_input, OTHER_SNIPPET);

        // 起動時のクリップボードは判定済み。後から別の内容がコピーされたときだけ取り込む
        state.update(UiAction::AutoReadClipboard);
        assert_eq!(state.raw_input, OTHER_SNIPPET);
        clipboard.set(Some(r#"{"mcpServers":{"c":{"command":"echo"}}}"#));
        state.update(UiAction::AutoReadClipboard);
        assert!(state.raw_input.contains("\"c\""));
    }

    #[test]
    fn test_update_handles_every_action() {
        let temp = tempdir().unwrap();
        let target = make_test_target(temp.path(), ClientTargetKind::Cursor, "mcp.json");
        let (mut state, clipboard) = new_state_with_clipboard(vec![target]);
        state.target_entries[0].selected = true;
        clipboard.set(Some(
            r#"{"mcpServers":{"s":{"command":"npx","args":["pkg"],"env":{"KEY":"YOUR_API_KEY"}}}}"#,
        ));

        state.update(UiAction::ReadClipboard);
        assert!(state.can_apply());
        let command = |state: &AppState| state.parsed_config.as_ref().unwrap()["s"].command.clone();
        assert_eq!(command(&state), "bunx");
        state.update(UiAction::SetReplaceNpxWithBunx(false));
        assert_eq!(command(&state), "npx");
        state.update(UiAction::SetReplaceNpxWithBunx(true));
        assert_eq!(command(&state), "bunx");

        // プレースホルダーに実値を入れる／空にして取り消す
        let field = state.placeholder_fields()[0].clone();
        assert_eq!(state.unresolved_placeholder_count(), 1);
        state.update(UiAction::SetPlaceholder {
            field: field.clone(),
            value: "real".to_string(),
        });
        assert_eq!(state.unresolved_placeholder_count(), 0);
        assert_eq!(state.placeholder_value(&field), "real");
        state.update(UiAction::SetPlaceholder {
            field: field.clone(),
            value: String::new(),
        });
        assert_eq!(state.unresolved_placeholder_count(), 1);

        state.update(UiAction::SetReplacePipxWithUvx(false));
        assert!(!state.replace_pipx_with_uvx);
        state.raw_input = SNIPPET.to_string();
        state.update(UiAction::InputEdited);
        assert!(state.parsed_config.as_ref().unwrap().contains_key("a"));

        state.update(UiAction::DryRun);
        assert!(!temp.path().join("mcp.json").exists());
        assert!(state.undo_hint().is_none());
        state.update(UiAction::Apply);
        assert!(temp.path().join("mcp.json").exists());
        assert!(state.undo_hint().is_some());
        state.update(UiAction::Undo);
        assert!(!temp.path().join("mcp.json").exists());
        assert!(state.undo_hint().is_none());

        state.update(UiAction::Clear);
        assert!(state.raw_input.is_empty());
        assert!(state.parsed_config.is_err());
        assert!(state.placeholder_fields().is_empty());
    }
    #[test]
    fn test_startup_options_default_matches_gui_defaults() {
        let startup = StartupOptions::default();
        assert!(startup.initial_input.is_none());
        assert!(startup.replace_npx_with_bunx && startup.replace_pipx_with_uvx);
    }
}
