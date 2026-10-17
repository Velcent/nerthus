//! The property area (`PropertyArea`): the selected node's header and
//! menu, its commands, its parameters in group panels (`EditorBase`
//! expanders) and the modifier toolbar at the bottom, laid out at the size
//! theme's sizes (`Size.Default.xaml`, or `Size.Compact.xaml` with Compact
//! Interface).

use std::sync::Arc;

use egui::{Align, Color32, FontId, Galley, Layout, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_engine::json::f32_value;
use nerthus_engine::{Catalogue, ParameterSpec, StateOverride};
use serde_json::Value;

use crate::editor::{DialogRequest, Editor};
use crate::icons;
use crate::node_ui::node_ui;
use crate::theme;
use crate::ui::bound_variable::{self, Bound};
use crate::ui::commands::Command;
use crate::ui::editors::{self, palette_color, Edit, EditorContext};
use crate::ui::graph_canvas::{node_color, node_icon};
use crate::ui::modifier_list::modifier_list;
use crate::ui::node_states::{self, StateAction};
use crate::ui::prompt::{task_button, task_link, TaskDialog};
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::{check_item, icon_menu, icon_menu_sized, link_button, menu_caption, menu_item, menu_item_enabled, menu_separator, submenu, toolbar_background, Tooltips};
use nerthus_engine::automation::VarType;

/// `ModifierStack`'s margin (0.2, 0, 0, 0) and its `WrapPanel`'s (4).
const STACK_MARGIN_LEFT: f32 = 0.2;
const WRAP_MARGIN: f32 = 4.0;
/// `ToggleButton-Expander-DownHeader`'s first column: the chevron's.
const GROUP_CHEVRON_COLUMN: f32 = 26.0;
/// `EditorBase.CONTROL_THICKNESS`: every editor row's margin.
const ROW_MARGIN: i8 = 4;
/// The parameter menu (`EditorMenu`): a 12px keyframe 4 from the editor.
const MENU_BUTTON_WIDTH: f32 = 16.0;
/// The editor scroller's right padding.
const SCROLL_PADDING: f32 = 8.0;
/// `Modifier-Toolbar` borders: padding and margin 4 around 24px toggles.
const GROUP_PADDING: f32 = 4.0;
const GROUP_MARGIN: f32 = 4.0;
const MODIFIER_BUTTON: f32 = 24.0;
/// `btnAddModifier`: 32 by 24.
const ADD_BUTTON_WIDTH: f32 = 32.0;
/// The node-state menu's `Border` (margin 8, 8, 12, 8) round its menu: a
/// `Toolbar.ButtonSize` icon item within `Toolbar.MenuMargin`.
const STATE_BORDER_MARGIN: [f32; 4] = [8.0, 8.0, 12.0, 8.0];
/// The Gate's and a baked node's panels, remembered apart from the
/// parameters' groups.
const GATE_PANEL_KEY: usize = usize::MAX - 1;
const BAKED_PANEL_KEY: usize = usize::MAX - 2;
/// Their buttons' padding and margin (16 across, 8 down).
const PANEL_BUTTON_PADDING: f32 = 8.0;
const PANEL_BUTTON_MARGIN: Vec2 = Vec2::new(16.0, 8.0);
/// The tinted notes (`DescendantAlert`, `InfoPortAdd`, `InfoHeightRef`):
/// margin 16, 4; a 1.5 border and padding 2 round their content; the
/// message's padding 4, 2.
const NOTE_MARGIN: Vec2 = Vec2::new(16.0, 4.0);
const NOTE_INSET: f32 = 1.5 + 2.0;
const NOTE_TEXT_PADDING: Vec2 = Vec2::new(4.0, 2.0);
/// The port note's close button (`Button-IconOnly`: a 20 icon, padding 2)
/// and its "More Info" link (`Button-Link`, padding 2, margin 0, 0, 4, 0),
/// docked right in that order.
const PORT_NOTE_CLOSE_ICON: f32 = 20.0;
const PORT_NOTE_CLOSE: f32 = PORT_NOTE_CLOSE_ICON + 2.0 * 2.0;
const PORT_NOTE_LINK_PADDING: Vec2 = Vec2::splat(2.0);
const PORT_NOTE_LINK_GAP: f32 = 4.0;
/// The Height note's link (`Button-Link`: padding 4, 2, at least 87 wide).
const HEIGHT_NOTE_LINK_PADDING: Vec2 = Vec2::new(4.0, 2.0);
const HEIGHT_NOTE_LINK_MIN_WIDTH: f32 = 87.0;

/// The modifier toggles: icon, tooltip, modifier type, their gaps (left
/// margins in `ModifierStack`) and accent (`Accent-Muted-Green`, else the
/// style's `Accent-Muted-Blue`).
struct ModifierToggle {
    icon: &'static str,
    tooltip: &'static str,
    type_name: &'static str,
    gap: f32,
    accent: Color32,
}

const fn modifier_toggle(icon: &'static str, tooltip: &'static str, type_name: &'static str, gap: f32, accent: Color32) -> ModifierToggle {
    ModifierToggle { icon, tooltip, type_name, gap, accent }
}

const MODIFIER_TOGGLES: [&[ModifierToggle]; 3] = [
    &[
        modifier_toggle("x-autolevel", "Autolevel", "Autolevel", 0.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("x-shaper", "Shaper", "Shaper", 4.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("ruler-measure-2", "Height Remap", "Multiplier", 4.0, theme::ACCENT_MUTED_GREEN),
    ],
    &[
        modifier_toggle("x-drop", "Drop", "Drop", 2.0, theme::ACCENT_MUTED_GREEN),
        modifier_toggle("x-invert", "Invert", "Invert", 0.0, theme::ACCENT_MUTED_GREEN),
    ],
    &[
        modifier_toggle("minimize", "Min", "Min", 4.0, theme::ACCENT_MUTED_BLUE),
        modifier_toggle("maximize", "Max", "Max", 4.0, theme::ACCENT_MUTED_BLUE),
    ],
];

pub fn show(ui: &mut Ui, rect: Rect, editor: &mut Editor, commands: &mut Vec<Command>) {
    crate::ui::presets::show_create_window(ui.ctx(), editor);
    crate::ui::math_editor::show(ui.ctx(), editor);
    crate::ui::painter::show(ui.ctx(), editor);
    ui.painter().rect_filled(rect, 4.0, theme::PANEL_BACKGROUND);
    let mut actions: Vec<StateAction> = Vec::new();
    let Some(node_id) = area_parts(ui, rect, editor, commands, &mut actions) else {
        return;
    };
    parameter_dialogs(ui, editor);
    for action in actions {
        node_states::apply(ui, editor, node_id, action, commands);
    }
}

/// The area's parts laid out in `area`, as its `DockPanel`s dock them: the
/// header, then for the active node its command row and a warning at the
/// top; in the rest, the modifier stack at the foot, the locked-preview
/// alert at the top, the port note at the foot over the stack, the Height
/// note under the alert, and the parameters in what is left. Returns the
/// node shown.
fn area_parts(ui: &mut Ui, area: Rect, editor: &mut Editor, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) -> Option<i32> {
    // `PropertyBorder`: a `Toolbar` (margin `Toolbar.Margin`, at least
    // `Toolbar.MinHeight` high).
    let sizes = theme::sizes(ui.ctx());
    let header_margin = sizes.toolbar_margin;
    let header = Rect::from_min_size(area.min + Vec2::splat(header_margin), Vec2::new(area.width() - 2.0 * header_margin, sizes.toolbar_min_height));
    header_toolbar(ui, header, editor, commands, actions);
    if node_states::node_changed(ui, editor, editor.active_node()) {
        editor.tiled_preview = false;
    }

    let node_id = editor.active_node()?;
    editor.document.node(node_id)?;
    node_states::note_shown_node(ui, editor, node_id);

    let mut body_top = header.bottom() + header_margin;
    if let Some(row) = command_row(ui, area, body_top, editor, node_id, commands) {
        body_top = row.bottom() + sizes.property_border[3];
    }
    // Kept through a rebuild until the build reports the node.
    if let Some(error) = editor.shown_error(node_id) {
        body_top = warning_row(ui, area, body_top, &error);
    }

    let stack_height = modifier_stack_height(area.width(), sizes);
    let stack_rect = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - stack_height), area.max);
    let mut body = Rect::from_min_max(Pos2::new(area.left(), body_top), Pos2::new(area.right(), stack_rect.top()));
    let note_width = (body.width() - 2.0 * NOTE_MARGIN.x).max(0.0);
    if let Some(alert) = descendant_alert(editor, node_id) {
        let alert_rect = notice(ui, body.min + NOTE_MARGIN, note_width, alert, theme::ACCENT_TRANSPARENT_ORANGE, theme::ACCENT_STRONG_ORANGE);
        body.min.y = alert_rect.bottom() + NOTE_MARGIN.y;
    }
    let can_add_ports = editor.document.node(node_id).is_some_and(|node| nerthus_engine::editing::can_add_ports(&node.type_name));
    let port_note_id = ui.id().with("port-note-dismissed");
    let port_note_dismissed: bool = ui.data(|data| data.get_temp(port_note_id)).unwrap_or(false);
    if can_add_ports && !port_note_dismissed {
        let (note_rect, click) = port_note(ui, Pos2::new(body.left() + NOTE_MARGIN.x, body.bottom() - NOTE_MARGIN.y), note_width);
        match click {
            Some(PortNoteClick::Dismiss) => {
                ui.data_mut(|data| data.insert_temp(port_note_id, true));
            }
            Some(PortNoteClick::MoreInfo) => {
                commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::ADD_PORTS.to_string())));
            }
            None => {}
        }
        body.max.y = note_rect.top() - NOTE_MARGIN.y;
    }
    let needs_height = editor.document.node(node_id).is_some_and(node_states::needs_height);
    if needs_height {
        let (note_rect, clicked) = height_note(ui, body.min + NOTE_MARGIN, note_width);
        if clicked {
            actions.push(StateAction::ConnectHeight);
        }
        body.min.y = note_rect.bottom() + NOTE_MARGIN.y;
    }

    // `editorContainer` fills what is left, nothing when nothing is.
    body.max.y = body.max.y.max(body.min.y);
    let builder = UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min));
    let mut body_ui = ui.new_child(builder);
    body_ui.shrink_clip_rect(body);
    let mut scroll = ScrollArea::vertical();
    scroll = scroll.id_salt(("properties", node_id));
    scroll = scroll.auto_shrink([false, false]);
    // Not egui's least of 64 past the body, over the modifier stack.
    scroll = scroll.min_scrolled_height(0.0);
    let area_width = area.width();
    scroll.show(&mut body_ui, |ui| {
        let content_width = (ui.available_width() - SCROLL_PADDING).max(0.0);
        ui.set_width(content_width);
        parameters(ui, editor, node_id, area_width, commands);
    });

    modifier_toolbar(ui, stack_rect, editor, node_id, commands, actions);
    Some(node_id)
}

/// The least height the window keeps the area at, `width` wide, at the
/// default sizes (the larger): its header, a command row and the modifier
/// stack, and between them room for the parameters to show a group
/// panel's header and first row. A warning and the notes take from that.
pub fn least_height(width: f32) -> f32 {
    let sizes = &theme::DEFAULT_SIZES;
    let header = sizes.toolbar_margin + sizes.toolbar_min_height + sizes.toolbar_margin;
    let [_, above, _, below] = sizes.property_border;
    let command_row = above + sizes.toolbar_min_height + below;
    let parameters = sizes.expander_margin[1] + sizes.expander_header + sizes.slider_height + 2.0 * f32::from(ROW_MARGIN);
    header + command_row + parameters + modifier_stack_height(width, sizes)
}

