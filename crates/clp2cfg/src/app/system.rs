//! Things the window needs from the OS: fonts, the app icon, and where settings are kept.

/// 自動読み取りトグルの永続化キー
pub const AUTO_READ_STORAGE_KEY: &str = "auto_read_clipboard";

/// OS 標準の日本語/CJKフォントを探索して egui のフォント定義に登録
pub fn setup_system_fonts(ctx: &egui::Context) {
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
    const ICON_BYTES: &[u8] = include_bytes!("../../../../assets/icon.png");
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
