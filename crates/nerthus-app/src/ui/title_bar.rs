//! The window's top bar (`Shell` `MenuBar`, 6 down): the brand, the main
//! menu, the file box (preview resolution and region, file name, save
//! buttons, autosave), the undo history, the version and the window
//! buttons. Empty space drags the window.
//!
//! The version box, `bUpdate` and the window buttons get their width at
//! the right first; the rest has what is left. As that runs out, the
//! version box goes, then `bUpdate`, then the file name is elided, then
//! the menus fold into one from Help back to File, then the brand goes.
//! Undo and the window buttons always stay. F10 opens the main menu
//! (`menu_keys`).

use egui::containers::menu::MenuButton;
use egui::{Align, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::elided_galley;
use crate::ui::{menu_keys, menus};
use crate::ui::tool_commands::ToolCommand;
use crate::files::{FileCommand, AUTOSAVE_CHOICES};
use crate::ui::widgets::{checked_icon_item, icon_button_sized, icon_menu, menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style, submenu, toolbar_button};
use crate::ui::UiState;

pub const HEIGHT: f32 = 36.0;
/// `MenuBar`'s top margin.
const BAR_TOP: f32 = 6.0;
/// A top-level menu item: at least 64 wide, overlapping its neighbours by
/// 5 (`Margin -5,0,-5,0`), its text 8 + 5 in each side.
const TOP_LEVEL_MIN_WIDTH: f32 = 64.0;
const TOP_LEVEL_OVERLAP: f32 = 5.0;
const TOP_LEVEL_HEIGHT: f32 = 22.0;
/// The menu holding the menus the bar has no room for: its icon 8 in
/// from each side.
const OVERFLOW_WIDTH: f32 = icons::DEFAULT_SIZE + 2.0 * 8.0;
/// Before the menus once the brand is gone, so they start 16 in as its
/// name did.
const LEAD_WITHOUT_BRAND: f32 = 8.0;
/// `Menu` margin 0, 0, 12, 0 and the file box's own margin 6.
const MENU_GAP: f32 = 12.0 + 6.0;
/// The file box's margin 6 and undo's 5.
const UNDO_GAP: f32 = 6.0 + 5.0;
/// The file box's `MinWidth`.
const FILE_BOX_MIN_WIDTH: f32 = 180.0;
/// The file name's room while anything else can still give way, and the
/// least it is elided to once nothing can.
const NAME_MIN_WIDTH: f32 = 140.0;
const NAME_LEAST_WIDTH: f32 = 48.0;
/// The file name's margin 8, 4.
const NAME_MARGIN: Vec2 = Vec2::new(8.0, 4.0);
/// `Button-IconOnly`: 14 with padding 6.
const SAVE_BUTTON: f32 = 14.0 + 2.0 * 6.0;
/// Before the version boxes (and between them and the window buttons),
/// and between the two boxes.
const RIGHT_GAP: f32 = 24.0;
const BOX_GAP: f32 = 8.0;
/// `Button-System`, and the window buttons with their margins 2 and the
/// 4 between them.
const SYSTEM_BUTTON: f32 = 29.0;
const WINDOW_BUTTONS_WIDTH: f32 = 2.0 + 3.0 * SYSTEM_BUTTON + 2.0 * 4.0 + 2.0;
const VERSION: &str = concat!("Nerthus ", env!("CARGO_PKG_VERSION"));

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
    let plan = plan(&widths, rect.width());
    let content = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + BAR_TOP), rect.max);
    let right_left = rect.right() - widths.right(&plan);

    // The brand, the menus and the file box, kept short of undo's place
    // even when the least the bar shows is too wide for it.
    let limit = (right_left - widths.undo - UNDO_GAP).max(rect.left());
    let left_rect = Rect::from_min_max(content.min, Pos2::new(limit, content.bottom()));
    let mut left = ui.new_child(UiBuilder::new().id_salt("title-left").max_rect(left_rect).layout(Layout::left_to_right(Align::Center)));
    left.set_clip_rect(ui.clip_rect().intersect(Rect::from_min_max(rect.min, Pos2::new(limit, rect.bottom()))));
    left.spacing_mut().item_spacing.x = 0.0;
    if plan.brand {
        brand(&mut left);
    } else {
        left.add_space(LEAD_WITHOUT_BRAND);
    }
    menu_bar(&mut left, widths.menu_bar(plan.menus), plan.menus, editor, ui_state, commands);
    left.add_space(MENU_GAP);
    file_box(&mut left, editor, plan.name_width, commands);

    let undo_left = left.min_rect().right().min(limit) + UNDO_GAP;
    let undo_rect = Rect::from_min_max(Pos2::new(undo_left, content.top()), Pos2::new(right_left.max(undo_left), content.bottom()));
    let mut undo = ui.new_child(UiBuilder::new().id_salt("title-undo").max_rect(undo_rect).layout(Layout::left_to_right(Align::Center)));
    undo_menu(&mut undo, editor, commands);

    let right_rect = Rect::from_min_max(Pos2::new(right_left, content.top()), content.max);
    let mut right = ui.new_child(UiBuilder::new().id_salt("title-right").max_rect(right_rect).layout(Layout::right_to_left(Align::Center)));
    right.spacing_mut().item_spacing.x = 0.0;
    // The gap before the next box.
    let mut gap = 0.0;
    // The browser's tab is the web build's window.
    if !cfg!(target_arch = "wasm32") {
        window_buttons(&mut right, commands);
        gap = RIGHT_GAP;
    }
    // `bAbout` (About Nerthus).
    if plan.version {
        right.add_space(gap);
        if version_button(&mut right, VERSION, theme::LABEL_DEFAULT, true).clicked() {
            commands.push(Command::Tools(ToolCommand::ShowAbout));
        }
        gap = BOX_GAP;
    }
    if plan.update
        && let Some(update) = &update
    {
        right.add_space(gap);
        if version_button(&mut right, &update.text, update.color, update.enabled).clicked() {
            commands.push(Command::Tools(ToolCommand::CheckForUpdates));
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

/// How wide each part of the bar is, but for the file name's room.
#[derive(Clone, Debug)]
struct Widths {
    brand: f32,
    menus: [f32; MENU_NAMES.len()],
    /// The file box less the file name's room.
    file_box_rest: f32,
    /// The file name, not elided.
    name: f32,
    undo: f32,
    version: f32,
    update: Option<f32>,
    /// None in the web build.
    window_buttons: Option<f32>,
}

/// What the bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Plan {
    brand: bool,
    /// How many menus, from File on, stay in the bar; the overflow menu
    /// holds the rest.
    menus: usize,
    version: bool,
    update: bool,
    /// The file name's room: the name is elided when wider.
    name_width: f32,
}

fn measure(ui: &Ui, editor: &Editor, update: Option<&UpdateBox>) -> Widths {
    let icon_menu = icon_menu_width(ui);
    let regions = if crate::ui::regions::menus_visible(editor) { icon_menu } else { 0.0 };
    // The resolution and region menus, the name's margins, save, save
    // incremental and the autosave menu, in the box's padding 1.
    let file_box_rest = 1.0 + icon_menu + regions + 2.0 * NAME_MARGIN.x + SAVE_BUTTON + 2.0 + SAVE_BUTTON + 2.0 + icon_menu + 1.0;
    let name = ui.painter().layout_no_wrap(editor.document.file_name(), theme::monospace_bold(11.0), Color32::WHITE);
    Widths {
        brand: brand_size(ui).x,
        menus: MENU_NAMES.map(|name| menu_slot(ui, name).x),
        file_box_rest,
        name: name.size().x,
        undo: icon_menu,
        version: version_size(ui, VERSION).x,
        update: update.map(|update| version_size(ui, &update.text).x),
        window_buttons: (!cfg!(target_arch = "wasm32")).then_some(WINDOW_BUTTONS_WIDTH),
    }
}

/// An icon menu's width: `Toolbar.ButtonSize`, or the icon in the button
/// padding when that is wider.
fn icon_menu_width(ui: &Ui) -> f32 {
    toolbar_button(ui).max(icons::DEFAULT_SIZE + 2.0 * ui.spacing().button_padding.x)
}

impl Widths {
    /// The menus that stay, and the overflow menu when any don't.
    fn menu_bar(&self, shown: usize) -> f32 {
        let overflow = if shown < MENU_NAMES.len() { OVERFLOW_WIDTH } else { 0.0 };
        self.menus[..shown].iter().sum::<f32>() + overflow
    }

    fn file_box(&self, name_width: f32) -> f32 {
        (self.file_box_rest + name_width).max(FILE_BOX_MIN_WIDTH)
    }

    /// Everything but the file box: the brand, the menus, undo, the
    /// version boxes and window buttons, and the gaps.
    fn others(&self, plan: &Plan) -> f32 {
        let lead = if plan.brand { self.brand } else { LEAD_WITHOUT_BRAND };
        lead + self.menu_bar(plan.menus) + MENU_GAP + UNDO_GAP + self.undo + self.right(plan)
    }

    /// The version boxes and window buttons with the gap before them.
    fn right(&self, plan: &Plan) -> f32 {
        let version = plan.version.then_some(self.version);
        let update = self.update.filter(|_| plan.update);
        let boxes = match (version, update) {
            (Some(version), Some(update)) => version + BOX_GAP + update,
            (version, update) => version.or(update).unwrap_or(0.0),
        };
        let window_buttons = match self.window_buttons {
            Some(buttons) if boxes > 0.0 => buttons + RIGHT_GAP,
            Some(buttons) => buttons,
            None => 0.0,
        };
        RIGHT_GAP + boxes + window_buttons
    }
}

/// The most the bar can show in `width`: each step gives up one more
/// thing until the rest fits, the file name taking what room is left up
/// to its full width. Past the last step the bar is clipped short of undo.
fn plan(widths: &Widths, width: f32) -> Plan {
    let full_name = widths.name.max(NAME_MIN_WIDTH);
    // Each step, with the least room its file name may have.
    let mut step = Plan { brand: true, menus: MENU_NAMES.len(), version: true, update: widths.update.is_some(), name_width: full_name };
    let mut steps = vec![(step, full_name)];
    step.version = false;
    steps.push((step, full_name));
    step.update = false;
    steps.push((step, full_name));
    steps.push((step, NAME_MIN_WIDTH));
    for menus in (0..MENU_NAMES.len()).rev() {
        step.menus = menus;
        steps.push((step, NAME_MIN_WIDTH));
    }
    step.brand = false;
    steps.push((step, NAME_MIN_WIDTH));
    steps.push((step, NAME_LEAST_WIDTH));
    for (candidate, least) in steps {
        let others = widths.others(&candidate);
        let name_width = (width - others - widths.file_box_rest).min(full_name);
        if name_width >= least && others + widths.file_box(name_width) <= width {
            return Plan { name_width, ..candidate };
        }
    }
    Plan { name_width: NAME_LEAST_WIDTH, ..step }
}

/// The name button (`Button-Link`, padding 16, 6): the name in the header font.
fn brand(ui: &mut Ui) {
    let galley = brand_galley(ui);
    let size = galley.size() + Vec2::new(32.0, 12.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let position = Pos2::new(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::ACCENT_PRIMARY);
}

fn brand_galley(ui: &Ui) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap("NERTHUS".to_string(), theme::semibold(16.0), theme::ACCENT_PRIMARY)
}

fn brand_size(ui: &Ui) -> Vec2 {
    brand_galley(ui).size() + Vec2::new(32.0, 12.0)
}

const MENU_NAMES: [&str; 8] = ["File", "Edit", "Graph", "Node", "Preview", "Project", "Tools", "Help"];

/// A top-level item's slot: at least 64 wide less its overlaps, its text
/// 8 + 5 in from each side.
fn menu_slot(ui: &Ui, name: &str) -> Vec2 {
    let text_width = ui.painter().layout_no_wrap(name.to_string(), theme::interface(11.0), Color32::WHITE).size().x;
    let item_width = (text_width + 2.0 * (8.0 + 5.0)).max(TOP_LEVEL_MIN_WIDTH);
    Vec2::new(item_width - 2.0 * TOP_LEVEL_OVERLAP, TOP_LEVEL_HEIGHT)
}

/// The main menu: each top-level item takes its slot, its text centred,
/// `Menu-TopLevel` under the pointer; past the first `shown`, the menus
/// are submenus of the overflow menu. `MenuBar` fills what it is given,
/// so it gets just its items' `width`.
fn menu_bar(ui: &mut Ui, width: f32, shown: usize, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    let size = Vec2::new(width, TOP_LEVEL_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| menu_items(ui, shown, editor, ui_state, commands));
}

fn menu_items(ui: &mut Ui, shown: usize, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().button_padding = Vec2::ZERO;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.open.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::NONE;
        ui.visuals_mut().widgets.hovered.corner_radius = egui::CornerRadius::same(4);
        ui.visuals_mut().widgets.open.corner_radius = egui::CornerRadius::same(4);
        // Each menu's popup, for the keys to go between them.
        let mut popups = Vec::new();
        for name in &MENU_NAMES[..shown] {
            let text = RichText::new(*name).font(theme::interface(11.0)).color(theme::MENU_FOREGROUND);
            let slot = menu_slot(ui, name);
            let mut button = egui::Button::new(text);
            button = button.min_size(slot);
            button = button.frame_when_inactive(false);
            let menu = MenuButton::from_button(button);
            let (response, _) = menu.ui(ui, |ui| {
                menu_style(ui);
                menu_contents(ui, name, editor, ui_state, commands);
            });
            popups.push(egui::Popup::default_response_id(&response));
        }
        if shown < MENU_NAMES.len() {
            let mut button = match icons::image("menu", icons::DEFAULT_SIZE, theme::MENU_FOREGROUND) {
                Some(image) => egui::Button::image(image),
                None => egui::Button::new(RichText::new("Menu").font(theme::interface(11.0)).color(theme::MENU_FOREGROUND)),
            };
            button = button.min_size(Vec2::new(OVERFLOW_WIDTH, TOP_LEVEL_HEIGHT));
            button = button.frame_when_inactive(false);
            let (response, _) = MenuButton::from_button(button).ui(ui, |ui| {
                menu_style(ui);
                for name in &MENU_NAMES[shown..] {
                    submenu(ui, None, None, name, |ui| menu_contents(ui, name, editor, ui_state, commands));
                }
            });
            popups.push(egui::Popup::default_response_id(&response));
            response.on_hover_text("Menu");
        }
        menu_keys::bar(ui.ctx(), &popups);
    });
}

