use crate::app::{AppState, UiAction};

/// キーボードショートカット (Esc, Ctrl+Enter, Ctrl+V)。Esc はウィンドウを閉じ、残りは操作として返す。
pub fn shortcuts(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let esc_pressed = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let ctrl_enter_pressed = ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
            || i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
    });
    let ctrl_v_pressed = ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::COMMAND, egui::Key::V)
            || i.consume_key(egui::Modifiers::CTRL, egui::Key::V)
    });

    let mut actions = Vec::new();

    // Esc: ウィンドウを安全に閉じる
    if esc_pressed {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // Ctrl+Enter: 選択ターゲットにマージを適用 (Apply)
    if ctrl_enter_pressed && state.can_apply() {
        actions.push(UiAction::Apply);
    }

    // Ctrl+V: テキスト入力ウィジェットにフォーカスがない場合にクリップボードから読み取り
    // （テキスト入力中の通常の貼り付けは TextEdit 側で自動処理される）
    if ctrl_v_pressed {
        let has_text_focus = ui.memory(|m| m.focused().is_some());
        if !has_text_focus {
            actions.push(UiAction::ReadClipboard);
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ports::FakeClipboard;
    use clp2cfg_core::apply::SystemIo;
    use clp2cfg_core::domain::{ClientTarget, ClientTargetKind};

    fn state(can_apply: bool) -> AppState {
        let target = ClientTarget::new(ClientTargetKind::Cursor, "mcp.json".into(), true);
        let mut state = AppState::new(
            vec![target],
            Box::new(SystemIo),
            Box::new(FakeClipboard::default()),
        );
        if can_apply {
            state.update(UiAction::SetInput(
                r#"{"srv":{"command":"node"}}"#.to_string(),
            ));
        }
        state
    }

    /// 1 フレームだけ、指定のキー入力で `shortcuts` を呼ぶ。返す操作と、閉じる指示の有無
    fn press(state: &AppState, key: egui::Key, ctrl: bool) -> (Vec<UiAction>, bool) {
        let ctx = egui::Context::default();
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl,
                ..Default::default()
            },
        });
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(raw_input, |ui| actions = shortcuts(ui, state));
        output.textures_delta.clear();
        let closes = output.viewport_output.values().any(|vp| {
            vp.commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::Close))
        });
        (actions, closes)
    }

    #[test]
    fn test_shortcuts_return_actions_and_only_escape_closes() {
        struct Case {
            name: &'static str,
            key: egui::Key,
            ctrl: bool,
            can_apply: bool,
            expected: Vec<UiAction>,
            closes: bool,
        }
        let cases = [
            Case {
                name: "Esc closes the window",
                key: egui::Key::Escape,
                ctrl: false,
                can_apply: true,
                expected: vec![],
                closes: true,
            },
            Case {
                name: "Ctrl+Enter applies when something can be applied",
                key: egui::Key::Enter,
                ctrl: true,
                can_apply: true,
                expected: vec![UiAction::Apply],
                closes: false,
            },
            Case {
                name: "Ctrl+Enter does nothing without a valid snippet and a target",
                key: egui::Key::Enter,
                ctrl: true,
                can_apply: false,
                expected: vec![],
                closes: false,
            },
            Case {
                name: "Enter alone does nothing",
                key: egui::Key::Enter,
                ctrl: false,
                can_apply: true,
                expected: vec![],
                closes: false,
            },
            Case {
                name: "Ctrl+V reads the clipboard when no text box has focus",
                key: egui::Key::V,
                ctrl: true,
                can_apply: false,
                expected: vec![UiAction::ReadClipboard],
                closes: false,
            },
        ];

        for case in cases {
            let (actions, closes) = press(&state(case.can_apply), case.key, case.ctrl);
            assert_eq!(actions, case.expected, "{}", case.name);
            assert_eq!(closes, case.closes, "{}", case.name);
        }
    }
}
