use crate::app::{AppState, UiAction};

/// キーボードショートカット (Esc, Ctrl+Enter, Ctrl+V)。Esc はウィンドウを閉じ、残りは操作として返す。
///
/// Ctrl+V は、キーイベントではなく貼り付けイベント（`Event::Paste`）で届く。eframe（egui-winit）が
/// クリップボードのショートカットを貼り付けイベントに変換して、キーイベントを渡さないため。
pub fn shortcuts(ui: &mut egui::Ui, state: &AppState) -> Vec<UiAction> {
    let esc_pressed = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let ctrl_enter_pressed = ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
            || i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
    });
    let paste_requested = ui.input(|i| {
        i.events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
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
    if paste_requested && !ui.ctx().text_edit_focused() {
        actions.push(UiAction::ReadClipboard);
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

    /// 実機で届く入力。Ctrl+V は、キーイベントではなく貼り付けイベントとして届く（eframe の変換）
    #[derive(Clone, Copy)]
    enum Input {
        Key { key: egui::Key, ctrl: bool },
        Paste,
    }

    fn event(input: Input) -> egui::Event {
        match input {
            Input::Key { key, ctrl } => egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl,
                    ..Default::default()
                },
            },
            Input::Paste => egui::Event::Paste("clipboard text".to_string()),
        }
    }

    fn closes(output: &egui::FullOutput) -> bool {
        output.viewport_output.values().any(|vp| {
            vp.commands
                .iter()
                .any(|cmd| matches!(cmd, egui::ViewportCommand::Close))
        })
    }

    /// 1 フレームだけ、指定の入力で `shortcuts` を呼ぶ。返す操作と、閉じる指示の有無
    fn press(state: &AppState, input: Input) -> (Vec<UiAction>, bool) {
        let ctx = egui::Context::default();
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(event(input));
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(raw_input, |ui| actions = shortcuts(ui, state));
        output.textures_delta.clear();
        (actions, closes(&output))
    }

    #[test]
    fn test_shortcuts_return_actions_and_only_escape_closes() {
        struct Case {
            name: &'static str,
            input: Input,
            can_apply: bool,
            expected: Vec<UiAction>,
            closes: bool,
        }
        let key = |key, ctrl| Input::Key { key, ctrl };
        let cases = [
            Case {
                name: "Esc closes the window",
                input: key(egui::Key::Escape, false),
                can_apply: true,
                expected: vec![],
                closes: true,
            },
            Case {
                name: "Ctrl+Enter applies when something can be applied",
                input: key(egui::Key::Enter, true),
                can_apply: true,
                expected: vec![UiAction::Apply],
                closes: false,
            },
            Case {
                name: "Ctrl+Enter does nothing without a valid snippet and a target",
                input: key(egui::Key::Enter, true),
                can_apply: false,
                expected: vec![],
                closes: false,
            },
            Case {
                name: "Enter alone does nothing",
                input: key(egui::Key::Enter, false),
                can_apply: true,
                expected: vec![],
                closes: false,
            },
            Case {
                name: "Ctrl+V (a paste event) reads the clipboard when no text box has focus",
                input: Input::Paste,
                can_apply: false,
                expected: vec![UiAction::ReadClipboard],
                closes: false,
            },
            Case {
                name: "a bare V key is not a paste",
                input: key(egui::Key::V, false),
                can_apply: false,
                expected: vec![],
                closes: false,
            },
        ];

        for case in cases {
            let (actions, closes) = press(&state(case.can_apply), case.input);
            assert_eq!(actions, case.expected, "{}", case.name);
            assert_eq!(closes, case.closes, "{}", case.name);
        }
    }

    #[test]
    fn test_paste_in_a_focused_text_box_is_left_to_the_text_box() {
        let state = state(false);
        let ctx = egui::Context::default();
        let mut text = String::new();

        // 1 フレーム目: テキスト欄にフォーカスを渡す
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let response = ui.text_edit_singleline(&mut text);
            response.request_focus();
        });
        output.textures_delta.clear();

        // 2 フレーム目: 貼り付け。テキスト欄にフォーカスがあるので、クリップボードの読み取りは返さない
        let mut raw_input = egui::RawInput::default();
        raw_input.events.push(event(Input::Paste));
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(raw_input, |ui| {
            ui.text_edit_singleline(&mut text);
            actions = shortcuts(ui, &state);
        });
        output.textures_delta.clear();

        assert_eq!(actions, Vec::<UiAction>::new());
    }
}
