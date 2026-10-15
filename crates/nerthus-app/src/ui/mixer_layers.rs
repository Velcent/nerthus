//! Mixer's layer panel (`LayerEditorPanel`): its layers from the top
//! (`Order` descending) in a `MixerListBox`, "Add layer" while there is
//! room, the ground layer last, and the chosen layer's `LayerEditor`: mask
//! source, its ranges and falloffs, warp, and how the layer blends.
//!
//! Layer previews show the layer's colour; thumbnails of the connected
//! maps and drag-to-reorder are not here yet.

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use indexmap::IndexMap;
use nerthus_engine::humanize::humanize_title;
use nerthus_engine::json::f32_value;
use nerthus_engine::NodeData;
use nerthus_nodes::random::XorShift64Star;
use serde_json::{json, Value};

use crate::icons;
use crate::theme;
use crate::ui::colour_button::colour_button;
use crate::ui::colour_picker::{linear_to_srgb, srgb_to_linear};
use crate::ui::editors::{dropdown_box, option_list, range_slider, slider, Edit, RangeSpec, SliderSpec};
use crate::ui::graph_canvas::elided_galley;

/// `MaskSource` as the source list names it.
const SOURCES: [(&str, &str); 5] = [("Constant", "None"), ("Height", "Height"), ("Slope", "Slope"), ("Both", "Mixed"), ("Custom", "Custom")];

/// `BlendMode`.
const BLEND_MODES: [&str; 24] = [
    "Blend", "Add", "Screen", "Subtract", "Difference", "Multiply", "Divide", "Divide2", "Max", "Min", "Hypotenuse", "Overlay", "Power", "Exclusion", "Dodge", "Burn", "SoftLight", "HardLight", "PinLight", "GrainMerge", "GrainExtract", "Reflect", "Glow", "Phoenix",
];

/// `LayerEditorPanel.brushes`: colours for layers whose own colour is
/// black.
const LAYER_COLORS: [u32; 15] = [
    0xd63939, 0xf76707, 0xf59f00, 0x82c91e, 0x2fb344, 0x0ca678, 0x17a2b8, 0x228be6, 0x066fd1, 0x4263eb, 0x7048e8, 0xae3ec9, 0xd6336c, 0xe64980, 0xf783ac,
];

/// `ParamsN-Color7`: the chosen layer's fill.
const SELECTED_LAYER: Color32 = Color32::from_rgb(0x34, 0x3a, 0x40);

struct Layer {
    key: String,
    value: Value,
    order: i64,
}

fn float(value: &Value, name: &str, fallback: f64) -> f64 {
    value.get(name).and_then(Value::as_f64).unwrap_or(fallback)
}

fn float2(value: &Value, name: &str) -> (f64, f64) {
    let member = value.get(name);
    let x = member.and_then(|member| member.get("X")).and_then(Value::as_f64).unwrap_or(0.0);
    let y = member.and_then(|member| member.get("Y")).and_then(Value::as_f64).unwrap_or(1.0);
    (x, y)
}

fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// The panel's colours, shuffled by the Mixer's id (a
/// Fisher-Yates over `XorShift64Star(id)`), the first shuffle of the
/// shared array.
fn layer_palette(node_id: i32) -> [Color32; 15] {
    let mut palette = LAYER_COLORS.map(rgb);
    let mut random = XorShift64Star::new(node_id);
    let mut index = palette.len();
    while index > 1 {
        index -= 1;
        let other = random.next_below(index as i32 + 1) as usize;
        palette.swap(index, other);
    }
    palette
}

