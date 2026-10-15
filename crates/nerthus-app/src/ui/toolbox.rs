//! The toolbox (`Toolbox`) over the graph's left side, in two styles:
//! menus (one button per toolbox category, `ToolboxMenuItem`, as
//! long icons, compact icon pairs or expanded with names, each opening the
//! category's families nested or as a flat list) or a panel of node
//! buttons in columns, by category or not. Its right-click menu holds the
//! settings; the most used nodes may come first and stand out. An entry
//! clicked adds its node after the selected one; one dragged onto the
//! graph lands there (onto a node, it follows it). The same menus add
//! nodes from the graph's context menu.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use egui::containers::menu::MenuConfig;
use egui::{Color32, CursorIcon, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, UiKind, UiStackInfo, Vec2};
use nerthus_engine::{Catalogue, NodeSpec, Registry};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::GraphState;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::widgets::{check_item, checked_icon_item, header_submenu, menu_caption, menu_item, menu_row_width, menu_rows_beside, menu_separator, menu_style, submenu, Tooltips};
use crate::user::{ToolboxMode, ToolboxStyle, UserOptions};

/// The toolbox categories, in toolbox order (`NodeCategory`).
pub const CATEGORIES: [&str; 10] = ["Primitive", "Terrain", "Modify", "Surface", "Simulate", "Derive", "Colorize", "Output", "Utility", "Macro"];

/// Nodes that hand terrain to other programs, which Nerthus leaves out.
const LEFT_OUT: [&str; 3] = ["HoudiniOut", "Unity", "Unreal"];

/// `ToolboxMenuItem`: a 28 icon column, the name with margins 0, 10, 5, 10.
const ICON_COLUMN: f32 = 28.0;
const TEXT_PADDING: f32 = 10.0;
/// `ToolboxButtonSize`, and the menus' width when compact or expanded
/// (`GetWidthForMode`).
const BUTTON_SIZE: f32 = 36.0;
const WIDE: f32 = BUTTON_SIZE * 2.3;
/// The menus' margin in their area (`ScrollViewer`).
const MENUS_MARGIN: f32 = 10.0;
/// `ToolboxButton-Hover` and `ToolboxButton-Pressed`.
const BUTTON_HOVER: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
const BUTTON_OPEN: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
/// The panel style's columns are about this wide (`ResizeToolbox`).
const PANEL_COLUMN: f32 = 80.0;
/// `GridSplitter` (10 wide) and how far a drag goes before it counts.
const SPLITTER: f32 = 10.0;
const SPLITTER_STEP: f32 = 10.0;

/// What the toolbox keeps between frames: the panel's closed categories
/// and a resize under way.
#[derive(Clone, Debug, Default)]
pub struct ToolboxState {
    collapsed: HashSet<&'static str>,
    resizing: Option<f32>,
}

/// Where a menu of nodes was opened from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// The toolbox: an entry adds its node after the selected one
    /// (`CreateNodeOnCanvas`) or is dragged onto the graph.
    Toolbox,
    /// The graph's context menu: at the right-click.
    Canvas,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(nerthus_nodes::registry)
}

pub(crate) fn listed(spec: &NodeSpec) -> bool {
    let hidden = spec.has_flag("Hidden") || spec.has_flag("Dev") || spec.has_flag("Obsolete");
    !hidden && !LEFT_OUT.contains(&spec.key())
}

/// A category's nodes in toolbox order: by type name, or (Pin To Top) the
/// most used first (`AddFamilyMembers`).
fn category_nodes(category: &str, options: &UserOptions, usage: &dyn Fn(&str) -> i32) -> Vec<&'static NodeSpec> {
    let in_category = Catalogue::get().nodes().filter(|spec| spec.toolbox.as_deref() == Some(category) && listed(spec));
    let mut nodes: Vec<&'static NodeSpec> = in_category.collect();
    nodes.sort_by(|left, right| left.key().cmp(right.key()));
    if options.toolbox_pin_high_use() {
        nodes.sort_by_key(|spec| std::cmp::Reverse(usage(spec.key())));
    }
    nodes
}

/// The nodes a category offers, by family (families by name).
fn category_families(category: &str, options: &UserOptions, usage: &dyn Fn(&str) -> i32) -> BTreeMap<String, Vec<&'static NodeSpec>> {
    let mut families: BTreeMap<String, Vec<&'static NodeSpec>> = BTreeMap::new();
    for spec in category_nodes(category, options, usage) {
        let family = spec.family.clone().unwrap_or_else(|| "Other".to_string());
        families.entry(family).or_default().push(spec);
    }
    families
}

fn node_label(spec: &NodeSpec) -> &str {
    spec.display_name.as_deref().unwrap_or(&spec.name)
}

/// `GetAverageUsage`: the mean use of every node type, as of the start.
fn average_usage(editor: &Editor) -> f64 {
    let usage = editor.user.usage.startup_usage();
    let nodes: Vec<&NodeSpec> = Catalogue::get().nodes().collect();
    if nodes.is_empty() {
        return 1.0;
    }
    let total: i64 = nodes.iter().map(|spec| usage.get(spec.key()).copied().unwrap_or(0) as i64).sum();
    total as f64 / nodes.len() as f64
}

