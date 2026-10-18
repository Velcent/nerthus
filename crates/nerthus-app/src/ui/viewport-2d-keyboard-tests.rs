use super::*;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1000.0, 700.0));
const VIEW: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(500.0, 500.0));
const FIELD: Rect = Rect::from_min_max(Pos2::new(600.0, 50.0), Pos2::new(850.0, 80.0));

fn editor() -> Editor {
    let mut document = crate::document::Document::new();
    let node = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
    let mut editor = Editor::new(document);
    editor.select(Some(node));
    editor
}

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE }
}

fn frame(
    context: &egui::Context,
    editor: &Editor,
    state: &mut Viewport2DState,
    events: Vec<egui::Event>,
    blocked: bool,
    mut panels: impl FnMut(&mut Ui),
) {
    let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
    context.run_ui(input, |ui| {
        state.begin_keyboard(ui.ctx(), blocked);
        panels(ui);
        let mut commands = Vec::new();
        show_docked(ui, VIEW, editor, state, &mut commands);
        state.finish_keyboard(ui.ctx());
    }).drop_without_applying_deltas();
}

fn assert_mode(state: &Viewport2DState, mode: &str) {
    assert_eq!(state.is_picking(), mode == "pick");
    assert_eq!(state.is_measuring(), mode == "measure");
}

#[test]
fn text_field_enter_and_escape_gestures_leave_measurement_modes_active() {
    let editor = editor();
    for mode in ["pick", "measure"] {
        for cancel_key in [egui::Key::Enter, egui::Key::Escape] {
            let context = egui::Context::default();
            let mut state = Viewport2DState::default();
            state.open_at_launch(mode);
            state.docked_focused = true;
            let field_id = Id::new("viewport-keyboard-field");
            let mut text = "12".to_string();
            frame(&context, &editor, &mut state, Vec::new(), false, |ui| {
                let field = egui::TextEdit::singleline(&mut text).id(field_id);
                ui.put(FIELD, field).request_focus();
            });
            assert!(context.egui_wants_keyboard_input());
            assert!(!state.docked_focused);

            let events = vec![key(cancel_key, true)];
            frame(&context, &editor, &mut state, events, false, |ui| {
                let field = egui::TextEdit::singleline(&mut text).id(field_id);
                ui.put(FIELD, field);
            });
            // Both keys surrender the text field's focus before key-up.
            assert!(!context.egui_wants_keyboard_input());
            assert_mode(&state, mode);

            let events = vec![key(cancel_key, false)];
            frame(&context, &editor, &mut state, events, false, |_| {});
            assert_mode(&state, mode);
            assert!(state.picked.is_none());

            // Leaving the field does not return focus to the viewport.
            for pressed in [true, false] {
                let events = vec![key(egui::Key::Escape, pressed)];
                frame(&context, &editor, &mut state, events, false, |_| {});
                assert_mode(&state, mode);
            }
        }
    }
}

#[test]
fn enter_chooses_a_node_search_result_without_cancelling_measurement_modes() {
    use crate::ui::graph_commands::GraphCommand;
    use crate::ui::graph_search::{SearchMode, SearchPopup};

    for mode in ["pick", "measure"] {
        let context = egui::Context::default();
        let mut editor = editor();
        let mut state = Viewport2DState::default();
        state.open_at_launch(mode);
        state.docked_focused = true;
        state.follow_selection(&editor);
        let selected = editor.active_node().unwrap();
        let mut search = SearchPopup::new(SearchMode::Navigate, Pos2::new(600.0, 150.0), [0.0, 0.0]);
        search.type_text("Mountain");
        let mut open = true;
        let mut commands = Vec::new();

        for events in [Vec::new(), vec![key(egui::Key::Enter, true)], vec![key(egui::Key::Enter, false)]] {
            let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
            context.run_ui(input, |ui| {
                state.begin_keyboard(ui.ctx(), open);
                let mut view_commands = Vec::new();
                show_docked(ui, VIEW, &editor, &mut state, &mut view_commands);
                if open {
                    open = crate::ui::graph_search::show(ui, &mut editor, &mut search, &mut commands);
                }
                state.finish_keyboard(ui.ctx());
            }).drop_without_applying_deltas();
            assert_mode(&state, mode);
        }
        assert!(!open);
        assert_eq!(commands, vec![Command::Graph(GraphCommand::Navigate(selected))]);
        let mut ui_state = crate::ui::UiState::default();
        crate::ui::graph_commands::run(GraphCommand::Navigate(selected), &mut editor, &mut ui_state);
        state.follow_selection(&editor);
        assert_eq!(editor.active_node(), Some(selected));
        assert_mode(&state, mode);
    }
}

#[test]
fn escape_requires_a_gesture_owned_by_the_docked_view() {
    let editor = editor();
    let context = egui::Context::default();
    let mut state = Viewport2DState::default();
    state.open_at_launch("measure");

    for events in [vec![key(egui::Key::Escape, true)], vec![key(egui::Key::Escape, false)]] {
        frame(&context, &editor, &mut state, events, false, |_| {});
        assert_mode(&state, "measure");
    }

    let pointer = Pos2::new(400.0, 400.0);
    let events = vec![
        egui::Event::PointerMoved(pointer),
        egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
    ];
    frame(&context, &editor, &mut state, events, false, |_| {});
    assert!(state.docked_focused);
    assert!(!context.egui_wants_keyboard_input());

    // The same Escape used to dismiss a popup belongs to that popup,
    // even when it closes before the release reaches the next frame.
    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, true)], true, |_| {});
    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, false)], false, |_| {});
    assert_mode(&state, "measure");

    // Nonmodal windows route their Escape before the viewport sees it.
    // Closing that window must not also cancel the measuring tool.
    let events = vec![key(egui::Key::Escape, true)];
    let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
    context.run_ui(input, |ui| {
        assert!(ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
        state.begin_keyboard(ui.ctx(), false);
        let mut commands = Vec::new();
        show_docked(ui, VIEW, &editor, &mut state, &mut commands);
        state.finish_keyboard(ui.ctx());
    }).drop_without_applying_deltas();
    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, false)], false, |_| {});
    assert_mode(&state, "measure");

    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, false)], false, |_| {});
    assert_mode(&state, "measure");
    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, true)], false, |_| {});
    assert_mode(&state, "measure");
    frame(&context, &editor, &mut state, vec![key(egui::Key::Escape, false)], false, |_| {});
    assert!(!state.is_measuring());
}
