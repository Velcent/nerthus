//! Graph notes (`NoteElement`): a box tinted with the note's swatch (10%
//! fill, 1px border) showing its Markdown, or just a note icon for
//! `Icon` notes (their text as a tooltip). Notes move by dragging, resize
//! by the corner thumb, flip between icon and text on double-click, and
//! offer Edit Note, their colour and Delete Note on right-click.

use std::collections::HashSet;

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use serde_json::Value;

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{drag_origin, GraphState, GraphView};
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::swatch_grid;
use crate::ui::markdown;
use crate::ui::widgets::{menu_caption, menu_item, menu_separator, menu_style, Tooltips};

/// `NoteElement`'s least height, and the least width of a text note
/// (`OnlyText`: `MinWidth` 100).
const MIN_HEIGHT: f32 = 60.0;
const MIN_TEXT_WIDTH: f32 = 100.0;
const THUMB: f32 = 20.0;
/// An `Icon` note: `SetNoteType` makes it 60 by 35, which its least
/// height of 60 overrules.
const ICON_BOX: f32 = 60.0;

/// A note in progress of being moved or resized.
#[derive(Clone, Copy, Debug)]
pub struct NoteDrag {
    id: i32,
    resizing: bool,
    pointer_start: Pos2,
    start: [f32; 2],
    checkpointed: bool,
}

impl NoteDrag {
    /// Whether the note moves (rather than takes a new size).
    pub(crate) fn moves(&self) -> bool {
        !self.resizing
    }
}

struct NoteLayout {
    id: i32,
    rect: Rect,
    color: Color32,
    markdown: String,
    icon_only: bool,
}

fn vector(fields: &serde_json::Map<String, Value>, name: &str, fallback: [f32; 2]) -> [f32; 2] {
    let Some(value) = fields.get(name) else {
        return fallback;
    };
    let component = |key: &str, default: f32| value.get(key).and_then(Value::as_f64).map_or(default, |number| number as f32);
    [component("X", fallback[0]), component("Y", fallback[1])]
}

fn layouts(editor: &Editor, tab: usize) -> Vec<NoteLayout> {
    let mut notes = Vec::new();
    for (id, fields) in editor.document.notes() {
        let graph_index = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        if graph_index != tab {
            continue;
        }
        let position = vector(fields, "Position", [0.0, 0.0]);
        let size = vector(fields, "Size", [200.0, 200.0]);
        let color_name = fields.get("Color").and_then(Value::as_str).unwrap_or("Gray");
        let markdown = fields.get("Markdown").and_then(Value::as_str).unwrap_or_default().to_string();
        // `Type` missing from the file is `Icon`, `NoteType`'s first member.
        let icon_only = fields.get("Type").and_then(Value::as_str) != Some("OnlyText");
        let origin = Pos2::new(position[0], position[1]);
        let rect = if icon_only { Rect::from_min_size(origin, Vec2::splat(ICON_BOX)) } else { Rect::from_min_size(origin, Vec2::new(size[0], size[1])) };
        let note = NoteLayout {
            id,
            rect,
            color: theme::swatch_color(color_name),
            markdown,
            icon_only,
        };
        notes.push(note);
    }
    notes
}

/// A note's rectangle on the canvas (`CenterNote`), when it is on `tab`.
pub fn note_rect(editor: &Editor, tab: usize, id: i32) -> Option<Rect> {
    let notes = layouts(editor, tab);
    notes.iter().find(|note| note.id == id).map(|note| note.rect)
}

/// The rectangles of the notes on `tab`, by id (`Extents`).
pub fn note_rects(editor: &Editor, tab: usize) -> Vec<(i32, Rect)> {
    layouts(editor, tab).into_iter().map(|note| (note.id, note.rect)).collect()
}

fn screen_rect(view: &GraphView, origin: Pos2, rect: Rect) -> Rect {
    Rect::from_min_max(view.to_screen(origin, rect.min), view.to_screen(origin, rect.max))
}