/// The layer's own colour, or `None` when it is black (unset).
fn own_color(layer: &Value) -> Option<Color32> {
    let color = layer.get("Color");
    let channel = |name: &str| color.and_then(|color| color.get(name)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let (red, green, blue) = (channel("R"), channel("G"), channel("B"));
    if red == 0.0 && green == 0.0 && blue == 0.0 {
        return None;
    }
    // scRGB, as `ToWPFColor` shows it.
    Some(Color32::from_rgb(linear_to_srgb(red), linear_to_srgb(green), linear_to_srgb(blue)))
}

/// The layers whose ports the node has (`Layer1`… while `LayerN` exists).
fn layers(values: &IndexMap<String, Value>, node: &NodeData) -> Vec<Layer> {
    let mut list = Vec::new();
    for index in 1..=15 {
        let key = format!("Layer{index}");
        if node.port(&key).is_none() {
            continue;
        }
        let Some(value) = values.get(&key) else { continue };
        let order = value.get("Order").and_then(Value::as_i64).unwrap_or(0);
        list.push(Layer { key, value: value.clone(), order });
    }
    list.sort_by_key(|layer| std::cmp::Reverse(layer.order));
    list
}

/// What feeds a port, else `fallback`.
fn source_name(node: &NodeData, nodes: &IndexMap<i32, NodeData>, port: &str, fallback: &str) -> String {
    let record = node.port(port).and_then(|port| port.record.as_ref());
    let source = record.and_then(|record| nodes.get(&record.from));
    source.map(|source| source.name.clone()).unwrap_or_else(|| fallback.to_string())
}

/// `LayerListItem`'s height: the 32 preview, padding 4 and the border.
const LAYER_ROW_HEIGHT: f32 = 32.0 + 8.0 + 2.0;

/// Where a `LayerListItem`'s parts go in its row (a `DockPanel`).
struct LayerRowLayout {
    eye: Rect,
    preview: Rect,
    /// What the preview and the simplify eye leave between them, past
    /// their margins: the source's and the layer's names side by side.
    names: Rect,
    simplify: Rect,
    swatch: Rect,
}

impl LayerRowLayout {
    fn of(row: Rect) -> Self {
        let content = row.shrink2(Vec2::new(16.0, 4.0));
        // `toggle` (`IconX-Toggle`, padding 8, margin 0, 0, 8, 0), then the
        // preview (32).
        let eye = Rect::from_min_size(Pos2::new(content.left() + 8.0, content.center().y - 8.0), Vec2::splat(16.0));
        let preview = Rect::from_min_size(Pos2::new(eye.right() + 8.0 + 8.0, content.center().y - 16.0), Vec2::splat(32.0));
        // From the right: the swatch (margin 8, 2, 0, 2) and the simplify
        // eye (`IconX-Toggle-Colored`, padding 8, margin 8, 0, 0, 0).
        let swatch = Rect::from_min_size(Pos2::new(content.right() - 18.0, content.top() + 2.0), Vec2::new(18.0, content.height() - 4.0));
        let simplify = Rect::from_min_size(Pos2::new(swatch.left() - 8.0 - 8.0 - 16.0, content.center().y - 8.0), Vec2::splat(16.0));
        let names_left = preview.right() + 8.0;
        let names_right = (simplify.left() - 8.0 - 8.0).max(names_left);
        let names = Rect::from_min_max(Pos2::new(names_left, content.top()), Pos2::new(names_right, content.bottom()));
        LayerRowLayout { eye, preview, names, simplify, swatch }
    }
}

/// The margin round each of a layer row's names.
const NAME_MARGIN: f32 = 4.0;

/// A `MixerListBoxItem` (`ModifierBorder`: cladding, padding 16, 4; the
/// first and last rounded; `#343a40` when chosen; a pale outline under the
/// pointer). Returns the row's rect and response.
fn list_item(ui: &mut Ui, height: f32, first: bool, last: bool, selected: bool, sense: Sense) -> (Rect, egui::Response) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), sense);
    let top = if first { 4 } else { 0 };
    let bottom = if last { 4 } else { 0 };
    let corners = egui::CornerRadius { nw: top, ne: top, sw: bottom, se: bottom };
    let fill = if selected { SELECTED_LAYER } else { theme::PANEL_CLADDING };
    ui.painter().rect_filled(rect, corners, fill);
    if response.hovered() && sense != Sense::hover() {
        let outline = Stroke::new(1.5, Color32::from_rgba_unmultiplied(0xa2, 0xc5, 0xd2, 0x66));
        ui.painter().rect_stroke(rect, corners, outline, StrokeKind::Inside);
    }
    (rect, response)
}

