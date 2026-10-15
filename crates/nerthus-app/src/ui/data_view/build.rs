//! The Build tab (`tabProject`): Build, the build folder, Build Settings,
//! regions and statistics in the toolbar; the resolution and destination
//! of the build; and the nodes marked for export (`SavablesArea`), each a
//! `Savable` row.

use egui::{Align, Color32, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::Catalogue;
use nerthus_export::build::{preview_destination, BuildDefinition};
use nerthus_export::destination::DEFAULT_TEMPLATE;
use nerthus_export::FileFormat;
use serde_json::{json, Value};

use crate::document::{node_tab, ExportSettings};
use crate::editor::{DialogRequest, Editor};
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::{section_title, subtle_button, toolbar, toolbar_divider, toolbar_row};
use crate::ui::editors::{dropdown_box, fitted_line};
use crate::ui::widgets::{check_item, dialog, dialog_button, icon_button, icon_menu, icon_menu_sized, menu_caption, menu_item, menu_item_enabled, menu_separator, Tooltips};

/// `cboResolution`'s items: the size and its text.
const RESOLUTIONS: [(usize, &str); 18] = [
    (512, "512 x 512"),
    (1024, "1K (1024)"),
    (2048, "2K (2048)"),
    (4096, "4K (4096)"),
    (8192, "8K (8192)"),
    (16384, "16K (16,384)"),
    (24576, "24K (24,576)"),
    (32768, "32K (32,768)"),
    (49152, "48K (49,152)"),
    (65536, "64K (65,536)"),
    (73728, "72K (73,728)"),
    (98304, "96K (98,304)"),
    (131072, "128K (131,072)"),
    (147456, "144K (147,456)"),
    (163840, "160K (163,840)"),
    (196608, "192K (196,608)"),
    (229376, "224K (229,376)"),
    (262144, "256K (262,144)"),
];

/// `Savable`: rows 34 high (margin 4, 1), a 2 colour bar, a 36 menu
/// column and a 105 format column; text 11.
const SAVABLE_HEIGHT: f32 = 34.0;
const SAVABLE_MENU_COLUMN: f32 = 36.0;
const SAVABLE_FORMAT_COLUMN: f32 = 105.0;
/// `Savable-Border`: the line under each row.
const SAVABLE_BORDER: Color32 = Color32::from_rgb(0x2f, 0x2f, 0x2f);
/// `TextBox-Border`.
const TEXTBOX_BORDER: Color32 = Color32::from_rgb(0x36, 0x36, 0x36);

/// The tab's state between frames.
#[derive(Default)]
pub struct BuildState {
    /// The destination as typed, while the field has the keyboard.
    destination: Option<String>,
    /// Export file names being typed, by node.
    filenames: Vec<(i32, String)>,
    /// A format for every export, waiting for Yes (`MiFormat_Click`).
    pending_format: Option<FileFormat>,
}

pub fn show(ui: &mut Ui, area: Rect, editor: &mut Editor, state: &mut BuildState, commands: &mut Vec<Command>) {
    let (inner, page_top) = toolbar(ui, area);
    let mut row = toolbar_row(ui, inner, false);
    if build_button(&mut row).tooltip("Execute Build").clicked() {
        commands.push(Command::BuildAndExport);
    }
    row.add_space(2.0);
    if icon_button(&mut row, "folder-symlink", "Open current build folder", None).clicked() {
        commands.push(Command::OpenBuildFolder);
    }
    toolbar_divider(&mut row, 2.0, 4.0);
    if icon_button(&mut row, "settings", "Build Settings", Some(theme::ACCENT_MUTED_BLUE)).clicked() {
        commands.push(Command::BuildSettings);
    }
    row.add_space(2.0);
    // `miDataViewRegionsPreview`, once the terrain has regions.
    if crate::ui::regions::menus_visible(editor) {
        let tooltip = crate::ui::regions::preview_tooltip(editor);
        icon_menu(&mut row, "artboard", &tooltip, Some(theme::ACCENT_GREEN), |ui| {
            crate::ui::regions::regions_menu(ui, editor, commands, false);
        });
    }
    if icon_button(&mut row, "histogram", "Build Statistics", None).clicked() {
        commands.push(Command::ShowBuildStatistics);
    }

    // `ScrollViewer` (padding 0, 0, 8, 0) around a stack with margin 8, 8,
    // 0, 0.
    let page = Rect::from_min_max(Pos2::new(area.left() + 8.0, page_top + 8.0), Pos2::new(area.right() - 8.0, area.bottom()));
    let builder = egui::UiBuilder::new().max_rect(page);
    let mut page_ui = ui.new_child(builder);
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("data-view-build");
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(&mut page_ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        build_settings(ui, editor, state, commands);
        ui.add_space(16.0);
        section_title(ui, "Nodes to Export");
        savables(ui, editor, state, commands);
    });
    confirm_format(ui, editor, state);
}