fn menu_contents(ui: &mut Ui, name: &str, editor: &Editor, ui_state: &UiState, commands: &mut Vec<Command>) {
    match name {
        "File" => menus::file_menu(ui, editor, commands),
        "Edit" => menus::edit_menu(ui, editor, commands),
        "Graph" => menus::graph_menu(ui, editor, ui_state, commands),
        "Node" => menus::node_menu(ui, editor, commands),
        "Preview" => menus::preview_menu(ui, editor, commands),
        "Project" => menus::project_menu(ui, commands),
        "Tools" => menus::tools_menu(ui, editor, commands),
        _ => menus::help_menu(ui, commands),
    }
}

/// The file box (`MinWidth 180`, padding 1, `Panel-Background`, corners
/// 3): the resolution and region menus, the file name (monospace 11 bold,
/// margin 8, 4) in `name_width`, elided with the whole name under the
/// pointer when longer, save, save incremental and the autosave menu.
fn file_box(ui: &mut Ui, editor: &Editor, name_width: f32, commands: &mut Vec<Command>) {
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
            let name = editor.document.file_name();
            // `TryAutosave` turns the name red once the autosave interval
            // passed with unsaved changes; a save turns it back.
            let name_color = if editor.files.autosave.reminder { theme::ACCENT_MUTED_RED } else { theme::LABEL_DEFAULT };
            let font = theme::monospace_bold(11.0);
            let mut galley = ui.painter().layout_no_wrap(name.clone(), font.clone(), name_color);
            let elided = galley.size().x > name_width;
            if elided {
                galley = elided_galley(ui.painter(), &name, font, name_color, name_width);
            }
            let size = Vec2::new(name_width, galley.size().y) + 2.0 * NAME_MARGIN;
            let (name_rect, response) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().galley(Pos2::new(name_rect.left() + NAME_MARGIN.x, name_rect.center().y - galley.size().y / 2.0), galley, name_color);
            if elided {
                response.on_hover_text(name);
            }
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
fn undo_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    icon_menu(ui, "arrow-back-up", "Undo", None, |ui| {
        menu_style(ui);
        let labels = editor.document.undo_labels();
        if labels.is_empty() {
            menu_caption(ui, "No undo history");
            return;
        }
        for (steps, label) in labels.iter().enumerate().take(20) {
            if menu_item(ui, None, None, label, "").clicked() {
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
    ui.painter().layout_no_wrap(text.to_string(), theme::semibold(10.0), color)
}

fn version_size(ui: &Ui, text: &str) -> Vec2 {
    let text_size = version_galley(ui, text, Color32::WHITE).size();
    Vec2::new((text_size.x + 48.0).max(87.0), text_size.y + 12.0)
}

/// Minimize, maximize and close (`Button-System`: Marlett's `0`, `1` and
/// `r` with padding 10 and margins 2, at 40% until hovered; close red).
fn window_buttons(ui: &mut Ui, commands: &mut Vec<Command>) {
    ui.add_space(2.0);
    if system_button(ui, SystemGlyph::Close).clicked() {
        commands.push(Command::Exit);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Maximize).clicked() {
        commands.push(Command::WindowMaximize);
    }
    ui.add_space(4.0);
    if system_button(ui, SystemGlyph::Minimize).clicked() {
        commands.push(Command::WindowMinimize);
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
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0));
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

    /// The bar's width for `plan`.
    fn total(widths: &Widths, plan: &Plan) -> f32 {
        widths.others(plan) + widths.file_box(plan.name_width)
    }

    #[test]
    fn the_bar_fits_from_the_least_it_can_show() {
        let update = update();
        for name in NAMES {
            for update in [None, Some(&update)] {
                let widths = measured(&editor_named(name), update);
                let least = Plan { brand: false, menus: 0, version: false, update: false, name_width: NAME_LEAST_WIDTH };
                let mut last: Option<Plan> = None;
                for tenth in 2000..=40000 {
                    let width = tenth as f32 / 10.0;
                    let plan = plan(&widths, width);
                    if width >= total(&widths, &least) {
                        assert!(total(&widths, &plan) <= width + 0.001, "{name}: {plan:?} is wider than {width}");
                        assert!(plan.name_width >= NAME_LEAST_WIDTH, "{name}: {plan:?} at {width}");
                    } else {
                        assert_eq!(plan, least, "{name}: at {width}");
                    }
                    assert!(plan.name_width <= widths.name.max(NAME_MIN_WIDTH));
                    // Wider, nothing goes, and the name only shrinks back
                    // to make room for what comes back.
                    if let Some(last) = last {
                        assert!(plan.brand >= last.brand && plan.menus >= last.menus, "{name}: {last:?} then {plan:?} at {width}");
                        assert!(plan.version >= last.version && plan.update >= last.update, "{name}: {last:?} then {plan:?} at {width}");
                        if (Plan { name_width: 0.0, ..plan }) == (Plan { name_width: 0.0, ..last }) {
                            assert!(plan.name_width >= last.name_width, "{name}: {last:?} then {plan:?} at {width}");
                        } else {
                            assert!(plan.name_width >= NAME_MIN_WIDTH, "{name}: {last:?} then {plan:?} at {width}");
                        }
                    }
                    last = Some(plan);
                }
                let wide = plan(&widths, 4000.0);
                assert_eq!(wide, Plan { brand: true, menus: MENU_NAMES.len(), version: true, update: update.is_some(), name_width: widths.name.max(NAME_MIN_WIDTH) });
            }
        }
    }

    #[test]
    fn the_version_goes_first_then_the_name_then_the_menus_then_the_brand() {
        let update = update();
        for name in NAMES {
            for update in [None, Some(&update)] {
                let widths = measured(&editor_named(name), update);
                let full_name = widths.name.max(NAME_MIN_WIDTH);
                for width in 200..=4000 {
                    let plan = plan(&widths, width as f32);
                    if plan.version || plan.update {
                        assert_eq!(plan.name_width, full_name, "{name}: {plan:?} at {width}");
                        assert_eq!(plan.menus, MENU_NAMES.len(), "{name}: {plan:?} at {width}");
                    }
                    if plan.version {
                        assert_eq!(plan.update, update.is_some(), "{name}: {plan:?} at {width}");
                    }
                    if plan.brand {
                        assert!(plan.name_width >= NAME_MIN_WIDTH, "{name}: {plan:?} at {width}");
                    }
                    if !plan.brand {
                        assert_eq!(plan.menus, 0, "{name}: {plan:?} at {width}");
                    }
                    if plan.name_width < NAME_MIN_WIDTH {
                        assert!(!plan.brand, "{name}: {plan:?} at {width}");
                    }
                }
            }
        }
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
            for shown in 0..=MENU_NAMES.len() {
                let width = widths.menu_bar(shown);
                assert_eq!(drawn(&mut |ui| menu_bar(ui, width, shown, &editor, &ui_state, &mut commands)), width, "{shown} menus");
            }
            for name_width in [NAME_LEAST_WIDTH, NAME_MIN_WIDTH, widths.name, 400.0] {
                let box_width = drawn(&mut |ui| file_box(ui, &editor, name_width, &mut commands));
                assert_eq!(box_width, widths.file_box(name_width), "the name in {name_width}");
            }
            assert_eq!(drawn(&mut |ui| undo_menu(ui, &editor, &mut commands)), widths.undo);
            let version = drawn(&mut |ui| {
                version_button(ui, VERSION, theme::LABEL_DEFAULT, true);
            });
            assert_eq!(version, widths.version);
            assert_eq!(drawn(&mut |ui| window_buttons(ui, &mut commands)), WINDOW_BUTTONS_WIDTH);
        });
        output.drop_without_applying_deltas();
    }

    /// Clicks at `at`: the pointer moves there, presses and lets go.
    fn click(context: &egui::Context, width: f32, editor: &Editor, ui_state: &UiState, at: Pos2) -> Vec<Command> {
        let button = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut commands = Vec::new();
        for events in [vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)], Vec::new()] {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, HEIGHT));
            let output = context.run_ui(input(width, events), |ui| show(ui, rect, editor, ui_state, &mut commands));
            output.drop_without_applying_deltas();
        }
        commands
    }

    #[test]
    fn undo_and_close_answer_clicks_at_any_width() {
        let ui_state = UiState::default();
        // 3800, 1920, 1600, 1366, 1340, 1320, 1280, 1024, 901, 800 and
        // 640 pixels wide at the default 120%, and narrower.
        let widths_in_points = [3166.7, 1600.0, 1333.3, 1138.3, 1116.7, 1100.0, 1066.7, 853.3, 750.8, 666.7, 533.3, 450.0];
        for name in NAMES {
            let editor = editor_named(name);
            let widths = measured(&editor, None);
            for width in widths_in_points {
                let plan = plan(&widths, width);
                let lead = if plan.brand { widths.brand } else { LEAD_WITHOUT_BRAND };
                let left_end = lead + widths.menu_bar(plan.menus) + MENU_GAP + widths.file_box(plan.name_width);
                let undo_left = left_end + UNDO_GAP;
                assert!(undo_left + widths.undo + widths.right(&plan) <= width + 0.001, "{name}: undo at {undo_left} is under the version box at {width}");
                let y = BAR_TOP + (HEIGHT - BAR_TOP) / 2.0;

                let context = themed_context(width);
                let commands = click(&context, width, &editor, &ui_state, Pos2::new(undo_left + widths.undo / 2.0, y));
                assert!(egui::Popup::is_any_open(&context), "{name}: undo's menu did not open at {width}");
                assert!(commands.is_empty(), "{name}: clicking undo at {width} gave {commands:?}");

                let context = themed_context(width);
                let close = Pos2::new(width - 2.0 - SYSTEM_BUTTON / 2.0, y);
                let commands = click(&context, width, &editor, &ui_state, close);
                assert_eq!(commands, [Command::Exit], "{name}: close at {width}");
            }
        }
    }

    /// Frames of the bar as the app draws it, the menus taking their keys
    /// first: one for each of `keys`, each followed by two without.
    fn keyed(context: &egui::Context, width: f32, editor: &Editor, keys: &[egui::Key]) -> Vec<Command> {
        let ui_state = UiState::default();
        let mut commands = Vec::new();
        let pressed = |key: egui::Key| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        for events in keys.iter().flat_map(|key| [pressed(*key), Vec::new(), Vec::new()]) {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, HEIGHT));
            let output = context.run_ui(input(width, events), |ui| {
                menu_keys::begin(ui.ctx());
                show(ui, rect, editor, &ui_state, &mut commands);
            });
            output.drop_without_applying_deltas();
        }
        commands
    }

    #[test]
    fn f10_opens_file_and_the_arrows_go_between_the_menus() {
        use egui::Key::{ArrowLeft, ArrowRight, Enter, F10};
        let editor = editor_named(NAMES[0]);
        let width = 1600.0;
        let context = themed_context(width);
        assert_eq!(keyed(&context, width, &editor, &[F10, Enter]), [Command::NewProject]);
        assert!(!egui::Popup::is_any_open(&context));
        // Edit has nothing to choose with nothing selected; Graph's first
        // row is New Graph Tab.
        assert_eq!(keyed(&context, width, &editor, &[F10, ArrowRight, ArrowRight, Enter]), [Command::NewGraphTab]);
        // Left from File goes round to Help.
        let help = Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string()));
        assert_eq!(keyed(&context, width, &editor, &[F10, ArrowLeft, Enter]), [help]);
        // F10 again closes the menu.
        assert!(keyed(&context, width, &editor, &[F10]).is_empty());
        assert!(egui::Popup::is_any_open(&context));
        assert!(keyed(&context, width, &editor, &[F10]).is_empty());
        assert!(!egui::Popup::is_any_open(&context));
    }

    #[test]
    fn f10_opens_the_folded_menus_when_the_bar_has_no_room() {
        use egui::Key::{ArrowRight, Enter, F10};
        let editor = editor_named(NAMES[2]);
        let widths = measured(&editor, None);
        let least = Plan { brand: false, menus: 0, version: false, update: false, name_width: NAME_LEAST_WIDTH };
        let width = total(&widths, &least);
        assert_eq!(plan(&widths, width).menus, 0);
        let context = themed_context(width);
        // The menu of menus opens on File, whose first row Right opens.
        assert_eq!(keyed(&context, width, &editor, &[F10, ArrowRight, Enter]), [Command::NewProject]);
    }

    #[test]
    fn the_least_bar_keeps_undo_clear_of_the_window_buttons() {
        let editor = editor_named(NAMES[2]);
        let ui_state = UiState::default();
        let widths = measured(&editor, None);
        let least = Plan { brand: false, menus: 0, version: false, update: false, name_width: NAME_LEAST_WIDTH };
        // Too narrow for the least the bar shows: the file box is cut short.
        let width = total(&widths, &least) - 60.0;
        let y = BAR_TOP + (HEIGHT - BAR_TOP) / 2.0;
        let undo_left = width - widths.right(&least) - widths.undo;
        let context = themed_context(width);
        let commands = click(&context, width, &editor, &ui_state, Pos2::new(undo_left + widths.undo / 2.0, y));
        assert!(egui::Popup::is_any_open(&context), "undo's menu did not open at {width}");
        assert!(commands.is_empty(), "clicking undo at {width} gave {commands:?}");
    }
}
