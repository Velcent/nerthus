//! The window's top bar (`Shell` `MenuBar`, 6 down): the brand, the main
//! menu, the file box (preview resolution and region, file name, save
//! buttons, autosave), the undo history, the version and the window
//! buttons. On macOS the system supplies the main menu and traffic lights.
//! Empty space drags the window.
//!
//! `MenuBar` is a `DockPanel`: the brand, the menus, the file box and undo
//! dock left, each as wide as it asks out of what those before it left;
//! then the window buttons and the version boxes dock right out of what
//! is left after that. A part given less room than it asks is laid out
//! whole from its slot's left and cut off at the slot's right, so as a
//! window narrows the version boxes go first, then the window buttons
//! (close first), then undo and the file box. F10 puts the main menu in
//! menu mode and Alt with an item's access key opens it (`menu_keys`).
//! On macOS the file box stays centered in the window, with undo beside
//! it and the brand and version boxes taking the remaining edge space.

use egui::containers::menu::MenuButton;
use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::{menu_keys, menus};
use crate::ui::tool_commands::{self, ToolCommand};
use crate::files::{FileCommand, AUTOSAVE_CHOICES};
use crate::ui::widgets::{checked_icon_item, icon_button_sized, menu_caption, menu_config, menu_item, menu_item_enabled, menu_rows, menu_separator, menu_style, toolbar_button};
#[cfg(not(target_os = "macos"))]
use crate::ui::widgets::icon_menu;
#[cfg(target_os = "macos")]
use crate::ui::widgets::icon_menu_sized;
use crate::ui::UiState;

/// Native dimensions and the padding around title controls are in Cocoa
/// points, independent of the editor's interface zoom.
const DEFAULT_NATIVE_HEIGHT: f32 = 32.0;
const MACOS_TOP_PADDING: f32 = 6.0;
const MACOS_BOTTOM_PADDING: f32 = 2.0;
const MACOS_VERTICAL_PADDING: f32 = MACOS_TOP_PADDING + MACOS_BOTTOM_PADDING;
pub const HEIGHT: f32 = if cfg!(target_os = "macos") { DEFAULT_NATIVE_HEIGHT + MACOS_VERTICAL_PADDING } else { 36.0 };
const NATIVE_HEIGHT_ID: &str = "nerthus-native-title-bar-height";
/// `MenuBar`'s top margin.
const BAR_TOP: f32 = if cfg!(target_os = "macos") { 0.0 } else { 6.0 };
/// Adjust these offsets to move the native controls without changing
/// their appearance or actions. Vertical placement is relative to content center.
const TRAFFIC_LIGHTS_LEFT: f32 = 12.0;
const TRAFFIC_LIGHTS_CENTER_OFFSET: f32 = 0.0;
const TRAFFIC_LIGHTS_WIDTH: f32 = TRAFFIC_LIGHTS_LEFT + 60.0 + 8.0;
/// A top-level menu item: at least 64 wide, overlapping its neighbours by
/// 5 (`Margin -5,0,-5,0`), its text 8 + 5 in each side.
const TOP_LEVEL_MIN_WIDTH: f32 = 64.0;
const TOP_LEVEL_OVERLAP: f32 = 5.0;
const TOP_LEVEL_HEIGHT: f32 = 22.0;
/// The main `Menu`'s margin 0, 0, 12, 0.
const MENU_MARGIN_RIGHT: f32 = 12.0;
/// The file box: `MinWidth 180`, margin 6, 0, 6, 0, padding 1.
const FILE_BOX_MIN_WIDTH: f32 = 180.0;
const FILE_BOX_MARGIN: f32 = 6.0;
/// `txbFileName`: `MinWidth 140`, margin 8, 4.
const NAME_MIN_WIDTH: f32 = 140.0;
const NAME_MARGIN: Vec2 = Vec2::new(8.0, 4.0);
/// `Button-IconOnly`: 14 with padding 6.
const SAVE_BUTTON: f32 = 14.0 + 2.0 * 6.0;
/// The undo `Menu`'s margin 5, 0, 0, 0.
const UNDO_MARGIN: f32 = 5.0;
/// The version boxes' `StackPanel` margin 24, 0, and `bUpdate`'s margin
/// 0, 0, 8, 0 before `bAbout`.
const VERSION_MARGIN: f32 = 24.0;
const BOX_GAP: f32 = 8.0;
/// `Button-System`, and the window buttons with their margins 2.
const SYSTEM_BUTTON: f32 = 29.0;
const WINDOW_BUTTONS_WIDTH: f32 = 3.0 * (2.0 + SYSTEM_BUTTON + 2.0);
const VERSION: &str = concat!("Nerthus ", env!("CARGO_PKG_VERSION"));

pub fn height(context: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") {
        (native_height(context) + MACOS_VERTICAL_PADDING) / context.zoom_factor()
    } else {
        HEIGHT
    }
}

/// The measured Cocoa height in logical pixels, before interface zoom.
pub fn native_height(context: &egui::Context) -> f32 {
    let id = egui::Id::new(NATIVE_HEIGHT_ID);
    context.data(|data| data.get_temp::<f32>(id)).unwrap_or(DEFAULT_NATIVE_HEIGHT)
}

fn content_height(context: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") {
        native_height(context) / context.zoom_factor()
    } else {
        HEIGHT - BAR_TOP
    }
}

fn top_padding(context: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") {
        MACOS_TOP_PADDING / context.zoom_factor()
    } else {
        0.0
    }
}

fn bottom_padding(context: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") {
        MACOS_BOTTOM_PADDING / context.zoom_factor()
    } else {
        0.0
    }
}

/// Read the live native title bar so older macOS releases and current
/// releases center their traffic lights within the same app bar.
#[cfg(target_os = "macos")]
pub fn update_native_metrics(context: &egui::Context, handles: &bevy::window::RawHandleWrapper) {
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;

    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    let RawWindowHandle::AppKit(handle) = handles.get_window_handle() else {
        return;
    };
    // SAFETY: Bevy owns this live window's content view, and the caller
    // holds its raw handle component while running on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    let content_height = view.bounds().size.height;
    let layout_height = window.contentLayoutRect().size.height;
    let measured_height = (content_height - layout_height) as f32;
    record_native_height(context, measured_height);

    let center_y = MACOS_TOP_PADDING + native_height(context) / 2.0 + TRAFFIC_LIGHTS_CENTER_OFFSET;
    let bar_height = native_height(context) + MACOS_VERTICAL_PADDING;
    crate::ui::macos_window::position_traffic_lights(handles, TRAFFIC_LIGHTS_LEFT as f64, center_y as f64, bar_height as f64);
}

#[cfg(target_os = "macos")]
fn record_native_height(context: &egui::Context, native_height: f32) {
    // Full screen can report zero while the title bar is hidden. Keep
    // the last useful height through that transition and invalid frames.
    if !(10.0..=80.0).contains(&native_height) {
        return;
    }
    let id = egui::Id::new(NATIVE_HEIGHT_ID);
    context.data_mut(|data| data.insert_temp(id, native_height));
}

fn leading_inset(context: &egui::Context) -> f32 {
    if cfg!(target_os = "macos") {
        TRAFFIC_LIGHTS_WIDTH / context.zoom_factor()
    } else {
        0.0
    }
}

pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);

    // Behind everything: a press on empty space drags the window.
    let drag_response = ui.interact(rect, ui.id().with("title-drag"), Sense::click_and_drag());
    if drag_response.drag_started() {
        commands.push(Command::WindowDrag);
    }
    if drag_response.double_clicked() {
        commands.push(Command::WindowMaximize);
    }

    let update = update_box(ui_state);
    let widths = measure(ui, editor, update.as_ref());
    let content = if cfg!(target_os = "macos") {
        let content_left = (rect.left() + leading_inset(ui.ctx())).min(rect.right());
        let content_top = rect.top() + BAR_TOP + top_padding(ui.ctx());
        let content_bottom = (rect.bottom() - bottom_padding(ui.ctx())).max(content_top);
        Rect::from_min_max(Pos2::new(content_left, content_top), Pos2::new(rect.right(), content_bottom))
    } else {
        Rect::from_min_max(Pos2::new(rect.left(), rect.top() + BAR_TOP), rect.max)
    };
    let slots = if cfg!(target_os = "macos") {
        dock_macos(&widths, content.left(), content.right(), rect.center().x)
    } else {
        dock(&widths, content.left(), content.right())
    };

    if let Some(mut part) = part_ui(ui, "title-brand", &slots, Part::Brand, &widths, content, 0.0, 0.0) {
        brand(&mut part);
    }
    if let Some(mut part) = part_ui(ui, "title-menus", &slots, Part::Menus, &widths, content, 0.0, MENU_MARGIN_RIGHT) {
        menu_bar(&mut part, editor, ui_state, commands);
    }
    if let Some(mut part) = part_ui(ui, "title-file-box", &slots, Part::FileBox, &widths, content, FILE_BOX_MARGIN, FILE_BOX_MARGIN) {
        file_box(&mut part, editor, commands);
    }
    if let Some(mut part) = part_ui(ui, "title-undo", &slots, Part::Undo, &widths, content, UNDO_MARGIN, 0.0) {
        undo_menu(&mut part, editor, commands);
    }
    // Cocoa and the browser provide their own window controls.
    if let Some(mut part) = part_ui(ui, "title-window-buttons", &slots, Part::WindowButtons, &widths, content, 0.0, 0.0) {
        window_buttons(&mut part, commands);
    }
    if let Some(mut part) = part_ui(ui, "title-versions", &slots, Part::Versions, &widths, content, VERSION_MARGIN, VERSION_MARGIN) {
        // `bUpdate` once a newer version is known, then `bAbout` (About
        // Nerthus).
        if let Some(update) = &update {
            if version_button(&mut part, &update.text, update.color, update.enabled).clicked() {
                commands.push(Command::Tools(ToolCommand::CheckForUpdates));
            }
            part.add_space(BOX_GAP);
        }
        if version_button(&mut part, VERSION, theme::LABEL_DEFAULT, true).clicked() {
            commands.push(Command::Tools(ToolCommand::ShowAbout));
        }
    }
}

/// `bUpdate` once a newer version is known. While the update downloads,
/// and after, it counts it and can't be clicked.
struct UpdateBox {
    text: String,
    color: Color32,
    enabled: bool,
}

fn update_box(ui_state: &UiState) -> Option<UpdateBox> {
    if let Some(text) = ui_state.updates.download_label() {
        return Some(UpdateBox { text, color: theme::LABEL_DEFAULT, enabled: false });
    }
    let available = ui_state.updates.update_available();
    available.then(|| UpdateBox { text: "Update".to_string(), color: theme::ACCENT_PRIMARY, enabled: true })
}

/// The bar's parts in `MenuBar`'s order: those docked left, then those
/// docked right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Brand,
    Menus,
    FileBox,
    Undo,
    WindowButtons,
    Versions,
}

const LEFT_PARTS: [Part; 4] = [Part::Brand, Part::Menus, Part::FileBox, Part::Undo];
const RIGHT_PARTS: [Part; 2] = [Part::WindowButtons, Part::Versions];

/// How wide each part asks to be, its margins included; None for a part
/// the bar doesn't have.
#[derive(Clone, Debug)]
struct Widths {
    brand: f32,
    menus: f32,
    file_box: f32,
    undo: f32,
    /// None when Cocoa or the browser supplies the window controls.
    window_buttons: Option<f32>,
    versions: f32,
}

impl Widths {
    fn of(&self, part: Part) -> Option<f32> {
        match part {
            Part::Brand => Some(self.brand),
            Part::Menus => Some(self.menus),
            Part::FileBox => Some(self.file_box),
            Part::Undo => Some(self.undo),
            Part::WindowButtons => self.window_buttons,
            Part::Versions => Some(self.versions),
        }
    }
}

fn measure(ui: &Ui, editor: &Editor, update: Option<&UpdateBox>) -> Widths {
    let menus = if cfg!(target_os = "macos") {
        0.0
    } else {
        let items: f32 = MENU_NAMES.iter().map(|name| menu_slot(ui, name).x).sum();
        items + MENU_MARGIN_RIGHT
    };
    let version = version_size(ui, VERSION).x;
    let boxes = match update {
        Some(update) => version_size(ui, &update.text).x + BOX_GAP + version,
        None => version,
    };
    Widths {
        brand: brand_size(ui).x,
        menus,
        file_box: file_box_width(ui, editor) + 2.0 * FILE_BOX_MARGIN,
        undo: UNDO_MARGIN + icon_menu_width(ui),
        window_buttons: (!cfg!(any(target_os = "macos", target_arch = "wasm32"))).then_some(WINDOW_BUTTONS_WIDTH),
        versions: VERSION_MARGIN + boxes + VERSION_MARGIN,
    }
}

/// Icon menus fit the theme's fixed `Toolbar.ButtonSize` slot.
fn icon_menu_width(ui: &Ui) -> f32 {
    let width = toolbar_button(ui);
    if cfg!(target_os = "macos") {
        width.min(content_height(ui.ctx()) - 2.0)
    } else {
        width
    }
}

fn save_button_size(ui: &Ui) -> f32 {
    if cfg!(target_os = "macos") {
        SAVE_BUTTON.min(content_height(ui.ctx()) - 2.0)
    } else {
        SAVE_BUTTON
    }
}

/// Each part's slot (left and right edges) in a bar from `left` to
/// `right`, as `DockPanel` measures and arranges them: each part docked
/// left gets as much as it asks of what those before it left, then each
/// docked right likewise from the right.
fn dock(widths: &Widths, left: f32, right: f32) -> Vec<(Part, f32, f32)> {
    let mut slots = Vec::new();
    let mut taken_left = left;
    for part in LEFT_PARTS {
        let Some(asked) = widths.of(part) else { continue };
        let width = asked.min((right - taken_left).max(0.0));
        slots.push((part, taken_left, taken_left + width));
        taken_left += width;
    }
    let mut taken_right = right;
    for part in RIGHT_PARTS {
        let Some(asked) = widths.of(part) else { continue };
        let width = asked.min((taken_right - taken_left).max(0.0));
        slots.push((part, taken_right - width, taken_right));
        taken_right -= width;
    }
    slots
}

/// Keep the file box itself at the window's center. The brand and
/// version boxes give up space before they can cover it or its undo menu.
fn dock_macos(widths: &Widths, left: f32, right: f32, center: f32) -> Vec<(Part, f32, f32)> {
    let center = center.clamp(left, right);
    let half_room = (center - left).min(right - center);
    let file_width = widths.file_box.min(2.0 * half_room);
    let file_left = center - file_width / 2.0;
    let file_right = center + file_width / 2.0;
    let brand_right = (left + widths.brand).min(file_left);
    let undo_right = (file_right + widths.undo).min(right);
    let versions_left = (right - widths.versions).max(undo_right);

    vec![
        (Part::Brand, left, brand_right),
        (Part::FileBox, file_left, file_right),
        (Part::Undo, file_right, undo_right),
        (Part::Versions, versions_left, right),
    ]
}

/// A `Ui` for `part`: laid out whole from its slot's left past
/// `margin_left`, cut off at its slot less its margins. None when the bar
/// has no room (or no such part) for any of it.
#[allow(clippy::too_many_arguments)]
fn part_ui(ui: &mut Ui, salt: &str, slots: &[(Part, f32, f32)], part: Part, widths: &Widths, content: Rect, margin_left: f32, margin_right: f32) -> Option<Ui> {
    let &(_, left, right) = slots.iter().find(|(slotted, _, _)| *slotted == part)?;
    let asked = widths.of(part)?;
    let visible = Rect::from_x_y_ranges(left + margin_left..=right - margin_right, content.y_range());
    if visible.width() <= 0.0 {
        return None;
    }
    let layout_right = if cfg!(target_os = "macos") && part == Part::FileBox {
        right
    } else {
        left + asked
    };
    let whole = Rect::from_x_y_ranges(left + margin_left..=layout_right - margin_right, content.y_range());
    let builder = UiBuilder::new().id_salt(salt).max_rect(whole).layout(Layout::left_to_right(Align::Center));
    let mut part_ui = ui.new_child(builder);
    part_ui.set_clip_rect(ui.clip_rect().intersect(visible));
    part_ui.spacing_mut().item_spacing.x = 0.0;
    Some(part_ui)
}

/// The name button (`Button-Link`, padding 16, 6): the name in the header font.
fn brand(ui: &mut Ui) {
    let galley = brand_galley(ui);
    let size = brand_size(ui);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let position = Pos2::new(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::ACCENT_PRIMARY);
}

