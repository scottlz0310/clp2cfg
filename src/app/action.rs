//! What the user can ask the window to do. Each one is handled by [`super::AppState::update`].

use crate::snippet::placeholder::EnvRef;

/// Every change a view asks of [`super::AppState`]. Views only read the state and return these;
/// the window applies them at the end of the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// 手動操作（ボタン・Ctrl+V）: クリップボードの内容を問わず入力欄へ取り込む
    ReadClipboard,
    /// 自動読み取り（フォーカス獲得時）: MCP 設定として有効で、前回判定と違う内容だけ取り込む
    AutoReadClipboard,
    /// 入力欄と、入力済みのプレースホルダー実値を空にする
    Clear,
    /// 入力欄が編集された（新しい内容全体）
    SetInput(String),
    /// 右ペインで差分を表示するターゲットを選ぶ
    SelectTarget(usize),
    /// 「適用」の対象にするかを切り替える
    SetTargetSelected {
        idx: usize,
        selected: bool,
    },
    /// プレースホルダー入力欄の値を平文で表示するか
    SetRevealEnv(bool),
    /// フォーカス獲得時の自動読み取りトグル（次回起動へ永続化される）
    SetAutoRead(bool),
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
