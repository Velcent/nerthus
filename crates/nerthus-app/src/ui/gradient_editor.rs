//! Gaea's `ClutEditor` for CLUTer's `GradientData` and Synth's `Data`:
//! a 96-pixel `ThumbCanvas` above the embedded `MinimalColorPicker`.
//! Stops keep scRGB colour components and an offset. Clicking the canvas
//! samples its gradient; thumbs move, or delete past either end. Changes
//! are written together after the source editor's 200 ms debounce.

use egui::{Color32, Id, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::json::f32_value;
use serde_json::{Value, json};

use crate::editor::Editor;
use crate::theme;
use crate::ui::colour_button::minimal_colour_picker;
use crate::ui::colour_picker::{linear_to_srgb, srgb_to_linear};
use crate::ui::editors::Edit;
use crate::ui::slider_popups;

const DEBOUNCE_SECONDS: f64 = 0.2;
const STRIP_HEIGHT: f32 = 96.0;
const PICKER_GAP: f32 = 36.0;
const THUMB_SIZE: f32 = 10.0;
/// `ThumbCanvas.multiplier`: offset per logical pixel of horizontal drag.
const DRAG_MULTIPLIER: f32 = 0.005;
const KEY_STEP: f32 = 0.01;

type Stop = [f32; 4];

#[derive(Clone, Copy)]
struct DraggingStop {
    index: usize,
    pointer_x: f32,
}

#[derive(Clone, Default)]
struct GradientState {
    saved: Option<Value>,
    selected: Option<usize>,
    pending: Option<Vec<Stop>>,
    changed_at: f64,
    dragging: Option<DraggingStop>,
}


#[derive(Clone, PartialEq)]
struct GradientProperty {
    document_serial: u64,
    node: i32,
    property: String,
    id: Id,
}

fn pending_id() -> Id {
    Id::new("pending-property-gradients")
}

/// Keep the source editor's debounce alive when the property area closes
/// or switches nodes. The owner also separates equal node ids in different
/// documents, whose saved gradient lists may happen to be identical.
pub fn track_property(context: &egui::Context, id: Id, document_serial: u64, node: i32, property: &str) {
    let owner = GradientProperty { document_serial, node, property: property.to_string(), id };
    let owner_id = id.with("gradient-owner");
    context.data_mut(|data| {
        let previous: Option<GradientProperty> = data.get_temp(owner_id);
        if previous.as_ref() != Some(&owner) {
            data.remove::<GradientState>(id);
            data.insert_temp(owner_id, owner.clone());
        }
        let pending = data.get_temp_mut_or_default::<Vec<GradientProperty>>(pending_id());
        pending.retain(|entry| entry.id != id);
        pending.push(owner);
    });
}

/// Run before shell commands, so Save, selection changes and Undo see
/// the most recent stop edits, including editors no longer drawn.
pub fn flush_pending(context: &egui::Context, editor: &mut Editor, force: bool) {
    let pending: Vec<GradientProperty> = context.data_mut(|data| data.remove_temp(pending_id())).unwrap_or_default();
    let now = context.input(|input| input.time);
    let mut waiting = Vec::new();
    for entry in pending {
        let Some(mut state) = context.data(|data| data.get_temp::<GradientState>(entry.id)) else { continue };
        if entry.document_serial != editor.document_serial {
            state.pending = None;
            state.dragging = None;
            context.data_mut(|data| data.insert_temp(entry.id, state));
            continue;
        }
        let Some(mut stops) = state.pending.clone() else { continue };
        let current = editor.document.node(entry.node).and_then(|node| node.values.get(&entry.property));
        if current != state.saved.as_ref() {
            state.pending = None;
            state.dragging = None;
            context.data_mut(|data| data.insert_temp(entry.id, state));
            continue;
        }
        let wait = DEBOUNCE_SECONDS - (now - state.changed_at);
        if !force && (wait > 0.0 || state.dragging.is_some()) {
            context.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.05)));
            waiting.push(entry);
            continue;
        }
        if let Some(dragging) = state.dragging.take()
            && let Some(stop) = stops.get(dragging.index)
            && !(0.0..=1.0).contains(&stop[3])
        {
            if stops.len() > 2 {
                stops.remove(dragging.index);
                state.selected = None;
            } else {
                stops[dragging.index][3] = stops[dragging.index][3].clamp(0.0, 1.0);
            }
        }
        let value = stops_value(&stops);
        if current != Some(&value) {
            editor.document.set_value(entry.node, &entry.property, value.clone());
            editor.parameter_changed(entry.node);
        }
        state.saved = Some(value);
        state.pending = None;
        context.data_mut(|data| data.insert_temp(entry.id, state));
    }
    context.data_mut(|data| data.insert_temp(pending_id(), waiting));
}