/// Draws the panel; edits come back as (property, edit).
pub fn show(ui: &mut Ui, id: Id, node: &NodeData, nodes: &IndexMap<i32, NodeData>, values: &IndexMap<String, Value>) -> Vec<(String, Edit)> {
    let mut edits = Vec::new();
    let list = layers(values, node);
    let palette = layer_palette(node.id);
    let selected_id = id.with("selected-layer");
    let mut selected: Option<String> = ui.data(|data| data.get_temp(selected_id));
    if selected.as_ref().is_none_or(|key| !list.iter().any(|layer| &layer.key == key)) {
        selected = list.first().map(|layer| layer.key.clone());
    }

    // `ModifierListBox`: a stack 4 in.
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(4);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let port_count = values.get("PortCount").and_then(Value::as_i64).unwrap_or(0);
        let can_add = port_count < 30;
        let row_count = list.len() + usize::from(can_add) + 1;
        let mut row_index = 0;
        if can_add {
            // "Add layer": grey, right-aligned, padding 4.
            let galley = ui.painter().layout_no_wrap("Add layer".to_string(), theme::interface(12.0), theme::ICON_DEFAULT);
            let height = galley.size().y + 8.0 + 8.0;
            let (rect, response) = list_item(ui, height, true, row_count == 1, false, Sense::click());
            let text_position = Pos2::new(rect.right() - 16.0 - 4.0 - galley.size().x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(text_position, galley, theme::ICON_DEFAULT);
            if response.clicked() {
                edits.push((String::new(), Edit::AddInput));
            }
            row_index += 1;
        }
        // Black layers all take the colour at the layer count in the list.
        let list_fallback = palette[list.len() % palette.len()];
        let labels: Vec<String> = list.iter().map(|layer| source_name(node, nodes, &layer.key, &layer.key)).collect();
        for (layer, label) in list.iter().zip(&labels) {
            let first = row_index == 0;
            let last = row_index + 1 == row_count;
            row_index += 1;
            let is_selected = selected.as_deref() == Some(layer.key.as_str());
            let color = own_color(&layer.value).unwrap_or(list_fallback);
            if let Some(edit) = layer_row(ui, id, layer, label, color, first, last, is_selected, &mut selected) {
                edits.push((layer.key.clone(), edit));
            }
        }
        ground_layer(ui, node, nodes, row_index == 0);
    });

    if let Some(key) = selected.clone()
        && let Some(layer) = list.iter().find(|layer| layer.key == key) {
            let label = source_name(node, nodes, &key, &key);
            let index = layer.value.get("Index").and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
            let color = own_color(&layer.value).unwrap_or(palette[index % palette.len()]);
            if let Some(changed) = layer_editor(ui, id.with(("layer", &key)), &layer.value, &label, &key, color) {
                edits.push((key, Edit::Set(changed)));
            }
        }
    ui.data_mut(|data| data.insert_temp(selected_id, selected));
    edits
}

/// A `LayerListItem`: the visibility eye, a 32 preview, the source's name
/// in the layer's colour, the layer's name, the simplify eye and the
/// colour swatch (18 wide).
#[allow(clippy::too_many_arguments)]
fn layer_row(ui: &mut Ui, id: Id, layer: &Layer, label: &str, color: Color32, first: bool, last: bool, is_selected: bool, selected: &mut Option<String>) -> Option<Edit> {
    let (row, response) = list_item(ui, LAYER_ROW_HEIGHT, first, last, is_selected, Sense::click());
    let layout = LayerRowLayout::of(row);
    let mut edit = None;

    let visible = layer.value.get("IsVisible").and_then(Value::as_bool).unwrap_or(true);
    let eye = ui.interact(layout.eye.expand(8.0), id.with(("eye", &layer.key)), Sense::click());
    let eye_icon = if visible { "eye" } else { "eye-closed" };
    icons::paint(ui, layout.eye, eye_icon, theme::ICON_DEFAULT);
    if eye.clicked() {
        let mut changed = layer.value.clone();
        changed["IsVisible"] = json!(!visible);
        edit = Some(Edit::Set(changed));
    }
    // The preview (32, `Accent-Muted-Gray` border): the layer's colour.
    ui.painter().rect_filled(layout.preview, 0.0, color);
    ui.painter().rect_stroke(layout.preview, 0.0, Stroke::new(1.0, theme::ACCENT_MUTED_GRAY), StrokeKind::Inside);

    // `color`: a `ColorPicker` opening the `MinimalColorPicker`; the colour
    // picked is the layer's (`ToColor`).
    let swatch_id = id.with(("colour", &layer.key));
    if let Some(picked) = colour_button(ui, swatch_id, layout.swatch, [color.r(), color.g(), color.b()], "", None) {
        let [red, green, blue] = picked.map(srgb_to_linear);
        let mut changed = layer.value.clone();
        changed["Color"] = json!({"R": f32_value(red), "G": f32_value(green), "B": f32_value(blue)});
        edit = Some(Edit::Set(changed));
    }
    let simplified = layer.value.get("IsSimplified").and_then(Value::as_bool).unwrap_or(false);
    let simplify = ui.interact(layout.simplify.expand(8.0), id.with(("simplify", &layer.key)), Sense::click());
    let simplify_color = if simplified { theme::ACCENT_MUTED_MAGENTA } else { theme::ICON_DARKISH };
    icons::paint(ui, layout.simplify, "eye-dotted", simplify_color);
    if simplify.clicked() {
        let mut changed = layer.value.clone();
        changed["IsSimplified"] = json!(!simplified);
        edit = Some(Edit::Set(changed));
    }

    // The names, docked: the layer's on the right, whole as far as there
    // is room, and the source's in what is left, cut with "…"
    // (`CharacterEllipsis`) and to nothing when no room is left.
    let names = layout.names;
    let font = theme::monospace(12.0);
    let layer_galley = ui.painter().layout_no_wrap(humanize_title(&layer.key), font.clone(), theme::LABEL_DEFAULT);
    let layer_width = (layer_galley.size().x + 2.0 * NAME_MARGIN).min(names.width());
    let layer_slot = Rect::from_min_max(Pos2::new(names.right() - layer_width, names.top()), names.max);
    let label_slot = Rect::from_min_max(names.min, Pos2::new(layer_slot.left(), names.bottom()));
    let layer_text = layer_slot.shrink2(Vec2::new(NAME_MARGIN, 0.0));
    let layer_position = Pos2::new(layer_text.right() - layer_galley.size().x, names.center().y - layer_galley.size().y / 2.0);
    ui.painter_at(layer_text).galley(layer_position, layer_galley, theme::LABEL_DEFAULT);
    let label_text = label_slot.shrink2(Vec2::new(NAME_MARGIN, 0.0));
    if label_text.width() > 0.0 {
        let label_galley = elided_galley(ui.painter(), label, font, color, label_text.width());
        let label_position = Pos2::new(label_text.left(), names.center().y - label_galley.size().y / 2.0);
        ui.painter_at(label_text).galley(label_position, label_galley, color);
    }

    if response.clicked() {
        *selected = Some(layer.key.clone());
    }
    edit
}