fn brand_galley(ui: &Ui) -> std::sync::Arc<egui::Galley> {
    let font_size = title_font_size(ui, 16.0);
    ui.painter().layout_no_wrap("NERTHUS".to_string(), theme::semibold(font_size), theme::ACCENT_PRIMARY)
}

fn brand_size(ui: &Ui) -> Vec2 {
    let galley = brand_galley(ui);
    let mut size = galley.size() + Vec2::new(32.0, 12.0);
    if cfg!(target_os = "macos") {
        size.y = size.y.min(content_height(ui.ctx()));
    }
    size
}

fn title_font_size(ui: &Ui, normal_size: f32) -> f32 {
    if cfg!(target_os = "macos") {
        normal_size.min((content_height(ui.ctx()) - 2.0) * 0.7)
    } else {
        normal_size
    }
}

const MENU_NAMES: [&str; 8] = ["File", "Edit", "Graph", "Node", "Preview", "Project", "Tools", "Help"];
/// Their access keys ("_File", ..., "P_review", ...).
const MENU_ACCESS_KEYS: [egui::Key; 8] = [egui::Key::F, egui::Key::E, egui::Key::G, egui::Key::N, egui::Key::R, egui::Key::P, egui::Key::T, egui::Key::H];

/// The access letter stays visible, including Preview's R, so the menu
/// can be found with Alt or while the bar is in menu mode.
fn menu_text(name: &str, access_key: egui::Key) -> egui::text::LayoutJob {
    let plain = egui::TextFormat::simple(theme::interface(11.0), theme::MENU_FOREGROUND);
    let mut marked = plain.clone();
    marked.underline = Stroke::new(1.0, theme::MENU_FOREGROUND);
    let uppercase = name.to_ascii_uppercase();
    let access_start = uppercase.find(access_key.name()).expect("a menu access key appears in its label");
    let access_end = access_start + access_key.name().len();
    let mut text = egui::text::LayoutJob::default();
    text.append(&name[..access_start], 0.0, plain.clone());
    text.append(&name[access_start..access_end], 0.0, marked);
    text.append(&name[access_end..], 0.0, plain);
    text
}

/// A top-level item's slot: at least 64 wide less its overlaps, its text
/// 8 + 5 in from each side.
fn menu_slot(ui: &Ui, name: &str) -> Vec2 {
    let text_width = ui.painter().layout_no_wrap(name.to_string(), theme::interface(11.0), Color32::WHITE).size().x;
    let item_width = (text_width + 2.0 * (8.0 + 5.0)).max(TOP_LEVEL_MIN_WIDTH);
    Vec2::new(item_width - 2.0 * TOP_LEVEL_OVERLAP, TOP_LEVEL_HEIGHT)
}

/// The main menu: each top-level item takes its slot, its text centred,
/// `Menu-TopLevel` under the pointer. `MenuBar` fills what it is given, so
/// it gets just its items' width.
fn menu_bar(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    let width: f32 = MENU_NAMES.iter().map(|name| menu_slot(ui, name).x).sum();
    let size = Vec2::new(width, TOP_LEVEL_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| menu_items(ui, editor, ui_state, commands));
}

fn menu_items(ui: &mut Ui, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    let modal = tool_commands::modal_open(ui.ctx(), ui_state);
    if modal {
        ui.disable();
    }
    egui::MenuBar::new().ui(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().button_padding = Vec2::ZERO;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.open.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        ui.visuals_mut().widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        ui.visuals_mut().widgets.open.corner_radius = egui::CornerRadius::same(4);
        // Each menu's popup and access key, for the keys to go between
        // them; in menu mode the item the keys are on looks as under the
        // pointer.
        let highlighted = menu_keys::bar_focus(ui.ctx());
        let mut popups = Vec::new();
        for (index, (name, access_key)) in MENU_NAMES.into_iter().zip(MENU_ACCESS_KEYS).enumerate() {
            let text = menu_text(name, access_key);
            let slot = menu_slot(ui, name);
            let lit = highlighted == Some(index);
            let mut button = egui::Button::new(text);
            button = button.min_size(slot);
            button = button.frame_when_inactive(lit);
            let inactive = ui.visuals().widgets.inactive;
            if lit {
                ui.visuals_mut().widgets.inactive = ui.visuals().widgets.hovered;
            }
            let menu = MenuButton::from_button(button).config(menu_config());
            let (response, _) = menu.ui(ui, |ui| {
                // Popup UIs are detached from the disabled bar. Leave
                // their commands undrawn until `bar` closes the roots.
                if !modal {
                    menu_rows(ui, |ui| menu_contents(ui, name, editor, ui_state, commands));
                }
            });
            ui.visuals_mut().widgets.inactive = inactive;
            popups.push((egui::Popup::default_response_id(&response), access_key));
        }
        menu_keys::bar(ui.ctx(), &popups, modal);
    });
}

fn menu_contents(ui: &mut Ui, name: &str, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    match name {
        "File" => menus::file_menu(ui, editor, commands),
        "Edit" => menus::edit_menu(ui, editor, ui_state, commands),
        "Graph" => menus::graph_menu(ui, editor, ui_state, commands),
        "Node" => menus::node_menu(ui, editor, ui_state.graph.tab, commands),
        "Preview" => menus::preview_menu(ui, editor, commands),
        "Project" => menus::project_menu(ui, commands),
        "Tools" => menus::tools_menu(ui, editor, commands),
        _ => menus::help_menu(ui, commands),
    }
}

/// The file name (monospace 11 bold) and the room it takes, at least 140.
fn file_name_galley(ui: &Ui, editor: &Editor, color: Color32) -> (std::sync::Arc<egui::Galley>, f32) {
    let font_size = title_font_size(ui, 11.0);
    let galley = ui.painter().layout_no_wrap(editor.document.file_name(), theme::monospace_bold(font_size), color);
    let width = galley.size().x.max(NAME_MIN_WIDTH);
    (galley, width)
}

/// The file box's width, its padding included: the resolution and region
/// menus, the name with its margins, save, save incremental and the
/// autosave menu, at least 180.
fn file_box_width(ui: &Ui, editor: &Editor) -> f32 {
    let icon_menu = icon_menu_width(ui);
    let regions = if crate::ui::regions::menus_visible(editor) { icon_menu } else { 0.0 };
    let (_, name_width) = file_name_galley(ui, editor, theme::LABEL_DEFAULT);
    let save_button = save_button_size(ui);
    let content = icon_menu + regions + name_width + 2.0 * NAME_MARGIN.x + save_button + 2.0 + save_button + 2.0 + icon_menu;
    (1.0 + content + 1.0).max(FILE_BOX_MIN_WIDTH)
}

/// The file box (`MinWidth 180`, padding 1, `Panel-Background`, corners
/// 3): the resolution and region menus, the file name (monospace 11 bold,
/// margin 8, 4, at least 140), save, save incremental and the autosave
/// menu.
#[cfg(not(target_os = "macos"))]
fn file_box(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_BACKGROUND;
    frame.corner_radius = egui::CornerRadius::same(3);
    frame.inner_margin = egui::Margin::same(1);
    frame.show(ui, |ui| {
        ui.set_min_width(FILE_BOX_MIN_WIDTH - 2.0);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            crate::ui::viewport_area::resolution_menu(ui, editor, commands);
            // `miRegionsPreview`, once the terrain has regions.
            if crate::ui::regions::menus_visible(editor) {
                let tooltip = crate::ui::regions::preview_tooltip(editor);
                icon_menu(ui, "artboard", &tooltip, Some(theme::ACCENT_GREEN), |ui| {
                    crate::ui::regions::regions_menu(ui, editor, commands, true);
                });
            }
            // `TryAutosave` turns the name red once the autosave interval
            // passed with unsaved changes; a save turns it back.
            let name_color = if editor.files.autosave.reminder { theme::ACCENT_MUTED_RED } else { theme::LABEL_DEFAULT };
            let (galley, name_width) = file_name_galley(ui, editor, name_color);
            let size = Vec2::new(name_width, galley.size().y) + 2.0 * NAME_MARGIN;
            let (name_rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().galley(Pos2::new(name_rect.left() + NAME_MARGIN.x, name_rect.center().y - galley.size().y / 2.0), galley, name_color);
            // `Button-IconOnly` (14 + padding 6), margin 0, 0, 2, 0.
            let button_size = Vec2::splat(SAVE_BUTTON);
            if icon_button_sized(ui, "floppy", "Save", None, button_size, 14.0).clicked() {
                commands.push(Command::Save);
            }
            ui.add_space(2.0);
            if icon_button_sized(ui, "copy-plus", "Save Incremental", None, button_size, 14.0).clicked() {
                commands.push(Command::File(FileCommand::SaveIncremental));
            }
            ui.add_space(2.0);
            icon_menu(ui, "clock-rotate-right", "Autosave", None, |ui| autosave_menu(ui, editor, commands));
        });
    });
}