/// `MiFormat_Click`'s question before every export changes format.
fn confirm_format(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState) {
    let Some(format) = state.pending_format else {
        return;
    };
    let modal = dialog(ui.ctx(), egui::Id::new("confirm-export-formats"));
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(320.0);
        let title = egui::RichText::new("Update file format").font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(title);
        ui.add_space(6.0);
        ui.label("Do you want to change the file format for all nodes?");
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if dialog_button(ui, "No", false).clicked() {
                state.pending_format = None;
            }
            if dialog_button(ui, "Yes", false).clicked() {
                editor.document.set_all_export_formats(format);
                state.pending_format = None;
            }
        });
    });
    if response.should_close() {
        state.pending_format = None;
    }
}

/// `ToolbarButton-IconXText`: `atom2` in `Accent-Muted-Green` 8 in, the
/// text 30 in (`Label-Default`), 32 high; the hover fill (corners 4)
/// brightens the icon.
fn build_button(ui: &mut Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("Build".to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let size = Vec2::new(30.0 + galley.size().x + 8.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered();
    if pressed {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    let icon_color = if hovered || pressed { theme::LABEL_EMPHASIS } else { theme::ACCENT_MUTED_GREEN };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, "atom2", icon_color);
    let text_position = Pos2::new(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, theme::LABEL_DEFAULT);
    response
}

/// The `uc` stack: Build Settings, the resolution (`DropdownX`, `Small`: a
/// 90 header column), More settings..., the destination with its browse
/// button and where it leads, and Open Folder.
fn build_settings(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState, commands: &mut Vec<Command>) {
    section_title(ui, "Build Settings");
    ui.add_space(8.0);
    let definition = editor.document.build_definition();
    // `ReloadBuildSettings`: with a region to build, its size.
    let region = definition.portion.as_ref().and_then(|portion| editor.document.regions().into_iter().find(|region| region.name == portion.name));
    let resolution = region.as_ref().map(|region| region.size.max(0) as usize).unwrap_or(definition.resolution);
    let current = resolution.to_string();
    let shown = RESOLUTIONS.iter().find(|(size, _)| *size == resolution).map(|(_, text)| *text).unwrap_or_default();
    let entries: Vec<(String, String)> = RESOLUTIONS.iter().map(|(size, text)| (size.to_string(), text.to_string())).collect();
    let id = ui.id().with("data-view-resolution");
    let chosen = dropdown_box(ui, id, Some("Resolution"), 90.0, 26.0, shown, &entries, &current, theme::interface(12.0));
    let chosen_size = chosen.and_then(|value| value.parse::<u64>().ok());
    // `CboResolution_SelectionChanged`.
    match (chosen_size, region) {
        (Some(size), Some(region)) => {
            let mut resized = region.clone();
            resized.size = size as i32;
            editor.document.update_region(&region.name, resized);
        }
        (Some(size), None) => editor.document.set_build_setting("Resolution", json!(size)),
        (None, _) => {}
    }

    ui.add_space(8.0);
    let more = ui.with_layout(Layout::right_to_left(Align::Min), |ui| subtle_button(ui, "More settings...")).inner;
    if more.clicked() {
        commands.push(Command::BuildSettings);
    }

    ui.add_space(16.0);
    // `Label` (padding 6, 4, 16, 2).
    let label_galley = ui.painter().layout_no_wrap("Build Destination".to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let (label_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), label_galley.size().y + 6.0), Sense::hover());
    ui.painter().galley(Pos2::new(label_rect.left() + 6.0, label_rect.top() + 4.0), label_galley, theme::LABEL_DEFAULT);

    let saved = if definition.destination.is_empty() { DEFAULT_TEMPLATE.to_string() } else { definition.destination.clone() };
    let typed = destination_field(ui, editor, state, &saved);
    let mut previewed = definition.clone();
    previewed.destination = typed;
    let preview = path_preview(editor, &previewed);
    // `txtPathPreview`: monospace, margin 16, 4, 8, 0, cut with an ellipsis.
    let preview_width = (ui.available_width() - 24.0).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(preview.clone(), egui::TextFormat::simple(theme::monospace(12.0), theme::LABEL_DEFAULT));
    job.wrap.max_width = preview_width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let (preview_rect, preview_response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), galley.size().y + 4.0), Sense::hover());
    ui.painter().galley(Pos2::new(preview_rect.left() + 16.0, preview_rect.top() + 4.0), galley, theme::LABEL_DEFAULT);
    preview_response.tooltip(preview);

    ui.add_space(8.0);
    let open = ui.with_layout(Layout::right_to_left(Align::Min), |ui| subtle_button(ui, "Open Folder")).inner;
    if open.clicked() {
        commands.push(Command::OpenBuildFolder);
    }
}