/// Draws this tab's notes (those collapsed groups hide aside) and handles
/// their mouse input.
pub fn show(ui: &mut Ui, painter: &egui::Painter, editor: &mut Editor, state: &mut GraphState, origin: Pos2, hidden: &HashSet<i32>, commands: &mut Vec<Command>) {
    let tab = state.tab;
    let view = state.view();
    let zoom = view.zoom;
    let notes: Vec<NoteLayout> = layouts(editor, tab).into_iter().filter(|note| !hidden.contains(&note.id)).collect();
    let mut responses: Vec<(i32, Response, Option<Response>)> = Vec::new();
    for note in &notes {
        let screen = screen_rect(&view, origin, note.rect);
        if !screen.intersects(state.rect) {
            continue;
        }
        let selected = state.selected_note == Some(note.id);
        let links = draw(ui, painter, note, screen, zoom, selected);
        let response = ui.interact(screen, ui.id().with(("note", note.id)), Sense::click_and_drag());
        let response = if state.popup.is_none() && note.icon_only && !note.markdown.is_empty() {
            response.tooltip_ui_at_pointer(|ui| {
                ui.set_max_width(400.0);
                let mut scroll = egui::ScrollArea::vertical();
                scroll = scroll.id_salt(("note-tooltip", note.id));
                scroll = scroll.max_height(400.0);
                scroll.show(ui, |ui| markdown::show_note(ui, &note.markdown));
            })
        } else {
            response
        };
        // `AllowSelection`: once a note has focus, its Markdown receives
        // input. Link hit boxes use screen coordinates so they stay under
        // their text at every graph zoom; the rest of the note still drags.
        if let Some(destination) = interact_note_links(ui, note.id, &links, selected) {
            crate::export_build::open_url(&destination);
        }
        let thumb = if note.icon_only {
            None
        } else {
            let thumb_rect = Rect::from_min_size(screen.max - Vec2::splat(THUMB * zoom), Vec2::splat(THUMB * zoom));
            let thumb_response = ui.interact(thumb_rect, ui.id().with(("note-thumb", note.id)), Sense::drag());
            if thumb_response.hovered() || thumb_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            Some(thumb_response)
        };
        responses.push((note.id, response, thumb));
    }

    let modifiers = ui.input(|input| input.modifiers);
    for (id, response, thumb) in &responses {
        if response.clicked() {
            if !(modifiers.command || modifiers.shift) {
                state.clear_element_selection();
                editor.select(None);
            }
            state.selected_note = Some(*id);
        }
        // `NoteElement_MouseDoubleClick`: icon and text swap.
        if response.double_clicked() {
            commands.push(Command::Graph(GraphCommand::ToggleNoteType(*id)));
        }
        crate::ui::widgets::context_menu(response, |ui| note_menu(ui, *id, commands));
        if let (Some(thumb), Some(note)) = (thumb, notes.iter().find(|note| note.id == *id))
            && thumb.drag_started_by(egui::PointerButton::Primary)
        {
            let start = [note.rect.width(), note.rect.height()];
            let pointer_start = drag_origin(ui, thumb).map(|point| view.to_canvas(origin, point)).unwrap_or_default();
            state.note_drag = Some(NoteDrag { id: *id, resizing: true, pointer_start, start, checkpointed: false });
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && state.note_drag.is_none()
            && let (Some(point), Some(note)) = (drag_origin(ui, response), notes.iter().find(|note| note.id == *id))
        {
            let start = [note.rect.left(), note.rect.top()];
            state.selected_note = Some(*id);
            state.note_drag = Some(NoteDrag { id: *id, resizing: false, pointer_start: view.to_canvas(origin, point), start, checkpointed: false });
        }
    }

    let Some(mut drag) = state.note_drag else {
        return;
    };
    let held = ui.input(|input| input.pointer.primary_down());
    let mut corner = drag.start;
    let pointer = ui.input(|input| input.pointer.interact_pos());
    if let Some(point) = pointer {
        let delta = view.to_canvas(origin, point) - drag.pointer_start;
        let (member, value) = if drag.resizing {
            let size = [(drag.start[0] + delta.x).max(MIN_TEXT_WIDTH), (drag.start[1] + delta.y).max(MIN_HEIGHT)];
            ("Size", size)
        } else {
            corner = [drag.start[0] + delta.x, drag.start[1] + delta.y];
            ("Position", corner)
        };
        if drag.checkpointed || value != drag.start {
            editor.document.set_note_vector(drag.id, member, value, !drag.checkpointed);
            drag.checkpointed = true;
        }
    }
    if held {
        state.note_drag = Some(drag);
        return;
    }
    // `NoteElement_MoveCompleted`: in or out of a group by its corner.
    if !drag.resizing {
        editor.document.settle_in_groups(&[(drag.id, corner, tab)]);
    }
    state.note_drag = None;
}

fn interact_note_links(ui: &Ui, note: i32, links: &[markdown::LinkRegion], selected: bool) -> Option<String> {
    if !selected {
        return None;
    }
    let mut clicked = None;
    for (index, link) in links.iter().enumerate() {
        let id = ui.id().with(("note-link", note, index));
        let response = ui.interact(link.rect, id, Sense::click());
        let response = response.tooltip(link.destination.as_str());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            clicked = Some(link.destination.clone());
        }
    }
    clicked
}