#[cfg(target_os = "macos")]
fn file_box(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let icon_size = Vec2::splat(icon_menu_width(ui));
    let button_size = Vec2::splat(save_button_size(ui));
    let save_icon_size = 14.0_f32.min(button_size.y);
    let (_, natural_name_width) = file_name_galley(ui, editor, theme::LABEL_DEFAULT);
    let fixed_width = file_box_width(ui, editor) - natural_name_width;
    let name_room = (ui.available_width() - fixed_width).max(0.0);
    let minimum_width = if cfg!(target_os = "macos") {
        (FILE_BOX_MIN_WIDTH - 2.0).min((ui.available_width() - 2.0).max(0.0))
    } else {
        FILE_BOX_MIN_WIDTH - 2.0
    };
    let mut frame = egui::Frame::new();
    frame.fill = theme::PANEL_BACKGROUND;
    frame.corner_radius = egui::CornerRadius::same(3);
    frame.inner_margin = egui::Margin::same(1);
    frame.show(ui, |ui| {
        ui.set_min_width(minimum_width);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            crate::ui::viewport_area::resolution_menu_sized(ui, editor, commands, icon_size);
            // `miRegionsPreview`, once the terrain has regions.
            if crate::ui::regions::menus_visible(editor) {
                let tooltip = crate::ui::regions::preview_tooltip(editor);
                icon_menu_sized(ui, "artboard", &tooltip, Some(theme::ACCENT_GREEN), icon_size, |ui| {
                    crate::ui::regions::regions_menu(ui, editor, commands, true);
                });
            }
            // `TryAutosave` turns the name red once the autosave interval
            // passed with unsaved changes; a save turns it back.
            let name_color = if editor.files.autosave.reminder { theme::ACCENT_MUTED_RED } else { theme::LABEL_DEFAULT };
            let (mut galley, mut name_width) = file_name_galley(ui, editor, name_color);
            if cfg!(target_os = "macos") {
                name_width = name_width.min(name_room);
                if galley.size().x > name_width {
                    let mut text_job = galley.job.as_ref().clone();
                    text_job.wrap = egui::text::TextWrapping::truncate_at_width(name_width);
                    galley = ui.painter().layout_job(text_job);
                }
            }
            let mut size = Vec2::new(name_width, galley.size().y) + 2.0 * NAME_MARGIN;
            if cfg!(target_os = "macos") {
                size.y = size.y.min(icon_size.y);
            }
            let (name_rect, name_response) = ui.allocate_exact_size(size, Sense::hover());
            if galley.elided {
                name_response.on_hover_text(editor.document.file_name());
            }
            ui.painter().galley(Pos2::new(name_rect.left() + NAME_MARGIN.x, name_rect.center().y - galley.size().y / 2.0), galley, name_color);
            // `Button-IconOnly` (14 + padding 6), margin 0, 0, 2, 0.
            if icon_button_sized(ui, "floppy", "Save", None, button_size, save_icon_size).clicked() {
                commands.push(Command::Save);
            }
            ui.add_space(2.0);
            if icon_button_sized(ui, "copy-plus", "Save Incremental", None, button_size, save_icon_size).clicked() {
                commands.push(Command::File(FileCommand::SaveIncremental));
            }
            ui.add_space(2.0);
            icon_menu_sized(ui, "clock-rotate-right", "Autosave", None, icon_size, |ui| autosave_menu(ui, editor, commands));
        });
    });
}

/// The Autosave menu: the interval (`SetAutosaveInterval`; the one in use
/// checked), Reset Timer, this session's latest five autosaves to revert
/// to (`mnuAutosaveEntry1`…`5`, "Autosave Entry" and disabled when there
/// are fewer), Open Autosave… and Revert to Saved.
fn autosave_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    menu_caption(ui, "Remind me to save every");
    for (minutes, text, icon) in AUTOSAVE_CHOICES {
        let checked = editor.files.autosave.is_interval(minutes);
        if checked_icon_item(ui, checked, icon, None, text, "") {
            commands.push(Command::File(FileCommand::SetAutosaveInterval(minutes)));
        }
    }
    if menu_item(ui, Some("time-duration-off"), Some(theme::ACCENT_MUTED_RED), "Reset Timer", "").clicked() {
        commands.push(Command::File(FileCommand::ResetAutosaveTimer));
    }
    menu_separator(ui);
    menu_caption(ui, "Revert to");
    let entries = editor.files.autosave.menu_entries();
    for slot in 0..5 {
        let icon = format!("number-{}-small", slot + 1);
        match entries.get(slot) {
            Some((label, path)) => {
                if menu_item(ui, Some(&icon), None, label, "").clicked() {
                    commands.push(Command::File(FileCommand::OpenAutosaveEntry(path.clone())));
                }
            }
            None => {
                menu_item_enabled(ui, Some(&icon), None, "Autosave Entry", "", false);
            }
        }
    }
    menu_separator(ui);
    if menu_item(ui, Some("folder"), Some(theme::ACCENT_MUTED_GREEN), "Open Autosave...", "").clicked() {
        commands.push(Command::File(FileCommand::OpenAutosave));
    }
    menu_separator(ui);
    if menu_item(ui, Some("restore"), Some(theme::ACCENT_MUTED_RED), "Revert to Saved", "").clicked() {
        commands.push(Command::File(FileCommand::RevertToSaved));
    }
}