/// `txtLocation` (margin 4, padding 4, 2, 12, 2, semibold in
/// `TextBox-File-Foreground`) and `btnBrowse` (`...`, 26 square) docked
/// right. The destination saves when the field lets go of the keyboard.
/// Returns the destination as it reads now.
fn destination_field(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState, saved: &str) -> String {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::hover());
    let button_rect = Rect::from_min_size(Pos2::new(rect.right() - 26.0, rect.center().y - 13.0), Vec2::splat(26.0));
    let field_rect = Rect::from_min_max(Pos2::new(rect.left() + 4.0, rect.top() + 4.0), Pos2::new(button_rect.left() - 4.0, rect.bottom() - 4.0).max(rect.min));

    let mut text = state.destination.clone().unwrap_or_else(|| saved.to_string());
    let field_id = ui.id().with("data-view-destination");
    let focused = ui.memory(|memory| memory.has_focus(field_id));
    let border = if focused { theme::TEXTBOX_FOCUSED_BORDER } else { TEXTBOX_BORDER };
    ui.painter().rect_filled(field_rect, 3.0, theme::TEXTBOX_BACKGROUND);
    ui.painter().rect_stroke(field_rect, 3.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let text_rect = Rect::from_min_max(field_rect.min + Vec2::new(5.0, 3.0), field_rect.max - Vec2::new(13.0, 3.0));
    let mut edit = egui::TextEdit::singleline(&mut text);
    edit = edit.id(field_id);
    edit = edit.frame(egui::Frame::NONE);
    edit = edit.font(theme::semibold(12.0));
    edit = edit.text_color(theme::TEXTBOX_FILE_FOREGROUND);
    edit = edit.desired_width(text_rect.width());
    let response = ui.put(text_rect, edit);
    if response.has_focus() {
        state.destination = Some(text.clone());
    } else if state.destination.take().is_some() && text != saved {
        editor.document.set_build_setting("Destination", Value::String(text.clone()));
    }

    let browse = ui.interact(button_rect, ui.id().with("data-view-browse"), Sense::click());
    let fill = if browse.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    ui.painter().rect_filled(button_rect, 2.0, fill);
    ui.painter().rect_stroke(button_rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), StrokeKind::Inside);
    let dots = ui.painter().layout_no_wrap("...".to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    ui.painter().galley(button_rect.center() - dots.size() / 2.0, dots, theme::LABEL_DEFAULT);
    let browse = browse.tooltip("Browse to manually select a build path");
    if browse.clicked() {
        editor.dialog = Some(DialogRequest::BuildDestination { current: text.clone() });
    }
    text
}