/// The modifier stack's height at the foot of an area `width` wide.
fn modifier_stack_height(width: f32, sizes: &theme::Sizes) -> f32 {
    let measure = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 0.0));
    stack_layout(measure, sizes.toolbar_button, sizes.toolbar_menu_margin).height
}

/// The command row (`CommandBorder`, a toolbar with the margins
/// `Property.Border.Thickness`, `top` being under the header's): a button
/// per command parameter the node shows (`ToolbarButton-IconXText`, icons
/// in `ParamsA-Color1..4`), then "Toggle Gizmo" for nodes with a
/// `[Gizmo]`.
fn command_row(ui: &mut Ui, area: Rect, top: f32, editor: &mut Editor, node_id: i32, commands: &mut Vec<Command>) -> Option<Rect> {
    let node = editor.document.node(node_id)?;
    let spec = Catalogue::get().node(&node.type_name)?;
    let asset = &editor.document.project.assets[0];
    let rules = node_ui(node, &asset.terrain.nodes, &asset.automation);
    let entries: Vec<&ParameterSpec> = spec.parameters.iter().filter(|parameter| parameter.kind == "Command" && !parameter.hidden && rules.visible(&parameter.name)).collect();
    let has_gizmo = spec.gizmo.is_some();
    if entries.is_empty() && !has_gizmo {
        return None;
    }
    let sizes = theme::sizes(ui.ctx());
    let [left, above, right, _] = sizes.property_border;
    let row = Rect::from_min_size(Pos2::new(area.left() + left, top + above), Vec2::new(area.width() - left - right, sizes.toolbar_min_height));
    toolbar_background(ui, row, theme::PANEL_BORDER, 1.0);
    // `Toolbar.Padding` inside the 1px border.
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = row.shrink(1.0).shrink2(Vec2::new(padding_across, padding_down));
    let builder = UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center));
    let mut bar = ui.new_child(builder);
    bar.spacing_mut().item_spacing.x = 0.0;
    const COMMAND_COLORS: [u32; 4] = [0xe4ba4a, 0xa9b871, 0x6eb699, 0xe4ba4a];
    for (index, parameter) in entries.iter().enumerate() {
        let icon = parameter.command.as_ref().and_then(Value::as_str).unwrap_or("square-rounded");
        // The `[Name]`, else the property name humanized (sentence case),
        // which is also the tooltip.
        let label = parameter.label.clone().unwrap_or_else(|| nerthus_engine::humanize::humanize(&parameter.name));
        let color_value = COMMAND_COLORS[(index + 1) % 4];
        let color = Color32::from_rgb((color_value >> 16) as u8, (color_value >> 8) as u8, color_value as u8);
        let response = icon_text_button(&mut bar, icon, &label, color, None);
        let response = response.tooltip(&label);
        if response.clicked() {
            open_command_window(ui, editor, node_id, &parameter.name, commands);
        }
    }
    if has_gizmo {
        let response = icon_text_button(&mut bar, "gizmo", "Toggle Gizmo", theme::ACCENT_MUTED_PURPLE, Some(editor.gizmo_active));
        if response.tooltip("Toggle Gizmo").clicked() {
            editor.gizmo_active = !editor.gizmo_active;
        }
    }
    Some(row)
}

/// A command button clicked: the commands that open a window of their own
/// (`OpenEditor` for Math), else the node command.
fn open_command_window(ui: &Ui, editor: &Editor, node_id: i32, name: &str, commands: &mut Vec<Command>) {
    let type_name = editor.document.node(node_id).map(|node| node.type_name.as_str()).unwrap_or_default();
    match (type_name, name) {
        ("MathX", "OpenEditor") => crate::ui::math_editor::open(ui.ctx(), editor, node_id),
        ("Draw", "OpenPainter") => crate::ui::painter::open(ui.ctx(), editor, node_id),
        ("Construction", "OpenGodMode") => commands.push(Command::OpenConstructor(node_id)),
        _ => commands.push(Command::NodeAction(name.to_string())),
    }
}

/// `ToolbarButton-IconXText` (and `ToggleButton-IconXText` when `checked`
/// is given): `Toolbar.ButtonSize` high, the icon 8 in, the text 30 in.
fn icon_text_button(ui: &mut Ui, icon: &str, text: &str, accent: Color32, checked: Option<bool>) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let width = 30.0 + galley.size().x + 8.0;
    let height = theme::sizes(ui.ctx()).toolbar_button;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let on = checked == Some(true);
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, accent), StrokeKind::Inside);
    }
    let icon_color = if hovered || on { theme::LABEL_EMPHASIS } else { accent };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, icon_color);
    ui.painter().galley(Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    response
}

/// `WarningBorder`: the node's error in red, a toolbar (margins
/// `Property.Border.Thickness`, padding `Toolbar.Padding`) with the text
/// `Property.Border.Padding` in.
fn warning_row(ui: &mut Ui, area: Rect, top: f32, error: &str) -> f32 {
    let sizes = theme::sizes(ui.ctx());
    let [left, above, right, below] = sizes.property_border;
    let width = area.width() - left - right;
    let text_inset = sizes.toolbar_padding[0] + sizes.property_padding[0];
    let galley = ui.painter().layout(error.to_string(), theme::interface(12.0), theme::ACCENT_RED, width - 2.0 - 2.0 * text_inset);
    let height = (galley.size().y + 2.0 + 2.0 * sizes.property_padding[1] + 2.0 * sizes.toolbar_padding[1]).max(sizes.toolbar_min_height);
    let row = Rect::from_min_size(Pos2::new(area.left() + left, top + above), Vec2::new(width, height));
    toolbar_background(ui, row, theme::PANEL_BORDER, 1.0);
    let text_top = row.center().y - galley.size().y / 2.0;
    ui.painter().galley(Pos2::new(row.left() + 1.0 + text_inset, text_top), galley, theme::ACCENT_RED);
    row.bottom() + below
}

/// `DescendantAlert`'s text.
const DESCENDANT_ALERT: &str = "This node is not a descendant of the LOCKED node.\nChanges will not show in the current preview.";

/// `DescendantAlert`: the preview is locked to a node this one does not
/// feed.
fn descendant_alert(editor: &Editor, node_id: i32) -> Option<&'static str> {
    if !editor.preview_locked {
        return None;
    }
    let locked = editor.preview_node?;
    let upstream = nerthus_engine::Engine::build_order(editor.document.terrain(), locked);
    if upstream.contains(&node_id) {
        return None;
    }
    Some(DESCENDANT_ALERT)
}

/// `DescendantAlert`: a tinted note box (`Accent-Transparent-…` fill,
/// `Accent-Strong-…` border) `width` wide from `top_left`, its text
/// (10, centred, wrapped) `NOTE_INSET` and `NOTE_TEXT_PADDING` in, the box
/// as high as that. Returns the box.
fn notice(ui: &mut Ui, top_left: Pos2, width: f32, text: &str, fill: egui::Color32, border: egui::Color32) -> Rect {
    let wrap_width = (width - 2.0 * (NOTE_INSET + NOTE_TEXT_PADDING.x)).max(0.0);
    let mut job = egui::text::LayoutJob::simple(text.to_string(), theme::interface(10.0), theme::LABEL_EMPHASIS, wrap_width);
    job.halign = Align::Center;
    let galley = ui.painter().layout_job(job);
    let height = galley.size().y + 2.0 * (NOTE_INSET + NOTE_TEXT_PADDING.y);
    let rect = Rect::from_min_size(top_left, Vec2::new(width, height));
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, border), egui::StrokeKind::Inside);
    let position = Pos2::new(rect.center().x, rect.top() + NOTE_INSET + NOTE_TEXT_PADDING.y);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    rect
}

/// What was clicked on the note that a node can add inputs.
enum PortNoteClick {
    Dismiss,
    MoreInfo,
}

/// Where a note's parts go (`InfoPortAdd`, `InfoHeightRef`), from its top
/// left, as its `DockPanel` lays them out: its links docked right, and the
/// message (padding 4, 2) wrapped in what they leave, centred down; the
/// note as high as the taller, inside its border and padding.
struct NoteLayout {
    size: Vec2,
    galley: Arc<Galley>,
    message: Rect,
    links: Rect,
}

/// `text` in `font` laid out in a note `width` wide whose links take
/// `links` on the right.
fn note_layout(ui: &Ui, text: &str, font: FontId, width: f32, links: Vec2) -> NoteLayout {
    let inner_width = (width - 2.0 * NOTE_INSET).max(0.0);
    let slot = (inner_width - links.x).max(0.0);
    let wrap_width = (slot - 2.0 * NOTE_TEXT_PADDING.x).max(0.0);
    let galley = ui.painter().layout(text.to_string(), font, theme::LABEL_EMPHASIS, wrap_width);
    let message_size = galley.size();
    let content = (message_size.y + 2.0 * NOTE_TEXT_PADDING.y).max(links.y);
    let size = Vec2::new(width, content + 2.0 * NOTE_INSET);
    let inner = Rect::from_min_size(Pos2::ZERO, size).shrink(NOTE_INSET);
    let message = Rect::from_min_size(Pos2::new(inner.left() + NOTE_TEXT_PADDING.x, inner.center().y - message_size.y / 2.0), message_size);
    let links = Rect::from_min_size(Pos2::new(inner.right() - links.x, inner.center().y - links.y / 2.0), links);
    NoteLayout { size, galley, message, links }
}

/// Paints a note laid out as `layout` from `top_left`: the box (`fill`,
/// a 1.5 `border`) and its message, cut to the message's slot. Returns the
/// box and where its links go.
fn paint_note(ui: &Ui, top_left: Pos2, layout: NoteLayout, fill: Color32, border: Color32) -> (Rect, Rect) {
    let offset = top_left.to_vec2();
    let rect = Rect::from_min_size(top_left, layout.size);
    let links = layout.links.translate(offset);
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, border), StrokeKind::Inside);
    let slot = Rect::from_min_max(rect.min + Vec2::splat(NOTE_INSET), Pos2::new(links.left(), rect.bottom() - NOTE_INSET));
    ui.painter().with_clip_rect(slot.intersect(ui.clip_rect())).galley(layout.message.min + offset, layout.galley, theme::LABEL_EMPHASIS);
    (rect, links)
}

/// The port note `width` wide: its close button and "More Info" link on
/// the right.
fn port_note_layout(ui: &Ui, width: f32) -> NoteLayout {
    let link = ui.painter().layout_no_wrap("More Info".to_string(), theme::interface(12.0), theme::ACCENT_BLUE).size() + 2.0 * PORT_NOTE_LINK_PADDING;
    let links = Vec2::new(link.x + PORT_NOTE_LINK_GAP + PORT_NOTE_CLOSE, link.y.max(PORT_NOTE_CLOSE));
    note_layout(ui, "This node can add additional input ports.", theme::interface(10.0), width, links)
}