/// `ctxNote`: Edit Note, the colours, Delete Note.
fn note_menu(ui: &mut Ui, id: i32, commands: &mut Vec<Command>) {
    menu_style(ui);
    if menu_item(ui, None, None, "Edit Note", "").clicked() {
        commands.push(Command::Graph(GraphCommand::EditNote(id)));
    }
    menu_caption(ui, "Color");
    if let Some(color) = swatch_grid(ui) {
        commands.push(Command::Graph(GraphCommand::SetNoteColor(id, color)));
        ui.close();
    }
    if menu_item(ui, None, None, "Delete Note", "").clicked() {
        commands.push(Command::Graph(GraphCommand::DeleteNote(id)));
    }
    menu_separator(ui);
}

fn draw(ui: &mut Ui, painter: &egui::Painter, note: &NoteLayout, screen: Rect, zoom: f32, selected: bool) -> Vec<markdown::LinkRegion> {
    painter.rect_filled(screen, 0.0, note.color.gamma_multiply(0.1));
    painter.rect_stroke(screen, 0.0, Stroke::new(1.0, note.color), StrokeKind::Inside);
    if selected {
        painter.rect_stroke(screen.expand(1.0), 0.0, Stroke::new(1.0, theme::ACCENT_MUTED_PRIMARY), StrokeKind::Outside);
    }
    if note.icon_only {
        let icon_rect = Rect::from_center_size(screen.center(), Vec2::splat(24.0 * zoom));
        icons::paint(ui, icon_rect, "note", note.color);
        return Vec::new();
    }
    let inner = screen.shrink(8.0 * zoom);
    let links = if inner.width() > 4.0 && zoom > 0.15 {
        let id = ui.id().with(("note-markdown", note.id));
        markdown::paint_note(ui, &note.markdown, inner, zoom, id)
    } else {
        Vec::new()
    };
    // `NoteThumb`: a 12px triangle in the corner.
    let corner = screen.max;
    let size = 12.0 * zoom;
    let triangle = vec![Pos2::new(corner.x, corner.y - size), corner, Pos2::new(corner.x - size, corner.y)];
    painter.add(Shape::convex_polygon(triangle, note.color, Stroke::NONE));
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    struct NoteFrame {
        links: Vec<markdown::LinkRegion>,
        destination: Option<String>,
        body: Response,
    }

    fn note_frame(context: &egui::Context, rect: Rect, zoom: f32, selected: bool, events: Vec<egui::Event>) -> NoteFrame {
        let text = "**Bold** [Open docs](https://example.com)\n\n| Link |\n| --- |\n| [Table docs](https://example.com/table) |";
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1600.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut result = None;
        let output = context.run_ui(input, |ui| {
            let links = markdown::paint_note(ui, text, rect, zoom, ui.id().with("test-note-content"));
            let body = ui.interact(rect, ui.id().with("test-note-body"), Sense::click_and_drag());
            let destination = interact_note_links(ui, 1, &links, selected);
            result = Some(NoteFrame { links, destination, body });
        });
        output.drop_without_applying_deltas();
        result.unwrap()
    }

    fn pointer_button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    fn resize_frame(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1000.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        state.rect = screen;
        context.run_ui(input, |ui| {
            let painter = ui.painter_at(screen);
            let hidden = HashSet::new();
            let mut commands = Vec::new();
            show(ui, &painter, editor, state, Pos2::ZERO, &hidden, &mut commands);
        }).drop_without_applying_deltas();
    }

    #[test]
    fn only_primary_resize_changes_the_note_or_its_history() {
        let buttons = [egui::PointerButton::Secondary, egui::PointerButton::Middle, egui::PointerButton::Primary];
        for button in buttons {
            let context = egui::Context::default();
            theme::apply(&context);
            let mut editor = Editor::new(Document::new());
            let note = editor.document.add_note([26100.0, 26100.0], 0);
            let history = editor.document.undo_labels();
            editor.document.dirty = false;
            let mut state = GraphState::default();
            let start = Pos2::new(295.0, 295.0);
            let end = start + Vec2::new(30.0, 20.0);
            let click = |pos, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
            resize_frame(&context, &mut editor, &mut state, Vec::new());
            resize_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(start)]);
            resize_frame(&context, &mut editor, &mut state, vec![click(start, true)]);
            resize_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(end)]);
            resize_frame(&context, &mut editor, &mut state, vec![click(end, false)]);
            let rect = note_rect(&editor, 0, note).unwrap();
            if button == egui::PointerButton::Primary {
                assert_eq!(rect.size(), Vec2::new(230.0, 220.0));
                assert_eq!(editor.document.undo_labels().len(), history.len() + 1);
                assert!(editor.document.dirty);
                assert!(editor.document.undo());
                assert_eq!(note_rect(&editor, 0, note).unwrap().size(), Vec2::splat(200.0));
            } else {
                assert_eq!(rect.size(), Vec2::splat(200.0));
                assert_eq!(editor.document.undo_labels(), history);
                assert!(!editor.document.dirty);
            }
            assert!(state.note_drag.is_none());
        }
    }

    #[test]
    fn resizing_a_note_against_its_minimum_adds_no_empty_edit() {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        let note = editor.document.add_note([26100.0, 26100.0], 0);
        editor.document.set_note_vector(note, "Size", [MIN_TEXT_WIDTH, MIN_HEIGHT], false);
        editor.document.dirty = false;
        let history = editor.document.undo_labels();
        let mut state = GraphState::default();
        let start = Pos2::new(195.0, 155.0);
        let end = start - Vec2::splat(30.0);
        resize_frame(&context, &mut editor, &mut state, Vec::new());
        resize_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(start)]);
        resize_frame(&context, &mut editor, &mut state, vec![pointer_button(start, true)]);
        resize_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(end)]);
        resize_frame(&context, &mut editor, &mut state, vec![pointer_button(end, false)]);
        assert_eq!(editor.document.undo_labels(), history);
        assert!(!editor.document.dirty);
    }

    #[test]
    fn selected_note_links_receive_clicks_at_each_zoom_and_leave_the_body_draggable() {
        for zoom in [0.5, 1.0, 2.0] {
            let context = egui::Context::default();
            theme::apply(&context);
            let rect = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(300.0, 300.0) * zoom);
            let frame = note_frame(&context, rect, zoom, false, Vec::new());
            assert_eq!(frame.links.len(), 2);
            let paragraph_link = frame.links[0].rect.center();
            // The first click focuses the note, as `AllowSelection` does.
            note_frame(&context, rect, zoom, false, vec![egui::Event::PointerMoved(paragraph_link)]);
            note_frame(&context, rect, zoom, false, vec![pointer_button(paragraph_link, true)]);
            let frame = note_frame(&context, rect, zoom, false, vec![pointer_button(paragraph_link, false)]);
            assert!(frame.body.clicked());
            assert!(frame.destination.is_none());

            let selected = note_frame(&context, rect, zoom, true, Vec::new());
            for link in selected.links {
                assert!(rect.contains_rect(link.rect));
                let point = link.rect.center();
                note_frame(&context, rect, zoom, true, vec![egui::Event::PointerMoved(point)]);
                note_frame(&context, rect, zoom, true, vec![pointer_button(point, true)]);
                let clicked = note_frame(&context, rect, zoom, true, vec![pointer_button(point, false)]);
                assert_eq!(clicked.destination, Some(link.destination));
                assert!(!clicked.body.clicked());
            }

            let start = rect.left_bottom() + Vec2::new(20.0, -20.0);
            note_frame(&context, rect, zoom, true, vec![egui::Event::PointerMoved(start)]);
            note_frame(&context, rect, zoom, true, vec![pointer_button(start, true)]);
            let moved = note_frame(&context, rect, zoom, true, vec![egui::Event::PointerMoved(start + Vec2::new(25.0, 0.0))]);
            assert!(moved.body.drag_started());
            assert!(moved.destination.is_none());
        }
    }
}