/// `UpdateLocationPreview`: the folder the build would go to, or why it
/// cannot.
fn path_preview(editor: &Editor, definition: &BuildDefinition) -> String {
    let Some(project) = &editor.document.path else {
        return "Save the project to see where it builds.".to_string();
    };
    if definition.destination.starts_with("\\\\") {
        return "UNC PATHS ARE NOT SUPPORTED.".to_string();
    }
    let preferences = editor.user.preferences.export_preferences();
    let folder = preview_destination(project, definition, &preferences);
    let text = folder.display().to_string();
    let colons = text.matches(':').count();
    let illegal = text.chars().any(|character| matches!(character, '<' | '>' | '"' | '|' | '?' | '*') || character.is_control());
    if colons > 1 || illegal {
        return "INVALID PATH. Check the path for illegal characters.".to_string();
    }
    text
}

/// `SavablesArea.RecreateExportView`: the menu changing every format,
/// then each graph tab's marked nodes (under an expander when there is
/// more than one tab), or a hint when nothing is marked. Duplicate file
/// names put a fix link at the top.
fn savables(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState, commands: &mut Vec<Command>) {
    let marked: Vec<i32> = editor.document.terrain().nodes.values().filter(|node| ExportSettings::of(node).is_some()).map(|node| node.id).collect();
    // `ExportStack` (margin 4, 0, 8, 8) and its stack (margin 8).
    let mut frame = egui::Frame::new();
    frame.inner_margin = egui::Margin { left: 12, right: 16, top: 8, bottom: 16 };
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if editor.document.has_duplicate_export_names() {
            let link = egui::RichText::new("There are duplicate filenames! Click to fix them!").font(theme::interface(12.0)).color(theme::ACCENT_RED);
            if ui.add(egui::Label::new(link).sense(Sense::click())).clicked() {
                editor.document.fix_duplicate_export_names();
                state.filenames.clear();
            }
        }
        if marked.is_empty() {
            let hint = egui::RichText::new("Mark nodes as Exportable (F3) to see them here.").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
            let mut hint_frame = egui::Frame::new();
            hint_frame.inner_margin = egui::Margin::same(8);
            hint_frame.show(ui, |ui| ui.add(egui::Label::new(hint).wrap()));
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            icon_menu(ui, "menu-deep", "Change formats for all nodes.", None, |ui| {
                menu_caption(ui, "Change formats for all nodes");
                for format in FileFormat::ALL {
                    if menu_item(ui, None, None, format.name(), "").clicked() {
                        state.pending_format = Some(format);
                    }
                }
            });
        });
        let tabs = editor.document.graph_tabs();
        let several = tabs.len() > 1;
        for tab in tabs {
            let in_tab: Vec<i32> = marked.iter().copied().filter(|id| editor.document.node(*id).is_some_and(|node| node_tab(node) == tab.index)).collect();
            if in_tab.is_empty() {
                continue;
            }
            if !several {
                for id in in_tab {
                    savable(ui, editor, state, id, commands);
                }
                continue;
            }
            let mut header = egui::CollapsingHeader::new(egui::RichText::new(tab.name.as_str()).font(theme::semibold(12.0)).color(theme::EXPANDER_HEADER));
            header = header.id_salt(("data-view-export-tab", tab.index));
            header = header.default_open(true);
            header = header.icon(group_chevron);
            header.show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for id in in_tab {
                    savable(ui, editor, state, id, commands);
                }
                ui.add_space(8.0);
            });
        }
    });
}

