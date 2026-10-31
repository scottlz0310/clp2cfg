//! What the user can ask the window to do. Each one is handled by [`super::AppState::update`].

use crate::snippet::placeholder::EnvRef;

/// An operation that changes more than what is shown (parsing, applying, reading the clipboard…).
/// Purely visual state (which target is highlighted, the checkboxes, "show values") is plain fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// 手動操作（ボタン・Ctrl+V）: クリップボードの内容を問わず入力欄へ取り込む
    ReadClipboard,
    /// 自動読み取り（フォーカス獲得時）: MCP 設定として有効で、前回判定と違う内容だけ取り込む
    AutoReadClipboard,
    /// 入力欄と、入力済みのプレースホルダー実値を空にする
    Clear,
    /// 入力欄（`raw_input`）がウィジェットで編集された
    InputEdited,
    SetReplaceNpxWithBunx(bool),
    SetReplacePipxWithUvx(bool),
    /// プレースホルダーの実値を入力した（空文字は入力の取り消し）
    SetPlaceholder {
        field: EnvRef,
        value: String,
    },
    /// 選択したターゲットへ書き込む／実行する
    Apply,
    /// 何も書き込まずに検証する
    DryRun,
    /// 直前の適用で書き込んだファイルを元に戻す
    Undo,
}