/// `InfoPortAdd`: the node can add inputs, in a note `width` wide that
/// ends at `bottom_left`; its close button (`Button-Background`, the hover
/// fill under the pointer) dismisses it, "More Info" opens the
/// documentation about adding ports. Returns the note and what was
/// clicked.
fn port_note(ui: &mut Ui, bottom_left: Pos2, width: f32) -> (Rect, Option<PortNoteClick>) {
    let layout = port_note_layout(ui, width);
    let top_left = bottom_left - Vec2::new(0.0, layout.size.y);
    let (rect, links) = paint_note(ui, top_left, layout, theme::ACCENT_TRANSPARENT_BLUE, theme::ACCENT_STRONG_BLUE);
    let builder = UiBuilder::new().max_rect(links).layout(Layout::right_to_left(Align::Center));
    let mut row = ui.new_child(builder);
    row.spacing_mut().item_spacing.x = 0.0;
    let mut clicked = None;
    let (close_rect, close) = row.allocate_exact_size(Vec2::splat(PORT_NOTE_CLOSE), Sense::click());
    if close.hovered() {
        row.painter().rect_filled(close_rect, 2.0, theme::BUTTON_HOVER);
    }
    icons::paint(&row, Rect::from_center_size(close_rect.center(), Vec2::splat(PORT_NOTE_CLOSE_ICON)), "x", theme::BUTTON_BACKGROUND);
    if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        clicked = Some(PortNoteClick::Dismiss);
    }
    row.add_space(PORT_NOTE_LINK_GAP);
    if link_button(&mut row, "More Info", theme::interface(12.0), PORT_NOTE_LINK_PADDING).clicked() {
        clicked = Some(PortNoteClick::MoreInfo);
    }
    (rect, clicked)
}

/// The Height note `width` wide, and its "Connect for me!" link (10
/// semibold, `Accent-Bright`) for the right.
fn height_note_layout(ui: &Ui, width: f32) -> (NoteLayout, Arc<Galley>) {
    let link = ui.painter().layout_no_wrap("Connect for me!".to_string(), theme::semibold(10.0), theme::ACCENT_BRIGHT);
    let size = link.size() + 2.0 * HEIGHT_NOTE_LINK_PADDING;
    let links = Vec2::new(size.x.max(HEIGHT_NOTE_LINK_MIN_WIDTH), size.y);
    let layout = note_layout(ui, "A Height connection is required.", theme::interface(9.0), width, links);
    (layout, link)
}

/// `InfoHeightRef`: a green note `width` wide from `top_left` that a
/// Height connection is needed, with its "Connect for me!" link on the
/// right (its text centred in the link's least width). Returns the note,
/// and whether the link was clicked.
fn height_note(ui: &mut Ui, top_left: Pos2, width: f32) -> (Rect, bool) {
    let (layout, link_galley) = height_note_layout(ui, width);
    let (rect, links) = paint_note(ui, top_left, layout, theme::ACCENT_TRANSPARENT_GREEN, theme::ACCENT_STRONG_GREEN);
    // Its own `Accent-Bright` outranks `Button-Link`'s hover colour.
    let link = ui.interact(links, ui.id().with("height-note-link"), Sense::click());
    ui.painter().galley(links.center() - link_galley.size() / 2.0, link_galley, theme::ACCENT_BRIGHT);
    if link.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (rect, link.clicked())
}

/// The header toolbar (`PropertyBorder`): the node's icon and name (the
/// family menu, 11px, with its corner arrow), then the presets and node
/// menus on the right.
fn header_toolbar(ui: &mut Ui, rect: Rect, editor: &Editor, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    toolbar_background(ui, rect, theme::PANEL_BORDER, 1.0);
    let sizes = theme::sizes(ui.ctx());
    let [padding_across, padding_down] = sizes.toolbar_padding;
    let inner = rect.shrink(1.0).shrink2(Vec2::new(padding_across, padding_down));
    let node = editor.active_node().and_then(|id| editor.document.node(id));

    // Two icon menus docked right, each `Toolbar.ButtonSize` wide with
    // `Toolbar.MenuMargin` either side.
    let button = sizes.toolbar_button;
    let menu_margin = sizes.toolbar_menu_margin;
    let menu_rect = Rect::from_center_size(Pos2::new(inner.right() - (button / 2.0 + menu_margin), inner.center().y), Vec2::splat(button));
    let presets_rect = menu_rect.translate(Vec2::new(-(button + 2.0 * menu_margin), 0.0));

    // `mnuIcon` (margin 8, 0, 4, 0), `mnuText` (margin 4, 3, 4, 4) and the
    // 8px `chevron-down-right` (margin 8, 1, 1, 1) at the bottom, in what
    // the menus leave of the frame: a long name runs on to its edge, where
    // it and the chevron after it are cut off.
    let (icon_name, icon_color, name) = match node {
        // `mnuText`: the name, and the type when the name is not it.
        Some(node) if node.name == node.type_name => (node_icon(&node.type_name), node_color(node), node.name.clone()),
        Some(node) => (node_icon(&node.type_name), node_color(node), format!("{} ({})", node.name, node.type_name)),
        None => ("square-rounded".to_string(), theme::ICON_DEFAULT, "(Select a node)".to_string()),
    };
    let frame = rect.shrink(1.0);
    let family_rect = Rect::from_min_max(frame.min, Pos2::new((presets_rect.left() - menu_margin).max(frame.left()), frame.bottom()));
    let mut family_ui = ui.new_child(UiBuilder::new().max_rect(family_rect));
    family_ui.shrink_clip_rect(family_rect);
    let icon_rect = Rect::from_min_size(Pos2::new(inner.left() + 8.0, inner.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(&family_ui, icon_rect, &icon_name, icon_color);
    let name_left = icon_rect.right() + 8.0;
    let name_galley = family_ui.painter().layout_no_wrap(name, theme::interface(11.0), theme::MENU_FOREGROUND);
    let name_rect = Rect::from_min_size(Pos2::new(name_left, inner.center().y - name_galley.size().y / 2.0), name_galley.size());
    family_ui.painter().galley(name_rect.min, name_galley, theme::MENU_FOREGROUND);
    let arrow_rect = Rect::from_min_size(Pos2::new(name_rect.right() + 4.0 + 8.0, inner.bottom() - 1.0 - 8.0 - 4.0), Vec2::splat(8.0));
    icons::paint(&family_ui, arrow_rect, "chevron-down-right", theme::ICON_DEFAULT);
    if let Some(node) = node {
        let spec = Catalogue::get().node(&node.type_name);
        let family = spec.and_then(|spec| spec.family.clone()).unwrap_or_default();
        let description = spec.and_then(|spec| spec.description.clone()).unwrap_or_default();
        let name_area = Rect::from_min_max(icon_rect.min, Pos2::new(arrow_rect.right(), inner.bottom())).intersect(family_rect);
        let response = ui.interact(name_area, ui.id().with("family-menu"), Sense::click());
        let response = response.tooltip(format!("{family}\n\n{description}"));
        crate::ui::graph_popups::family_menu(&response, editor, node, commands);
    }

    let has_node = node.is_some();
    let intent = node.and_then(|node| node.base.get("RenderIntentOverride")).and_then(Value::as_str).unwrap_or("Default").to_string();
    let saved_state = node_states::has_saved_state(ui);
    let mut menu_ui = ui.new_child(UiBuilder::new().max_rect(menu_rect));
    icon_menu(&mut menu_ui, "menu", "Node", None, |ui| {
        menu_caption(ui, "Node State");
        state_items(ui, has_node, saved_state, commands, actions);
        menu_separator(ui);
        if menu_item(ui, None, None, "Bind all properties", "").clicked() && has_node {
            actions.push(StateAction::BindAll);
        }
        menu_separator(ui);
        menu_caption(ui, "Render Intent (Override)");
        for choice in ["Default", "Heightfield", "Mask"] {
            if check_item(ui, intent == choice, choice, "").clicked() && has_node {
                let static_choice: &'static str = match choice {
                    "Heightfield" => "Heightfield",
                    "Mask" => "Mask",
                    _ => "Default",
                };
                commands.push(Command::SetRenderIntent(static_choice));
            }
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Show Help for this Node", "").clicked() {
            commands.push(Command::Tools(ToolCommand::ShowNodeHelp));
        }
    });
    let mut presets_ui = ui.new_child(UiBuilder::new().max_rect(presets_rect));
    icon_menu(&mut presets_ui, "cube-sphere", "Presets", None, |ui| {
        crate::ui::presets::presets_menu(ui, editor, commands);
    });
}

/// One `EditorBase` expander's worth of parameters, with each editor's
/// accent.
struct ParameterGroup<'a> {
    title: String,
    /// Which `[Group]` started it (the parameter's index), for the
    /// expander's remembered state.
    key: usize,
    expanded: bool,
    entries: Vec<(&'a ParameterSpec, Color32)>,
}

/// The parameters as `EditorBase.CreateParameters` groups them: an
/// expander titled with the node type until the first `[Group]`, then one
/// per `[Group]`. Each `[Group]` moves the editor colours to the next
/// palette (`colorFlip`, Processors start one along) and each editor made
/// takes the next colour of it.
/// Also returns the palette the last group left (`colorFlip`), where the
/// modifiers carry on.
fn parameter_groups<'a>(spec: &'a nerthus_engine::NodeSpec) -> (Vec<ParameterGroup<'a>>, usize) {
    let processor = spec.classification.as_deref() == Some("Processor");
    let mut color_flip = if processor { 1 } else { 0 };
    let mut made = 0usize;
    let mut groups: Vec<ParameterGroup> = Vec::new();
    for (index, parameter) in spec.parameters.iter().enumerate() {
        if parameter.group_start {
            color_flip = (color_flip + 1) % 3;
            let group = ParameterGroup {
                title: parameter.group.clone().unwrap_or_default(),
                key: index,
                expanded: !parameter.group_collapsed,
                entries: Vec::new(),
            };
            groups.push(group);
        }
        if parameter.hidden {
            continue;
        }
        if groups.is_empty() {
            let group = ParameterGroup {
                title: spec.name.clone(),
                key: usize::MAX,
                expanded: true,
                entries: Vec::new(),
            };
            groups.push(group);
        }
        // `Command` parameters have no editor (they are the command row).
        if parameter.kind == "Command" {
            continue;
        }
        let color = palette_color(color_flip, made % 8);
        made += 1;
        if let Some(group) = groups.last_mut() {
            group.entries.push((parameter, color));
        }
    }
    (groups, color_flip)
}