/// A graph tab's expander in the list: `chevron-down` while open,
/// `chevron-right` while closed (as elsewhere in the Data View), in
/// `Expander-Header`, `Accent-Gray` under the pointer.
fn group_chevron(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let chevron = if openness > 0.5 { "chevron-down" } else { "chevron-right" };
    let color = if response.hovered() { theme::ACCENT_GRAY } else { theme::EXPANDER_HEADER };
    icons::paint(ui, Rect::from_center_size(response.rect.center(), Vec2::splat(icons::DEFAULT_SIZE)), chevron, color);
}

/// `Savable`: the toolbox colour bar, the ports menu (`menu-deep`), the
/// file name (semibold in the toolbox colour, `Accent-Muted-Red` when the
/// export is off) and the format (`LiteComboBox`, monospace).
fn savable(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState, id: i32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(id).cloned() else {
        return;
    };
    let Some(export) = ExportSettings::of(&node) else {
        return;
    };
    let category = Catalogue::get().node(&node.type_name).and_then(|spec| spec.toolbox.clone()).unwrap_or_default();
    let color = theme::toolbox_color(&category);

    ui.add_space(1.0);
    let width = ui.available_width();
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, SAVABLE_HEIGHT), Sense::hover());
    ui.add_space(1.0);
    let rect = outer.shrink2(Vec2::new(4.0, 0.0));
    let line_y = rect.bottom() - 0.75;
    ui.painter().line_segment([Pos2::new(rect.left(), line_y), Pos2::new(rect.right(), line_y)], Stroke::new(1.5, SAVABLE_BORDER));
    let bar = Rect::from_min_max(rect.min, Pos2::new(rect.left() + 2.0, rect.bottom() - 1.5));
    ui.painter().rect_filled(bar, 0.0, color);

    // The ports menu: a 24 icon menu centred in its column.
    let menu_center = Pos2::new(bar.right() + SAVABLE_MENU_COLUMN / 2.0, bar.center().y);
    let menu_rect = Rect::from_center_size(menu_center, Vec2::splat(24.0));
    let mut menu_ui = ui.new_child(egui::UiBuilder::new().max_rect(menu_rect));
    icon_menu_sized(&mut menu_ui, "menu-deep", "Ports", None, Vec2::splat(24.0), |ui| ports_menu(ui, editor, &node, &export, commands));

    // The name and the format share a 20 high line through the bar's centre.
    let format_rect = Rect::from_min_max(Pos2::new(rect.right() - SAVABLE_FORMAT_COLUMN, bar.top()), Pos2::new(rect.right() - 6.0, bar.bottom()));
    let format_center = Rect::from_center_size(format_rect.center(), Vec2::new(format_rect.width(), 20.0));
    let name_rect = Rect::from_min_max(Pos2::new(bar.right() + SAVABLE_MENU_COLUMN + 8.0, format_center.top()), Pos2::new(format_rect.left() - 8.0, format_center.bottom()));
    filename_field(ui, editor, state, &node, &export, name_rect, color);

    let mut format_ui = ui.new_child(egui::UiBuilder::new().max_rect(format_center));
    format_ui.add_enabled_ui(export.enabled, |ui| {
        let entries: Vec<(String, String)> = FileFormat::ALL.iter().map(|format| (format.name().to_string(), format.name().to_string())).collect();
        let format_id = ui.id().with(("data-view-format", id));
        let chosen = dropdown_box(ui, format_id, None, 0.0, 20.0, export.format.name(), &entries, export.format.name(), theme::monospace(11.0));
        let chosen_format = chosen.and_then(|name| FileFormat::from_name(&name));
        if let Some(format) = chosen_format {
            editor.document.edit_export(id, "Export format", |settings| settings.format = format);
        }
    });
}