/// Everything an entry needs to know of the settings and the usage.
struct EntryLook<'a> {
    editor: &'a Editor,
    average: f64,
}

impl EntryLook<'_> {
    fn usage(&self, type_name: &str) -> i32 {
        self.editor.user.usage.usage(type_name)
    }

    /// Highlight: entries used less than the average are faint.
    fn faint(&self, spec: &NodeSpec) -> bool {
        self.editor.user.options.toolbox_highlight_high_use() && (self.usage(spec.key()) as f64) < self.average
    }
}

/// One node entry (`AddFamilyMembers`): its `x-` icon when it has one (and
/// icons are shown), the name, the short code as the shortcut text and
/// the description as the tooltip; dimmed when Nerthus cannot build it,
/// faint when highlighting and little used.
fn node_entry(ui: &mut Ui, spec: &'static NodeSpec, look: &EntryLook, origin: Origin, state: &mut GraphState, commands: &mut Vec<Command>) {
    let implemented = registry().implements(spec.key());
    let icon_name = format!("x-{}", spec.name.to_lowercase());
    let show_icon = look.editor.user.options.show_icons_in_toolbox() && icons::exists(&icon_name);
    let icon = if show_icon { Some(icon_name.as_str()) } else { None };
    let short_code = spec.short_code.clone().unwrap_or_default();
    let faint = look.faint(spec);
    let mut response = ui
        .add_enabled_ui(implemented, |ui| {
            if faint {
                ui.multiply_opacity(0.5);
            }
            menu_item(ui, icon, None, node_label(spec), &short_code)
        })
        .inner;
    if let Some(description) = &spec.description {
        response = response.tooltip(description);
    }
    if !implemented {
        response = response.disabled_tooltip("Not available in Nerthus yet");
    }
    // `Member_PreviewMouseMove`: pressed and moved, the entry is dragged.
    let pressed_here = implemented && response.is_pointer_button_down_on();
    let moved = ui.input(|input| input.pointer.is_decidedly_dragging());
    if origin == Origin::Toolbox && pressed_here && moved {
        state.toolbox_drag = Some(spec.key().to_string());
        ui.close();
        return;
    }
    if !response.clicked() {
        return;
    }
    let command = match origin {
        Origin::Toolbox => Command::Graph(GraphCommand::ToolboxCreate(spec.key().to_string())),
        Origin::Canvas => Command::AddNode { type_name: spec.key().to_string(), position: None },
    };
    commands.push(command);
}

/// A category's families (`CreateFamilyMenu`): nested, each a header in
/// the category's colour opening its nodes; flat, the headers as captions
/// over their nodes, separated.
fn category_menu(ui: &mut Ui, category: &str, look: &EntryLook, origin: Origin, state: &mut GraphState, commands: &mut Vec<Command>) {
    menu_style(ui);
    if category == "Macro" {
        macro_menu(ui, commands);
        return;
    }
    let color = theme::toolbox_color(category);
    let options = &look.editor.user.options;
    let usage = |name: &str| look.usage(name);
    let families = category_families(category, options, &usage);
    let nested = options.toolbox_nested();
    for (index, (family, nodes)) in families.into_iter().enumerate() {
        let family_icon = format!("family-{}", family.to_lowercase());
        let icon = if icons::exists(&family_icon) { family_icon } else { "square-rounded".to_string() };
        if nested {
            header_submenu(ui, &icon, color, &family, |ui| {
                for spec in nodes {
                    node_entry(ui, spec, look, origin, state, commands);
                }
            });
            continue;
        }
        if index > 0 {
            menu_separator(ui);
        }
        family_caption(ui, &icon, color, &family);
        for spec in nodes {
            node_entry(ui, spec, look, origin, state, commands);
        }
    }
}