/// The parameters in their group panels, then the modifiers; applies the
/// edits made. A Gate shows its bake panel instead, and a baked node its
/// own (`EditorBase.GenerateControls`).
fn parameters(ui: &mut Ui, editor: &mut Editor, node_id: i32, area_width: f32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(node_id) else {
        return;
    };
    let Some(spec) = Catalogue::get().node(&node.type_name) else {
        ui.label("Unknown node type");
        return;
    };
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    let state = StateOverride::of(node);
    if node.type_name == "Gate" {
        gate_panel(ui, node_id, state, commands);
        return;
    }
    if state == StateOverride::Baked {
        baked_panel(ui, node_id, commands);
        return;
    }
    let asset = &editor.document.project.assets[0];
    let mut rules = node_ui(node, &asset.terrain.nodes, &asset.automation);
    crate::node_ui::update_file_summaries(editor, node, &mut rules);
    let values = node.values.clone();
    let (groups, last_color_flip) = parameter_groups(spec);
    // What terrain-mark sliders mark: the node's own output as last built.
    let terrain_range = editor.status(node_id).range.map(|(lowest, highest)| (lowest as f64, highest as f64));
    // What a bound parameter's expression of its variable is worked out
    // against.
    let expression_context = nerthus_engine::automation::ExpressionContext {
        resolution: nerthus_engine::Resolution::square(editor.preview_resolution),
        terrain_height: asset.terrain.height,
    };
    let mut edits: Vec<(String, Edit)> = Vec::new();
    // Each seed box's id, for the seeds its dice draws.
    let mut seed_boxes: std::collections::HashMap<String, egui::Id> = std::collections::HashMap::new();
    for group in &groups {
        group_panel(ui, (node_id, group.key), &group.title, group.expanded, |ui| {
            for (parameter, color) in &group.entries {
                let shown = parameter.visible && rules.visible(&parameter.name);
                if !shown {
                    continue;
                }
                if parameter.separator {
                    separator(ui);
                }
                let computed = rules.texts.get(&parameter.name).map(|text| Value::String(text.clone()));
                let value = computed.or_else(|| values.get(&parameter.name).cloned()).unwrap_or(Value::Null);
                let id = ui.id().with(("parameter", node_id, &parameter.name));
                let enabled = parameter.enabled && rules.enabled(&parameter.name);
                let context = EditorContext {
                    color: *color,
                    default: spec.defaults.get(&parameter.name),
                    values: Some(&values),
                    area_width,
                    terrain_range,
                };
                take_launch_right_click(ui.ctx(), &parameter.name, id);
                // A bound parameter shows its variable, whatever its own
                // editor would be (`EditorControlFactory.GetControl` asks
                // first).
                let binding = asset.automation.bindings.iter().find(|binding| binding.node == node_id && binding.property == parameter.name);
                if let Some(binding) = binding {
                    let bound = Bound {
                        node: node_id,
                        parameter,
                        key: &binding.variable,
                        automation: &asset.automation,
                        context: &expression_context,
                    };
                    let tooltip = bound_variable::tooltip(parameter, &binding.variable);
                    let bind_type = bind_type(&asset.automation, node_id, &parameter.name);
                    let automated = Automated::of(&asset.automation, node_id, &parameter.name);
                    let edit = editor_row(ui, enabled, Some(&tooltip), bind_type, &automated, |ui| bound_variable::bound_variable(ui, id, &bound));
                    edits.push((parameter.name.clone(), edit));
                    continue;
                }
                // Labels stand alone (`EditorLabel`, `ComputedString`).
                let is_label = parameter.value_type == nerthus_engine::ValueType::Byte || parameter.kind == "ComputedString";
                if is_label {
                    let row = ui.add_enabled_ui(enabled, |ui| editors::parameter(ui, id, parameter, &value, &context));
                    if let Some(help) = &parameter.description {
                        row.response.tooltip(help);
                    }
                    edits.push((parameter.name.clone(), row.inner));
                    continue;
                }
                let custom_row = ui.scope(|ui| custom_editor(ui, id, parameter, &value, node, &asset.terrain.nodes, &editor.watched_outputs, editor.document_serial));
                if let Some(custom) = custom_row.inner {
                    if let Some(help) = &parameter.description {
                        custom_row.response.tooltip(help);
                    }
                    edits.extend(custom);
                    continue;
                }
                if parameter.is_seed() {
                    seed_boxes.insert(parameter.name.clone(), id);
                }
                let bind_type = bind_type(&asset.automation, node_id, &parameter.name);
                let automated = Automated::of(&asset.automation, node_id, &parameter.name);
                // An option bar without a label is named by its tooltip
                // until the documentation gives it one.
                let unlabelled = parameter.kind == "HorizontalListNoLabel" && area_width > 300.0;
                let unlabelled_name = unlabelled.then_some(parameter.name.as_str());
                let help = parameter.description.as_deref().or(unlabelled_name);
                let edit = editor_row(ui, enabled, help, bind_type, &automated, |ui| editors::parameter(ui, id, parameter, &value, &context));
                edits.push((parameter.name.clone(), edit));
            }
        });
    }
    let modifier_edits = modifier_list(ui, node, node_id, last_color_flip, area_width);
    ui.add_space(16.0);

    if let Some((place, shift)) = modifier_edits.close {
        commands.push(Command::RemoveModifier { node: node_id, place, ask: !shift });
    }
    if let Some((from, to)) = modifier_edits.moved
        && editor.document.move_modifier(node_id, from, to)
    {
        editor.changed();
    }
    for (index, name, edit) in modifier_edits.edits {
        let Some(modifier) = editor.document.node(node_id).and_then(|data| data.modifiers.get(index)) else {
            continue;
        };
        let value = match edit {
            Edit::Set(value) => Some(value),
            Edit::Reset => Catalogue::get().modifier(&modifier.type_name).and_then(|spec| spec.defaults.get(&name).cloned()),
            _ => None,
        };
        if let Some(value) = value {
            editor.document.set_modifier_value(node_id, index, &name, value);
            editor.modifier_changed();
        }
    }

    for (name, edit) in edits {
        match edit {
            Edit::None => {}
            Edit::Set(value) => {
                editor.document.set_value(node_id, &name, value);
                editor.parameter_changed(node_id);
            }
            Edit::SetValues { values, label } => {
                if editor.document.set_values(node_id, &values, label) {
                    editor.parameter_changed(node_id);
                }
            }
            Edit::Reset => {
                // `EditorMenu.ResetProperty`: the type's default value.
                let default = spec.defaults.get(&name).cloned();
                if let Some(default) = default {
                    editor.document.set_value(node_id, &name, default);
                    editor.parameter_changed(node_id);
                }
            }
            Edit::Command => commands.push(Command::NodeAction(name)),
            Edit::BoundText(text) => {
                // `VariableEditor.EditablePopup_EditSuccess`: the text must
                // make a value of the parameter's type.
                let bindings = &editor.document.automation().bindings;
                let binding = bindings.iter().find(|binding| binding.node == node_id && binding.property == name);
                let key = binding.map(|binding| binding.variable.clone());
                let converts = spec.parameter(&name).is_some_and(|parameter| bound_variable::converts(parameter, &text));
                match key {
                    Some(key) if converts => {
                        editor.document.set_variable_text(&key, &text);
                        editor.changed();
                    }
                    Some(_) => {
                        let caption = "Invalid value".to_string();
                        let text = "The value was not valid for this property.".to_string();
                        commands.push(Command::ShowMessage { caption, text });
                    }
                    None => {}
                }
            }
            Edit::ShowVariables => commands.push(Command::ShowDataTab(crate::ui::data_view::DataTab::Vars)),
            Edit::ShowDocs(target) => commands.push(Command::Tools(ToolCommand::ShowDocs(target.to_string()))),
            Edit::AddInput => {
                if editor.document.add_input(node_id) {
                    editor.changed();
                }
            }
            Edit::BindNew => {
                editor.document.bind_new_variable(node_id, &name);
                editor.changed();
            }
            Edit::BindTo(variable) => {
                editor.document.bind_variable(node_id, &name, &variable);
                editor.changed();
            }
            Edit::AskUnbind => {
                ui.data_mut(|data| data.get_temp_mut_or_default::<ParameterDialogs>(dialogs_id()).unbind = Some((node_id, name.clone())));
            }
            Edit::Expression => commands.push(Command::EditExpression { node: node_id, property: name.clone() }),
            Edit::NewSeed => {
                // `XorShift64Star.Next(0, 65536)`.
                let seed = editor.document.random_seed_below(65536);
                editor.document.set_value(node_id, &name, Value::from(seed));
                editor.parameter_changed(node_id);
                if let Some(id) = seed_boxes.get(&name) {
                    editors::remember_drawn_seed(ui.ctx(), *id, seed as i64);
                }
            }
            Edit::SeedText(text) => {
                // `EditablePopup_EditSuccess`: the text as a formula, then
                // `Convert.ToInt32`.
                let terrain = editor.document.terrain();
                let context = nerthus_engine::automation::ExpressionContext {
                    resolution: nerthus_engine::Resolution::square(editor.preview_resolution),
                    terrain_height: terrain.height,
                };
                let typed = editor.document.automation().calculate_number(&text, &context);
                if let Some(seed) = typed.and_then(convert_to_int32) {
                    editor.document.set_value(node_id, &name, Value::from(seed));
                    editor.parameter_changed(node_id);
                }
            }
            Edit::Browse => {
                let folder = spec.parameter(&name).is_some_and(|parameter| parameter.kind == "Folder");
                let extensions = file_extensions(&spec.name);
                let current = values.get(&name).and_then(Value::as_str).unwrap_or_default().to_string();
                editor.dialog = Some(DialogRequest::BrowseFile { node: node_id, parameter: name, extensions, folder, current });
            }
        }
    }
}

fn launch_right_click_id() -> egui::Id {
    egui::Id::new("right-click-at-launch")
}

/// `--show slider-popup:<property>:<along>`: the shown node's slider for
/// the property is right-clicked that share along its width (0.5 if not
/// given) when it next draws.
pub fn right_click_at_launch(ctx: &egui::Context, request: &str) {
    let (property, along) = request.split_once(':').unwrap_or((request, "0.5"));
    let along: f32 = along.parse().unwrap_or(0.5);
    ctx.data_mut(|data| data.insert_temp(launch_right_click_id(), (property.to_string(), along)));
}

/// Hands a launch right-click to the slider `id` when it is for `property`.
fn take_launch_right_click(ctx: &egui::Context, property: &str, id: egui::Id) {
    let request: Option<(String, f32)> = ctx.data(|data| data.get_temp(launch_right_click_id()));
    if let Some((wanted, along)) = request
        && wanted.eq_ignore_ascii_case(property)
    {
        ctx.data_mut(|data| data.remove::<(String, f32)>(launch_right_click_id()));
        crate::ui::slider_popups::request_right_click(ctx, id, along);
    }
}

/// `CreateBakeNodeUI`: a Gate shows a "Gate Node" panel and nothing else:
/// Bake Node, or Unbake Node once it is baked.
fn gate_panel(ui: &mut Ui, node_id: i32, state: StateOverride, commands: &mut Vec<Command>) {
    group_panel(ui, (node_id, GATE_PANEL_KEY), "Gate Node", true, |ui| match state {
        StateOverride::None => {
            if panel_button(ui, "Bake Node") {
                commands.push(Command::BakeSelected);
            }
        }
        StateOverride::Baked => {
            if panel_button(ui, "Unbake Node") {
                commands.push(Command::UnbakeNode(node_id));
            }
        }
        StateOverride::Bypassed => {}
    });
}

/// `CreateBakedUI`: a baked node shows a "Baked Node" panel in place of
/// its parameters and modifiers, with Load Offloaded Cache.
fn baked_panel(ui: &mut Ui, node_id: i32, commands: &mut Vec<Command>) {
    group_panel(ui, (node_id, BAKED_PANEL_KEY), "Baked Node", true, |ui| {
        if panel_button(ui, "Load Offloaded Cache") {
            commands.push(Command::LoadOffloadedCache(node_id));
        }
    });
}