/// `miPortMenu`: the node, Enabled, each output port, then Primary Only,
/// Mark all and Mark none (dimmed while the export is off).
fn ports_menu(ui: &mut Ui, editor: &mut Editor, node: &nerthus_engine::NodeData, export: &ExportSettings, commands: &mut Vec<Command>) {
    if menu_item(ui, None, None, &format!("Node: {}", node.name), "").clicked() {
        commands.push(Command::SelectNode(node.id));
        commands.push(Command::ZoomSelection);
    }
    menu_separator(ui);
    if check_item(ui, export.enabled, "Enabled", "").clicked() {
        let enabled = !export.enabled;
        editor.document.edit_export(node.id, "Export enabled", |settings| settings.enabled = enabled);
    }
    menu_separator(ui);
    let outputs: Vec<&nerthus_engine::PortData> = node.ports.iter().filter(|port| port.is_output()).collect();
    for port in &outputs {
        let icon = if port.is_exporting { Some("check") } else { None };
        let clicked = menu_item_enabled(ui, icon, Some(theme::ACCENT_BLUE), &port.name, "", export.enabled).clicked();
        if clicked {
            editor.document.set_port_exporting(node.id, &port.name, !port.is_exporting);
        }
    }
    menu_separator(ui);
    let names: Vec<String> = outputs.iter().map(|port| port.name.clone()).collect();
    if menu_item_enabled(ui, None, None, "Primary Only", "", export.enabled).clicked() {
        for name in &names {
            editor.document.set_port_exporting(node.id, name, name == "Out");
        }
    }
    if menu_item_enabled(ui, None, None, "Mark all", "", export.enabled).clicked() {
        for name in &names {
            editor.document.set_port_exporting(node.id, name, true);
        }
    }
    if menu_item_enabled(ui, None, None, "Mark none", "", export.enabled).clicked() {
        for name in &names {
            editor.document.set_port_exporting(node.id, name, false);
        }
    }
}