/// A flat list's family header (`Tag="HEADER"`, disabled): its icon and
/// small capitals in the category's colour.
fn family_caption(ui: &mut Ui, icon: &str, color: Color32, family: &str) {
    let galley = ui.painter().layout_no_wrap(family.to_uppercase(), theme::interface(11.0 * 0.8), color);
    let width = menu_row_width(ui, 32.0 + galley.size().x + 8.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 23.0), Sense::hover());
    let icon_rect = Rect::from_center_size(Pos2::new(rect.left() + 15.0, rect.center().y), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    ui.painter().galley(Pos2::new(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// The Macro category: the installed macros by author
/// (`NodeMetadata.Family`), each by name with the `x-macro` icon and its
/// description as the tooltip.
fn macro_menu(ui: &mut Ui, commands: &mut Vec<Command>) {
    let color = theme::toolbox_color("Macro");
    let authors = crate::files::installed_macros();
    if authors.is_empty() {
        menu_caption(ui, "No macros installed");
        return;
    }
    for (author, entries) in authors {
        header_submenu(ui, "x-macro", color, &author, |ui| {
            for entry in entries {
                let mut response = menu_item(ui, Some("x-macro"), None, &entry.metadata.name, "");
                if !entry.metadata.description.is_empty() {
                    response = response.tooltip(&entry.metadata.description);
                }
                if response.clicked() {
                    commands.push(Command::File(crate::files::FileCommand::AddMacro(entry.metadata.id())));
                }
            }
        });
    }
}

/// Every category as a submenu (the graph's context menu).
pub fn add_node_menus(ui: &mut Ui, editor: &Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let look = EntryLook { editor, average: average_usage(editor) };
    for category in CATEGORIES {
        let color = theme::toolbox_color(category);
        let icon = format!("toolbox-{}", category.to_lowercase());
        submenu(ui, Some(&icon), Some(color), category, |ui| category_menu(ui, category, &look, Origin::Canvas, state, commands));
    }
}

/// The toolbox at the top-left of `area` (`Toolbox`'s place in the graph
/// area, its margin taken off).
pub fn show(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    match editor.user.options.toolbox_style() {
        ToolboxStyle::Menus => show_menus(ui, area, editor, state, commands),
        ToolboxStyle::Panel => show_panel(ui, area, editor, state, commands),
    }
}

/// The menus style (`ScrollViewer`, margin 10): Long is one column of
/// icons, Compact two, Expanded one of icons with names. A column taller
/// than its area is cut off at the margin and scrolls, its handle in
/// sight while there is more to see.
fn show_menus(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let mode = editor.user.options.toolbox_mode();
    let (column, button, per_row) = menus_layout(ui, area, mode);
    let rows = CATEGORIES.len().div_ceil(per_row);
    let content = Vec2::new(column.width(), rows as f32 * button.y);
    let look = EntryLook { editor, average: average_usage(editor) };
    let mut settings: Vec<SettingsChange> = Vec::new();
    let builder = UiBuilder::new().max_rect(column).layout(egui::Layout::top_down(egui::Align::Min));
    let mut column_ui = ui.new_child(builder);
    column_ui.shrink_clip_rect(column);
    let scroll = &mut column_ui.spacing_mut().scroll;
    scroll.dormant_handle_opacity = scroll.active_handle_opacity;
    ScrollArea::vertical().id_salt("toolbox-menus-scroll").max_height(column.height()).show(&mut column_ui, |ui| {
        let (buttons, _) = ui.allocate_exact_size(content, Sense::hover());
        for (index, category) in CATEGORIES.iter().enumerate() {
            let row = (index / per_row) as f32;
            let place = (index % per_row) as f32;
            let corner = buttons.min + Vec2::new(place * button.x, row * button.y);
            let rect = Rect::from_min_size(corner, button);
            category_button(ui, rect, category, mode, &look, state, commands, &mut settings);
        }
    });
    resize_menus(ui, column, mode, &mut editor.user.options, state);
    apply_settings(&mut editor.user.options, state, settings);
}

/// Where `show` puts the toolbox in `area`, worked out before it is drawn
/// so the graph under it can leave room for it.
pub fn place(ui: &Ui, area: Rect, options: &UserOptions) -> Rect {
    match options.toolbox_style() {
        ToolboxStyle::Menus => menus_layout(ui, area, options.toolbox_mode()).0,
        ToolboxStyle::Panel => panel_rect(area, options.toolbox_columns()),
    }
}

/// The menus' column at the top-left of `area` (margin 10), as much of it
/// as shows (no lower than the margin above the area's bottom), a
/// button's size and how many buttons share a row.
fn menus_layout(ui: &Ui, area: Rect, mode: ToolboxMode) -> (Rect, Vec2, usize) {
    let font = theme::semibold(10.0);
    let line_height = ui.painter().layout_no_wrap("Primitive".to_string(), font, theme::LABEL_DEFAULT).size().y;
    let button_height = line_height + 2.0 * TEXT_PADDING;
    let (width, button_width, per_row) = match mode {
        ToolboxMode::Single => (BUTTON_SIZE, BUTTON_SIZE, 1),
        ToolboxMode::Double => (WIDE, BUTTON_SIZE, 2),
        ToolboxMode::Expanded => (WIDE, WIDE, 1),
    };
    let rows = CATEGORIES.len().div_ceil(per_row);
    let room = (area.height() - 2.0 * MENUS_MARGIN).max(0.0);
    let height = (rows as f32 * button_height).min(room);
    let column = Rect::from_min_size(area.min + Vec2::splat(MENUS_MARGIN), Vec2::new(width, height));
    (column, Vec2::new(button_width, button_height), per_row)
}

/// The panel down the left of `area`, `columns` buttons wide.
fn panel_rect(area: Rect, columns: usize) -> Rect {
    let width = columns as f32 * PANEL_COLUMN;
    Rect::from_min_max(area.min, Pos2::new(area.left() + width + 10.0, area.bottom()))
}

/// The toolbox's right edge (`GridSplitter`): dragged wider or narrower,
/// the menus switch between Long, Compact and Expanded (`ResizeToolbox`).
fn resize_menus(ui: &mut Ui, column: Rect, mode: ToolboxMode, options: &mut UserOptions, state: &mut GraphState) {
    let handle = Rect::from_min_size(Pos2::new(column.right(), column.top()), Vec2::new(SPLITTER, column.height()));
    let response = ui.interact(handle, ui.id().with("toolbox-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() {
        state.toolbox.resizing = None;
        return;
    }
    let travelled = state.toolbox.resizing.unwrap_or(0.0) + response.drag_delta().x;
    if travelled.abs() <= SPLITTER_STEP {
        state.toolbox.resizing = Some(travelled);
        return;
    }
    state.toolbox.resizing = Some(0.0);
    let width = column.width() + travelled;
    let next = match mode {
        ToolboxMode::Single if width > BUTTON_SIZE * 2.0 => ToolboxMode::Double,
        ToolboxMode::Double if width < BUTTON_SIZE => ToolboxMode::Single,
        ToolboxMode::Double if width > BUTTON_SIZE * 4.0 => ToolboxMode::Expanded,
        ToolboxMode::Expanded if width < BUTTON_SIZE * 2.0 => ToolboxMode::Double,
        _ => mode,
    };
    if next != mode {
        options.set_toolbox_mode(next);
    }
}

/// A `ToolboxMenuItem`: `MainRectangle` (margin 1, `ToolboxButton-Background`
/// at 0.8, corners 2), the category icon in its colour (centred, or in a
/// 28 column beside the name when expanded), the name in 10px semibold;
/// its menu opens to the right, its right-click menu holds the settings.
#[allow(clippy::too_many_arguments)]
fn category_button(ui: &mut Ui, rect: Rect, category: &str, mode: ToolboxMode, look: &EntryLook, state: &mut GraphState, commands: &mut Vec<Command>, settings: &mut Vec<SettingsChange>) {
    let response = ui.interact(rect, ui.id().with(("toolbox-category", category)), Sense::click());
    // The flyout has its own id: the right-click menu takes the response's
    // default popup id, and two popups on one id clash and close each other.
    let popup_id = response.id.with("category-flyout");
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let hovered = response.hovered();
    let fill = if open {
        BUTTON_OPEN
    } else if hovered {
        BUTTON_HOVER
    } else {
        theme::TOOLBOX_BUTTON_BACKGROUND.gamma_multiply(0.8)
    };
    let body = rect.shrink(1.0);
    ui.painter().rect_filled(body, 2.0, fill);
    ui.painter().rect_stroke(body, 2.0, Stroke::new(1.0, theme::TOOLBOX_BUTTON_BACKGROUND), StrokeKind::Inside);
    let color = theme::toolbox_color(category);
    let icon = format!("toolbox-{}", category.to_lowercase());
    let expanded = mode == ToolboxMode::Expanded;
    let icon_center = if expanded { Pos2::new(rect.left() + ICON_COLUMN / 2.0, rect.center().y) } else { rect.center() };
    icons::paint(ui, Rect::from_center_size(icon_center, Vec2::splat(icons::DEFAULT_SIZE)), &icon, color);
    if expanded {
        let text_color = if hovered { theme::MENU_FOREGROUND_HOVER } else { theme::LABEL_DEFAULT };
        let galley = ui.painter().layout_no_wrap(category.to_string(), theme::semibold(10.0), text_color);
        ui.painter().galley(Pos2::new(rect.left() + ICON_COLUMN, rect.center().y - galley.size().y / 2.0), galley, text_color);
    }
    let response = response.tooltip(category);
    response.context_menu(|ui| settings_menu(ui, look.editor, settings));

    // The menu (`_SubMenuPopup`, placed right, margin 6). A flat list runs
    // long: its rows scroll in the room beside the button (egui's anchor).
    let info = UiStackInfo::new(UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, MenuConfig::default());
    let anchor = egui::PopupAnchor::from(&response).rect(popup_id, ui.ctx()).unwrap_or(response.interact_rect);
    let mut popup = egui::Popup::menu(&response).id(popup_id);
    popup = popup.align(egui::RectAlign::RIGHT_START);
    popup = popup.gap(6.0);
    popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup = popup.info(info);
    popup.show(|ui| menu_rows_beside(ui, anchor, |ui| category_menu(ui, category, look, Origin::Toolbox, state, commands)));
}

/// The panel style (`GeneratePanelStyleToolbox`, on `Panel-Background`,
/// the area's height): node buttons in columns, under each category's
/// expander when categorized.
fn show_panel(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    let options = &editor.user.options;
    let columns = options.toolbox_columns();
    let width = columns as f32 * PANEL_COLUMN;
    let panel = panel_rect(area, columns);
    ui.painter().rect_filled(panel, 0.0, theme::PANEL_BACKGROUND);
    let look = EntryLook { editor, average: average_usage(editor) };
    let mut settings: Vec<SettingsChange> = Vec::new();
    let background = ui.interact(panel, ui.id().with("toolbox-panel"), Sense::click());
    background.context_menu(|ui| settings_menu(ui, look.editor, &mut settings));
    let inner = panel.shrink2(Vec2::new(5.0, 0.0));
    let builder = UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min));
    let mut panel_ui = ui.new_child(builder);
    panel_ui.shrink_clip_rect(panel);
    let categorized = look.editor.user.options.toolbox_categorized();
    ScrollArea::vertical().id_salt("toolbox-panel-scroll").max_height(inner.height()).show(&mut panel_ui, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        if categorized {
            for category in CATEGORIES {
                if category == "Macro" {
                    continue;
                }
                let open = !state.toolbox.collapsed.contains(category);
                ui.add_space(5.0);
                if expander_header(ui, category, open, &look, &mut settings) {
                    toggle(&mut state.toolbox.collapsed, category);
                }
                if open {
                    panel_grid(ui, &[category], columns, &look, state, commands, &mut settings);
                }
                ui.add_space(5.0);
            }
        } else {
            let categories: Vec<&str> = CATEGORIES.iter().copied().filter(|category| *category != "Macro").collect();
            panel_grid(ui, &categories, columns, &look, state, commands, &mut settings);
        }
    });
    resize_panel(ui, panel, &mut editor.user.options, state);
    apply_settings(&mut editor.user.options, state, settings);
}

fn toggle(set: &mut HashSet<&'static str>, category: &'static str) {
    if !set.remove(category) {
        set.insert(category);
    }
}

/// `Expander-Toolbox`: the chevron (grey, `Accent-Gray` under the
/// pointer) and the category's name. Returns whether it was clicked.
fn expander_header(ui: &mut Ui, category: &str, open: bool, look: &EntryLook, settings: &mut Vec<SettingsChange>) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::click());
    let chevron = if open { "chevron-down" } else { "chevron-right" };
    let tint = if response.hovered() { theme::ACCENT_GRAY } else { theme::EXPANDER_HEADER };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 2.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, chevron, tint);
    let galley = ui.painter().layout_no_wrap(category.to_string(), theme::semibold(11.0), theme::LABEL_DEFAULT);
    ui.painter().galley(Pos2::new(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);
    response.context_menu(|ui| settings_menu(ui, look.editor, settings));
    response.clicked()
}

