//! Resolving where the snippet text comes from.

use super::Environment;
use std::path::Path;

/// Where the snippet text came from (decides the wording of a parse error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    Clipboard,
    Stdin,
    File,
    Literal,
}

/// Resolve the snippet text: `None` = clipboard, `-` = stdin, an existing file = its content,
/// anything else = the JSON text itself.
pub fn read_input(
    input: Option<&str>,
    env: &dyn Environment,
) -> Result<(String, InputSource), String> {
    match input {
        None => env
            .clipboard_text()
            .filter(|text| !text.trim().is_empty())
            .map(|text| (text, InputSource::Clipboard))
            .ok_or_else(|| "クリップボードに文字列がありません".to_string()),
        Some("-") => env
            .stdin_text()
            .map(|text| (text, InputSource::Stdin))
            .map_err(|e| format!("標準入力を読み取れません: {e}")),
        Some(value) if Path::new(value).is_file() => std::fs::read_to_string(value)
            .map(|text| (text, InputSource::File))
            .map_err(|e| format!("ファイルを読み取れません: {value}: {e}")),
        Some(value) => Ok((value.to_string(), InputSource::Literal)),
    }
}