/// A panel's `Button` (padding 8, margin 16, 8, 16, 8), as wide as the
/// panel: `Button-Background` (darker under the pointer and while
/// pressed) in a 1px `Button-Border`, corners of 2.
fn panel_button(ui: &mut Ui, text: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let height = galley.size().y + 2.0 * PANEL_BUTTON_PADDING + 2.0 * PANEL_BUTTON_MARGIN.y;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let button = row.shrink2(PANEL_BUTTON_MARGIN);
    let response = ui.interact(button, ui.id().with(("panel-button", text)), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(button, 2.0, fill);
    ui.painter().rect_stroke(button, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    let text_position = button.center() - galley.size() / 2.0;
    ui.painter().galley(text_position, galley, theme::LABEL_DEFAULT);
    response.clicked()
}

/// `Convert.ToInt32(float)`: the nearest integer, halves to even; `None`
/// where .NET throws (out of range, NaN).
fn convert_to_int32(value: f32) -> Option<i32> {
    let value = value as f64;
    let in_range = (-2147483648.5..2147483647.5).contains(&value);
    if !in_range {
        return None;
    }
    Some(value.round_ties_even() as i32)
}

/// An editor row (`DockPanel`, margin 4): the editor, then its parameter
/// menu docked right and centred down the row. The tooltip is `help`.
fn editor_row(ui: &mut Ui, enabled: bool, help: Option<&str>, bind_type: BindType, automated: &Automated, control: impl FnOnce(&mut Ui) -> Edit) -> Edit {
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin::same(ROW_MARGIN);
    let row = frame.show(ui, |ui| {
        let control_width = (ui.available_width() - MENU_BUTTON_WIDTH).max(40.0);
        let top_left = ui.cursor().min;
        let control_rect = Rect::from_min_size(top_left, Vec2::new(control_width, 0.0));
        let builder = UiBuilder::new().max_rect(Rect::from_min_size(top_left, Vec2::new(control_width, f32::INFINITY))).layout(Layout::top_down(Align::Min));
        let mut control_ui = ui.new_child(builder);
        control_ui.spacing_mut().item_spacing = Vec2::ZERO;
        control_ui.set_width(control_width);
        let control_edit = control_ui.add_enabled_ui(enabled, control).inner;
        let used = control_ui.min_rect();
        let height = used.height().max(1.0);
        let menu_rect = Rect::from_min_size(Pos2::new(control_rect.right(), top_left.y), Vec2::new(MENU_BUTTON_WIDTH, height));
        let menu_edit = parameter_menu(ui, menu_rect, enabled, bind_type, automated);
        ui.allocate_rect(Rect::from_min_size(top_left, Vec2::new(control_width + MENU_BUTTON_WIDTH, height)), Sense::hover());
        menu_edit.unwrap_or(control_edit)
    });
    if let Some(help) = help {
        row.response.tooltip(help);
    }
    row.inner
}

/// Parameters with a `[CustomEditor]`: the SatelliteMap library list, the curve
/// editor and Mixer's layers; `None` for the rest. Edits name the
/// property they change.
#[allow(clippy::too_many_arguments)]
fn custom_editor(ui: &mut Ui, id: egui::Id, parameter: &ParameterSpec, value: &Value, node: &nerthus_engine::NodeData, nodes: &indexmap::IndexMap<i32, nerthus_engine::NodeData>, watched: &std::collections::HashMap<i32, crate::worker::WatchedNode>, document_serial: u64) -> Option<Vec<(String, Edit)>> {
    let name = parameter.name.clone();
    if parameter.kind == "GradientEditor" {
        crate::ui::gradient_editor::track_property(ui.ctx(), id, document_serial, node.id, &name);
        let edit = crate::ui::gradient_editor::gradient_editor(ui, id, value);
        return Some(vec![(name, edit)]);
    }
    // `Dictionary<string, Variable>` (Macro) gets the variable editors.
    let editor = if parameter.value_type == nerthus_engine::ValueType::Dictionary { Some("VariableEditorControl") } else { parameter.editor.as_deref() };
    match editor? {
        "SatelliteMapListBox" => {
            let library = node.values.get("Library").and_then(Value::as_str).unwrap_or("Rock");
            let current = value.as_i64().unwrap_or(-1);
            let chosen = editors::library_list(ui, id, library, current);
            let edit = chosen.map_or(Edit::None, |index| Edit::Set(Value::from(index)));
            Some(vec![(name, edit)])
        }
        "LayerEditorPanel" => Some(crate::ui::mixer_layers::show(ui, id, node, nodes, &node.values, watched)),
        "VariableEditorControl" => {
            let changed = editors::macro_variables(ui, id, value, theme::ACCENT_PRIMARY);
            let edit = changed.map_or(Edit::None, Edit::Set);
            Some(vec![(name, edit)])
        }
        "CurveEditor" => {
            let points: Vec<[f32; 2]> = value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| {
                            let x = item.get("X").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                            let y = item.get("Y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                            [x, y]
                        })
                        .collect()
                })
                .unwrap_or_default();
            let Some(edited) = editors::curve_editor(ui, id, &points) else {
                return Some(Vec::new());
            };
            let list: Vec<Value> = edited.iter().map(|[x, y]| serde_json::json!({"X": f32_value(*x), "Y": f32_value(*y)})).collect();
            Some(vec![(name, Edit::Set(Value::Array(list)))])
        }
        _ => None,
    }
}

/// How a parameter is automated (`EditorMenu.BindType`).
#[derive(Clone, Copy, PartialEq)]
enum BindType {
    None,
    Variable,
    Expression,
    Both,
}

fn bind_type(automation: &nerthus_engine::automation::Automation, node: i32, property: &str) -> BindType {
    let bound = automation.bindings.iter().any(|binding| binding.node == node && binding.property == property);
    let key = format!("{node}_{property}");
    let expression = automation.expressions.get(&key).is_some_and(|text| !text.trim().is_empty());
    match (bound, expression) {
        (true, true) => BindType::Both,
        (true, false) => BindType::Variable,
        (false, true) => BindType::Expression,
        (false, false) => BindType::None,
    }
}

/// What the parameter menu needs to know about the property's automation.
struct Automated {
    bound_to: Option<String>,
    /// The project's variables (decorations left out) and their types.
    variables: Vec<(String, VarType)>,
    has_expression: bool,
}

impl Automated {
    fn of(automation: &nerthus_engine::automation::Automation, node: i32, property: &str) -> Self {
        let bound_to = automation.bindings.iter().find(|binding| binding.node == node && binding.property == property).map(|binding| binding.variable.clone());
        let variables = automation.variables.iter().filter(|(_, variable)| variable.var_type != VarType::Decoration).map(|(name, variable)| (name.clone(), variable.var_type)).collect();
        let key = format!("{node}_{property}");
        let has_expression = automation.expressions.get(&key).is_some_and(|text| !text.trim().is_empty());
        Self { bound_to, variables, has_expression }
    }
}

/// `EditorMenu`: a 12px keyframe (4 from the editor), filled and coloured
/// when the parameter is bound, at 0.75 under the pointer and 0.5 when
/// disabled, with the parameter's menu (`Ctx_Opened`): Bind to (a new
/// variable, any variable, Unbind), Reset (not while bound), Expression.
fn parameter_menu(ui: &mut Ui, rect: Rect, enabled: bool, bind_type: BindType, automated: &Automated) -> Option<Edit> {
    let response = ui.interact(rect, ui.id().with(("parameter-menu", rect.min.x as i32, rect.min.y as i32)), Sense::click());
    let base = match bind_type {
        BindType::None => theme::ICON_DEFAULT,
        BindType::Variable => theme::ACCENT_MUTED_PURPLE,
        BindType::Expression => theme::ACCENT_MUTED_TEAL,
        BindType::Both => theme::ACCENT_MUTED_ORANGE,
    };
    let opacity = if !enabled {
        0.5
    } else if response.hovered() {
        0.75
    } else {
        1.0
    };
    let color = base.gamma_multiply(opacity);
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 4.0 + 6.0, rect.center().y), Vec2::splat(12.0));
    if bind_type == BindType::None {
        icons::paint(ui, icon_rect, "keyframe", color);
    } else {
        icons::paint_filled(ui, icon_rect, "keyframe", color);
    }
    let bound = automated.bound_to.is_some();
    // A bound property takes expressions only through a number variable.
    let bound_type = automated.bound_to.as_ref().and_then(|name| automated.variables.iter().find(|(candidate, _)| candidate == name)).map(|(_, kind)| *kind);
    let expression_allowed = bound_type.is_none_or(|kind| matches!(kind, VarType::Float | VarType::Int));
    let mut edit = None;
    egui::Popup::menu(&response).show(|ui| {
        crate::ui::widgets::menu_style(ui);
        menu_caption(ui, "EDIT PARAMETER");
        submenu(ui, None, None, "Bind to", |ui| {
            crate::ui::widgets::menu_style(ui);
            if menu_item(ui, Some("variable-plus"), Some(theme::ACCENT_GREEN), "New variable", "").clicked() {
                edit = Some(Edit::BindNew);
            }
            if !automated.variables.is_empty() {
                menu_separator(ui);
            }
            for (name, _) in &automated.variables {
                let checked = automated.bound_to.as_deref() == Some(name.as_str());
                if crate::ui::widgets::check_item(ui, checked, name, "").clicked() {
                    edit = Some(Edit::BindTo(name.clone()));
                }
            }
            if bound {
                menu_separator(ui);
                if menu_item(ui, Some("xmark-circle"), Some(theme::ACCENT_RED), "Unbind", "").clicked() {
                    edit = Some(Edit::AskUnbind);
                }
            }
        });
        if menu_item_enabled(ui, None, None, "Reset", "", !bound).clicked() {
            edit = Some(Edit::Reset);
        }
        menu_separator(ui);
        if menu_item_enabled(ui, if automated.has_expression { Some("check") } else { None }, Some(theme::ACCENT_MUTED_TEAL), "Expression...", "", expression_allowed).clicked() {
            edit = Some(Edit::Expression);
        }
        menu_separator(ui);
        if menu_item(ui, None, None, "Help with Automation", "").clicked() {
            edit = Some(Edit::ShowDocs(crate::docs::PROPERTY_AUTOMATION));
        }
    });
    edit
}

/// Dialogs the parameter menu opens, kept between frames.
#[derive(Clone, Default)]
struct ParameterDialogs {
    /// `Mx2_Click`: unbind only, or unbind and delete the variable.
    unbind: Option<(i32, String)>,
}

fn dialogs_id() -> egui::Id {
    egui::Id::new("parameter-dialogs")
}

/// Shows the open parameter dialogs and applies their answers.
fn parameter_dialogs(ui: &mut Ui, editor: &mut Editor) {
    let mut dialogs: ParameterDialogs = ui.data(|data| data.get_temp(dialogs_id())).unwrap_or_default();
    if let Some((node, property)) = dialogs.unbind.clone() {
        let heading = format!("Unbind {property} and delete variable?");
        let modal = TaskDialog::new(ui.ctx(), egui::Id::new("unbind-variable"), 380.0);
        let response = modal.show(ui.ctx(), |ui, task| {
            task.body(ui, &heading, false, |ui| {
                ui.add(egui::Label::new(format!("You are about to unbind {property}. You can choose to keep the underlying variable it was bound to, or delete it.")).wrap());
                if task_link(ui, "Unbind only", "Keep the underlying variable.").clicked() {
                    editor.document.unbind(node, &property, false);
                    editor.changed();
                    dialogs.unbind = None;
                }
                if task_link(ui, "Unbind and delete variable", "Remove the underlying variable from the project.").clicked() {
                    editor.document.unbind(node, &property, true);
                    editor.changed();
                    dialogs.unbind = None;
                }
            });
            task.buttons(ui, |ui| {
                if task_button(ui, "Cancel").clicked() {
                    dialogs.unbind = None;
                }
            });
            task.footer(ui, |ui| {
                let warning = "If you delete the variable, it will unbind any other properties it was bound to.";
                ui.add(egui::Label::new(RichText::new(warning).color(theme::ACCENT_YELLOW)).wrap());
            });
        });
        if response.should_close() {
            dialogs.unbind = None;
        }
    }
    ui.data_mut(|data| data.insert_temp(dialogs_id(), dialogs));
}