/// `AddCategoryButtonsToPanel` for some categories, in a uniform grid:
/// each node a subtle button (10px, the category's colour, centred or
/// left), margin 2 and padding 6 (compact: 1 and 4, 2); highlighting, the
/// little used at 0.75, the rest semibold.
#[allow(clippy::too_many_arguments)]
fn panel_grid(ui: &mut Ui, categories: &[&str], columns: usize, look: &EntryLook, state: &mut GraphState, commands: &mut Vec<Command>, settings: &mut Vec<SettingsChange>) {
    let options = &look.editor.user.options;
    let compact = options.toolbox_compact();
    let left_aligned = options.toolbox_left_align();
    let highlight = options.toolbox_highlight_high_use();
    let margin = if compact { 1.0 } else { 2.0 };
    let padding = if compact { Vec2::new(4.0, 2.0) } else { Vec2::splat(6.0) };
    let usage = |name: &str| look.usage(name);
    let mut entries: Vec<(&'static NodeSpec, Color32)> = Vec::new();
    for category in categories {
        let color = theme::toolbox_color(category);
        for spec in category_nodes(category, options, &usage) {
            entries.push((spec, color));
        }
    }
    let line_height = ui.painter().layout_no_wrap("Mountain".to_string(), theme::interface(10.0), Color32::WHITE).size().y;
    let cell_height = line_height + 2.0 * padding.y + 2.0 * margin;
    let rows = entries.len().div_ceil(columns.max(1));
    let (grid, _) = ui.allocate_exact_size(Vec2::new(columns as f32 * PANEL_COLUMN, rows as f32 * cell_height), Sense::hover());
    for (index, (spec, color)) in entries.into_iter().enumerate() {
        let column = (index % columns) as f32;
        let row = (index / columns) as f32;
        let cell = Rect::from_min_size(grid.min + Vec2::new(column * PANEL_COLUMN, row * cell_height), Vec2::new(PANEL_COLUMN, cell_height));
        let button = cell.shrink(margin);
        let implemented = registry().implements(spec.key());
        let sense = if implemented { Sense::click_and_drag() } else { Sense::hover() };
        let response = ui.interact(button, ui.id().with(("toolbox-node", spec.key())), sense);
        let fill = if response.is_pointer_button_down_on() {
            theme::BUTTON_PRESSED
        } else if response.hovered() {
            theme::BUTTON_HOVER
        } else {
            theme::BUTTON_BACKGROUND
        };
        ui.painter().rect_filled(button, 2.0, fill);
        let faint = highlight && (look.usage(spec.key()) as f64) < look.average;
        let mut opacity = if faint { 0.75 } else { 1.0 };
        if !implemented {
            opacity *= 0.5;
        }
        let font = if highlight && !faint { theme::semibold(10.0) } else { theme::interface(10.0) };
        let text_color = color.gamma_multiply(opacity);
        let galley = crate::ui::graph_canvas::elided_galley(ui.painter(), node_label(spec), font, text_color, button.width() - 2.0 * padding.x);
        let x = if left_aligned { button.left() + padding.x } else { button.center().x - galley.size().x / 2.0 };
        ui.painter().galley(Pos2::new(x, button.center().y - galley.size().y / 2.0), galley, text_color);
        let mut response = match &spec.description {
            Some(description) => response.tooltip(description),
            None => response,
        };
        if !implemented {
            response = response.tooltip("Not available in Nerthus yet");
        }
        response.context_menu(|ui| settings_menu(ui, look.editor, settings));
        if response.drag_started_by(egui::PointerButton::Primary) {
            state.toolbox_drag = Some(spec.key().to_string());
        } else if response.clicked() {
            commands.push(Command::Graph(GraphCommand::ToolboxCreate(spec.key().to_string())));
        }
    }
}

/// The panel's right edge: its columns follow the width, one per 80, from
/// one to six (`ResizeToolbox`).
fn resize_panel(ui: &mut Ui, panel: Rect, options: &mut UserOptions, state: &mut GraphState) {
    let handle = Rect::from_min_size(Pos2::new(panel.right() - SPLITTER / 2.0, panel.top()), Vec2::new(SPLITTER, panel.height()));
    let response = ui.interact(handle, ui.id().with("toolbox-panel-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() {
        state.toolbox.resizing = None;
        return;
    }
    let travelled = state.toolbox.resizing.unwrap_or(0.0) + response.drag_delta().x;
    if travelled.abs() <= SPLITTER_STEP {
        state.toolbox.resizing = Some(travelled);
        return;
    }
    state.toolbox.resizing = Some(0.0);
    let width = options.toolbox_columns() as f32 * PANEL_COLUMN + travelled;
    let columns = ((width / PANEL_COLUMN) as usize).clamp(1, 6);
    options.set_toolbox_columns(columns);
}

/// A setting the toolbox's menu changed.
enum SettingsChange {
    Style(ToolboxStyle),
    Nested(bool),
    Mode(ToolboxMode),
    ExpandAll(bool),
    Compact(bool),
    Categorized(bool),
    LeftAlign(bool),
    Pin(bool),
    Highlight(bool),
}

/// `RecreateContextMenu`: the style; for menus the list style and size,
/// for the panel its options; then the high use items.
fn settings_menu(ui: &mut Ui, editor: &Editor, settings: &mut Vec<SettingsChange>) {
    menu_style(ui);
    let options = &editor.user.options;
    let style = options.toolbox_style();
    menu_caption(ui, "Style");
    if checked_icon_item(ui, style == ToolboxStyle::Panel, Some("toolbox-panel"), None, "Panel", "") {
        settings.push(SettingsChange::Style(ToolboxStyle::Panel));
    }
    if checked_icon_item(ui, style == ToolboxStyle::Menus, Some("toolbox-menus"), None, "Menus", "") {
        settings.push(SettingsChange::Style(ToolboxStyle::Menus));
    }
    match style {
        ToolboxStyle::Menus => {
            menu_separator(ui);
            menu_caption(ui, "Menu Style");
            let nested = options.toolbox_nested();
            if checked_icon_item(ui, !nested, Some("list"), None, "Flat List", "") {
                settings.push(SettingsChange::Nested(false));
            }
            if checked_icon_item(ui, nested, Some("list-tree"), None, "Nested", "") {
                settings.push(SettingsChange::Nested(true));
            }
            menu_separator(ui);
            menu_caption(ui, "Toolbox Size");
            let mode = options.toolbox_mode();
            let sizes = [(ToolboxMode::Single, "Long"), (ToolboxMode::Double, "Compact"), (ToolboxMode::Expanded, "Expanded")];
            for (size, text) in sizes {
                if check_item(ui, mode == size, text, "").clicked() {
                    settings.push(SettingsChange::Mode(size));
                }
            }
        }
        ToolboxStyle::Panel => {
            let categorized = options.toolbox_categorized();
            if categorized {
                menu_separator(ui);
                if menu_item(ui, None, None, "Expand All", "").clicked() {
                    settings.push(SettingsChange::ExpandAll(true));
                }
                if menu_item(ui, None, None, "Collapse All", "").clicked() {
                    settings.push(SettingsChange::ExpandAll(false));
                }
            }
            menu_separator(ui);
            menu_caption(ui, "Options");
            let compact = options.toolbox_compact();
            if check_item(ui, compact, "Compact Mode", "").clicked() {
                settings.push(SettingsChange::Compact(!compact));
            }
            if check_item(ui, categorized, "Categorized", "").clicked() {
                settings.push(SettingsChange::Categorized(!categorized));
            }
            let left_aligned = options.toolbox_left_align();
            if check_item(ui, left_aligned, "Align Left", "").clicked() {
                settings.push(SettingsChange::LeftAlign(!left_aligned));
            }
        }
    }
    menu_separator(ui);
    menu_caption(ui, "High Use Items");
    let pinned = options.toolbox_pin_high_use();
    if check_item(ui, pinned, "Pin To Top", "").clicked() {
        settings.push(SettingsChange::Pin(!pinned));
    }
    let highlighted = options.toolbox_highlight_high_use();
    if check_item(ui, highlighted, "Highlight", "").clicked() {
        settings.push(SettingsChange::Highlight(!highlighted));
    }
}

fn apply_settings(options: &mut UserOptions, state: &mut GraphState, settings: Vec<SettingsChange>) {
    for change in settings {
        match change {
            SettingsChange::Style(style) => options.set_toolbox_style(style),
            SettingsChange::Nested(nested) => options.set_toolbox_nested(nested),
            SettingsChange::Mode(mode) => options.set_toolbox_mode(mode),
            SettingsChange::ExpandAll(true) => state.toolbox.collapsed.clear(),
            SettingsChange::ExpandAll(false) => state.toolbox.collapsed = CATEGORIES.iter().copied().collect(),
            SettingsChange::Compact(on) => options.set_toolbox_compact(on),
            SettingsChange::Categorized(on) => options.set_toolbox_categorized(on),
            SettingsChange::LeftAlign(on) => options.set_toolbox_left_align(on),
            SettingsChange::Pin(on) => options.set_toolbox_pin_high_use(on),
            SettingsChange::Highlight(on) => options.set_toolbox_highlight_high_use(on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::ui::title_bar;

    /// Windows 1280 x 800 and 1280 x 500 pixels at the default 120%, where
    /// the flat Modify and Primitive lists are taller than the window.
    const WINDOWS: [Vec2; 2] = [Vec2::new(1066.7, 666.7), Vec2::new(1066.7, 416.7)];

    fn input(screen: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() }
    }

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set.
    fn themed_context(screen: Vec2) -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context.run_ui(input(screen, Vec::new()), |_| {}).drop_without_applying_deltas();
        context
    }

    /// The graph, under the title bar and its tabs.
    fn graph_area(screen: Vec2) -> Rect {
        Rect::from_min_max(Pos2::new(0.0, title_bar::HEIGHT + 30.0), screen.to_pos2())
    }

    /// What the test window draws over the graph.
    #[derive(Clone, Copy)]
    enum Drawn {
        Toolbox,
        /// The graph's context menu, with every category's nodes.
        ContextMenu,
    }

    /// A window `screen` big, its toolbox showing Organization = Show All
    /// (flat lists).
    struct Window {
        context: egui::Context,
        screen: Vec2,
        editor: Editor,
        state: GraphState,
        drawn: Drawn,
    }

    impl Window {
        fn new(screen: Vec2, drawn: Drawn) -> Self {
            let mut editor = Editor::new(Document::new());
            editor.user.options.set_toolbox_nested(false);
            Window { context: themed_context(screen), screen, editor, state: GraphState::default(), drawn }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<Command> {
            let (commands, output) = self.run(events);
            output.drop_without_applying_deltas();
            commands
        }

        /// The text a frame with no input draws.
        fn text(&mut self) -> Vec<String> {
            let (_, output) = self.run(Vec::new());
            let mut drawn = Vec::new();
            output.shapes.iter().for_each(|clipped| texts(&clipped.shape, &mut drawn));
            output.drop_without_applying_deltas();
            drawn
        }

        /// A frame with `events`: what it asked for and what it drew.
        fn run(&mut self, events: Vec<egui::Event>) -> (Vec<Command>, egui::FullOutput) {
            let mut commands = Vec::new();
            let area = graph_area(self.screen);
            let output = self.context.run_ui(input(self.screen, events), |ui| match self.drawn {
                Drawn::Toolbox => show(ui, area, &mut self.editor, &mut self.state, &mut commands),
                Drawn::ContextMenu => {
                    let graph = ui.interact(area, ui.id().with("graph"), Sense::click());
                    graph.context_menu(|ui| {
                        menu_style(ui);
                        add_node_menus(ui, &self.editor, &mut self.state, &mut commands);
                    });
                }
            });
            (commands, output)
        }

        /// A frame with `events`, then the frames after as the app draws
        /// them, for the menus to settle.
        fn settle(&mut self, events: Vec<egui::Event>) -> Vec<Command> {
            let mut commands = self.frame(events);
            for _ in 0..3 {
                commands.extend(self.frame(Vec::new()));
            }
            commands
        }

        /// A click of `button` at `at`: the pointer moves there, presses
        /// and lets go.
        fn click(&mut self, at: Pos2, button: egui::PointerButton) -> Vec<Command> {
            let press = |pressed: bool| egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE };
            let mut commands = self.frame(vec![egui::Event::PointerMoved(at)]);
            commands.extend(self.frame(vec![press(true)]));
            commands.extend(self.settle(vec![press(false)]));
            commands
        }

        /// The menus open, as their areas were laid out: a menu before its
        /// submenu.
        fn open_menus(&self) -> Vec<Rect> {
            self.context.memory(|memory| {
                let visible = memory.areas().visible_layer_ids();
                let open = memory.layer_ids().filter(|layer| layer.order == egui::Order::Foreground && visible.contains(layer));
                open.filter_map(|layer| memory.area_rect(layer.id)).collect()
            })
        }

        /// `category`'s button in the toolbox.
        fn button(&self, category: &str) -> Rect {
            let index = CATEGORIES.iter().position(|name| *name == category).unwrap();
            let mode = self.editor.user.options.toolbox_mode();
            let mut rect = Rect::NOTHING;
            let context = themed_context(self.screen);
            let output = context.run_ui(input(self.screen, Vec::new()), |ui| {
                let (column, button, per_row) = menus_layout(ui, graph_area(self.screen), mode);
                let corner = column.min + Vec2::new((index % per_row) as f32 * button.x, (index / per_row) as f32 * button.y);
                rect = Rect::from_min_size(corner, button);
            });
            output.drop_without_applying_deltas();
            rect
        }

        /// The wheel turned all the way down over `menu`, and the frames
        /// after for it to scroll there.
        fn scroll_down(&mut self, menu: Rect) {
            let over = Pos2::new(menu.left() + 40.0, menu.center().y);
            let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
            self.frame(vec![egui::Event::PointerMoved(over)]);
            self.frame(vec![wheel]);
            for _ in 0..60 {
                self.frame(Vec::new());
            }
        }

        /// A click on `menu`'s last row.
        fn click_last_row(&mut self, menu: Rect) -> Vec<Command> {
            self.click(Pos2::new(menu.left() + 40.0, menu.bottom() - 6.0), egui::PointerButton::Primary)
        }

        /// The last node of `category`'s flat list.
        fn last_node(&self, category: &str) -> &'static str {
            let usage = |name: &str| self.editor.user.usage.usage(name);
            let families = category_families(category, &self.editor.user.options, &usage);
            let last = families.values().last().and_then(|nodes| nodes.last()).expect("the category has nodes");
            assert!(registry().implements(last.key()), "{} can't be added", last.key());
            last.key()
        }
    }

    /// The text in `shape`, nested shapes' too.
    fn texts(shape: &egui::Shape, into: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => into.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, into)),
            _ => {}
        }
    }

    /// `menu` on a window of `screen`: under the title bar and in the
    /// window.
    fn assert_in_the_window(menu: Rect, screen: Vec2, what: &str) {
        assert!(menu.top() >= title_bar::HEIGHT - 0.5, "{what} at {menu:?} covers the title bar at {screen:?}");
        assert!(menu.bottom() <= screen.y + 0.5, "{what} at {menu:?} runs past the window's bottom at {screen:?}");
    }

    #[test]
    fn a_flat_category_menu_opens_beside_its_button_and_scrolls() {
        for screen in WINDOWS {
            let mut window = Window::new(screen, Drawn::Toolbox);
            window.frame(Vec::new());
            let button = window.button("Modify");
            let commands = window.click(button.center(), egui::PointerButton::Primary);
            assert!(commands.is_empty(), "opening Modify gave {commands:?} at {screen:?}");
            let [menu] = window.open_menus()[..] else { panic!("Modify's menu does not open at {screen:?}: {:?}", window.open_menus()) };
            assert_in_the_window(menu, screen, "Modify's menu");
            assert!(menu.left() >= button.right(), "Modify's menu at {menu:?} covers its button at {button:?}");
            let beside = (menu.top() - button.top()).abs() < 1.0 || (menu.bottom() - button.bottom()).abs() < 1.0;
            assert!(beside, "Modify's menu at {menu:?} is not beside its button at {button:?} at {screen:?}");

            window.scroll_down(menu);
            let last = window.last_node("Modify");
            let commands = window.click_last_row(menu);
            assert_eq!(commands, [Command::Graph(GraphCommand::ToolboxCreate(last.to_string()))], "at {screen:?}");
        }
    }

    #[test]
    fn a_flat_category_submenu_of_the_graph_menu_fits_and_scrolls() {
        for screen in WINDOWS {
            let mut window = Window::new(screen, Drawn::ContextMenu);
            window.frame(Vec::new());
            let commands = window.click(Pos2::new(300.0, graph_area(screen).top() + 20.0), egui::PointerButton::Secondary);
            assert!(commands.is_empty(), "opening the graph's menu gave {commands:?} at {screen:?}");
            let [menu] = window.open_menus()[..] else { panic!("the graph's menu does not open at {screen:?}") };
            // Primitive, its first row.
            window.settle(vec![egui::Event::PointerMoved(Pos2::new(menu.center().x, menu.top() + 12.0))]);
            let [_, primitive] = window.open_menus()[..] else { panic!("hovering Primitive does not open its submenu at {screen:?}") };
            assert_in_the_window(primitive, screen, "Primitive's submenu");
            let beside = primitive.left() >= menu.right() - 0.5 || primitive.right() <= menu.left() + 0.5;
            assert!(beside, "Primitive's submenu at {primitive:?} covers the graph's menu at {menu:?} at {screen:?}");

            window.scroll_down(primitive);
            let last = window.last_node("Primitive");
            let commands = window.click_last_row(primitive);
            assert_eq!(commands, [Command::AddNode { type_name: last.to_string(), position: None }], "at {screen:?}");
        }
    }

    #[test]
    fn a_right_click_on_a_category_opens_the_settings() {
        let mut window = Window::new(WINDOWS[0], Drawn::Toolbox);
        window.frame(Vec::new());
        let button = window.button("Primitive");
        let commands = window.click(button.center(), egui::PointerButton::Secondary);
        assert!(commands.is_empty(), "right-clicking Primitive gave {commands:?}");
        // The settings stay open, without egui's ID clash warning: the
        // category's menu used to share the right-click menu's id.
        let drawn = window.text();
        for setting in ["Panel", "Menus", "Flat List", "Nested", "Long", "Compact", "Expanded", "Pin To Top", "Highlight"] {
            assert!(drawn.iter().any(|text| text == setting), "{setting} missing from {drawn:?}");
        }
        assert!(!drawn.iter().any(|text| text.contains("widget ID")), "{drawn:?}");
    }

    #[test]
    fn a_click_on_a_category_opens_its_menu_not_the_settings() {
        let mut window = Window::new(WINDOWS[0], Drawn::Toolbox);
        window.frame(Vec::new());
        let button = window.button("Primitive");
        window.click(button.center(), egui::PointerButton::Primary);
        let drawn = window.text();
        // The flat list's first family caption, at its top.
        let families = category_families("Primitive", &window.editor.user.options, &|_| 0);
        let first = families.keys().next().expect("Primitive has families");
        assert!(drawn.iter().any(|text| text.eq_ignore_ascii_case(first)), "{first} missing from {drawn:?}");
        assert!(!drawn.iter().any(|text| text == "Pin To Top"), "{drawn:?}");
        assert!(!drawn.iter().any(|text| text.contains("widget ID")), "{drawn:?}");
    }
}
