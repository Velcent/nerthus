//! The Math node's editor window (`EditMath`, the node's Open Editor
//! command): its expression in a syntax-coloured box (`SyntaxBox` rules),
//! checked half a second after each change (`RunValidation` runs the
//! expression over a 16 × 16 map), with Apply and Run, Accept Changes and
//! Cancel. The window stays on top and does not block the editor.

use egui::{Id, RichText, Stroke, Vec2};
use nerthus_engine::automation::VariableValue;
use serde_json::Value;

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::syntax_box;
use crate::ui::tool_window;

/// `debounceInterval`.
const DEBOUNCE_SECONDS: f64 = 0.5;
/// The map the check runs over.
const CHECK_SIZE: usize = 16;
/// The expression box's least height, however short the window.
const MIN_BOX_HEIGHT: f32 = 80.0;
/// The buttons' height.
const BUTTON_HEIGHT: f32 = 28.0;

/// The window while it is open.
#[derive(Clone, Debug)]
struct MathEditor {
    node: i32,
    text: String,
    /// `oldExpression`: what Cancel puts back.
    old_expression: String,
    /// When the text last changed, for the delayed check.
    changed_at: Option<f64>,
    /// `result`: the check's message and whether it passed.
    result: Option<(String, bool)>,
}

fn window_id() -> Id {
    Id::new("math-editor")
}

/// The window's layer, separate from the editor state in egui's memory.
pub fn area_id() -> Id {
    window_id().with("window")
}

pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<MathEditor>(window_id()).is_some())
}

/// Drops the previous document's editor without applying Cancel's rollback.
pub fn reset(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
    tool_window::close_requested(ctx, area_id());
}

/// `Editor_CommandExecuted("OpenEditor")`: the window for a Math node.
pub fn open(ctx: &egui::Context, editor: &Editor, node: i32) {
    tool_window::close_requested(ctx, area_id());
    let expression = editor.document.node(node).and_then(|data| data.values.get("Expression")).and_then(Value::as_str).unwrap_or_default().to_string();
    let state = MathEditor { node, text: expression.clone(), old_expression: expression, changed_at: None, result: None };
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

/// Draws the window while it is open and applies its buttons.
pub fn show(ctx: &egui::Context, editor: &mut Editor) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<MathEditor>(window_id())) else {
        return;
    };
    let cancel = tool_window::close_requested(ctx, area_id());
    if editor.document.node(state.node).is_none_or(|node| node.type_name != "MathX") {
        ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
        return;
    }
    if cancel {
        set_expression(editor, state.node, &state.old_expression);
        ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
        return;
    }
    let now = ctx.input(|input| input.time);
    if let Some(changed_at) = state.changed_at {
        if now - changed_at >= DEBOUNCE_SECONDS {
            state.changed_at = None;
            state.result = Some(check_expression(editor, state.node, &state.text));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE_SECONDS - (now - changed_at)));
        }
    }

    let mut open = true;
    let mut action = None;
    let mut window = egui::Window::new("Edit Math Node");
    window = window.open(&mut open);
    window = window.collapsible(false);
    window = window.order(egui::Order::Foreground);
    window = window.frame(egui::Frame::window(&ctx.global_style()).fill(theme::WINDOW_BACKGROUND).inner_margin(tool_window::MARGIN));
    // 480 × 400 at the interface scale (the window's `UIScaler`).
    window = tool_window::place(ctx, window, area_id(), Vec2::new(480.0, 400.0));
    window.show(ctx, |ui| {
        let box_id = window_id().with("expression");
        // Tab and Enter are the `SyntaxBox`'s, taken before the box runs.
        let changed_by_keys = syntax_box::take_keys(ui, box_id, &mut state.text);
        // From the bottom up: the buttons, the check's result and the help
        // go in first and the box takes the room left above them, so that
        // the window keeps its size whatever shows under the box.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            // As tall as the buttons, which would otherwise hang below the
            // window's bottom and make it grow.
            let row = Vec2::new(ui.available_width(), BUTTON_HEIGHT);
            ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if apply_button(ui).clicked() {
                    action = Some(MathAction::Apply);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if dialog_button(ui, "Cancel", false).clicked() {
                        action = Some(MathAction::Cancel);
                    }
                    ui.add_space(8.0);
                    if dialog_button(ui, "Accept Changes", true).clicked() {
                        action = Some(MathAction::Accept);
                    }
                });
            });
            ui.add_space(24.0);
            if let Some((message, valid)) = &state.result {
                let accent = if *valid { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_MUTED_RED };
                let result = RichText::new(message.as_str()).font(theme::monospace_bold(12.0)).color(accent.gamma_multiply(0.7));
                ui.add(egui::Label::new(result).wrap());
                ui.add_space(6.0);
            }
            let help = RichText::new("Use a, b, c to access inputs. Use x, y for coordinates.").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT.gamma_multiply(0.7));
            ui.add(egui::Label::new(help).wrap());
            ui.add_space(8.0);
            // In a window too short for the box it reaches up past the
            // window's top, and the window grows once to fit it.
            let room = Vec2::new(ui.available_width(), ui.available_height().max(8.0 + MIN_BOX_HEIGHT));
            ui.allocate_ui_with_layout(room, egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(8.0);
                let editor_height = ui.available_height();
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = syntax_box::highlight(text.as_str(), theme::monospace(14.0));
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                let mut text_edit = egui::TextEdit::multiline(&mut state.text);
                text_edit = text_edit.id(box_id);
                text_edit = text_edit.font(theme::monospace(14.0));
                text_edit = text_edit.desired_width(f32::INFINITY);
                text_edit = text_edit.code_editor();
                // Tab and Enter are the `SyntaxBox`'s.
                text_edit = text_edit.return_key(None);
                text_edit = text_edit.layouter(&mut layouter);
                let scroll = egui::ScrollArea::vertical().max_height(editor_height).id_salt("math-expression");
                let response = scroll.show(ui, |ui| ui.add_sized(Vec2::new(ui.available_width(), editor_height), text_edit)).inner;
                if response.changed() || changed_by_keys {
                    state.changed_at = Some(now);
                }
            });
        });
    });
    if !open {
        action = action.or(Some(MathAction::Cancel));
    }
    let node = state.node;
    match action {
        // `BtnValidate_OnClick`: the expression set and the node built.
        Some(MathAction::Apply) => set_expression(editor, node, &state.text),
        // `BtnOK_OnClick`.
        Some(MathAction::Accept) => {
            set_expression(editor, node, &state.text);
            ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
            return;
        }
        // `btnClose_Click`: back to what it was when the window opened.
        Some(MathAction::Cancel) => {
            set_expression(editor, node, &state.old_expression);
            ctx.data_mut(|data| data.remove::<MathEditor>(window_id()));
            return;
        }
        None => {}
    }
    ctx.data_mut(|data| data.insert_temp(window_id(), state));
}

