//! The Note Editor (`NoteWindow`, opened by a note's Edit Note or F2): the
//! note's Markdown on the left and how it reads on the right, in a 720
//! wide window (smaller in a window that cannot hold it); Close (or
//! Escape, `IsCancel`) keeps the text.

use egui::{Align, Frame, Id, Key, Layout, Margin, RichText, ScrollArea, Stroke, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::graph_canvas::GraphState;
use crate::ui::markdown;

/// `Width="720" Height="640"` less the grid's margin (20, 10, 20, 30).
const PANEL_SIZE: Vec2 = Vec2::new(680.0, 600.0);
/// The room kept round the panel when the window is too small for it, 20
/// each side as `dialog_window` keeps.
const WINDOW_MARGIN: f32 = 40.0;
/// The caption and the Close button's row, with the space round them: the
/// text columns take the rest of the panel's height.
const CHROME_HEIGHT: f32 = 74.0;
/// The input box's lines when its column has the room for them.
const INPUT_ROWS: usize = 24;
/// Round the input box's text (egui's own).
const INPUT_MARGIN: Margin = Margin::symmetric(4, 2);

pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut GraphState) {
    let Some((id, mut text)) = state.editing_note.take() else {
        return;
    };
    let mut close = ctx.input(|input| input.key_pressed(Key::Escape));
    let mut frame = Frame::new();
    frame = frame.fill(theme::MENU_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::MENU_BORDER));
    frame = frame.corner_radius(6.0);
    frame = frame.inner_margin(Margin::same(13));
    // As big as fits the window less its margin, the text columns taking
    // what is left of its height and scrolling past it. Held to that size:
    // egui starts a modal at the size it had the frame before.
    let panel = PANEL_SIZE.min(ctx.content_rect().size() - Vec2::splat(WINDOW_MARGIN));
    let content = panel - frame.total_margin().sum();
    let modal = egui::Modal::new(Id::new("note-editor")).frame(frame);
    modal.show(ctx, |ui| {
        ui.set_min_size(content);
        ui.set_max_size(content);
        let caption = RichText::new("Note Editor").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(caption);
        ui.add_space(6.0);
        let column_width = (ui.available_width() - 20.0) / 2.0;
        let column_height = (content.y - CHROME_HEIGHT).max(0.0);
        // As many lines as the column holds, so that the box is whole in it.
        let font = theme::interface(12.0);
        let line = ui.fonts_mut(|fonts| fonts.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        let rows = (((column_height - INPUT_MARGIN.sum().y) / line) as usize).clamp(1, INPUT_ROWS);
        ui.horizontal_top(|ui| {
            ui.allocate_ui(Vec2::new(column_width, column_height), |ui| {
                let mut edit = egui::TextEdit::multiline(&mut text);
                edit = edit.font(font);
                edit = edit.desired_width(column_width);
                edit = edit.desired_rows(rows);
                edit = edit.margin(INPUT_MARGIN);
                edit = edit.id(Id::new(("note-editor-text", id)));
                ScrollArea::vertical().id_salt("note-editor-input").max_height(column_height).show(ui, |ui| {
                    let response = ui.add(edit);
                    if !response.has_focus() && !response.lost_focus() {
                        response.request_focus();
                    }
                });
            });
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(Vec2::new(column_width, column_height), Layout::top_down(Align::Min), |ui| {
                ScrollArea::vertical().id_salt("note-editor-output").max_height(column_height).show(ui, |ui| {
                    ui.set_max_width(column_width - 10.0);
                    markdown::show_note(ui, &text);
                });
            });
        });
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = RichText::new("Close").font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
            let mut button = egui::Button::new(label);
            button = button.fill(theme::BUTTON_BACKGROUND);
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
            button = button.min_size(Vec2::new(80.0, 26.0));
            if ui.add(button).clicked() {
                close = true;
            }
        });
    });
    if close {
        // `btnCancel_Click`: the note takes the text.
        editor.document.set_note_text(id, &text);
        return;
    }
    state.editing_note = Some((id, text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::{AreaState, Pos2, RawInput, Rect};

    /// A few frames of the editor open on a screen this size. Returns the
    /// panel's rect and where its caption and Close button were drawn.
    fn shown(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, screen: Vec2) -> (Rect, Rect, Rect) {
        let mut texts = Vec::new();
        for _ in 0..3 {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
            let output = context.run_ui(input, |ui| show(ui.ctx(), editor, state));
            texts = output.shapes.iter().filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) => Some((shape.galley.text().to_owned(), Rect::from_min_size(shape.pos, shape.galley.size()))),
                _ => None,
            }).collect();
            output.drop_without_applying_deltas();
        }
        let drawn = |text: &str| texts.iter().find(|(drawn, _)| drawn == text).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("{text} is not drawn"));
        let area = AreaState::load(context, Id::new("note-editor")).filter(|area| area.size.is_some());
        (area.map(|area| area.rect()).unwrap_or(Rect::NOTHING), drawn("Note Editor"), drawn("Close"))
    }

    #[test]
    fn the_panel_fits_the_window_and_grows_back_with_it() {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        state.editing_note = Some((1, "# Title\n\nSome *notes* on the graph.\n\n- one\n- two".repeat(8)));
        // 1600 × 960 at 120%: the whole panel.
        let large = Vec2::new(1333.0, 800.0);
        let (rect, _, _) = shown(&context, &mut editor, &mut state, large);
        assert!((rect.size() - PANEL_SIZE).length() < 0.5, "{rect:?} is not the panel's size");
        // Within the point egui rounds it to.
        assert!((rect.center() - (Pos2::ZERO + large / 2.0)).length() <= 1.0, "{rect:?} is not centred");
        // 1280 × 720, 1024 × 600, 800 × 600 and 800 × 450 at 120%: as big as
        // fits 20 short of each edge, caption and Close button in it.
        for screen in [Vec2::new(1067.0, 600.0), Vec2::new(853.0, 500.0), Vec2::new(667.0, 500.0), Vec2::new(667.0, 375.0)] {
            let (rect, caption, close) = shown(&context, &mut editor, &mut state, screen);
            let room = Rect::from_min_size(Pos2::ZERO, screen).shrink(20.0);
            assert!((rect.size() - PANEL_SIZE.min(room.size())).length() < 0.5, "{rect:?} does not fit {room:?}");
            assert!(room.expand(0.5).contains_rect(rect), "{rect:?} is not in {room:?}");
            assert!(rect.contains_rect(caption) && rect.contains_rect(close), "{caption:?} or {close:?} is outside {rect:?}");
        }
        let (rect, _, _) = shown(&context, &mut editor, &mut state, large);
        assert!((rect.size() - PANEL_SIZE).length() < 0.5, "{rect:?} did not grow back to the panel's size");
        assert!(state.editing_note.is_some(), "the editor closed");
    }
}