/// `displayLabel`: one line centred down `rect`, cut with an ellipsis
/// while it isn't being typed in (the whole name in its tooltip). Saved
/// on Enter or Tab (empty: the node's name); Escape puts the saved name
/// back.
fn filename_field(ui: &mut Ui, editor: &mut Editor, state: &mut BuildState, node: &nerthus_engine::NodeData, export: &ExportSettings, rect: Rect, color: Color32) {
    let index = match state.filenames.iter().position(|(node_id, _)| *node_id == node.id) {
        Some(index) => index,
        None => {
            state.filenames.push((node.id, export.filename.clone()));
            state.filenames.len() - 1
        }
    };
    let text_color = if export.enabled { color } else { theme::ACCENT_MUTED_RED };
    let font = theme::semibold(11.0);
    let field_id = ui.id().with(("data-view-filename", node.id));
    // The box's own text shows only while typing; otherwise the name is
    // painted over it, cut to fit.
    let typing = ui.memory(|memory| memory.has_focus(field_id));
    let mut frame = egui::Frame::NONE;
    frame.inner_margin = egui::Margin::symmetric(4, 0);
    let mut edit = egui::TextEdit::singleline(&mut state.filenames[index].1);
    edit = edit.id(field_id);
    edit = edit.frame(frame);
    edit = edit.font(font.clone());
    edit = edit.text_color(if typing { text_color } else { Color32::TRANSPARENT });
    edit = edit.desired_width(rect.width());
    edit = edit.min_size(Vec2::new(0.0, rect.height()));
    edit = edit.vertical_align(Align::Center);
    let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    let shown = field_ui.add_enabled_ui(export.enabled, |ui| {
        let output = edit.show(ui);
        let response = output.response.response;
        if typing {
            return response;
        }
        let name = output.galley.text().to_string();
        let galley = fitted_line(ui, &name, font, text_color, output.text_clip_rect.width());
        let elided = galley.elided;
        ui.painter().galley(output.galley_pos, galley, text_color);
        if elided { response.tooltip(name) } else { response }
    });
    let response = shown.inner;
    // Show the change of look on the next frame without waiting for input.
    if response.has_focus() != typing {
        ui.ctx().request_repaint();
    }
    let committed = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Tab));
    let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
    if committed {
        let typed = state.filenames[index].1.trim().to_string();
        let filename = if typed.is_empty() { node.name.clone() } else { typed };
        state.filenames[index].1 = filename.clone();
        editor.document.edit_export(node.id, "Export filename", |settings| settings.filename = filename);
        return;
    }
    // Escape, or the name changed elsewhere (undo, fixing duplicates).
    let cancelled = response.lost_focus() && escaped;
    let stale = !response.has_focus() && state.filenames[index].1 != export.filename;
    if cancelled || stale {
        state.filenames[index].1 = export.filename.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    const LONG_NAME: &str = "export_file_name_that_is_rather_long_1_and_longer_still";

    /// A text a frame painted: what it says, whether it was cut short,
    /// where its line sits and whether it shows.
    struct PaintedText {
        text: String,
        elided: bool,
        rect: Rect,
        shows: bool,
    }

    /// A 400 wide row for a Mountain exporting as PNG16 under `LONG_NAME`.
    fn long_named_row() -> (egui::Context, Editor, BuildState, i32) {
        let context = egui::Context::default();
        theme::apply(&context);
        egui_extras::install_image_loaders(&context);
        let mut editor = Editor::new(Document::new());
        let id = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        editor.document.toggle_export(&[id], FileFormat::Png16);
        editor.document.rename_export_file(id, LONG_NAME);
        (context, editor, BuildState::default(), id)
    }

    /// One frame of the row with these events, and the texts it painted.
    fn row_frame(context: &egui::Context, editor: &mut Editor, state: &mut BuildState, id: i32, events: Vec<egui::Event>) -> Vec<PaintedText> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 200.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            let area = Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(400.0, 60.0));
            let mut area = ui.new_child(egui::UiBuilder::new().max_rect(area));
            savable(&mut area, editor, state, id, &mut Vec::new());
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let texts = shapes.iter().filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(PaintedText {
                text: text.galley.text().to_string(),
                elided: text.galley.elided,
                rect: text.galley.rect.translate(text.pos.to_vec2()),
                shows: text.galley.job.sections.iter().any(|section| section.format.color.a() > 0),
            }),
            _ => None,
        });
        texts.collect()
    }

    fn shown<'a>(texts: &'a [PaintedText], text: &str) -> &'a PaintedText {
        texts.iter().find(|painted| painted.shows && painted.text == text).unwrap_or_else(|| panic!("{text:?} shows"))
    }

    #[test]
    fn a_long_file_name_is_cut_short_level_with_its_format() {
        let (context, mut editor, mut state, id) = long_named_row();
        row_frame(&context, &mut editor, &mut state, id, Vec::new());
        let texts = row_frame(&context, &mut editor, &mut state, id, Vec::new());
        let name = shown(&texts, LONG_NAME);
        let format = shown(&texts, FileFormat::Png16.name());
        assert!(name.elided, "the name is cut short with an ellipsis");
        assert!(name.rect.right() <= format.rect.left(), "the name {:?} runs into the format {:?}", name.rect, format.rect);
        let offset = name.rect.center().y - format.rect.center().y;
        assert!(offset.abs() <= 0.5, "the name's line sits {offset} below the format's");
    }

    #[test]
    fn a_long_file_name_shows_whole_while_typed_in() {
        let (context, mut editor, mut state, id) = long_named_row();
        row_frame(&context, &mut editor, &mut state, id, Vec::new());
        let texts = row_frame(&context, &mut editor, &mut state, id, Vec::new());
        let name = shown(&texts, LONG_NAME).rect;
        let at = Pos2::new(name.left() + 20.0, name.center().y);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        row_frame(&context, &mut editor, &mut state, id, vec![egui::Event::PointerMoved(at)]);
        row_frame(&context, &mut editor, &mut state, id, vec![button(true)]);
        row_frame(&context, &mut editor, &mut state, id, vec![button(false)]);
        let texts = row_frame(&context, &mut editor, &mut state, id, Vec::new());
        let typed = shown(&texts, LONG_NAME);
        assert!(!typed.elided, "the whole name is there to edit");
        assert!((typed.rect.center().y - name.center().y).abs() <= 0.5, "the name stays on its line while typed in");
        assert_eq!(texts.iter().filter(|painted| painted.shows && painted.text == LONG_NAME).count(), 1, "the name shows once");
    }
}