#[derive(Clone, Copy, PartialEq)]
enum MathAction {
    Apply,
    Accept,
    Cancel,
}

fn set_expression(editor: &mut Editor, node: i32, text: &str) {
    let current = editor.document.node(node).and_then(|data| data.values.get("Expression")).and_then(Value::as_str).unwrap_or_default().to_string();
    if current == text {
        return;
    }
    editor.document.set_value(node, "Expression", Value::from(text));
    editor.changed();
}

/// `RunValidation`: `MathX.RunComputation` over a 16 × 16 map of zeros
/// with the project's variables (at the preview's
/// resolution): ExprTk's message, or "Expression is valid.".
fn check_expression(editor: &Editor, node: i32, text: &str) -> (String, bool) {
    let automation = editor.document.automation();
    let variables: Vec<(String, nerthus_engine::automation::VarType, VariableValue)> = automation.variables.values().map(|variable| (variable.name.clone(), variable.var_type, variable.value.clone())).collect();
    let variables_text = nerthus_nodes::mathx::variables_string(editor.preview_resolution, &variables);
    let normalized = editor.document.node(node).and_then(|data| data.values.get("NormalizedCoordinates")).and_then(Value::as_bool).unwrap_or(false);
    let zeros = vec![0.0f32; CHECK_SIZE * CHECK_SIZE];
    let inputs = [Some(zeros.as_slice()), Some(zeros.as_slice()), Some(zeros.as_slice())];
    match nerthus_nodes::mathx::compute(text, &variables_text, inputs, CHECK_SIZE, CHECK_SIZE, normalized) {
        Ok(_) => ("Expression is valid.".to_string(), true),
        Err(error) => (nerthus_nodes::mathx::node_message(&error), false),
    }
}

