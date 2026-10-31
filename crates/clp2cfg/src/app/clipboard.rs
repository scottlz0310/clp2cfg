//! The OS clipboard.

use super::ports::ClipboardSource;

/// Windows 等で並行テスト実行時に OLE/COM クリップボードの同時初期化によるヒープ破損を防ぐためのロック
static CLIPBOARD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The real clipboard.
pub struct ArboardClipboard;

impl ClipboardSource for ArboardClipboard {
    fn text(&self) -> Option<String> {
        let _guard = CLIPBOARD_LOCK.lock().ok()?;
        let mut clipboard = arboard::Clipboard::new().ok()?;
        clipboard.get_text().ok()
    }
}