/// The undo button: its menu lists the edits that can be undone.
#[cfg(not(target_os = "macos"))]
fn undo_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    icon_menu(ui, "arrow-back-up", "Undo", None, |ui| {
        menu_style(ui);
        let history = editor.document.undo_history();
        if history.is_empty() {
            menu_caption(ui, "No undo history");
            return;
        }
        for (steps, item) in history.iter().enumerate() {
            if item.separator_before {
                menu_separator(ui);
            }
            if menu_item(ui, Some(item.icon), None, item.label, "").clicked() {
                for _ in 0..=steps {
                    commands.push(Command::Undo);
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn undo_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let icon_size = Vec2::splat(icon_menu_width(ui));
    icon_menu_sized(ui, "arrow-back-up", "Undo", None, icon_size, |ui| {
        menu_style(ui);
        let history = editor.document.undo_history();
        if history.is_empty() {
            menu_caption(ui, "No undo history");
            return;
        }
        for (steps, item) in history.iter().enumerate() {
            if item.separator_before {
                menu_separator(ui);
            }
            if menu_item(ui, Some(item.icon), None, item.label, "").clicked() {
                for _ in 0..=steps {
                    commands.push(Command::Undo);
                }
            }
        }
    });
}

/// `bAbout` and `bUpdate`: plain buttons (`Button`: padding 24, 6, at
/// least 87 wide) in 10px semibold.
fn version_button(ui: &mut Ui, text: &str, color: Color32, enabled: bool) -> egui::Response {
    let galley = version_galley(ui, text, color);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(version_size(ui, text), sense);
    let fill = if !enabled {
        theme::BUTTON_BACKGROUND
    } else if response.is_pointer_button_down_on() {
        theme::BUTTON_PRESSED
    } else if response.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON_BACKGROUND
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

fn version_galley(ui: &Ui, text: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    let font_size = title_font_size(ui, 10.0);
    ui.painter().layout_no_wrap(text.to_string(), theme::semibold(font_size), color)
}

fn version_size(ui: &Ui, text: &str) -> Vec2 {
    let text_size = version_galley(ui, text, Color32::WHITE).size();
    let mut size = Vec2::new((text_size.x + 48.0).max(87.0), text_size.y + 12.0);
    if cfg!(target_os = "macos") {
        size.y = size.y.min(content_height(ui.ctx()));
    }
    size
}

/// Minimize, maximize and close, left to right (`Button-System`:
/// Marlett's `0`, `1` and `r` with padding 10 and margins 2, at 40% until
/// hovered; close red).
fn window_buttons(ui: &mut Ui, commands: &mut Vec<Command>) {
    ui.add_space(2.0);
    if system_button(ui, SystemGlyph::Minimize).clicked() {
        commands.push(Command::WindowMinimize);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Maximize).clicked() {
        commands.push(Command::WindowMaximize);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Close).clicked() {
        commands.push(Command::Exit);
    }
    ui.add_space(2.0);
}

#[derive(Clone, Copy, PartialEq)]
enum SystemGlyph {
    Minimize,
    Maximize,
    Close,
}

fn system_button(ui: &mut Ui, glyph: SystemGlyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(29.0, 29.0), Sense::click());
    let hovered = response.hovered();
    let base = match glyph {
        SystemGlyph::Close => theme::ACCENT_RED,
        _ if hovered => theme::LABEL_EMPHASIS,
        _ => theme::LABEL_DEFAULT,
    };
    let color = if hovered { base } else { base.gamma_multiply(0.4) };
    let center = rect.center();
    let painter = ui.painter();
    match glyph {
        SystemGlyph::Minimize => {
            let bar = Rect::from_center_size(center + Vec2::new(0.0, 3.0), Vec2::new(8.0, 2.0));
            painter.rect_filled(bar, 0.0, color);
        }
        SystemGlyph::Maximize => {
            let frame = Rect::from_center_size(center, Vec2::splat(9.0));
            painter.rect_stroke(frame, 0.0, Stroke::new(1.0, color), StrokeKind::Inside);
            let top = Rect::from_min_size(frame.min, Vec2::new(frame.width(), 2.0));
            painter.rect_filled(top, 0.0, color);
        }
        SystemGlyph::Close => {
            let stroke = Stroke::new(1.5, color);
            let half = 4.0;
            painter.line_segment([center + Vec2::new(-half, -half), center + Vec2::new(half, half)], stroke);
            painter.line_segment([center + Vec2::new(-half, half), center + Vec2::new(half, -half)], stroke);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::document::Document;

    /// An example's name, the 66 and 100 character names that broke the
    /// bar at 1600 pixels, and a short one.
    const NAMES: [&str; 4] = [
        "Alpine Valley.terrain",
        "Alpine Valley with Erosion and Snow and Rivers - final v12.terrain",
        "Alpine Valley with Erosion and Snow and Rivers and Lakes and Glaciers and Roads - final final v12.terrain",
        "A.terrain",
    ];

    fn editor_named(name: &str) -> Editor {
        let mut document = Document::new();
        document.path = Some(PathBuf::from(name));
        Editor::new(document)
    }

    fn input(width: f32, events: Vec<egui::Event>) -> egui::RawInput {
        sized_input(Vec2::new(width, 600.0), events)
    }

    fn sized_input(screen: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        let screen = Rect::from_min_size(Pos2::ZERO, screen);
        egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }
    }

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set.
    fn themed_context(width: f32) -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context.run_ui(input(width, Vec::new()), |_| {}).drop_without_applying_deltas();
        context
    }

    fn measured(editor: &Editor, update: Option<&UpdateBox>) -> Widths {
        let context = themed_context(1000.0);
        let mut widths = None;
        context.run_ui(input(1000.0, Vec::new()), |ui| widths = Some(measure(ui, editor, update))).drop_without_applying_deltas();
        widths.unwrap()
    }

    fn update() -> UpdateBox {
        UpdateBox { text: "Downloading 42.0%...".to_string(), color: theme::LABEL_DEFAULT, enabled: false }
    }

    fn slot(slots: &[(Part, f32, f32)], part: Part) -> (f32, f32) {
        let &(_, left, right) = slots.iter().find(|(slotted, _, _)| *slotted == part).expect("the part has a slot");
        (left, right)
    }

    #[test]
    fn the_parts_dock_left_then_right_as_a_dock_panel_does() {
        let update = update();
        for name in NAMES {
            for update in [None, Some(&update)] {
                let widths = measured(&editor_named(name), update);
                let asked: f32 = LEFT_PARTS.iter().chain(&RIGHT_PARTS).filter_map(|part| widths.of(*part)).sum();
                for tenth in 1000..=40000 {
                    let width = tenth as f32 / 10.0;
                    let slots = dock(&widths, 0.0, width);
                    // Docked left: one after another from the left edge,
                    // each as wide as it asks or what is left.
                    let mut left = 0.0;
                    for part in LEFT_PARTS {
                        let Some(asked) = widths.of(part) else { continue };
                        let (start, end) = slot(&slots, part);
                        assert_eq!(start, left, "{name}: {part:?} at {width}");
                        let room = (width - left).max(0.0);
                        assert_eq!(end - start, asked.min(room), "{name}: {part:?} at {width}");
                        left = end;
                    }
                    // Docked right: one before another from the right edge,
                    // out of what the left left them.
                    let mut right = width;
                    for part in RIGHT_PARTS {
                        let Some(asked) = widths.of(part) else { continue };
                        let (start, end) = slot(&slots, part);
                        assert_eq!(end, right, "{name}: {part:?} at {width}");
                        let room = (right - left).max(0.0);
                        assert_eq!(end - start, asked.min(room), "{name}: {part:?} at {width}");
                        right = start;
                    }
                    assert!(right >= left - 0.001, "{name}: the parts overlap at {width}");
                    if width >= asked {
                        for (part, start, end) in &slots {
                            assert_eq!(end - start, widths.of(*part).unwrap(), "{name}: {part:?} is cut at {width}");
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_centers_the_file_box_separately_from_undo_and_the_edge_controls() {
        let update = update();
        for name in NAMES {
            for update in [None, Some(&update)] {
                let widths = measured(&editor_named(name), update);
                for origin in [-250.0_f32, 0.0, 125.0] {
                    for width in [0.0, 40.0, 80.0, 160.0, 320.0, 480.0, 800.0, 1600.0, 2400.0] {
                        let right = origin + width;
                        let left = (origin + 80.0).min(right);
                        let center = origin + width / 2.0;
                        let slots = dock_macos(&widths, left, right, center);
                        let (file_left, file_right) = slot(&slots, Part::FileBox);
                        let actual_center = (file_left + file_right) / 2.0;
                        assert!((actual_center - center.clamp(left, right)).abs() < 0.001, "{name} at {width}");
                        assert_eq!(slot(&slots, Part::Undo).0, file_right);
                        assert_eq!(slot(&slots, Part::Brand).0, left);
                        assert_eq!(slot(&slots, Part::Versions).1, right);

                        let mut previous_end = left;
                        for (part, start, end) in slots {
                            assert!(start >= previous_end, "{part:?} overlaps another part at {width}");
                            assert!(end >= start && end <= right, "{part:?} leaves its safe area at {width}");
                            previous_end = end;
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_draws_the_file_box_at_the_window_center_even_with_long_names() {
        let screen_width = 3000.0;
        let context = themed_context(screen_width);
        let ui_state = UiState::default();
        let mut saw_elided_name = false;
        for name in NAMES {
            let editor = editor_named(name);
            for zoom in [0.5, 1.0, 1.5, 3.0] {
                context.set_zoom_factor(zoom);
                for width in [480.0, 800.0, 1600.0, 2400.0] {
                    for origin in [0.0, 125.0] {
                        let mut commands = Vec::new();
                        let mut expected_center = 0.0;
                        let output = context.run_ui(input(screen_width, Vec::new()), |ui| {
                            let rect = Rect::from_min_size(Pos2::new(origin, 0.0), Vec2::new(width, height(ui.ctx())));
                            expected_center = rect.center().x;
                            show(ui, rect, &editor, &ui_state, &mut commands);
                        });
                        let frames: Vec<_> = output.shapes.iter().filter_map(|shape| {
                            match &shape.shape {
                                egui::Shape::Rect(frame) if frame.fill == theme::PANEL_BACKGROUND => Some(frame),
                                _ => None,
                            }
                        }).collect();
                        assert_eq!(frames.len(), 1, "{name} at {width}, zoom {zoom}");
                        let actual_center = frames[0].rect.center().x;
                        let rounding = 4.0 * egui::emath::GUI_ROUNDING;
                        assert!((actual_center - expected_center).abs() <= rounding, "{name} at {width}, zoom {zoom}: {} != {expected_center}", actual_center);
                        let elided = output.shapes.iter().any(|shape| {
                            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.elided)
                        });
                        saw_elided_name |= elided;
                        assert!(commands.is_empty());
                        output.drop_without_applying_deltas();
                    }
                }
            }
        }
        assert!(saw_elided_name, "long filenames must shorten to keep their component centered");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_version_boxes_give_way_first_then_the_window_buttons() {
        let widths = measured(&editor_named(NAMES[0]), None);
        let left: f32 = LEFT_PARTS.iter().filter_map(|part| widths.of(*part)).sum();
        let buttons = widths.window_buttons.unwrap();
        // Room for all but half the version boxes: they are cut, the rest
        // whole.
        let slots = dock(&widths, 0.0, left + buttons + widths.versions / 2.0);
        let (start, end) = slot(&slots, Part::Versions);
        assert!((end - start - widths.versions / 2.0).abs() < 0.001);
        let (start, end) = slot(&slots, Part::WindowButtons);
        assert_eq!(end - start, buttons);
        // Room for half the window buttons: the version boxes have none.
        let slots = dock(&widths, 0.0, left + buttons / 2.0);
        let (start, end) = slot(&slots, Part::Versions);
        assert_eq!(end - start, 0.0);
        let (start, end) = slot(&slots, Part::WindowButtons);
        assert!((end - start - buttons / 2.0).abs() < 0.001);
        // Narrower than the left parts: undo and then the file box are cut.
        let slots = dock(&widths, 0.0, left - widths.undo - 50.0);
        let (start, end) = slot(&slots, Part::Undo);
        assert_eq!(end - start, 0.0);
        let (start, end) = slot(&slots, Part::FileBox);
        assert!((end - start - (widths.file_box - 50.0)).abs() < 0.001);
    }

    #[test]
    fn the_measured_widths_are_the_drawn_ones() {
        let editor = editor_named(NAMES[0]);
        let ui_state = UiState::default();
        let widths = measured(&editor, None);
        let context = themed_context(1000.0);
        let mut commands = Vec::new();
        let output = context.run_ui(input(1000.0, Vec::new()), |ui| {
            let mut row = ui.new_child(UiBuilder::new().layout(Layout::left_to_right(Align::Center)));
            row.spacing_mut().item_spacing.x = 0.0;
            let mut drawn = |draw: &mut dyn FnMut(&mut Ui)| {
                let start = row.cursor().min.x;
                draw(&mut row);
                row.cursor().min.x - start
            };
            assert_eq!(drawn(&mut |ui| brand(ui)), widths.brand);
            if !cfg!(target_os = "macos") {
                assert_eq!(drawn(&mut |ui| menu_bar(ui, &editor, &ui_state, &mut commands)), widths.menus - MENU_MARGIN_RIGHT);
            }
            assert_eq!(drawn(&mut |ui| file_box(ui, &editor, &mut commands)), widths.file_box - 2.0 * FILE_BOX_MARGIN);
            assert_eq!(drawn(&mut |ui| undo_menu(ui, &editor, &mut commands)), widths.undo - UNDO_MARGIN);
            let version = drawn(&mut |ui| {
                version_button(ui, VERSION, theme::LABEL_DEFAULT, true);
            });
            assert_eq!(version, widths.versions - 2.0 * VERSION_MARGIN);
            if let Some(buttons) = widths.window_buttons {
                assert_eq!(drawn(&mut |ui| window_buttons(ui, &mut commands)), buttons);
            }
        });
        output.drop_without_applying_deltas();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn measured_native_heights_follow_interface_zoom_and_survive_full_screen() {
        let context = egui::Context::default();
        assert_eq!(native_height(&context), DEFAULT_NATIVE_HEIGHT);
        for measured_height in [22.0, 28.0, 32.0] {
            record_native_height(&context, measured_height);
            for zoom in [0.5, 1.0, 1.2, 1.5, 2.0, 3.0] {
                context.set_zoom_factor(zoom);
                let output = context.run_ui(input(1000.0, Vec::new()), |ui| {
                    assert_eq!(native_height(ui.ctx()), measured_height);
                    let padded_height = measured_height + MACOS_VERTICAL_PADDING;
                    assert!((height(ui.ctx()) * zoom - padded_height).abs() < 0.001);
                    assert!((content_height(ui.ctx()) * zoom - measured_height).abs() < 0.001);
                    assert!((leading_inset(ui.ctx()) * zoom - TRAFFIC_LIGHTS_WIDTH).abs() < 0.001);
                });
                output.drop_without_applying_deltas();
            }
        }

        record_native_height(&context, 28.0);
        for unavailable_height in [0.0, -1.0, 9.0, 81.0, f32::NAN, f32::INFINITY] {
            record_native_height(&context, unavailable_height);
            assert_eq!(native_height(&context), 28.0);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_chrome_keeps_its_size_and_space_at_every_interface_scale() {
        let editor = editor_named(NAMES[0]);
        let ui_state = UiState::default();
        let width = 1600.0;
        let context = themed_context(width);
        for zoom in [0.5, 1.0, 1.2, 1.5, 2.0, 3.0] {
            context.set_zoom_factor(zoom);
            let mut commands = Vec::new();
            let output = context.run_ui(input(width, Vec::new()), |ui| {
                let bar_height = height(ui.ctx());
                let inset = leading_inset(ui.ctx());
                assert!((bar_height * zoom - HEIGHT).abs() < 0.001);
                assert!((inset * zoom - TRAFFIC_LIGHTS_WIDTH).abs() < 0.001);

                let widths = measure(ui, &editor, None);
                assert_eq!(widths.menus, 0.0);
                assert_eq!(widths.window_buttons, None);
                let slots = dock(&widths, inset, width);
                assert_eq!(slot(&slots, Part::Brand).0, inset);

                let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, bar_height));
                show(ui, rect, &editor, &ui_state, &mut commands);
            });
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    assert!(!MENU_NAMES.contains(&text.galley.text()), "an inline menu is drawn at {zoom}");
                }
            }
            assert!(commands.is_empty());
            output.drop_without_applying_deltas();
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_title_controls_fit_inside_the_compact_bar_at_every_scale() {
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        let parts = [
            (Part::Brand, 0.0, 0.0),
            (Part::FileBox, FILE_BOX_MARGIN, FILE_BOX_MARGIN),
            (Part::Undo, UNDO_MARGIN, 0.0),
            (Part::Versions, VERSION_MARGIN, VERSION_MARGIN),
        ];
        for measured_height in [22.0, 28.0, 32.0] {
            record_native_height(&context, measured_height);
            for zoom in [0.5, 1.0, 1.2, 1.5, 2.0, 3.0] {
                context.set_zoom_factor(zoom);
                let mut commands = Vec::new();
                let output = context.run_ui(input(width, Vec::new()), |ui| {
                    let bar_height = height(ui.ctx());
                    let widths = measure(ui, &editor, None);
                    let top = top_padding(ui.ctx());
                    let bottom = bottom_padding(ui.ctx());
                    let content_min = Pos2::new(leading_inset(ui.ctx()), top);
                    let content_max = Pos2::new(width, bar_height - bottom);
                    let content = Rect::from_min_max(content_min, content_max);
                    let slots = dock(&widths, content.left(), content.right());
                    for (part, margin_left, margin_right) in parts {
                        let salt = format!("compact-{part:?}");
                        let mut part_ui = part_ui(ui, &salt, &slots, part, &widths, content, margin_left, margin_right).unwrap();
                        match part {
                            Part::Brand => brand(&mut part_ui),
                            Part::FileBox => file_box(&mut part_ui, &editor, &mut commands),
                            Part::Undo => undo_menu(&mut part_ui, &editor, &mut commands),
                            Part::Versions => { version_button(&mut part_ui, VERSION, theme::LABEL_DEFAULT, true); }
                            _ => unreachable!(),
                        }
                        let used = part_ui.min_rect();
                        // egui snaps layout rectangles to 1/32 of a UI
                        // point, while native height / zoom can be fractional.
                        let rounding = egui::emath::GUI_ROUNDING;
                        assert!(used.top() >= top - rounding, "{part:?} loses its top margin at {zoom}: {used:?}");
                        assert!(used.bottom() <= bar_height - bottom + rounding, "{part:?} loses its bottom margin in the {measured_height}-point bar at {zoom}: {used:?}");
                    }
                });
                output.drop_without_applying_deltas();
            }
        }
    }

    /// Clicks at `at`: the pointer moves there, presses and lets go.
    #[cfg(not(target_os = "macos"))]
    fn click(context: &egui::Context, width: f32, editor: &Editor, ui_state: &UiState, at: Pos2) -> Vec<Command> {
        click_in(context, Vec2::new(width, 600.0), editor, ui_state, at)
    }

    /// Clicks at `at` on a window of `screen`.
    #[cfg(not(target_os = "macos"))]
    fn click_in(context: &egui::Context, screen: Vec2, editor: &Editor, ui_state: &UiState, at: Pos2) -> Vec<Command> {
        let button = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut commands = Vec::new();
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)], Vec::new()] {
            commands.extend(bar_frame(context, screen, editor, ui_state, events));
        }
        commands
    }

    /// A frame of the bar across a window of `screen`.
    #[cfg(not(target_os = "macos"))]
    fn bar_frame(context: &egui::Context, screen: Vec2, editor: &Editor, ui_state: &UiState, events: Vec<egui::Event>) -> Vec<Command> {
        let mut commands = Vec::new();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(screen.x, height(context)));
        let output = context.run_ui(sized_input(screen, events), |ui| show(ui, rect, editor, ui_state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    /// `bar_frame` with `events`, then the frames after as the app draws
    /// them, for the menus to settle.
    #[cfg(not(target_os = "macos"))]
    fn settle(context: &egui::Context, screen: Vec2, editor: &Editor, ui_state: &UiState, events: Vec<egui::Event>) {
        bar_frame(context, screen, editor, ui_state, events);
        for _ in 0..3 {
            bar_frame(context, screen, editor, ui_state, Vec::new());
        }
    }

    /// The menus open, as their areas were laid out: a menu before its
    /// submenu.
    #[cfg(not(target_os = "macos"))]
    fn open_menus(context: &egui::Context) -> Vec<Rect> {
        context.memory(|memory| {
            let visible = memory.areas().visible_layer_ids();
            let open = memory.layer_ids().filter(|layer| layer.order == egui::Order::Foreground && visible.contains(layer));
            open.filter_map(|layer| memory.area_rect(layer.id)).collect()
        })
    }

    /// The middle of the `index`th menu's title in the bar.
    #[cfg(not(target_os = "macos"))]
    fn title_at(widths: &Widths, ui: &Ui, index: usize) -> Pos2 {
        let before: f32 = MENU_NAMES[..index].iter().map(|name| menu_slot(ui, name).x).sum();
        let x = widths.brand + before + menu_slot(ui, MENU_NAMES[index]).x / 2.0;
        Pos2::new(x, BAR_TOP + (HEIGHT - BAR_TOP) / 2.0)
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_menu_taller_than_the_window_opens_under_the_bar_and_scrolls() {
        // 400 high (480 pixels at the default 120%): Preview is taller
        // than the room under the bar.
        let screen = Vec2::new(1000.0, 400.0);
        let editor = editor_named(NAMES[0]);
        let ui_state = UiState::default();
        let widths = measured(&editor, None);
        let preview = MENU_NAMES.iter().position(|name| *name == "Preview").unwrap();
        let context = themed_context(screen.x);
        let mut title = Pos2::ZERO;
        context.run_ui(input(screen.x, Vec::new()), |ui| title = title_at(&widths, ui, preview)).drop_without_applying_deltas();
        let commands = click_in(&context, screen, &editor, &ui_state, title);
        assert!(commands.is_empty(), "opening Preview gave {commands:?}");
        settle(&context, screen, &editor, &ui_state, Vec::new());
        let menus = open_menus(&context);
        let [menu] = menus[..] else { panic!("Preview does not open alone: {menus:?}") };
        let title_bottom = title.y + TOP_LEVEL_HEIGHT / 2.0;
        assert!(menu.top() >= title_bottom - 0.5, "the menu at {menu:?} covers the bar, its title ending at {title_bottom}");
        assert!(menu.bottom() <= screen.y + 0.5, "the menu at {menu:?} runs past the window's bottom");
        let over = Pos2::new(menu.left() + 40.0, menu.center().y);
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        bar_frame(&context, screen, &editor, &ui_state, vec![egui::Event::PointerMoved(over)]);
        bar_frame(&context, screen, &editor, &ui_state, vec![wheel]);
        for _ in 0..60 {
            bar_frame(&context, screen, &editor, &ui_state, Vec::new());
        }
        let last = Pos2::new(menu.left() + 40.0, menu.bottom() - 6.0);
        let commands = click_in(&context, screen, &editor, &ui_state, last);
        assert_eq!(commands, [Command::SetRenderMode(crate::editor::RenderMode::Realistic)]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn undo_and_close_answer_clicks_while_they_show() {
        let ui_state = UiState::default();
        let y = BAR_TOP + (HEIGHT - BAR_TOP) / 2.0;
        for name in NAMES {
            let editor = editor_named(name);
            let widths = measured(&editor, None);
            let left: f32 = LEFT_PARTS.iter().filter_map(|part| widths.of(*part)).sum();
            let whole = left + widths.window_buttons.unwrap() + widths.versions;
            for width in [whole + 400.0, whole, left + widths.window_buttons.unwrap()] {
                let slots = dock(&widths, 0.0, width);
                let (undo_left, _) = slot(&slots, Part::Undo);
                let context = themed_context(width);
                let commands = click(&context, width, &editor, &ui_state, Pos2::new(undo_left + UNDO_MARGIN + 10.0, y));
                assert!(egui::Popup::is_any_open(&context), "{name}: undo's menu did not open at {width}");
                assert!(commands.is_empty(), "{name}: clicking undo at {width} gave {commands:?}");

                let context = themed_context(width);
                let close = Pos2::new(width - 2.0 - SYSTEM_BUTTON / 2.0, y);
                let commands = click(&context, width, &editor, &ui_state, close);
                assert_eq!(commands, [Command::Exit], "{name}: close at {width}");
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_window_buttons_slot_too_narrow_cuts_off_close() {
        // The buttons go left to right, minimize first, so a slot short of
        // them loses close: a click where it would be does nothing.
        let ui_state = UiState::default();
        let editor = editor_named(NAMES[0]);
        let widths = measured(&editor, None);
        let left: f32 = LEFT_PARTS.iter().filter_map(|part| widths.of(*part)).sum();
        let width = left + 2.0 + SYSTEM_BUTTON + 2.0 + 10.0;
        let y = BAR_TOP + (HEIGHT - BAR_TOP) / 2.0;
        let context = themed_context(width);
        let commands = click(&context, width, &editor, &ui_state, Pos2::new(width - 4.0, y));
        assert!(!commands.contains(&Command::Exit), "close took a click at {width}: {commands:?}");
        let context = themed_context(width);
        let minimize = Pos2::new(left + 2.0 + SYSTEM_BUTTON / 2.0, y);
        let commands = click(&context, width, &editor, &ui_state, minimize);
        assert_eq!(commands, [Command::WindowMinimize]);
    }

    /// Frames of the bar as the app draws it, the menus taking their keys
    /// first: one for each of `keys` (held with `modifiers`), each
    /// followed by two without.
    #[cfg(not(target_os = "macos"))]
    fn keyed_with(context: &egui::Context, width: f32, editor: &Editor, keys: &[egui::Key], modifiers: egui::Modifiers) -> Vec<Command> {
        let ui_state = UiState::default();
        keyed_in_state(context, width, editor, &ui_state, keys, modifiers)
    }

    #[cfg(not(target_os = "macos"))]
    fn keyed_in_state(context: &egui::Context, width: f32, editor: &Editor, ui_state: &UiState, keys: &[egui::Key], modifiers: egui::Modifiers) -> Vec<Command> {
        let mut commands = Vec::new();
        let pressed = |key: egui::Key| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        for events in keys.iter().flat_map(|key| [pressed(*key), Vec::new(), Vec::new()]) {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, HEIGHT));
            let output = context.run_ui(input(width, events), |ui| {
                menu_keys::begin(ui.ctx());
                show(ui, rect, editor, ui_state, &mut commands);
            });
            output.drop_without_applying_deltas();
        }
        commands
    }

    #[cfg(not(target_os = "macos"))]
    fn keyed(context: &egui::Context, width: f32, editor: &Editor, keys: &[egui::Key]) -> Vec<Command> {
        keyed_with(context, width, editor, keys, egui::Modifiers::NONE)
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn move_to_tab_excludes_the_shown_tab_for_notes_and_groups() {
        use egui::Key::{ArrowRight, ArrowUp, End, Enter, N};

        for select_group in [false, true] {
            let mut editor = editor_named(NAMES[0]);
            let mut ui_state = UiState::default();
            let tab = editor.document.add_graph_tab();
            ui_state.graph.tab = tab;
            if select_group {
                let group = editor.document.add_group(&[], [0.0, 0.0], [200.0, 200.0], tab);
                ui_state.graph.selected_groups = vec![group];
            } else {
                let note = editor.document.add_note([0.0, 0.0], tab);
                ui_state.graph.selected_note = Some(note);
            }

            let width = 1600.0;
            let context = themed_context(width);
            keyed_in_state(&context, width, &editor, &ui_state, &[N], egui::Modifiers::ALT);
            // Move to Tab is just before Presets; its first target must
            // be Graph 1 when a note or group on Graph 2 is selected.
            let commands = keyed_in_state(&context, width, &editor, &ui_state, &[End, ArrowUp, ArrowRight, Enter], egui::Modifiers::NONE);
            let expected = Command::Graph(crate::ui::graph_commands::GraphCommand::MoveToTab(crate::document::TabTarget::Existing(0)));
            assert_eq!(commands, [expected], "group selected: {select_group}");
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn setup_and_help_windows_keep_f10_and_alt_access_keys() {
        use egui::Key::{ArrowDown, Enter, F, F10};

        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        for window in ["setup", "whats-new", "shortcuts", "about", "update"] {
            let mut ui_state = UiState::default();
            match window {
                "setup" => ui_state.onboard = Some(crate::ui::onboard::Onboard::open(true)),
                "whats-new" => ui_state.help.whats_new = Some(0),
                "shortcuts" => ui_state.help.shortcuts = true,
                "about" => ui_state.help.about = true,
                _ => ui_state.help.update = Some(crate::ui::help_windows::UpdateDialog::sample()),
            }
            let context = themed_context(width);
            let commands = keyed_in_state(&context, width, &editor, &ui_state, &[F10, ArrowDown, Enter], egui::Modifiers::NONE);
            assert!(commands.is_empty(), "{window}: {commands:?}");
            let commands = keyed_in_state(&context, width, &editor, &ui_state, &[F], egui::Modifiers::ALT);
            assert!(commands.is_empty(), "{window}: {commands:?}");
            assert!(!egui::Popup::is_any_open(&context), "{window}");
            assert_eq!(menu_keys::bar_focus(&context), None, "{window}");
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_modal_window_cancels_menu_mode_and_an_already_open_menu() {
        use egui::Key::{ArrowDown, Enter, F10};

        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        for open_menu in [false, true] {
            let context = themed_context(width);
            keyed(&context, width, &editor, &[F10]);
            if open_menu {
                keyed(&context, width, &editor, &[Enter]);
                assert!(egui::Popup::is_any_open(&context));
            }
            let mut ui_state = UiState::default();
            ui_state.help.whats_new = Some(0);
            let commands = keyed_in_state(&context, width, &editor, &ui_state, &[Enter, ArrowDown, Enter], egui::Modifiers::NONE);
            assert!(commands.is_empty(), "menu open: {open_menu}: {commands:?}");
            assert!(!egui::Popup::is_any_open(&context));
            assert!(!menu_keys::active(&context));
            assert_eq!(menu_keys::bar_focus(&context), None);
            ui_state.help.whats_new = None;
            let commands = keyed_in_state(&context, width, &editor, &ui_state, &[ArrowDown, Enter], egui::Modifiers::NONE);
            assert!(commands.is_empty());
            assert!(!egui::Popup::is_any_open(&context));
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn f10_highlights_file_and_the_keys_step_between_and_open_the_menus() {
        use egui::Key::{ArrowDown, ArrowLeft, ArrowRight, Enter, Escape, F10};
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        // F10 puts the bar in menu mode, File highlighted but not open;
        // Enter opens it on its first row, and Enter again chooses that.
        assert!(keyed(&context, width, &editor, &[F10]).is_empty());
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), Some(0));
        assert_eq!(keyed(&context, width, &editor, &[Enter, Enter]), [Command::NewProject]);
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), None);
        // Right twice highlights Graph; Down opens it on New Graph Tab.
        assert_eq!(keyed(&context, width, &editor, &[F10, ArrowRight, ArrowRight, ArrowDown, Enter]), [Command::NewGraphTab]);
        // Left from File goes round to Help.
        let help = Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string()));
        assert_eq!(keyed(&context, width, &editor, &[F10, ArrowLeft, Enter, Enter]), [help]);
        // Escape out of an open menu comes back to its item, and again
        // leaves menu mode.
        keyed(&context, width, &editor, &[F10, Enter]);
        assert!(egui::Popup::is_any_open(&context));
        keyed(&context, width, &editor, &[Escape]);
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), Some(0));
        keyed(&context, width, &editor, &[Escape]);
        assert_eq!(menu_keys::bar_focus(&context), None);
        // F10 with a menu open closes it and leaves menu mode.
        keyed(&context, width, &editor, &[F10, Enter, F10]);
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn alt_and_an_access_key_open_its_menu() {
        use egui::Key::{Enter, F, H};
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        // Alt+F opens File on its first row; Alt+H, Help.
        keyed_with(&context, width, &editor, &[F], egui::Modifiers::ALT);
        assert!(egui::Popup::is_any_open(&context));
        assert_eq!(keyed(&context, width, &editor, &[Enter]), [Command::NewProject]);
        keyed_with(&context, width, &editor, &[H], egui::Modifiers::ALT);
        let help = Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string()));
        assert_eq!(keyed(&context, width, &editor, &[Enter]), [help]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn access_keys_switch_an_open_menu_and_choose_from_menu_mode() {
        use egui::Key::{Enter, F, F10, G, H};
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        keyed_with(&context, width, &editor, &[F], egui::Modifiers::ALT);
        assert!(context.egui_wants_keyboard_input());
        keyed_with(&context, width, &editor, &[H], egui::Modifiers::ALT);
        let help = Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string()));
        assert_eq!(keyed(&context, width, &editor, &[Enter]), [help]);
        assert_eq!(keyed(&context, width, &editor, &[F10, G, Enter]), [Command::NewGraphTab]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn alt_access_keys_leave_a_focused_text_field_for_the_menu() {
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        let mut text = "node name".to_string();
        context.run_ui(input(width, Vec::new()), |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        }).drop_without_applying_deltas();
        assert!(context.text_edit_focused());
        keyed_with(&context, width, &editor, &[egui::Key::F], egui::Modifiers::ALT);
        assert_eq!(keyed(&context, width, &editor, &[egui::Key::Enter]), [Command::NewProject]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn ctrl_alt_access_letters_do_not_open_or_leave_menu_mode() {
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        let alt_gr = egui::Modifiers::CTRL | egui::Modifiers::ALT;
        keyed_with(&context, width, &editor, &[egui::Key::F], alt_gr);
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), None);
        keyed(&context, width, &editor, &[egui::Key::F10]);
        keyed_with(&context, width, &editor, &[egui::Key::G], alt_gr);
        assert!(!egui::Popup::is_any_open(&context));
        assert_eq!(menu_keys::bar_focus(&context), Some(0));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn preview_uses_alt_r_and_project_uses_alt_p() {
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        keyed_with(&context, width, &editor, &[egui::Key::R], egui::Modifiers::ALT);
        assert_eq!(keyed(&context, width, &editor, &[egui::Key::Enter]), [Command::SetPreviewResolution(512)]);
        keyed_with(&context, width, &editor, &[egui::Key::P], egui::Modifiers::ALT);
        let project_command = if cfg!(target_arch = "wasm32") { Command::BuildSettings } else { Command::BuildAndExport };
        assert_eq!(keyed(&context, width, &editor, &[egui::Key::Enter]), [project_command]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn each_menu_draws_its_access_letter_underlined() {
        let editor = editor_named(NAMES[0]);
        let ui_state = UiState::default();
        let width = 1600.0;
        let context = themed_context(width);
        let mut commands = Vec::new();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, HEIGHT));
        let output = context.run_ui(input(width, Vec::new()), |ui| {
            show(ui, rect, &editor, &ui_state, &mut commands);
        });
        for (name, access_key) in MENU_NAMES.into_iter().zip(MENU_ACCESS_KEYS) {
            let text = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text() == name => Some(&shape.galley.job),
                _ => None,
            }).expect("the menu label is drawn");
            let mut underlined = String::new();
            for section in &text.sections {
                if section.format.underline.width > 0.0 {
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    underlined.push_str(&text.text[range]);
                }
            }
            assert_eq!(underlined.to_ascii_uppercase(), access_key.name(), "{name}");
        }
        output.drop_without_applying_deltas();
    }
}
