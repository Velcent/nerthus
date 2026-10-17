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
    let (now, pointer_down) = context.input(|input| (input.time, input.pointer.primary_down()));
    let mut waiting = Vec::new();
    for entry in pending {
        let Some(mut state) = context.data(|data| data.get_temp::<GradientState>(entry.id)) else { continue };
        if entry.document_serial != editor.document_serial {
            state.pending = None;
            state.dragging = None;
            context.data_mut(|data| data.insert_temp(entry.id, state));
            continue;
        }
        // The property area presents an absent parameter as JSON null.
        // Preserve that baseline while still rejecting a deleted node.
        let missing = Value::Null;
        let current = editor.document.node(entry.node).map(|node| node.values.get(&entry.property).unwrap_or(&missing));
        if current != state.saved.as_ref() {
            state.pending = None;
            state.dragging = None;
            context.data_mut(|data| data.insert_temp(entry.id, state));
            continue;
        }
        let Some(mut stops) = state.pending.clone() else {
            // Selecting a thumb starts a drag before it changes a value.
            // Keep tracking it until a hidden release ends the gesture.
            if state.dragging.is_some() {
                if pointer_down {
                    context.request_repaint_after(std::time::Duration::from_millis(50));
                    waiting.push(entry);
                } else {
                    state.dragging = None;
                    context.data_mut(|data| data.insert_temp(entry.id, state));
                }
            }
            continue;
        };
        let wait = DEBOUNCE_SECONDS - (now - state.changed_at);
        // A node can stop being drawn while its thumb is held. Finish
        // that drag after release even when its property area is hidden.
        let dragging = state.dragging.is_some() && pointer_down;
        if !force && (wait > 0.0 || dragging) {
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
    let previous_selection = state.selected;
    // Undo, reset and Synthesize Colors replace the stored list. They
    // must supersede any edit still waiting for its debounce.
    let replaced = state.saved.as_ref() != Some(value);
    if replaced {
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
        let (pointer, pointer_down, press_origin) = ui.input(|input| {
            let press_origin = if input.pointer.primary_pressed() { input.pointer.press_origin() } else { None };
            (input.pointer.interact_pos(), input.pointer.primary_down(), press_origin)
        });

        for (index, stop) in stops.iter().enumerate() {
            let thumb_id = id.with(("thumb", index));
            let response = ui.interact(thumb_rect(stop), thumb_id, Sense::click_and_drag());
            if response.is_pointer_button_down_on()
                && state.dragging.is_none()
                && let Some(origin) = press_origin.or(pointer)
            {
                state.selected = Some(index);
                state.dragging = Some(DraggingStop { index, pointer_x: origin.x });
                response.request_focus();
            }
        }
        if let Some(mut dragging) = state.dragging
            && dragging.index < stops.len()
        {
            let can_remove = stops.len() > 2;
            let stop = &mut stops[dragging.index];
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
            if !pointer_down {
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
        // A typing popup belongs to the stop it opened for. Drop its
        // uncommitted text when selection or the saved gradient changes.
        if previous_selection != state.selected || replaced {
            let previous_picker = id.with(("colour", previous_selection));
            slider_popups::close_typing(ui.ctx(), previous_picker.with("picker-typing"));
        }
        let picker_id = id.with(("colour", state.selected));
        let colour = state.selected.map(|index| stop_colour(&stops[index])).unwrap_or([255; 3]);
        if let Some(colour) = minimal_colour_picker(ui, picker_id, colour)
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
                let value = stops_value(&pending);
                state.saved = Some(value.clone());
                edit = Edit::Set(value);
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

    fn type_colour(context: &egui::Context, value: &Value, text: &str, time: &mut f64) -> Id {
        let position = Pos2::new(188.0, 240.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let events = [vec![egui::Event::PointerMoved(position)], vec![button(true)], vec![button(false)], Vec::new()];
        for events in events {
            *time += 0.01;
            frame(context, value, *time, events);
        }
        *time += 0.01;
        frame(context, value, *time, vec![egui::Event::Text(text.to_string())]);
        let picker_id = Id::new(EDITOR_ID).with(("colour", state(context).selected));
        let typing_id = picker_id.with("picker-typing");
        assert_eq!(slider_popups::typing_text(context, typing_id).as_deref(), Some(text));
        typing_id
    }

    #[test]
    fn selecting_another_stop_drops_uncommitted_colour_text() {
        for accept in [false, true] {
            let context = themed_context();
            let settings = theme::InterfaceSettings { accept_on_lost_focus: accept, ..Default::default() };
            theme::set_settings(&context, settings);
            let value = black_to_white();
            let mut time = 0.0;
            frame(&context, &value, time, Vec::new());
            let left_thumb = Pos2::new(38.0, MIDDLE_THUMB.y);
            let right_thumb = Pos2::new(338.0, MIDDLE_THUMB.y);
            click(&context, &value, left_thumb, &mut time);
            let typing_id = type_colour(&context, &value, "#00FF00", &mut time);

            for events in [vec![egui::Event::PointerMoved(right_thumb)], vec![pointer_button(right_thumb, true)]] {
                time += 0.01;
                frame(&context, &value, time, events);
            }
            assert_eq!(state(&context).selected, Some(1));
            assert!(!slider_popups::typing_open(&context, typing_id));
            key(&context, &value, Key::Enter, &mut time);
            time += 0.01;
            frame(&context, &value, time, vec![pointer_button(right_thumb, false)]);
            click(&context, &value, left_thumb, &mut time);
            assert!(!slider_popups::typing_open(&context, typing_id), "returning to the stop must not restore its draft");
            assert!(state(&context).pending.is_none(), "selecting stops must leave both colours unchanged");
            assert!(matches!(frame(&context, &value, time + 0.3, Vec::new()), Edit::None));
        }
    }

    #[test]
    fn colour_text_commits_to_its_selected_stop() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        click(&context, &value, Pos2::new(38.0, MIDDLE_THUMB.y), &mut time);
        let typing_id = type_colour(&context, &value, "#00FF00", &mut time);
        key(&context, &value, Key::Enter, &mut time);
        assert!(!slider_popups::typing_open(&context, typing_id));
        let edit = frame(&context, &value, time + 0.21, Vec::new());
        let Edit::Set(value) = edit else { panic!("Enter commits the typed stop colour") };
        let expected = vec![[0.0, 1.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0]];
        assert_eq!(stops_of(&value), Some(expected));
    }

    #[test]
    fn replacing_the_gradient_drops_uncommitted_colour_text() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        click(&context, &value, Pos2::new(38.0, MIDDLE_THUMB.y), &mut time);
        let typing_id = type_colour(&context, &value, "#00FF00", &mut time);
        let replacement = stops_value(&[[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]]);
        frame(&context, &replacement, time + 0.01, Vec::new());
        assert!(!slider_popups::typing_open(&context, typing_id));
        key(&context, &replacement, Key::Enter, &mut time);
        assert!(state(&context).pending.is_none());
        assert!(matches!(frame(&context, &replacement, time + 0.3, Vec::new()), Edit::None));
    }

    #[test]
    fn a_local_gradient_commit_preserves_the_active_colour_draft() {
        let context = themed_context();
        let value = black_to_white();
        let mut time = 0.0;
        frame(&context, &value, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        let typing_id = type_colour(&context, &value, "#00FF00", &mut time);

        time += 0.21;
        let edit = frame(&context, &value, time, Vec::new());
        let Edit::Set(committed) = edit else { panic!("the earlier stop insertion finishes its debounce") };
        time += 0.01;
        frame(&context, &committed, time, Vec::new());
        assert_eq!(slider_popups::typing_text(&context, typing_id).as_deref(), Some("#00FF00"));
        key(&context, &committed, Key::Enter, &mut time);

        let edit = frame(&context, &committed, time + 0.21, Vec::new());
        let Edit::Set(value) = edit else { panic!("the colour draft still commits on Enter") };
        let expected = vec![[0.0, 0.0, 0.0, 0.0], [1.0, 1.0, 1.0, 1.0], [0.0, 1.0, 0.0, 0.5]];
        assert_eq!(stops_of(&value), Some(expected));
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

    const TRACKED_PROPERTY: &str = "GradientData";

    fn tracked_editor() -> (egui::Context, Editor, i32, i32) {
        let context = themed_context();
        let mut document = crate::document::Document::new();
        let node = document.add_node("CLUTer", [26000.0, 26000.0]).unwrap();
        let other = document.add_node("Constant", [26400.0, 26000.0]).unwrap();
        let values = [(TRACKED_PROPERTY.to_string(), black_to_white())];
        document.set_values_quietly(node, &values);
        let mut editor = Editor::new(document);
        editor.select(Some(node));
        (context, editor, node, other)
    }

    fn tracked_frame(context: &egui::Context, editor: &mut Editor, node: i32, time: f64, events: Vec<egui::Event>) {
        let current_node = editor.document.node(node).unwrap();
        let value = current_node.values.get(TRACKED_PROPERTY).cloned().unwrap_or(Value::Null);
        track_property(context, Id::new(EDITOR_ID), editor.document_serial, node, TRACKED_PROPERTY);
        if let Edit::Set(value) = frame(context, &value, time, events) {
            editor.document.set_value(node, TRACKED_PROPERTY, value);
            editor.parameter_changed(node);
        }
    }

    fn hidden_flush(context: &egui::Context, editor: &mut Editor, time: f64, events: Vec<egui::Event>, force: bool) {
        let input = egui::RawInput { screen_rect: Some(SCREEN), time: Some(time), events, ..Default::default() };
        let output = context.run_ui(input, |_| flush_pending(context, editor, force));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn a_hidden_gradient_drag_finishes_on_release_and_preserves_undo_and_redo() {
        let (context, mut editor, node, other) = tracked_editor();
        let value = black_to_white();
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        flush_pending(&context, &mut editor, true);
        let first_edit = editor.document.node(node).unwrap().values[TRACKED_PROPERTY].clone();
        editor.document.finish_value_edit();

        let end = MIDDLE_THUMB + Vec2::new(20.0, 0.0);
        let frames = [
            vec![egui::Event::PointerMoved(MIDDLE_THUMB)],
            vec![pointer_button(MIDDLE_THUMB, true)],
            vec![egui::Event::PointerMoved(end)],
        ];
        for events in frames {
            time += 0.01;
            tracked_frame(&context, &mut editor, node, time, events);
        }
        assert!(state(&context).dragging.is_some());
        let second_edit = stops_value(&state(&context).pending.unwrap());
        assert_ne!(second_edit, first_edit);
        editor.select(Some(other));

        hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), false);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], first_edit);
        assert!(state(&context).pending.is_some());
        let released = vec![pointer_button(end, false)];
        hidden_flush(&context, &mut editor, time + 0.31, released, false);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], second_edit);
        assert!(state(&context).pending.is_none());
        assert!(state(&context).dragging.is_none());

        assert!(editor.document.undo());
        hidden_flush(&context, &mut editor, time + 0.6, Vec::new(), false);
        editor.select(Some(node));
        tracked_frame(&context, &mut editor, node, time + 0.7, Vec::new());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], first_edit);
        assert!(editor.document.can_redo());
        assert!(editor.document.redo());
        tracked_frame(&context, &mut editor, node, time + 0.8, Vec::new());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], second_edit);
        assert!(state(&context).pending.is_none());
    }

    #[test]
    fn a_hidden_external_replacement_supersedes_pending_gradient_edits() {
        for force in [false, true] {
            let (context, mut editor, node, other) = tracked_editor();
            let value = black_to_white();
            let mut time = 0.0;
            tracked_frame(&context, &mut editor, node, time, Vec::new());
            click(&context, &value, MIDDLE, &mut time);
            editor.select(Some(other));
            let replacement = stops_value(&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 1.0, 1.0]]);
            editor.document.set_value_as(node, TRACKED_PROPERTY, replacement.clone(), "External gradient replacement");
            let history = editor.document.undo_labels();

            hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), force);
            assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], replacement);
            assert_eq!(editor.document.undo_labels(), history);
            assert!(state(&context).pending.is_none());
            editor.select(Some(node));
            tracked_frame(&context, &mut editor, node, time + 0.4, Vec::new());
            assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], replacement);
            assert!(state(&context).pending.is_none());
        }
    }

    #[test]
    fn forced_hidden_gradient_commit_is_undoable_without_replaying_pending_edits() {
        let (context, mut editor, node, other) = tracked_editor();
        let value = black_to_white();
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        editor.select(Some(other));
        let pending = stops_value(&state(&context).pending.unwrap());

        hidden_flush(&context, &mut editor, time + 0.01, Vec::new(), true);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], pending);
        assert!(editor.document.undo());
        hidden_flush(&context, &mut editor, time + 0.4, Vec::new(), false);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], value);
        assert!(editor.document.can_redo());
        assert!(editor.document.redo());
        hidden_flush(&context, &mut editor, time + 0.5, Vec::new(), false);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], pending);
        assert!(state(&context).pending.is_none());
    }

    #[test]
    fn clicking_off_centre_selects_a_thumb_without_changing_the_gradient() {
        let stops = [[0.1, 0.2, 0.3, 0.0], [0.4, 0.5, 0.6, 0.5], [0.7, 0.8, 0.9, 1.0]];
        let value = stops_value(&stops);
        for (index, side) in [(0, 0.3), (1, -0.3), (1, 0.3), (2, -0.3)] {
            let context = themed_context();
            let mut time = 0.0;
            frame(&context, &value, time, Vec::new());
            let thumb_id = Id::new(EDITOR_ID).with(("thumb", index));
            let thumb = context.read_response(thumb_id).expect("the thumb is shown").rect;
            let position = thumb.center() + Vec2::new(thumb.width() * side, 0.0);

            click(&context, &value, position, &mut time);

            let selected = state(&context);
            assert_eq!(selected.selected, Some(index));
            assert!(selected.dragging.is_none());
            assert!(selected.pending.is_none(), "selecting thumb {index} must not edit its offset");
            let edit = frame(&context, &value, time + DEBOUNCE_SECONDS, Vec::new());
            assert!(matches!(edit, Edit::None));
        }
    }

    #[test]
    fn thumb_dragging_keeps_movement_coalesced_with_press_and_release() {
        let stops = [[0.0, 0.0, 0.0, 0.0], [0.2, 0.4, 0.6, 0.5], [1.0, 1.0, 1.0, 1.0]];
        let value = stops_value(&stops);
        let context = themed_context();
        frame(&context, &value, 0.0, Vec::new());
        let thumb_id = Id::new(EDITOR_ID).with(("thumb", 1_usize));
        let thumb = context.read_response(thumb_id).expect("the thumb is shown").rect;
        let press = thumb.center() - Vec2::new(2.0, 0.0);
        frame(&context, &value, 0.01, vec![egui::Event::PointerMoved(press)]);

        let moved = press + Vec2::new(4.0, 0.0);
        let events = vec![pointer_button(press, true), egui::Event::PointerMoved(moved)];
        frame(&context, &value, 0.02, events);
        let pending = state(&context).pending.expect("movement after the press changes the offset");
        assert!((pending[1][3] - 0.52).abs() < 1e-6);

        let release = press + Vec2::new(8.0, 0.0);
        let events = vec![egui::Event::PointerMoved(release), pointer_button(release, false)];
        frame(&context, &value, 0.03, events);
        let finished = state(&context);
        assert!(finished.dragging.is_none());
        let pending = finished.pending.expect("movement before release changes the offset");
        assert!((pending[1][3] - 0.54).abs() < 1e-6);
    }

    #[test]
    fn thumb_dragging_uses_relative_logical_pixels_at_each_interface_scale() {
        let stops = [[0.0, 0.0, 0.0, 0.0], [0.2, 0.4, 0.6, 0.5], [1.0, 1.0, 1.0, 1.0]];
        let value = stops_value(&stops);
        for zoom in [0.8, 1.0, 1.2, 1.5] {
            let context = themed_context();
            context.set_zoom_factor(zoom);
            let mut time = 0.0;
            frame(&context, &value, time, Vec::new());
            time += 0.01;
            frame(&context, &value, time, Vec::new());
            let scale = slider_popups::popup_scale(&context);
            let thumb_id = Id::new(EDITOR_ID).with(("thumb", 1_usize));
            let thumb = context.read_response(thumb_id).expect("the thumb is shown").rect;
            let press = thumb.center() + Vec2::new(3.0 * scale, 0.0);
            let events = [vec![egui::Event::PointerMoved(press)], vec![pointer_button(press, true)]];
            for events in events {
                time += 0.01;
                frame(&context, &value, time, events);
            }
            assert_eq!(state(&context).selected, Some(1));
            assert!(state(&context).pending.is_none(), "pressing at zoom {zoom} must only select");

            // Gaea's ThumbCanvas applies HorizontalChange * 0.005,
            // independent of where the thumb was grabbed or canvas width.
            let movements = [(12.0, 30.0), (7.0, -20.0), (4.0, 0.0)];
            for (horizontal, vertical) in movements {
                let position = press + Vec2::new(horizontal * scale, vertical * scale);
                time += 0.01;
                let events = vec![egui::Event::PointerMoved(position)];
                frame(&context, &value, time, events);
                let pending = state(&context).pending.expect("dragging edits the selected stop");
                let expected_offset = 0.5 + horizontal * 0.005;
                assert!((pending[1][3] - expected_offset).abs() < 1e-6, "at zoom {zoom}: {:?}", pending[1]);
                assert_eq!(&pending[1][..3], &stops[1][..3]);
                assert_eq!(pending[0], stops[0]);
                assert_eq!(pending[2], stops[2]);
            }

            let release = press + Vec2::new(4.0 * scale, 0.0);
            time += 0.01;
            frame(&context, &value, time, vec![pointer_button(release, false)]);
            assert!(state(&context).dragging.is_none());
            let edit = frame(&context, &value, time + DEBOUNCE_SECONDS, Vec::new());
            let Edit::Set(value) = edit else { panic!("the completed drag commits after its debounce") };
            let committed = stops_of(&value).unwrap();
            assert!((committed[1][3] - 0.52).abs() < 1e-6);
        }
    }

    #[test]
    fn an_absent_gradient_property_keeps_edits_across_global_flushes() {
        let (context, mut editor, node, other) = tracked_editor();
        let target = editor.document.project.terrain_mut().nodes.get_mut(&node).unwrap();
        target.values.shift_remove(TRACKED_PROPERTY);
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        flush_pending(&context, &mut editor, false);

        let frames = [
            vec![egui::Event::PointerMoved(MIDDLE)],
            vec![pointer_button(MIDDLE, true)],
            vec![pointer_button(MIDDLE, false)],
            Vec::new(),
        ];
        for events in frames {
            time += 0.01;
            tracked_frame(&context, &mut editor, node, time, events);
            flush_pending(&context, &mut editor, false);
        }
        assert!(editor.document.node(node).unwrap().values.get(TRACKED_PROPERTY).is_none());
        assert_eq!(state(&context).pending.as_ref().map(Vec::len), Some(3));
        editor.select(Some(other));

        hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), false);
        let committed = &editor.document.node(node).unwrap().values[TRACKED_PROPERTY];
        assert_eq!(stops_of(committed).unwrap().len(), 3);
        assert!(state(&context).pending.is_none());
        assert!(editor.document.undo());
        assert!(editor.document.node(node).unwrap().values.get(TRACKED_PROPERTY).is_none());
        assert!(editor.document.can_redo());
    }

    #[test]
    fn undo_while_a_gradient_is_hidden_discards_pending_edits_and_keeps_redo() {
        let (context, mut editor, node, other) = tracked_editor();
        let original = black_to_white();
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        click(&context, &original, MIDDLE, &mut time);
        flush_pending(&context, &mut editor, true);
        let committed = editor.document.node(node).unwrap().values[TRACKED_PROPERTY].clone();
        editor.document.finish_value_edit();

        time += 0.01;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        click(&context, &committed, Pos2::new(248.0, MIDDLE.y), &mut time);
        assert!(state(&context).pending.is_some());
        editor.select(Some(other));
        assert!(editor.document.undo());

        hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), false);
        editor.select(Some(node));
        tracked_frame(&context, &mut editor, node, time + 0.4, Vec::new());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], original);
        assert!(state(&context).pending.is_none());
        assert!(editor.document.can_redo());
        assert!(editor.document.redo());
        tracked_frame(&context, &mut editor, node, time + 0.5, Vec::new());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], committed);
        assert!(state(&context).pending.is_none());
    }

    #[test]
    fn a_hidden_thumb_press_without_motion_ends_on_release_without_a_value_edit() {
        let (context, mut editor, node, other) = tracked_editor();
        let original = black_to_white();
        let history = editor.document.undo_labels();
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        let thumb_id = Id::new(EDITOR_ID).with(("thumb", 0_usize));
        let thumb = context.read_response(thumb_id).unwrap().rect.center();
        let frames = [vec![egui::Event::PointerMoved(thumb)], vec![pointer_button(thumb, true)]];
        for events in frames {
            time += 0.01;
            tracked_frame(&context, &mut editor, node, time, events);
            flush_pending(&context, &mut editor, false);
        }
        assert!(state(&context).dragging.is_some());
        assert!(state(&context).pending.is_none());
        editor.select(Some(other));

        hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), false);
        assert!(state(&context).dragging.is_some());
        let released = vec![pointer_button(thumb, false)];
        hidden_flush(&context, &mut editor, time + 0.31, released, false);
        assert!(state(&context).dragging.is_none());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], original);
        assert_eq!(editor.document.undo_labels(), history);

        let elsewhere = Pos2::new(500.0, 500.0);
        let pressed = vec![egui::Event::PointerMoved(elsewhere), pointer_button(elsewhere, true)];
        hidden_flush(&context, &mut editor, time + 0.5, pressed, false);
        editor.select(Some(node));
        let moved = vec![egui::Event::PointerMoved(elsewhere + Vec2::new(20.0, 0.0))];
        tracked_frame(&context, &mut editor, node, time + 0.6, moved);
        assert!(state(&context).dragging.is_none());
        assert!(state(&context).pending.is_none());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], original);
        assert_eq!(editor.document.undo_labels(), history);
    }

    #[test]
    fn a_new_document_discards_hidden_gradient_edits_even_for_identical_node_data() {
        let (context, mut editor, node, _) = tracked_editor();
        let value = black_to_white();
        let mut time = 0.0;
        tracked_frame(&context, &mut editor, node, time, Vec::new());
        click(&context, &value, MIDDLE, &mut time);
        let copied_node = editor.document.node(node).unwrap().clone();
        let mut replacement = crate::document::Document::new();
        replacement.project.terrain_mut().nodes.insert(node, copied_node);
        editor.set_document(replacement);

        hidden_flush(&context, &mut editor, time + 0.3, Vec::new(), true);
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], value);
        assert!(!editor.document.can_undo());
        assert!(state(&context).pending.is_none());
        tracked_frame(&context, &mut editor, node, time + 0.4, Vec::new());
        assert!(state(&context).pending.is_none());
        assert_eq!(editor.document.node(node).unwrap().values[TRACKED_PROPERTY], value);
    }

}