/// The ground layer (disabled): the In map's 48 preview and "(Ground
/// Layer)" with its note, a line each, cut off at the row's padding
/// (laid out in a sideways `StackPanel`, they never wrap).
fn ground_layer(ui: &mut Ui, node: &NodeData, nodes: &IndexMap<i32, NodeData>, first: bool) {
    let (row, _) = list_item(ui, 48.0 + 16.0 + 8.0, first, true, false, Sense::hover());
    let content = row.shrink2(Vec2::new(16.0, 4.0));
    let preview = Rect::from_min_size(Pos2::new(content.left(), content.top() + 8.0), Vec2::splat(48.0));
    ui.painter().rect_filled(preview, 0.0, theme::PANEL_BACKGROUND);
    ui.painter().rect_stroke(preview, 0.0, Stroke::new(1.0, theme::ACCENT_MUTED_GRAY), StrokeKind::Inside);
    let source = source_name(node, nodes, "In", "");
    let title = ui.painter().layout_no_wrap(format!("{source} (Ground Layer)"), theme::semibold(12.0), theme::LABEL_DEFAULT);
    let note = ui.painter().layout_no_wrap("Each layer is added above this, bottom to top.".to_string(), theme::interface(10.0), theme::ICON_DEFAULT);
    let text_left = preview.right() + 8.0;
    let text_top = preview.top() + 8.0;
    let note_top = text_top + title.size().y;
    let painter = ui.painter_at(content);
    painter.galley(Pos2::new(text_left, text_top), title, theme::LABEL_DEFAULT);
    painter.galley(Pos2::new(text_left, note_top), note, theme::ICON_DEFAULT);
}