/// `Separator` (`Separator.Margin`: 16 across, 8 or 2 down).
fn separator(ui: &mut Ui) {
    let gap = theme::sizes(ui.ctx()).separator_margin;
    ui.add_space(gap);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    let line = [Pos2::new(rect.left() + 16.0, rect.center().y), Pos2::new(rect.right() - 16.0, rect.center().y)];
    ui.painter().line_segment(line, Stroke::new(1.0, theme::MENU_SEPARATOR));
    ui.add_space(gap);
}

/// A group panel (`Expander`: `#303030`, margin `Expander.Margin`, padding
/// `Expander.Padding`): a header `Expander.Toggle.MinHeight` high
/// (`ToggleButton-Expander-DownHeader`: the chevron in a 26 column at
/// `Expander.Toggle.IconMargin`, the title in 12px semibold 42 in), then
/// the editors (margin 0, 0, 4, `Editor.BottomMargin`) while open.
pub(crate) fn group_panel(ui: &mut Ui, key: (i32, usize), title: &str, expanded: bool, contents: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(("group", key));
    let open: bool = ui.data(|data| data.get_temp(id)).unwrap_or(expanded);
    let sizes = theme::sizes(ui.ctx());
    let [margin_left, margin_top, margin_right, margin_bottom] = sizes.expander_margin;
    let mut frame = egui::Frame::new();
    frame.fill = theme::EXPANDER_BACKGROUND;
    frame.outer_margin = egui::Margin { left: margin_left as i8, right: margin_right as i8, top: margin_top as i8, bottom: margin_bottom as i8 };
    frame.inner_margin = egui::Margin::same(sizes.expander_padding as i8);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let header_height = sizes.expander_header;
        let (header, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), header_height), Sense::click());
        let chevron = if open { "chevron-down" } else { "chevron-right" };
        // Centred in what its margin leaves of the column, or against the
        // margin where it does not fit.
        let [icon_left, icon_top] = sizes.expander_icon_margin;
        let room = GROUP_CHEVRON_COLUMN - icon_left;
        let chevron_left = header.left() + icon_left + ((room - 16.0) / 2.0).max(0.0);
        let chevron_top = header.top() + icon_top + (header_height - icon_top - 16.0) / 2.0;
        let chevron_rect = Rect::from_min_size(Pos2::new(chevron_left, chevron_top), Vec2::splat(16.0));
        icons::paint(ui, chevron_rect, chevron, theme::EXPANDER_HEADER);
        let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::EXPANDER_HEADER);
        let title_position = Pos2::new(header.left() + GROUP_CHEVRON_COLUMN + 16.0, header.center().y - galley.size().y / 2.0);
        ui.painter().galley(title_position, galley, theme::EXPANDER_HEADER);
        if response.clicked() {
            ui.data_mut(|data| data.insert_temp(id, !open));
        }
        if open {
            let mut content = egui::Frame::new();
            content.inner_margin = egui::Margin { left: 0, right: 4, top: 0, bottom: sizes.editor_bottom as i8 };
            content.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                contents(ui);
            });
        }
    });
}

/// Files a file parameter offers (`FileBrowser`): the mesh formats for
/// Object nodes, images for the rest.
fn file_extensions(type_name: &str) -> Vec<String> {
    const MESHES: [&str; 7] = ["obj", "fbx", "dae", "gltf", "glb", "ply", "xyz"];
    const IMAGES: [&str; 21] = [
        "exr", "graw", "jpg", "png", "r16", "r32", "raw", "tif", "jpeg", "webp", "tiff", "svg", "rgb", "miff", "jp2", "hdr", "eps", "emf", "dds", "dng", "bmp",
    ];
    let extensions: &[&str] = if type_name == "Object" { &MESHES } else { &IMAGES };
    extensions.iter().map(|extension| extension.to_string()).collect()
}

/// Widths of the four groups: the add menu, then the toggles with their
/// margins.
fn modifier_group_widths() -> [f32; 4] {
    let toggles = |group: &[ModifierToggle]| group.iter().map(|toggle| toggle.gap + MODIFIER_BUTTON).sum::<f32>();
    let inner = [ADD_BUTTON_WIDTH, toggles(MODIFIER_TOGGLES[0]), toggles(MODIFIER_TOGGLES[1]), toggles(MODIFIER_TOGGLES[2])];
    inner.map(|width| width + 2.0 * GROUP_PADDING + 2.0)
}

/// `ModifierStack` laid out: the groups, the node-state menu and the part
/// of the stack its `Border` has, and the stack's height.
struct StackLayout {
    groups: Vec<Rect>,
    state_menu: Rect,
    state_border: Rect,
    height: f32,
}

/// `ModifierStack` from `area`'s top left across its width, as its
/// `DockPanel` (`LastChildFill="False"`) lays it out: the groups'
/// `WrapPanel` (margin 4) docked left and measured in the whole width, as
/// wide as its widest row; the node-state menu's `Border` docked right in
/// what that leaves, cut off there when that is less than it needs, the
/// menu (`button` square, its item spilling `menu_margin` past the menu)
/// right in it and centred down the stack.
fn stack_layout(area: Rect, button: f32, menu_margin: f32) -> StackLayout {
    let stack_left = area.left() + STACK_MARGIN_LEFT;
    let wrap_left = stack_left + WRAP_MARGIN;
    let wrap_right = area.right() - WRAP_MARGIN;
    let group_height = MODIFIER_BUTTON + 2.0 * GROUP_PADDING + 2.0;
    let row_height = group_height + 2.0 * GROUP_MARGIN;
    let mut x = wrap_left;
    let mut y = area.top() + WRAP_MARGIN;
    let mut widest: f32 = 0.0;
    let mut groups = Vec::new();
    for width in modifier_group_widths() {
        let outer = width + 2.0 * GROUP_MARGIN;
        if x > wrap_left && x + outer > wrap_right {
            x = wrap_left;
            y += row_height;
        }
        groups.push(Rect::from_min_size(Pos2::new(x + GROUP_MARGIN, y + GROUP_MARGIN), Vec2::new(width, group_height)));
        x += outer;
        widest = widest.max(x - wrap_left);
    }
    let wrap_height = y + row_height + WRAP_MARGIN - area.top();
    let wrap_width = WRAP_MARGIN + widest + WRAP_MARGIN;
    let [margin_left, margin_top, margin_right, margin_bottom] = STATE_BORDER_MARGIN;
    let height = wrap_height.max(margin_top + button + margin_bottom);
    let border_wanted = margin_left + button + 2.0 * menu_margin + margin_right;
    let border_width = (area.right() - stack_left - wrap_width).clamp(0.0, border_wanted);
    let state_border = Rect::from_min_max(Pos2::new(area.right() - border_width, area.top()), Pos2::new(area.right(), area.top() + height));
    let menu_right = state_border.right() - margin_right - menu_margin;
    let menu_center = area.top() + margin_top + (height - margin_top - margin_bottom) / 2.0;
    let state_menu = Rect::from_min_size(Pos2::new(menu_right - button, menu_center - button / 2.0), Vec2::splat(button));
    StackLayout { groups, state_menu, state_border, height }
}

/// `ModifierToggleButton-IconX`: 24 square, a 14px icon; on, a pressed
/// fill and the accent's outline; under the pointer, the hover fill.
fn modifier_toggle_button(ui: &mut Ui, toggle: &ModifierToggle, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(MODIFIER_BUTTON), Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, toggle.accent), StrokeKind::Inside);
    }
    let icon_color = if (hovered || on) && !pressed { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(14.0));
    icons::paint(ui, icon_rect, toggle.icon, icon_color);
    response.tooltip(toggle.tooltip)
}

/// The modifier stack (`ModifierStack`): the add menu and toggles for the
/// common modifiers in their bordered groups, the node-state menu right.
fn modifier_toolbar(ui: &mut Ui, rect: Rect, editor: &Editor, node_id: i32, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    let modifiers = editor.document.node(node_id).map(|node| node.modifiers.as_slice()).unwrap_or_default();
    let present: Vec<&str> = modifiers.iter().map(nerthus_engine::editing::modifier_name).collect();
    let sizes = theme::sizes(ui.ctx());
    let stack = stack_layout(rect, sizes.toolbar_button, sizes.toolbar_menu_margin);
    let groups = &stack.groups;
    for group in groups {
        toolbar_background(ui, *group, theme::PANEL_BORDER, 1.0);
    }

    // `btnAddModifier`: teal while the node has modifiers; its entries are
    // checked by the modifiers present.
    let add_rect = Rect::from_min_size(groups[0].min + Vec2::splat(1.0 + GROUP_PADDING), Vec2::new(ADD_BUTTON_WIDTH, MODIFIER_BUTTON));
    let mut add_ui = ui.new_child(UiBuilder::new().max_rect(add_rect));
    let add_tint = if modifiers.is_empty() { None } else { Some(theme::ACCENT_TEAL) };
    let is_color = crate::ui::graph_commands::is_color(editor, node_id);
    let add_response = icon_menu_sized(&mut add_ui, "square-rounded-plus-2", "Add Modifiers", add_tint, add_rect.size(), |ui| {
        let groups: [&[(&str, &str, &'static str)]; 6] = [
            &[("x-influence", "Influence", "Influence"), ("x-difference", "Difference", "Difference")],
            &[("x-slope", "Mask by Slope", "Slope"), ("x-height", "Mask by Height", "Height")],
            &[("x-clamp", "Clamp", "Clamp"), ("x-clip", "Clip", "Clip")],
            &[("x-equalize", "Equalize", "Equalize"), ("", "Strong", "Strong")],
            &[("x-blur", "Blur", "Blur"), ("x-warp", "Warp", "Warp"), ("x-threshold", "Threshold", "Threshold")],
            &[("x-match", "Match Height", "MatchHeight")],
        ];
        for (index, entries) in groups.iter().enumerate() {
            if index > 0 {
                menu_separator(ui);
            }
            for (icon, text, type_name) in entries.iter().copied() {
                let checked = present.contains(&type_name);
                let shown_icon = if checked { Some("check") } else if icon.is_empty() { None } else { Some(icon) };
                let accent = if checked { theme::ACCENT_TEAL } else { theme::ACCENT_MUTED_GRAY };
                // `PropertyArea.UpdateModifiers`: these four require a
                // heightfield, including when a saved modifier is checked.
                let enabled = !is_color || !matches!(type_name, "Clip" | "MatchHeight" | "Height" | "Slope");
                if menu_item_enabled(ui, shown_icon, Some(accent), text, "", enabled).clicked() {
                    commands.push(Command::ToggleModifier(type_name));
                }
            }
        }
    });
    // M (`btnAddModifier.IsSubmenuOpen`): the menu opens as if clicked.
    if crate::ui::tool_commands::add_modifiers_wanted(ui.ctx()) {
        let popup_id = egui::Popup::default_response_id(&add_response);
        egui::Popup::open_id(ui.ctx(), popup_id);
    }

    for (group, toggles) in groups[1..].iter().zip(MODIFIER_TOGGLES) {
        let inner = group.shrink(1.0 + GROUP_PADDING);
        let builder = UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center));
        let mut group_ui = ui.new_child(builder);
        group_ui.spacing_mut().item_spacing.x = 0.0;
        for toggle in toggles {
            group_ui.add_space(toggle.gap);
            let on = present.contains(&toggle.type_name);
            if modifier_toggle_button(&mut group_ui, toggle, on).clicked() {
                commands.push(Command::ToggleModifier(toggle.type_name));
            }
        }
    }

    // The node-state menu, cut off with its `Border` where the groups
    // leave it too little.
    if stack.state_border.width() <= 0.0 {
        return;
    }
    let saved_state = node_states::has_saved_state(ui);
    let mut state_ui = ui.new_child(UiBuilder::new().max_rect(stack.state_menu));
    state_ui.shrink_clip_rect(stack.state_border);
    icon_menu(&mut state_ui, "arrow-back-up", "Node State", None, |ui| {
        state_items(ui, true, saved_state, commands, actions);
    });
}