/// `Button-IconText-Bordered` with `player-play`: Apply and Run.
fn apply_button(ui: &mut egui::Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("Apply and Run".to_string(), theme::interface(12.0), theme::LABEL_EMPHASIS);
    let size = Vec2::new(8.0 + 16.0 + 6.0 + galley.size().x + 10.0, BUTTON_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if response.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    let icon_rect = egui::Rect::from_min_size(egui::Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, "player-play", theme::ACCENT_GREEN);
    ui.painter().galley(egui::Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_EMPHASIS);
    response
}

/// `Button`, or with `important` `Button-Important`.
fn dialog_button(ui: &mut egui::Ui, text: &str, important: bool) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, BUTTON_HEIGHT));
    ui.add(widget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::{AreaState, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect};

    /// 1600 × 960 at 120%, in points.
    const SCREEN: Vec2 = Vec2::new(1333.0, 800.0);

    fn context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context
    }

    /// The editor with a Math node holding `expression`, and the window
    /// open on it.
    fn opened(context: &egui::Context, expression: &str) -> Editor {
        let mut editor = Editor::new(Document::new());
        let node = editor.document.add_node("MathX", [25100.0, 25100.0]).unwrap();
        editor.document.set_value(node, "Expression", Value::from(expression));
        open(context, &editor, node);
        editor
    }

    #[test]
    fn document_reset_closes_math_without_restoring_the_old_expression() {
        let context = context();
        let mut editor = opened(&context, "1 - a");
        let state = context.data(|data| data.get_temp::<MathEditor>(window_id())).unwrap();
        let node = state.node;
        let mut replacement = Document::new();
        replacement.project = editor.document.project.clone();
        replacement.set_value(node, "Expression", Value::from("1 - a * 0.5"));
        replacement.dirty = false;
        editor.set_document(replacement);
        let history = editor.document.undo_labels();
        tool_window::request_close(&context, area_id());

        reset(&context);
        frame(&context, &mut editor, 0.0, Vec::new());

        assert!(!is_open(&context));
        assert_eq!(editor.document.node(node).unwrap().values["Expression"], "1 - a * 0.5");
        assert!(!editor.document.dirty);
        assert_eq!(editor.document.undo_labels(), history);
    }

    #[test]
    fn escape_cancels_applied_math_changes_while_the_expression_has_focus() {
        let context = context();
        let mut editor = opened(&context, "a");
        let mut ui_state = crate::ui::UiState::default();
        for _ in 0..3 {
            frame(&context, &mut editor, 0.0, Vec::new());
        }
        let mut state = context.data(|data| data.get_temp::<MathEditor>(window_id())).unwrap();
        let node = state.node;
        // Apply and Run changed the node, followed by another unsaved edit.
        set_expression(&mut editor, node, "a + 1");
        state.text = "a + 2".to_string();
        context.data_mut(|data| data.insert_temp(window_id(), state));
        context.memory_mut(|memory| memory.request_focus(window_id().with("expression")));
        frame(&context, &mut editor, 0.0, Vec::new());
        tool_window::note_keyboard(&context);
        editor.building = true;
        let input = RawInput { events: vec![key(Key::Escape)], ..Default::default() };
        let output = context.run_ui(input, |ui| {
            crate::ui::tool_commands::take_escape(ui.ctx(), &mut editor, &mut ui_state);
            show(ui.ctx(), &mut editor);
            assert!(!ui.input(|input| input.key_pressed(Key::Escape)));
        });
        output.drop_without_applying_deltas();
        assert!(context.data(|data| data.get_temp::<MathEditor>(window_id())).is_none());
        assert_eq!(editor.document.node(node).unwrap().values["Expression"], "a");
        assert!(!editor.cancel_build);
    }

    /// A frame at `time` with these events. Returns the window's rect.
    fn frame(context: &egui::Context, editor: &mut Editor, time: f64, events: Vec<Event>) -> Rect {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), time: Some(time), events, ..Default::default() };
        let output = context.run_ui(input, |ui| show(ui.ctx(), editor));
        output.drop_without_applying_deltas();
        let rect = AreaState::load(context, window_id().with("window")).filter(|area| area.size.is_some()).map(|area| area.rect());
        rect.unwrap_or(Rect::NOTHING)
    }

    fn box_rect(context: &egui::Context) -> Rect {
        context.read_response(window_id().with("expression")).map(|response| response.rect).unwrap_or(Rect::NOTHING)
    }

    fn result(context: &egui::Context) -> Option<(String, bool)> {
        context.data(|data| data.get_temp::<MathEditor>(window_id())).and_then(|state| state.result)
    }

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// Types `typed` at the end of the box, lets the check run, then moves
    /// the pointer over the app for a while. Returns the window's rect and
    /// the box's before the typing, and each frame's after the check.
    fn type_and_wait(typed: &str, valid: bool) -> ((Rect, Rect), Vec<(Rect, Rect)>) {
        let context = context();
        let mut editor = opened(&context, "a");
        let mut time = 0.0;
        let mut before = Rect::NOTHING;
        for _ in 0..5 {
            before = frame(&context, &mut editor, time, Vec::new());
            time += 0.1;
        }
        let box_before = box_rect(&context);
        context.memory_mut(|memory| memory.request_focus(window_id().with("expression")));
        frame(&context, &mut editor, time, Vec::new());
        frame(&context, &mut editor, time, vec![key(Key::End)]);
        frame(&context, &mut editor, time, vec![Event::Text(typed.to_string())]);
        // Past the half second the check waits for.
        time += 1.0;
        frame(&context, &mut editor, time, Vec::new());
        let (_, passed) = result(&context).expect("the check did not run");
        assert_eq!(passed, valid, "{:?}", result(&context));
        let mut after = Vec::new();
        for step in 0..60 {
            time += 1.0 / 30.0;
            let pointer = Pos2::new(100.0 + step as f32 * 15.0, 400.0 + (step % 7) as f32 * 20.0);
            let rect = frame(&context, &mut editor, time, vec![Event::PointerMoved(pointer)]);
            after.push((rect, box_rect(&context)));
        }
        ((before, box_before), after)
    }

    fn assert_keeps_its_size(typed: &str, valid: bool) {
        let ((before, box_before), after) = type_and_wait(typed, valid);
        let body = tool_window::body(&context());
        assert!(body.expand(0.5).contains_rect(before), "{before:?} is outside the body {body:?}");
        for (rect, expression) in after {
            assert!((rect.size() - before.size()).abs().max_elem() <= 0.5, "the window grew from {before:?} to {rect:?}");
            assert!((rect.min - before.min).abs().max_elem() <= 0.5, "the window moved from {before:?} to {rect:?}");
            // The box gives up the room the result line takes.
            assert!(expression.height() < box_before.height(), "the box kept {box_before:?}: {expression:?}");
            assert!(expression.height() > 150.0, "the box is only {expression:?}");
            assert!(rect.contains_rect(expression), "the box {expression:?} is outside the window {rect:?}");
        }
    }

    #[test]
    fn the_window_keeps_its_size_once_the_expression_is_valid() {
        assert_keeps_its_size(" + 1", true);
    }

    #[test]
    fn the_window_keeps_its_size_once_the_expression_has_an_error() {
        assert_keeps_its_size(" +* (", false);
    }

    #[test]
    fn a_window_made_too_short_for_the_box_settles() {
        let context = context();
        let mut editor = opened(&context, "a");
        let mut time = 0.0;
        let mut rect = Rect::NOTHING;
        for _ in 0..5 {
            rect = frame(&context, &mut editor, time, Vec::new());
            time += 0.1;
        }
        // Its bottom edge dragged up most of the way to its title bar.
        let from = Pos2::new(rect.center().x, rect.bottom() - 1.0);
        let to = Pos2::new(from.x, rect.top() + 60.0);
        let button = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        for events in [vec![Event::PointerMoved(from)], vec![button(from, true)], vec![Event::PointerMoved(from.lerp(to, 0.5))], vec![Event::PointerMoved(to)], vec![button(to, false)]] {
            frame(&context, &mut editor, time, events);
            time += 0.1;
        }
        let mut rects = Vec::new();
        for _ in 0..30 {
            rects.push(frame(&context, &mut editor, time, Vec::new()));
            time += 1.0 / 30.0;
        }
        let settled = rects[10];
        assert!(settled.height() < rect.height() - 50.0, "{settled:?} was not made shorter than {rect:?}");
        for later in &rects[10..] {
            assert!((later.size() - settled.size()).abs().max_elem() <= 0.5, "the window went on from {settled:?} to {later:?}");
        }
        let expression = box_rect(&context);
        assert!(expression.height() >= MIN_BOX_HEIGHT - 0.5, "the box is only {expression:?}");
        assert!(settled.contains_rect(expression), "the box {expression:?} is outside the window {settled:?}");
    }
}