/// `LayerEditor` (margin 16; its stack 4 wider each side) for one layer;
/// returns the changed layer.
fn layer_editor(ui: &mut Ui, id: Id, layer: &Value, label: &str, key: &str, color: Color32) -> Option<Value> {
    let mut changed = layer.clone();
    let mut any = false;
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::symmetric(12, 16);
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        // The header: the source in the colour (monospace bold) and the
        // layer's name (margin 8, 4).
        let (header, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
        let name = ui.painter().layout_no_wrap(label.to_string(), theme::monospace_bold(12.0), color);
        ui.painter().galley(Pos2::new(header.left() + 8.0, header.center().y - name.size().y / 2.0), name, color);
        let layer_name = ui.painter().layout_no_wrap(humanize_title(key), theme::monospace(12.0), theme::LABEL_DEFAULT);
        let layer_position = Pos2::new(header.right() - 8.0 - layer_name.size().x, header.center().y - layer_name.size().y / 2.0);
        ui.painter().galley(layer_position, layer_name, theme::LABEL_DEFAULT);

        // The mask source (`lstSource`: `ListBox-OptionsX` at 10, its
        // `UniformGrid` one row at any width).
        let source = layer.get("Mask").and_then(Value::as_str).unwrap_or("Height").to_string();
        let entries: Vec<String> = SOURCES.iter().map(|(_, text)| text.to_string()).collect();
        let current = SOURCES.iter().position(|(member, _)| *member == source);
        if let Some(index) = option_list(ui, id.with("source"), &entries, current, entries.len(), 10.0) {
            changed["Mask"] = json!(SOURCES[index].0);
            any = true;
        }

        // `SliderVisibility`: ranges for Height, Slope and Mixed; Mixed has
        // a second (slope) range. Ranges and falloffs sit 4 apart, the
        // falloffs 22 high, none inset.
        let ranged = matches!(source.as_str(), "Height" | "Slope" | "Both");
        let mixed = source == "Both";
        let (range_name, falloff_name) = if mixed { ("Height Range", "Height Falloff") } else { ("Range", "Falloff") };
        let mut range_spec = RangeSpec::linear(0.0, 1.0);
        range_spec.inset = Some(0.0);
        let mut falloff_spec = SliderSpec::linear(0.0001, 1.0);
        falloff_spec.inset = Some(0.0);
        falloff_spec.height = Some(22.0);
        let mut ranges = vec![];
        if ranged {
            ranges.push(("Range", "Falloff", range_name, falloff_name));
        }
        if mixed {
            ranges.push(("Range2", "Falloff2", "Slope Range", "Slope Falloff"));
        }
        for (range_key, falloff_key, range_label, falloff_label) in ranges {
            ui.add_space(4.0);
            let (low, high) = float2(layer, range_key);
            if let Some((low, high)) = range_slider(ui, id.with(range_key), range_label, low, high, &range_spec, color) {
                changed[range_key] = json!({"X": f32_value(low as f32), "Y": f32_value(high as f32)});
                any = true;
            }
            ui.add_space(8.0);
            if let Some(value) = slider(ui, id.with(falloff_key), falloff_label, float(layer, falloff_key, 0.0001), &falloff_spec, color) {
                changed[falloff_key] = f32_value(value as f32);
                any = true;
            }
            ui.add_space(4.0);
        }

        // The separator (margin 0, 4, 0, 0), then Warp and Size side by
        // side (8 down, 4 apart each side of the middle).
        ui.add_space(4.0);
        let (rule, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment([rule.left_center(), rule.right_center()], Stroke::new(1.0, theme::MENU_SEPARATOR));
        ui.add_space(8.0);
        let mut plain = SliderSpec::linear(0.0, 1.0);
        plain.inset = Some(0.0);
        let plain_height = plain.height.unwrap_or(crate::theme::sizes(ui.ctx()).slider_height);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let half = (ui.available_width() / 2.0).floor();
            ui.allocate_ui(Vec2::new(half - 4.0, plain_height), |ui| {
                if ranged
                    && let Some(value) = slider(ui, id.with("warp"), "Warp", float(layer, "Warp", 0.0), &plain, color) {
                        changed["Warp"] = f32_value(value as f32);
                        any = true;
                    }
            });
            ui.add_space(8.0);
            ui.allocate_ui(Vec2::new(ui.available_width(), plain_height), |ui| {
                if let Some(value) = slider(ui, id.with("size"), "Size", float(layer, "Size", 0.25), &plain, color) {
                    changed["Size"] = f32_value(value as f32);
                    any = true;
                }
            });
        });

        // Blending and the blend mode (`LiteComboBox`, margin 8, 0, 0, 0).
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let blend_width = (ui.available_width() - 8.0 - 100.0).max(60.0);
            ui.allocate_ui(Vec2::new(blend_width, plain_height), |ui| {
                if let Some(value) = slider(ui, id.with("blend"), "Blending", float(layer, "Blending", 1.0), &plain, color) {
                    changed["Blending"] = f32_value(value as f32);
                    any = true;
                }
            });
            ui.add_space(8.0);
            let mode = layer.get("BlendMode").and_then(Value::as_str).unwrap_or("Blend").to_string();
            let entries: Vec<(String, String)> = BLEND_MODES.iter().map(|member| (member.to_string(), humanize_title(member))).collect();
            ui.allocate_ui(Vec2::new(ui.available_width(), plain_height), |ui| {
                let picked = dropdown_box(ui, id.with("blend-mode"), None, 0.0, 20.0, &humanize_title(&mode), &entries, theme::interface(12.0));
                if let Some(member) = picked.filter(|member| member != &mode) {
                    changed["BlendMode"] = json!(member);
                    any = true;
                }
            });
        });
    });
    if any { Some(changed) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::epaint::{ClippedShape, Shape, TextShape};

    /// What the panel has at 1600 by 960 and 120%: the property area at
    /// its narrowest.
    const DEFAULT_WIDTH: f32 = 234.0;
    const WIDTHS: [f32; 6] = [DEFAULT_WIDTH, 260.0, 300.0, 340.0, 420.0, 640.0];

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        egui_extras::install_image_loaders(&context);
        context.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        context
    }

    /// A Mixer with `layers` layers.
    fn mixer(layers: usize) -> (Document, i32) {
        let mut document = Document::new();
        let mixer = document.add_node("Mixer", [26000.0, 26000.0]).unwrap();
        while layers_of(&document, mixer).len() < layers {
            document.add_new_input(mixer).unwrap();
        }
        (document, mixer)
    }

    fn layers_of(document: &Document, mixer: i32) -> Vec<Layer> {
        let node = document.node(mixer).unwrap();
        layers(&node.values, node)
    }

    /// One frame of the panel `width` across: what it drew.
    fn frame(context: &egui::Context, document: &Document, mixer: i32, width: f32) -> Vec<ClippedShape> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1200.0));
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let node = document.node(mixer).unwrap();
        let nodes = &document.project.assets[0].terrain.nodes;
        let mut output = context.run_ui(input, |ui| {
            let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(20.0, 0.0), Vec2::new(width, 1200.0))));
            panel.spacing_mut().item_spacing = Vec2::ZERO;
            show(&mut panel, Id::new("mixer-test"), node, nodes, &node.values);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    }

    /// The topmost text saying `text` (in the list rather than the chosen
    /// layer's header below it) and its clip.
    fn text_saying<'a>(shapes: &'a [ClippedShape], text: &str) -> (Rect, &'a TextShape) {
        let found = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Text(shape) if shape.galley.text() == text => Some((clipped.clip_rect, shape)),
            _ => None,
        });
        let topmost = found.min_by(|(_, one), (_, other)| one.pos.y.total_cmp(&other.pos.y));
        topmost.unwrap_or_else(|| panic!("{text:?} shows"))
    }

    #[test]
    fn layer_names_dock_the_layer_s_right_and_the_source_s_in_what_is_left() {
        let context = themed_context();
        let (document, mixer) = mixer(3);
        for width in WIDTHS {
            let shapes = frame(&context, &document, mixer, width);
            // Past the preview (32 wide, 8 + 16 + 8 + 8 in from the row, 4
            // inside the list) and its margin, and short of the simplify
            // eye's button and margin (16 + 8 + 8 in from the swatch).
            let names_left = 20.0 + 4.0 + 16.0 + 8.0 + 16.0 + 8.0 + 8.0 + 32.0 + 8.0;
            let names_right = 20.0 + width - 4.0 - 16.0 - 18.0 - 8.0 - 32.0 - 8.0;
            for index in 1..=3 {
                let (layer_clip, layer) = text_saying(&shapes, &format!("Layer {index}"));
                assert!(layer_clip.left() >= names_left - 0.01 && layer_clip.right() <= names_right - 4.0 + 0.01, "at {width}: {layer_clip:?}");
                let layer_rect = layer.visual_bounding_rect().intersect(layer_clip);
                // The source's name, when there is room for it, keeps left
                // of the layer's and is cut with "…" to its room.
                let source = shapes.iter().find_map(|clipped| match &clipped.shape {
                    Shape::Text(shape) if shape.galley.text() == format!("Layer{index}") && shape.pos.y < layer.pos.y + 1.0 => Some(clipped.clip_rect),
                    _ => None,
                });
                if let Some(source_clip) = source {
                    assert!(source_clip.left() >= names_left + 4.0 - 0.01 && source_clip.right() <= layer_rect.left() - 4.0 + 0.5, "at {width}: {source_clip:?} beside {layer_rect:?}");
                }
            }
        }
    }

    #[test]
    fn layer_names_sit_side_by_side_where_they_fit() {
        let context = themed_context();
        let (document, mixer) = mixer(3);
        let shapes = frame(&context, &document, mixer, 420.0);
        let (_, source_shape) = text_saying(&shapes, "Layer2");
        let source = source_shape.visual_bounding_rect();
        let layer = text_saying(&shapes, "Layer 2").1.visual_bounding_rect();
        assert!(!source_shape.galley.elided, "Layer2 is cut");
        assert!(source.right() < layer.left() && source.center().y == layer.center().y, "{source:?} beside {layer:?}");
    }

    #[test]
    fn the_ground_layer_note_is_cut_off_at_the_row_s_padding() {
        let context = themed_context();
        let (document, mixer) = mixer(2);
        for width in WIDTHS {
            let shapes = frame(&context, &document, mixer, width);
            let list_right = 20.0 + width - 4.0;
            for text in [" (Ground Layer)", "Each layer is added above this, bottom to top."] {
                let (clip, shape) = text_saying(&shapes, text);
                assert!(clip.right() <= list_right - 16.0 + 0.01, "at {width}, {text:?} is cut off at {}, past the row's padding ({list_right})", clip.right());
                assert_eq!(shape.galley.rows.len(), 1, "at {width}, {text:?} wraps");
                assert!(!shape.galley.elided, "at {width}, {text:?} is cut short with an ellipsis");
            }
        }
    }

    #[test]
    fn a_layer_s_swatch_opens_the_minimal_picker() {
        let context = themed_context();
        let (document, mixer) = mixer(1);
        let width = 420.0;
        let shapes = frame(&context, &document, mixer, width);
        // The first row's swatch: 18 wide at the row's content edge.
        let swatch = shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if rect.rect.width() == 18.0 => Some(rect.rect),
            _ => None,
        });
        let swatch = swatch.expect("the swatch is drawn");
        let at = swatch.center();
        let node = document.node(mixer).unwrap();
        let nodes = &document.project.assets[0].terrain.nodes;
        let press = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        for events in [vec![egui::Event::PointerMoved(at)], vec![press(true)], vec![press(false)], Vec::new()] {
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 1200.0));
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let output = context.run_ui(input, |ui| {
                let mut panel = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(20.0, 0.0), Vec2::new(width, 1200.0))));
                panel.spacing_mut().item_spacing = Vec2::ZERO;
                show(&mut panel, Id::new("mixer-test"), node, nodes, &node.values);
            });
            output.drop_without_applying_deltas();
        }
        // The picker under it, at its least size.
        let pickers: Vec<Rect> = context.memory(|memory| {
            let layers = memory.areas().visible_layer_ids().into_iter().filter(|layer| layer.order == egui::Order::Foreground);
            layers.filter_map(|layer| memory.area_rect(layer.id)).collect()
        });
        let picker = pickers.into_iter().find(|area| (area.width() - 200.0).abs() < 0.5).expect("the swatch opens the picker");
        assert!((picker.top() - (swatch.bottom() - 10.0)).abs() < 0.5 && (picker.left() - swatch.left()).abs() < 0.5, "{picker:?} under {swatch:?}");
    }

    #[test]
    fn mask_sources_keep_to_one_row() {
        let context = themed_context();
        let (document, mixer) = mixer(1);
        for width in WIDTHS.into_iter().chain([200.0, 224.0, 312.0, 324.0]) {
            let shapes = frame(&context, &document, mixer, width);
            let rows: Vec<f32> = SOURCES.iter().map(|(_, text)| text_saying(&shapes, text).1.pos.y).collect();
            assert!(rows.iter().all(|row| (row - rows[0]).abs() < 0.5), "at {width}: {rows:?}");
        }
    }

    #[test]
    fn palette_shuffle_is_a_permutation() {
        let palette = layer_palette(1234);
        let mut original = LAYER_COLORS.map(rgb).to_vec();
        let mut shuffled = palette.to_vec();
        original.sort_by_key(|color| color.to_array());
        shuffled.sort_by_key(|color| color.to_array());
        assert_eq!(original, shuffled);
        assert_ne!(palette.to_vec(), LAYER_COLORS.map(rgb).to_vec());
    }
}