/// The node-state items both menus have: Revert Changes, Reset to
/// Defaults, and Save State (`mnuState1`, `mnuState2`), which reads Load
/// State with an upward arrow while a state is saved.
fn state_items(ui: &mut Ui, has_node: bool, saved_state: bool, commands: &mut Vec<Command>, actions: &mut Vec<StateAction>) {
    if menu_item(ui, Some("arrow-back-up-double"), None, "Revert Changes", "").clicked() && has_node {
        actions.push(StateAction::Revert);
    }
    if menu_item(ui, Some("restore"), None, "Reset to Defaults", "").clicked() && has_node {
        commands.push(Command::ResetNode);
    }
    let (state_icon, state_text) = if saved_state { ("arrow-bar-to-up", "Load State") } else { ("arrow-bar-to-down", "Save State") };
    if menu_item(ui, Some(state_icon), Some(theme::ACCENT_MUTED_PURPLE), state_text, "").clicked() && has_node {
        actions.push(StateAction::ToggleSavedState);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::ui::commands::run;
    use crate::ui::UiState;
    use bevy::prelude::Window;
    use serde_json::json;

    fn modifier_menu_frame(context: &egui::Context, editor: &Editor, node: i32, events: Vec<egui::Event>) -> (Vec<Command>, Vec<egui::epaint::ClippedShape>) {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 800.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let toolbar = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(700.0, 100.0));
        let mut commands = Vec::new();
        let mut actions = Vec::new();
        let output = context.run_ui(input, |ui| {
            modifier_toolbar(ui, toolbar, editor, node, &mut commands, &mut actions);
        });
        let shapes = output.shapes.clone();
        output.drop_without_applying_deltas();
        (commands, shapes)
    }

    /// Opens the actual toolbar menu and clicks the entry's painted text.
    fn click_modifier_entry(editor: &Editor, node: i32, entry: &str) -> Vec<Command> {
        let context = egui::Context::default();
        theme::apply(&context);
        modifier_menu_frame(&context, editor, node, Vec::new());
        let toolbar = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(700.0, 100.0));
        let sizes = theme::sizes(&context);
        let stack = stack_layout(toolbar, sizes.toolbar_button, sizes.toolbar_menu_margin);
        let add_origin = stack.groups[0].min + Vec2::splat(1.0 + GROUP_PADDING);
        let add_center = add_origin + Vec2::new(ADD_BUTTON_WIDTH, MODIFIER_BUTTON) / 2.0;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        modifier_menu_frame(&context, editor, node, vec![egui::Event::PointerMoved(add_center)]);
        modifier_menu_frame(&context, editor, node, vec![button(add_center, true)]);
        modifier_menu_frame(&context, editor, node, vec![button(add_center, false)]);
        let (_, shapes) = modifier_menu_frame(&context, editor, node, Vec::new());
        let entries = texts(&shapes);
        let (_, rect) = entries.iter().find(|(text, _)| text == entry).expect("modifier entry is visible");
        let position = rect.center();
        modifier_menu_frame(&context, editor, node, vec![egui::Event::PointerMoved(position)]);
        modifier_menu_frame(&context, editor, node, vec![button(position, true)]);
        modifier_menu_frame(&context, editor, node, vec![button(position, false)]).0
    }

    #[test]
    fn colour_nodes_disable_the_four_heightfield_modifier_entries() {
        for node_type in ["CLUTer", "Combine"] {
            let mut document = Document::new();
            let node = document.add_node(node_type, [26000.0, 26000.0]).unwrap();
            // Checked modifiers are disabled too, as Gaea leaves them.
            document.toggle_modifier(node, "Clip");
            let mut editor = Editor::new(document);
            if node_type == "Combine" {
                let mut cache = crate::worker::NodeCache::default();
                cache.port_intents.insert("Out".to_string(), nerthus_engine::RenderIntent::Color);
                editor.node_caches.insert(node, cache);
            }
            for entry in ["Clip", "Match Height", "Mask by Height", "Mask by Slope"] {
                let commands = click_modifier_entry(&editor, node, entry);
                assert!(commands.is_empty(), "{node_type}: {entry} must be disabled");
            }
            let commands = click_modifier_entry(&editor, node, "Blur");
            assert!(matches!(commands.as_slice(), [Command::ToggleModifier("Blur")]));
        }
    }

    #[test]
    fn heightfield_nodes_keep_the_four_modifier_entries_enabled() {
        let mut document = Document::new();
        let node = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        let editor = Editor::new(document);
        let entries = [("Clip", "Clip"), ("Match Height", "MatchHeight"), ("Mask by Height", "Height"), ("Mask by Slope", "Slope")];
        for (entry, expected) in entries {
            let commands = click_modifier_entry(&editor, node, entry);
            assert!(matches!(commands.as_slice(), [Command::ToggleModifier(name)] if *name == expected), "{entry} must stay enabled");
        }
    }

    #[test]
    fn a_modifier_close_button_asks_unless_shift_is_held() {
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_modifier(perlin, "Blur");
        document.toggle_modifier(perlin, "Invert");
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::RemoveModifier { node: perlin, place: 0, ask: true }], &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("it asks first");
        assert_eq!(question.caption, "Delete Modifier?");
        assert_eq!(question.text, "Are you sure you want to delete this modifier?");
        assert_eq!(editor.document.node(perlin).unwrap().modifiers.len(), 2);
        run(vec![question.yes.expect("Yes removes it")], &mut editor, &mut ui_state, &mut window);
        let left: Vec<String> = editor.document.node(perlin).unwrap().modifiers.iter().map(|modifier| modifier.type_name.clone()).collect();
        assert_eq!(left, ["Invert"]);
        // Shift held: straight away.
        run(vec![Command::RemoveModifier { node: perlin, place: 0, ask: false }], &mut editor, &mut ui_state, &mut window);
        assert!(ui_state.prompt.is_none());
        assert!(editor.document.node(perlin).unwrap().modifiers.is_empty());
    }

    #[test]
    fn modifier_boxes_keep_their_lists_own_margin() {
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_modifier(perlin, "Blur");
        let node = document.node(perlin).unwrap().clone();
        // The items host is 4 in from the list in both size themes, not
        // the group panels' `Expander.Margin`.
        for compact in [false, true] {
            let context = egui::Context::default();
            theme::apply(&context);
            theme::set_settings(&context, theme::InterfaceSettings { compact, ..Default::default() });
            context.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 600.0));
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut boxes = Vec::new();
            let output = context.run_ui(input, |ui| {
                let mut column = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 600.0))));
                column.spacing_mut().item_spacing = Vec2::ZERO;
                boxes = modifier_list(&mut column, &node, perlin, 0, 300.0).boxes;
            });
            output.drop_without_applying_deltas();
            assert_eq!(boxes.len(), 1);
            let sides = (boxes[0].left(), boxes[0].top(), boxes[0].right());
            assert_eq!(sides, (4.0, 4.0, 296.0), "compact: {compact}");
        }
    }

    /// Mountain, then a baked Gate, then a baked Blur.
    fn baked_chain() -> (Editor, i32, i32) {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let gate = document.add_node("Gate", [26300.0, 26000.0]).unwrap();
        let blur = document.add_node("Blur", [26600.0, 26000.0]).unwrap();
        document.connect(mountain, "Out", gate, "In");
        document.connect(gate, "Out", blur, "In");
        document.set_member(gate, "StateOverride", json!("Baked"), "Bake", true);
        document.set_member(blur, "StateOverride", json!("Baked"), "Bake", true);
        let mut editor = Editor::new(document);
        editor.select(Some(gate));
        (editor, gate, blur)
    }

    #[test]
    fn the_gate_panel_unbakes_after_asking_and_leaves_the_file_unsaved() {
        let (mut editor, gate, blur) = baked_chain();
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::UnbakeNode(gate)], &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("it asks first");
        assert_eq!(question.caption, "Unbake Node?");
        assert_eq!(question.text, "Are you sure you want to unbake this node?\n\nAny baked child nodes will also be unbaked.");
        assert_eq!(question.yes, Some(Command::UnbakeNodeConfirmed(gate)));
        run(vec![Command::UnbakeNodeConfirmed(gate)], &mut editor, &mut ui_state, &mut window);
        for node in [gate, blur] {
            let data = editor.document.node(node).unwrap();
            assert_eq!(StateOverride::of(data), StateOverride::None);
        }
        assert!(editor.document.dirty);
        assert!(editor.dialog.is_none(), "no save is asked for");
    }

    #[test]
    fn a_region_preview_keeps_a_gate_baked() {
        let (mut editor, gate, _) = baked_chain();
        editor.preview_region = Some("Region 1".to_string());
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::UnbakeNode(gate)], &mut editor, &mut ui_state, &mut window);
        let refusal = ui_state.prompt.take().expect("it refuses");
        assert_eq!(refusal.caption, "Unbaking Critical Node");
        assert_eq!(refusal.yes, None);
    }

    #[test]
    fn load_offloaded_cache_reads_the_node_and_its_baked_descendants() {
        let (mut editor, gate, blur) = baked_chain();
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        run(vec![Command::LoadOffloadedCache(gate)], &mut editor, &mut ui_state, &mut window);
        assert_eq!(editor.pending_load_bakes(), [gate, blur]);
    }

    #[test]
    fn the_state_menu_takes_what_the_groups_leave() {
        let (button, menu_margin) = (32.0, -5.0);
        for tenth in 1020..=8000 {
            let width = tenth as f32 / 10.0;
            let area = Rect::from_min_size(Pos2::new(37.5, 610.25), Vec2::new(width, 0.0));
            let stack = stack_layout(area, button, menu_margin);
            assert_eq!(stack.groups.len(), 4);
            let border = stack.state_border;
            assert_eq!(border.right(), area.right(), "width {width}");
            // A group wider than the panel runs past it, cut off there.
            let widest = modifier_group_widths().into_iter().fold(0.0, f32::max) + 2.0 * GROUP_MARGIN;
            if width < STACK_MARGIN_LEFT + 2.0 * WRAP_MARGIN + widest {
                continue;
            }
            for group in &stack.groups {
                // The `WrapPanel` ends where the `Border` starts.
                assert!(group.right() + GROUP_MARGIN + WRAP_MARGIN <= border.left() + 1e-3, "width {width}: {group:?} meets the menu's border {border:?}");
                assert!(group.bottom() + GROUP_MARGIN + WRAP_MARGIN <= area.top() + stack.height + 1e-3, "width {width}: the height leaves out {group:?}");
            }
            // With room, the menu 12 (less the item's spill of 5) in from
            // the right and centred down the stack; otherwise cut off.
            if border.width() >= 8.0 + 22.0 + 12.0 - 1e-3 {
                assert_eq!(stack.state_menu.right(), area.right() - 7.0, "width {width}");
            }
            let middle = area.top() + 8.0 + (stack.height - 16.0) / 2.0;
            assert!((stack.state_menu.center().y - middle).abs() < 1e-3, "width {width}");
        }
    }

    #[test]
    fn the_groups_wrap_in_the_whole_width() {
        let rows = |width: f32| {
            let stack = stack_layout(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 0.0)), 32.0, -5.0);
            let mut tops: Vec<f32> = stack.groups.iter().map(|group| group.top()).collect();
            tops.dedup();
            stack.groups.iter().map(|group| tops.iter().position(|top| *top == group.top()).unwrap()).collect::<Vec<_>>()
        };
        // The four groups (290 with their margins) in the wrap panel's
        // margins of 4, past the stack's 0.2: 298.2, whatever the menu has.
        assert_eq!(rows(298.2), [0, 0, 0, 0]);
        assert_eq!(rows(298.1), [0, 0, 0, 1]);
        assert_eq!(rows(256.0), [0, 0, 0, 1]);
        assert_eq!(rows(224.2), [0, 0, 0, 1]);
        assert_eq!(rows(224.1), [0, 0, 1, 1]);
        assert_eq!(rows(150.2), [0, 1, 2, 2]);
        assert_eq!(rows(150.1), [0, 1, 2, 3]);
        // At the least property width the groups leave the menu no room.
        let stack = stack_layout(Rect::from_min_size(Pos2::ZERO, Vec2::new(256.0, 0.0)), 32.0, -5.0);
        assert!(stack.state_border.width() < 42.0);
    }

    /// Runs `check` in a frame with the app's fonts.
    fn with_ui(check: impl FnMut(&mut Ui)) {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), check);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn notes_keep_their_message_beside_their_links() {
        with_ui(|ui| {
            for tenth in 1000..=8000 {
                let width = tenth as f32 / 10.0;
                for (name, layout) in [("port", port_note_layout(ui, width)), ("height", height_note_layout(ui, width).0)] {
                    let inner = Rect::from_min_size(Pos2::ZERO, layout.size).shrink(NOTE_INSET).expand(1e-3);
                    let (message, links) = (layout.message, layout.links);
                    assert!(inner.contains_rect(links), "{name} note {width} wide: its links {links:?} leave {inner:?}");
                    assert_eq!(links.right(), layout.size.x - NOTE_INSET, "{name} note {width} wide");
                    assert!((message.left() - (NOTE_INSET + NOTE_TEXT_PADDING.x)).abs() < 1e-3, "{name} note {width} wide: {message:?}");
                    // Wrapped in what the links leave, less its padding.
                    let rows_fit = layout.galley.rows.iter().all(|row| row.rect().width() <= links.left() - NOTE_TEXT_PADDING.x - message.left() + 1.0 || row.glyphs.len() <= 1 || row.text().split_whitespace().count() <= 1);
                    assert!(rows_fit, "{name} note {width} wide: its message runs under its links");
                }
            }
        });
    }

    #[test]
    fn notes_wrap_beside_their_links_at_the_least_property_width() {
        with_ui(|ui| {
            // The property area at its least, less the notes' 16 either
            // side.
            let width = crate::ui::MIN_PROPERTIES - 32.0;
            for (name, layout) in [("port", port_note_layout(ui, width)), ("height", height_note_layout(ui, width).0)] {
                assert!(layout.message.right() + NOTE_TEXT_PADDING.x <= layout.links.left() + 0.5, "{name} note: its message runs under its links");
                assert!(layout.galley.rows.len() <= 2, "{name} note: its message takes more than two rows");
            }
        });
    }

    /// A Combine node (a command row, the port note, a parameter group)
    /// in the property area `height` high at its least width, once `setup`
    /// has had the editor and the node, and after a frame for each of
    /// `events`: the area, and what it drew there.
    fn combine_shown_with(height: f32, setup: impl FnOnce(&mut Editor, i32), events: Vec<Vec<egui::Event>>) -> (Rect, Vec<egui::epaint::ClippedShape>) {
        let mut document = Document::new();
        let combine = document.add_node("Combine", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.select(Some(combine));
        setup(&mut editor, combine);
        let context = egui::Context::default();
        theme::apply(&context);
        // Icons as icons, not egui's ⚠ for an image it cannot load.
        egui_extras::install_image_loaders(&context);
        context.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let rect = Rect::from_min_size(Pos2::new(10.0, 50.0), Vec2::new(crate::ui::MIN_PROPERTIES - 4.0, height));
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1000.0));
        let mut shapes = Vec::new();
        // Laid out again once it has measured itself.
        let frames = std::iter::repeat_with(Vec::new).take(3).chain(events);
        for events in frames {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut commands = Vec::new();
            let mut output = context.run_ui(input, |ui| show(ui, rect, &mut editor, &mut commands));
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        (rect, shapes)
    }

    /// The modifier toolbar's groups drawn, each with the part of it that
    /// shows.
    fn modifier_groups(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Rect, Rect)> {
        let widths = modifier_group_widths();
        let height = MODIFIER_BUTTON + 2.0 * GROUP_PADDING + 2.0;
        let group = |clipped: &egui::epaint::ClippedShape| match &clipped.shape {
            egui::Shape::Rect(shape) if shape.fill == theme::PANEL_CLADDING => {
                let size = shape.rect.size();
                let is_group = (size.y - height).abs() < 0.01 && widths.iter().any(|width| (size.x - width).abs() < 0.01);
                is_group.then(|| (shape.rect, shape.rect.intersect(clipped.clip_rect)))
            }
            _ => None,
        };
        shapes.iter().filter_map(group).collect()
    }

    /// Fails where a modifier group shows over a text that shows.
    fn assert_no_text_under_the_modifier_groups(shapes: &[egui::epaint::ClippedShape]) {
        let groups = modifier_groups(shapes);
        assert_eq!(groups.len(), 4);
        for (text, visible) in texts(shapes) {
            for (_, group) in &groups {
                let both_show = visible.is_positive() && group.is_positive();
                assert!(!both_show || !visible.intersect(*group).is_positive(), "{text:?} shows at {visible:?}, under a modifier group at {group:?}");
            }
        }
    }

    /// The texts drawn, each with the part of it that shows.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Rect)> {
        let text = |clipped: &egui::epaint::ClippedShape| match &clipped.shape {
            egui::Shape::Text(text) => Some((text.galley.text().to_string(), text.visual_bounding_rect().intersect(clipped.clip_rect))),
            _ => None,
        };
        shapes.iter().filter_map(text).collect()
    }

    #[test]
    fn a_dropdown_name_wraps_and_the_row_tooltip_is_its_help() {
        let mut document = Document::new();
        let mesher = document.add_node("Mesher", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.select(Some(mesher));
        let context = egui::Context::default();
        theme::apply(&context);
        context.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        // At its least width the area shows the enum as a dropdown, its
        // name wrapped in the 100 column.
        let rect = Rect::from_min_size(Pos2::new(10.0, 50.0), Vec2::new(crate::ui::MIN_PROPERTIES - 4.0, 900.0));
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1000.0));
        let mut frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut commands = Vec::new();
            let mut output = context.run_ui(input, |ui| show(ui, rect, &mut editor, &mut commands));
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            texts(&shapes)
        };
        let name = "Artifact Reduction";
        let help = Catalogue::get().node("Mesher").and_then(|spec| spec.parameter("ArtifactReduction")).and_then(|parameter| parameter.description.clone()).unwrap();
        let mut shown = Vec::new();
        for _ in 0..3 {
            shown = frame(Vec::new());
        }
        let (_, label) = shown.iter().find(|(text, _)| text == name).expect("the name is drawn whole");
        frame(vec![egui::Event::PointerMoved(label.center())]);
        let shows = (0..120).any(|_| frame(Vec::new()).iter().any(|(text, _)| *text == help));
        assert!(shows, "no tooltip says {help:?}");
    }

    #[test]
    fn the_modifier_stack_keeps_to_the_foot_and_the_parameters_to_what_is_left() {
        let width = crate::ui::MIN_PROPERTIES - 4.0;
        let least = least_height(width);
        let error = "Input B is not connected.";
        let cases = [
            ("the least height", least, None),
            ("70 short", least - 70.0, None),
            ("a warning", least, Some(error)),
            ("the locked-preview alert", least - 30.0, Some(DESCENDANT_ALERT)),
        ];
        for (case, height, message) in cases {
            let setup = move |editor: &mut Editor, node: i32| match message {
                Some(text) if text == error => {
                    editor.statuses.insert(node, crate::editor::NodeStatus { error: Some(text.to_string()), ..Default::default() });
                }
                Some(_) => {
                    let mountain = editor.document.add_node("Mountain", [25000.0, 26000.0]).unwrap();
                    editor.preview_node = Some(mountain);
                    editor.preview_locked = true;
                }
                None => {}
            };
            let (rect, shapes) = combine_shown_with(height, setup, Vec::new());
            // The stack at the area's foot, nothing shown over it.
            assert_no_text_under_the_modifier_groups(&shapes);
            let stack_top = rect.bottom() - modifier_stack_height(rect.width(), &theme::DEFAULT_SIZES);
            for (group, _) in modifier_groups(&shapes) {
                assert!(group.top() >= stack_top - 0.5 && group.bottom() <= rect.bottom() + 0.5, "{case}: a group at {group:?}");
            }
            // The port note always shows, above the stack.
            let shown = texts(&shapes);
            let note = shown.iter().find(|(text, visible)| text == "This node can add additional input ports." && visible.is_positive());
            let note = note.unwrap_or_else(|| panic!("{case}: the port note does not show"));
            assert!(note.1.bottom() <= stack_top, "{case}: the port note at {:?}", note.1);
            for name in ["Swap Inputs"].into_iter().chain(message) {
                assert!(shown.iter().any(|(text, visible)| text == name && visible.is_positive()), "{case}: {name:?} does not show");
            }
        }
    }

    #[test]
    fn notes_are_as_high_as_their_message_or_links() {
        with_ui(|ui| {
            let port = port_note_layout(ui, 600.0);
            assert_eq!(port.galley.rows.len(), 1);
            let message = port.galley.size().y + 2.0 * NOTE_TEXT_PADDING.y;
            assert_eq!(port.size.y, message.max(PORT_NOTE_CLOSE) + 2.0 * NOTE_INSET);
            let (height, link) = height_note_layout(ui, 600.0);
            assert_eq!(height.galley.rows.len(), 1);
            let message = height.galley.size().y + 2.0 * NOTE_TEXT_PADDING.y;
            let link = link.size().y + 2.0 * HEIGHT_NOTE_LINK_PADDING.y;
            assert_eq!(height.size.y, message.max(link) + 2.0 * NOTE_INSET);
        });
    }

    #[test]
    fn groups_cycle_palettes_like_editor_base() {
        let catalogue = Catalogue::get();
        let perlin = catalogue.node("Perlin").expect("Perlin");
        let (groups, _) = parameter_groups(perlin);
        let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
        assert_eq!(titles, ["Noise", "Warp", "Transform"]);
        // Noise starts the ParamsX palette (index 1); Scale takes its first
        // colour, Octaves the second.
        assert_eq!(groups[0].entries[0].1, palette_color(1, 0));
        assert_eq!(groups[0].entries[1].1, palette_color(1, 1));
    }
}