fn stops_of(value: &Value) -> Option<Vec<Stop>> {
    let items = value.as_array()?;
    let component = |item: &Value, key: &str| item.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let stops = items.iter().map(|item| [component(item, "X"), component(item, "Y"), component(item, "Z"), component(item, "W")]).collect();
    Some(stops)
}

fn stops_value(stops: &[Stop]) -> Value {
    let items: Vec<Value> = stops.iter().map(|[x, y, z, w]| json!({"X": f32_value(*x), "Y": f32_value(*y), "Z": f32_value(*z), "W": f32_value(*w)})).collect();
    Value::Array(items)
}

fn stop_colour(stop: &Stop) -> [u8; 3] {
    [linear_to_srgb(stop[0]), linear_to_srgb(stop[1]), linear_to_srgb(stop[2])]
}

/// `ThumbCanvas.Redraw`: interpolate the colour channels as scRGB floats,
/// as the node does. Adding a stop keeps these sampled floats directly.
fn colour_at(stops: &[Stop], offset: f32) -> [f32; 3] {
    let mut sorted: Vec<&Stop> = stops.iter().collect();
    sorted.sort_by(|left, right| left[3].total_cmp(&right[3]));
    let Some(first) = sorted.first() else {
        return [0.0; 3];
    };
    if offset <= first[3] {
        return [first[0], first[1], first[2]];
    }
    for pair in sorted.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if offset <= right[3] {
            let span = right[3] - left[3] + 1e-24;
            let fraction = (offset - left[3]) / span;
            let mix = |index: usize| left[index] * (1.0 - fraction) + right[index] * fraction;
            return [mix(0), mix(1), mix(2)];
        }
    }
    let last = sorted[sorted.len() - 1];
    [last[0], last[1], last[2]]
}

