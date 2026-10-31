//! The OS clipboard.

/// Windows 等で並行テスト実行時に OLE/COM クリップボードの同時初期化によるヒープ破損を防ぐためのロック
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// OS クリップボードの文字列を取得
pub fn get_clipboard_text() -> Option<String> {
    let _guard = CLIPBOARD_LOCK.lock().ok()?;
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}