pub fn gradient_editor(ui: &mut Ui, id: Id, value: &Value) -> Edit {
    let mut state: GradientState = ui.data(|data| data.get_temp(id)).unwrap_or_default();
    // Undo, reset and Synthesize Colors replace the stored list. They
    // must supersede any edit still waiting for its debounce.
    if state.saved.as_ref() != Some(value) {
        state.saved = Some(value.clone());
        state.pending = None;
        state.dragging = None;
    }
    let saved = stops_of(value);
    let black_to_white = vec![[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
    let mut stops = state.pending.clone().or(saved.clone()).unwrap_or(black_to_white);
    state.selected = state.selected.filter(|index| *index < stops.len());
    let now = ui.input(|input| input.time);
    let scale = slider_popups::popup_scale(ui.ctx());
    let mut edited = false;

    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin { left: (16.0 * scale) as i8, ..Default::default() };
    frame.show(ui, |ui| {
        let width = ui.available_width();
        let size = Vec2::new(width, STRIP_HEIGHT * scale);
        let (strip, _) = ui.allocate_exact_size(size, Sense::hover());
        let canvas = strip.shrink(2.0 * scale);
        let thumb_size = THUMB_SIZE * scale;
        let thumb_rect = |stop: &Stop| {
            let x = canvas.left() + stop[3] * canvas.width();
            let top = canvas.bottom() + thumb_size / 2.0;
            Rect::from_min_size(Pos2::new(x - thumb_size / 2.0, top), Vec2::splat(thumb_size))
        };
        let bar_response = ui.interact(canvas, id.with("bar"), Sense::click());
        let (pointer, pointer_down) = ui.input(|input| (input.pointer.interact_pos(), input.pointer.primary_down()));

        for (index, stop) in stops.iter().enumerate() {
            let thumb_id = id.with(("thumb", index));
            let response = ui.interact(thumb_rect(stop), thumb_id, Sense::click_and_drag());
            if response.is_pointer_button_down_on()
                && state.dragging.is_none()
                && let Some(pointer) = pointer
            {
                state.selected = Some(index);
                state.dragging = Some(DraggingStop { index, pointer_x: pointer.x });
                response.request_focus();
            }
        }
        if let Some(mut dragging) = state.dragging
            && dragging.index < stops.len()
        {
            let can_remove = stops.len() > 2;
            let stop = &mut stops[dragging.index];
            if pointer_down {
                if let Some(pointer) = pointer {
                    let delta = (pointer.x - dragging.pointer_x) / scale;
                    let offset = (stop[3] + delta * DRAG_MULTIPLIER).clamp(-0.05, 1.05);
                    if offset != stop[3] {
                        stop[3] = offset;
                        edited = true;
                    }
                    dragging.pointer_x = pointer.x;
                    state.dragging = Some(dragging);
                }
            } else {
                if !(0.0..=1.0).contains(&stop[3]) {
                    if can_remove {
                        stops.remove(dragging.index);
                        state.selected = None;
                    } else {
                        stop[3] = stop[3].clamp(0.0, 1.0);
                    }
                    edited = true;
                }
                state.dragging = None;
            }
        }
        if bar_response.clicked()
            && let Some(position) = bar_response.interact_pointer_pos()
        {
            let offset = ((position.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0);
            let [red, green, blue] = colour_at(&stops, offset);
            stops.push([red, green, blue, offset]);
            let index = stops.len() - 1;
            state.selected = Some(index);
            ui.memory_mut(|memory| memory.request_focus(id.with(("thumb", index))));
            edited = true;
        }

        if let Some(index) = state.selected {
            let thumb_id = id.with(("thumb", index));
            let focused = ui.memory(|memory| memory.has_focus(thumb_id));
            if focused && state.dragging.is_none() {
                let filter = egui::EventFilter { horizontal_arrows: true, ..Default::default() };
                ui.memory_mut(|memory| memory.set_focus_lock_filter(thumb_id, filter));
                // The source handles key-up and ignores keyboard repeat.
                let keys = ui.input_mut(|input| {
                    let mut keys = Vec::new();
                    input.events.retain(|event| {
                        if let egui::Event::Key { key, pressed, repeat, .. } = event
                            && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::Delete)
                        {
                            if !pressed && !repeat {
                                keys.push(*key);
                            }
                            return false;
                        }
                        true
                    });
                    keys
                });
                for key in keys {
                    match key {
                        Key::ArrowLeft | Key::ArrowRight => {
                            let direction = if key == Key::ArrowLeft { -1.0 } else { 1.0 };
                            let offset = (stops[index][3] + direction * KEY_STEP).clamp(0.0, 1.0);
                            edited |= offset != stops[index][3];
                            stops[index][3] = offset;
                        }
                        Key::Delete if stops.len() > 2 => {
                            stops.remove(index);
                            state.selected = None;
                            edited = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        ui.painter().rect_filled(strip, 2.0 * scale, Color32::BLACK);
        paint_bar(ui, canvas, &stops);
        ui.painter().rect_stroke(strip, 2.0 * scale, Stroke::new(2.0 * scale, theme::WINDOW_DARK), StrokeKind::Inside);
        if let Some(index) = state.selected {
            let stop = stops[index];
            if (0.0..=1.0).contains(&stop[3]) {
                let x = canvas.left() + stop[3] * canvas.width();
                let contrast = 1.0 - (stop[0] + stop[1] + stop[2]) / 3.0;
                let colour = Color32::from_gray(linear_to_srgb(contrast)).gamma_multiply(0.5);
                ui.painter().line_segment([Pos2::new(x, canvas.top()), Pos2::new(x, canvas.bottom())], Stroke::new(scale, colour));
            }
        }
        for (index, stop) in stops.iter().enumerate() {
            let removing = !(0.0..=1.0).contains(&stop[3]);
            paint_thumb(ui, thumb_rect(stop), stop_colour(stop), state.selected == Some(index), removing, scale);
        }

        ui.add_space(PICKER_GAP * scale);
        let colour = state.selected.map(|index| stop_colour(&stops[index])).unwrap_or([255; 3]);
        if let Some(colour) = minimal_colour_picker(ui, id.with("colour"), colour)
            && let Some(index) = state.selected
        {
            let selected = &mut stops[index];
            [selected[0], selected[1], selected[2]] = colour.map(srgb_to_linear);
            edited = true;
        }
    });

    if edited {
        state.pending = Some(stops);
        state.changed_at = now;
    }
    let mut edit = Edit::None;
    if let Some(pending) = state.pending.clone() {
        let waited = now - state.changed_at;
        if waited >= DEBOUNCE_SECONDS && state.dragging.is_none() {
            state.pending = None;
            if saved.as_ref() != Some(&pending) {
                edit = Edit::Set(stops_value(&pending));
            }
        } else {
            let wait = (DEBOUNCE_SECONDS - waited).max(0.05);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
    }
    ui.data_mut(|data| data.insert_temp(id, state));
    edit
}

fn paint_bar(ui: &Ui, bar: Rect, stops: &[Stop]) {
    let columns = (bar.width() / 2.0).max(1.0) as usize;
    let mut mesh = egui::Mesh::default();
    for column in 0..=columns {
        let offset = column as f32 / columns as f32;
        // `ThumbCanvas` writes its map to RGB24 by multiplying by 255
        // and truncating; its preview does not use the thumb's WPF gamma.
        let [red, green, blue] = colour_at(stops, offset).map(|component| (component * 255.0).clamp(0.0, 255.0) as u8);
        let colour = Color32::from_rgb(red, green, blue);
        let x = bar.left() + offset * bar.width();
        mesh.colored_vertex(Pos2::new(x, bar.top()), colour);
        mesh.colored_vertex(Pos2::new(x, bar.bottom()), colour);
    }
    for column in 0..columns {
        let top = (column * 2) as u32;
        mesh.add_triangle(top, top + 1, top + 2);
        mesh.add_triangle(top + 1, top + 3, top + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// The source's ten-pixel circular `ColorThumb`, with white inside a
/// black, selected-accent or delete-red outline.
fn paint_thumb(ui: &Ui, rect: Rect, colour: [u8; 3], selected: bool, removing: bool, scale: f32) {
    let outline = if removing {
        theme::ACCENT_RED
    } else if selected {
        theme::ACCENT_PRIMARY
    } else {
        Color32::BLACK
    };
    let centre = rect.center();
    let radius = rect.width() / 2.0;
    ui.painter().circle_filled(centre, radius, outline);
    ui.painter().circle_filled(centre, radius - scale, Color32::WHITE);
    ui.painter().circle_filled(centre, radius - 2.0 * scale, Color32::from_rgb(colour[0], colour[1], colour[2]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_colours_interpolate_in_scrgb_and_preserve_stop_data() {
        let stops = vec![[1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]];
        assert_eq!(colour_at(&stops, 0.5), [0.5; 3]);
        assert_eq!(colour_at(&stops, -1.0), [0.0; 3]);
        assert_eq!(colour_at(&stops, 2.0), [1.0; 3]);
        let value = stops_value(&stops);
        assert_eq!(stops_of(&value), Some(stops));
    }

    const EDITOR_ID: &str = "gradient-test";
    const PANEL: Rect = Rect::from_min_max(Pos2::new(20.0, 20.0), Pos2::new(340.0, 520.0));
    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(600.0, 600.0));
    const MIDDLE: Pos2 = Pos2::new(188.0, 70.0);
    const MIDDLE_THUMB: Pos2 = Pos2::new(188.0, 124.0);

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context
    }

    fn frame(context: &egui::Context, value: &Value, time: f64, events: Vec<egui::Event>) -> Edit {
        let input = egui::RawInput { screen_rect: Some(SCREEN), time: Some(time), events, ..Default::default() };
        let mut edit = Edit::None;
        let output = context.run_ui(input, |ui| {
            let builder = egui::UiBuilder::new().max_rect(PANEL);
            let mut child = ui.new_child(builder);
            edit = gradient_editor(&mut child, Id::new(EDITOR_ID), value);
        });
        output.drop_without_applying_deltas();
        edit
    }

    fn state(context: &egui::Context) -> GradientState {
        context.data(|data| data.get_temp(Id::new(EDITOR_ID))).expect("the editor keeps its state")
    }

    fn pointer_button(position: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    fn click(context: &egui::Context, value: &Value, position: Pos2, time: &mut f64) {
        let events = [vec![egui::Event::PointerMoved(position)], vec![pointer_button(position, true)], vec![pointer_button(position, false)], Vec::new()];
        for events in events {
            *time += 0.01;
            frame(context, value, *time, events);
        }
    }

    fn key(context: &egui::Context, value: &Value, key: Key, time: &mut f64) {
        for pressed in [true, false] {
            *time += 0.01;
            let event = egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
            frame(context, value, *time, vec![event]);
        }
    }

    fn black_to_white() -> Value {
        stops_value(&[[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]])
    }

    #[test]
    fn clicking_samples_the_gradient_and_keys_move_then_delete_the_selected_stop() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        assert_eq!(state(&context).selected, None);
        click(&context, &value, MIDDLE, &mut time);
        let pending = state(&context).pending.expect("clicking adds a stop");
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[2], [0.5, 0.5, 0.5, 0.5]);
        assert_eq!(state(&context).selected, Some(2));

        key(&context, &value, Key::ArrowRight, &mut time);
        let pending = state(&context).pending.unwrap();
        assert!((pending[2][3] - 0.51).abs() < 1e-6);
        key(&context, &value, Key::ArrowLeft, &mut time);
        let pending = state(&context).pending.unwrap();
        assert!((pending[2][3] - 0.5).abs() < 1e-6);
        key(&context, &value, Key::Delete, &mut time);
        let pending = state(&context).pending.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(state(&context).selected, None);
    }

    #[test]
    fn dragging_past_either_horizontal_end_deletes_but_keeps_two_stops() {
        for direction in [-1.0, 1.0] {
            let context = themed_context();
            let value = black_to_white();
            let mut time = 0.0;
            frame(&context, &value, time, Vec::new());
            click(&context, &value, MIDDLE, &mut time);
            let end = MIDDLE_THUMB + Vec2::new(direction * 120.0, 0.0);
            let events =
                [vec![egui::Event::PointerMoved(MIDDLE_THUMB)], vec![pointer_button(MIDDLE_THUMB, true)], vec![egui::Event::PointerMoved(end)], vec![pointer_button(end, false)]];
            for events in events {
                time += 0.01;
                frame(&context, &value, time, events);
            }
            assert_eq!(state(&context).pending.unwrap().len(), 2);
            assert_eq!(state(&context).selected, None);

            let left_thumb = Pos2::new(38.0, MIDDLE_THUMB.y);
            click(&context, &value, left_thumb, &mut time);
            key(&context, &value, Key::Delete, &mut time);
            assert_eq!(state(&context).pending.unwrap().len(), 2);
            let events = [
                vec![pointer_button(left_thumb, true)],
                vec![egui::Event::PointerMoved(left_thumb - Vec2::new(50.0, 0.0))],
                vec![pointer_button(left_thumb - Vec2::new(50.0, 0.0), false)],
            ];
            for events in events {
                time += 0.01;
                frame(&context, &value, time, events);
            }
            let pending = state(&context).pending.unwrap();
            assert_eq!(pending.len(), 2);
            assert_eq!(pending[0][3], 0.0);
        }
    }

    #[test]
    fn changes_commit_after_two_hundred_milliseconds() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        let before = frame(&context, &value, time + 0.1, Vec::new());
        assert!(matches!(before, Edit::None));
        let after = frame(&context, &value, time + 0.21, Vec::new());
        let Edit::Set(value) = after else { panic!("the edit commits after the debounce") };
        assert_eq!(stops_of(&value).unwrap().len(), 3);
    }

    #[test]
    fn an_external_gradient_change_supersedes_pending_edits() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        assert!(state(&context).pending.is_some());
        let replacement = stops_value(&[[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]]);
        let edit = frame(&context, &replacement, time + 0.21, Vec::new());
        assert!(matches!(edit, Edit::None));
        assert!(state(&context).pending.is_none());
    }

}
