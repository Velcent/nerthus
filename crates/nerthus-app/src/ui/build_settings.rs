//! Build Settings (`BuildSettingsWindow`): the build definition's region,
//! resolution, subdivision and output, destination and options, the tiles'
//! naming and layout, the nodes marked for export (`SavablesArea`), the
//! post-build script, the terrain's size, its regions and the build
//! profiles.

use egui::{Align, Color32, Id, Layout, RichText, Sense, Stroke, Ui, Vec2};
use nerthus_engine::region::portion_json;
use nerthus_engine::Region;
use nerthus_export::build::{preview_destination, BuildDefinition, OutputType, Subdivision};
use nerthus_export::destination::{TEMPLATE_PRESETS, TOKEN_HELP};
use nerthus_export::tiles::{tile_suffix, OrganizeFiles, TILE_PATTERN_PRESETS};
use nerthus_export::FileFormat;
use serde_json::{json, Value};

use crate::document::ExportSettings;
use crate::editor::Editor;
use crate::theme;
use crate::ui::command_line_window;
use crate::ui::commands::Command;
use crate::ui::input_box::{InputAnswer, InputBox};
use crate::ui::prompt::{task_button, TaskDialog};
use crate::ui::regions::{self, RegionsState};
use crate::ui::tool_commands::ToolCommand;
use crate::ui::tool_window;
use crate::ui::widgets::{icon_menu, link_button, menu_item, menu_separator, scroll_bar_gutter, selectable_label, tab_list_item, Tooltips, TAB_ITEM_SIZE};

/// `BuildSettingsWindow`'s tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Resolution,
    Build,
    Tiles,
    Nodes,
    Script,
    Terrain,
    Regions,
    Profiles,
}

/// The tabs; Tiles is enabled with tiled output (`TileControlsVisibility`).
const TABS: [(SettingsTab, &str, bool, Color32); 8] = [
    (SettingsTab::Resolution, "Resolution", true, theme::ACCENT_MUTED_PURPLE),
    (SettingsTab::Build, "Build", true, theme::ACCENT_MUTED_PURPLE),
    (SettingsTab::Tiles, "Tiles", true, theme::ACCENT_MUTED_PURPLE),
    (SettingsTab::Nodes, "Nodes", true, theme::ACCENT_TRANSPARENT_BLUE),
    (SettingsTab::Script, "Script", true, theme::ACCENT_TRANSPARENT_GREEN),
    (SettingsTab::Terrain, "Terrain", true, theme::ACCENT_TRANSPARENT_BLUE),
    (SettingsTab::Regions, "Regions", true, theme::ACCENT_TRANSPARENT_GREEN),
    (SettingsTab::Profiles, "Profiles", true, theme::ACCENT_TRANSPARENT_GREEN),
];

/// `cboBlend`: the margin each side of a bucket, as a share of it.
const BLENDINGS: [(f32, &str); 4] = [(0.0, "0"), (0.25, "25%"), (0.5, "50%"), (1.0, "100%")];

/// `cboTileOrganization`.
const ORGANIZATIONS: [(OrganizeFiles, &str); 3] = [
    (OrganizeFiles::None, "All files in same folder"),
    (OrganizeFiles::NodeSubFolder, "Folders for each Node"),
    (OrganizeFiles::NodeAndPortSubFolder, "Node folders with Port subfolders"),
];

/// `cboOverlapingPixels`: 0 to 16.
const MOST_OVERLAP: usize = 16;

const DESTINATION_FIELD: &str = "build-destination";
const TILE_PATTERN_FIELD: &str = "build-tile-pattern";
const SCRIPT_FIELD: &str = "build-post-script";

/// `cboResolution`: the resolutions offered.
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

/// `Colorspace`, in order.
const COLOR_SPACES: [&str; 40] = [
    "Undefined", "CMY", "CMYK", "Gray", "HCL", "HCLp", "HSB", "HSI", "HSL", "HSV", "HWB", "Lab", "LCH", "LCHab", "LCHuv", "Log", "LMS", "Luv", "OHTA", "Rec601YCbCr", "Rec709YCbCr", "RGB", "scRGB", "sRGB", "Transparent", "XyY", "XYZ", "YCbCr", "YCC", "YDbDr", "YIQ", "YPbPr", "YUV", "LinearGray", "Jzazbz", "DisplayP3", "Adobe98", "ProPhoto", "Oklab", "Oklch",
];

/// The window's state while it is open.
#[derive(Clone, Debug)]
pub struct BuildSettingsState {
    pub tab: SettingsTab,
    /// File names being edited, by node.
    filenames: Vec<(i32, String)>,
    /// The Regions tab's canvas.
    pub regions: RegionsState,
    /// The Profiles tab: the profile selected (`lstProfiles`), the name
    /// Save As asks for, and the question waiting for Yes or No.
    profile: Option<String>,
    profile_name: Option<InputBox>,
    profile_question: Option<ProfileQuestion>,
}

/// What the Profiles tab asks before it changes a profile.
#[derive(Clone, Debug, PartialEq)]
enum ProfileQuestion {
    /// "A profile with this name already exists. Do you want to overwrite
    /// it?"
    Overwrite(String),
    /// "Are you sure you want to delete this profile?"
    Delete(String),
}

impl BuildSettingsState {
    pub fn new(tab: SettingsTab) -> Self {
        Self {
            tab,
            filenames: Vec::new(),
            regions: RegionsState::default(),
            profile: None,
            profile_name: None,
            profile_question: None,
        }
    }
}

/// Typing stays one undo gesture until the build text field loses focus.
/// The shell's per-key boundaries still apply to other value controls.
pub fn editing_text(ctx: &egui::Context) -> bool {
    let focused = ctx.memory(|memory| memory.focused());
    let fields = [DESTINATION_FIELD, TILE_PATTERN_FIELD, SCRIPT_FIELD];
    fields.iter().any(|field| focused == Some(Id::new(field)))
}

/// A section's title bar (`TextBlock` on `Panel-Border`, semibold).
fn section(ui: &mut Ui, title: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let position = egui::Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    ui.add_space(8.0);
}

/// `ListBox-OptionsX`: one segment per choice; returns the one clicked.
fn options(ui: &mut Ui, choices: &[(&str, bool)], current: usize) -> Option<usize> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
        for (index, (name, enabled)) in choices.iter().enumerate() {
            let selected = index == current;
            let text_color = if selected { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
            let text = RichText::new(*name).font(theme::interface(11.0)).color(text_color);
            let mut button = egui::Button::new(text);
            button = button.fill(if selected { theme::OPTIONS_SELECTION } else { theme::BUTTON_BACKGROUND });
            button = button.stroke(Stroke::new(1.0, theme::BUTTON_BORDER));
            button = button.min_size(Vec2::new(0.0, 24.0));
            if ui.add_enabled(*enabled, button).clicked() && !selected {
                chosen = Some(index);
            }
        }
    });
    chosen
}

fn caption(ui: &mut Ui, text: &str, color: Color32) {
    let label = RichText::new(text).font(theme::interface(12.0)).color(color);
    ui.label(label);
}

/// The window: tabs on the left, the tab's page, Execute Build, Copy
/// Command Line and Close below.
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut Option<BuildSettingsState>, commands: &mut Vec<Command>) {
    if tool_window::close_requested(ctx, Id::new("build-settings")) {
        if let Some(settings) = state.as_mut() {
            finish_filename_edits(ctx, editor, settings);
        }
        editor.document.finish_value_edit();
        *state = None;
        return;
    }
    let Some(settings) = state.as_mut() else {
        return;
    };
    let mut open = true;
    let mut close = false;
    let mut window = egui::Window::new("Build Settings and Regions");
    window = window.collapsible(false);
    window = window.resizable(true);
    window = tool_window::place(ctx, window, Id::new("build-settings"), Vec2::new(920.0, 640.0));
    window = window.open(&mut open);
    window.show(ctx, |ui| {
        let footer_height = 36.0;
        let body_height = (ui.available_height() - footer_height).max(200.0);
        ui.horizontal(|ui| {
            ui.set_min_height(body_height);
            let tiled = editor.document.build_definition().output_type == OutputType::CroppedTiles;
            if settings.tab == SettingsTab::Tiles && !tiled {
                settings.tab = SettingsTab::Resolution;
            }
            // `tabs` (`TabList`): each tab on its own brush when chosen, an
            // empty, disabled tab between Script and Terrain.
            ui.vertical(|ui| {
                ui.set_width(TAB_ITEM_SIZE.x);
                ui.spacing_mut().item_spacing.y = 2.0;
                for (tab, name, enabled, brush) in TABS {
                    if tab == SettingsTab::Terrain {
                        ui.add_space(TAB_ITEM_SIZE.y + 2.0);
                    }
                    let selected = settings.tab == tab;
                    let enabled = if tab == SettingsTab::Tiles { tiled } else { enabled };
                    if tab_list_item(ui, name, selected, enabled, brush).clicked() {
                        finish_filename_edits(ctx, editor, settings);
                        editor.document.finish_value_edit();
                        settings.tab = tab;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                scroll_bar_gutter(ui);
                let mut page = egui::ScrollArea::vertical();
                page = page.id_salt("build-settings-page");
                page = page.max_height(body_height);
                page = page.auto_shrink([false, false]);
                page.show(ui, |ui| match settings.tab {
                    SettingsTab::Resolution => resolution_tab(ui, editor),
                    SettingsTab::Build => build_tab(ui, editor),
                    SettingsTab::Nodes => nodes_tab(ui, editor, settings, commands),
                    SettingsTab::Script => script_tab(ui, editor, commands),
                    SettingsTab::Terrain => terrain_tab(ui, editor),
                    SettingsTab::Tiles => tiles_tab(ui, editor),
                    SettingsTab::Regions => regions::regions_tab(ui, editor, &mut settings.regions, commands),
                    SettingsTab::Profiles => profiles_tab(ui, editor, settings),
                });
            });
        });
        ui.separator();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            if ui.button("Copy Command Line").clicked() {
                let command_line = command_line_window::build_command_line(editor);
                ui.ctx().copy_text(command_line);
                editor.report("Copied the build command line");
            }
            if ui.button("Execute Build").clicked() {
                commands.push(Command::BuildAndExport);
                close = true;
            }
        });
    });
    profile_dialogs(ctx, editor, settings);
    if close || !open {
        finish_filename_edits(ctx, editor, settings);
        editor.document.finish_value_edit();
        *state = None;
    }
}

/// A tab change or external close can remove a filename field before it
/// gets to observe focus loss. Finish only drafts still being edited,
/// whose saved name has not changed underneath them.
fn finish_filename_edits(ctx: &egui::Context, editor: &mut Editor, settings: &mut BuildSettingsState) {
    let escaped = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    for (id, typed) in &mut settings.filenames {
        let field_id = Id::new(("build-settings-filename", *id));
        let editing_id = field_id.with("editing");
        let Some(original) = ctx.data_mut(|data| data.remove_temp::<String>(editing_id)) else {
            continue;
        };
        ctx.memory_mut(|memory| memory.surrender_focus(field_id));
        let Some(node) = editor.document.node(*id) else {
            continue;
        };
        let Some(export) = ExportSettings::of(node) else {
            continue;
        };
        if escaped || original != export.filename {
            *typed = export.filename;
            continue;
        }
        let trimmed = typed.trim();
        let filename = if trimmed.is_empty() { node.name.clone() } else { trimmed.to_string() };
        *typed = filename.clone();
        if filename != export.filename {
            editor.document.edit_export(*id, "Export filename", |settings| settings.filename = filename);
        }
    }
}

/// The Profiles tab: the profiles on the left, the selected one's settings
/// on the right (`ShowProfile`), and Load, Save, Save As… and Delete below.
fn profiles_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState) {
    section(ui, "Build Profiles");
    let names = editor.document.build_profiles();
    if settings.profile.as_ref().is_some_and(|selected| !names.contains(selected)) {
        settings.profile = None;
    }
    let list_height = (ui.available_height() - 48.0).max(240.0);
    // Two even columns (`Width="*"`): the list 4 short of the middle, the
    // box 4 past it.
    let column_width = ui.available_width() / 2.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let column = Layout::top_down(Align::Min);
        ui.allocate_ui_with_layout(Vec2::new(column_width - 4.0, list_height), column, |ui| {
            let list = egui::Frame::new().stroke(Stroke::new(1.0, theme::BUTTON_BORDER)).inner_margin(4.0);
            list.show(ui, |ui| {
                ui.set_min_height(list_height);
                ui.set_width(ui.available_width());
                for name in &names {
                    let selected = settings.profile.as_deref() == Some(name.as_str());
                    if selectable_label(ui, selected, name.as_str()).clicked() {
                        settings.profile = Some(name.clone());
                    }
                }
            });
        });
        ui.add_space(8.0);
        let summary = settings.profile.as_deref().and_then(|name| editor.document.build_profile(name)).map(|definition| nerthus_export::swarm::profile_summary(&definition));
        ui.allocate_ui_with_layout(Vec2::new(ui.available_width(), list_height), column, |ui| {
            profile_summary(ui, &summary.unwrap_or_default(), list_height);
        });
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let selected = settings.profile.clone();
        let chosen = selected.is_some();
        if ui.add_enabled(chosen, egui::Button::new("Load")).clicked()
            && let Some(name) = &selected
        {
            editor.document.load_build_profile(name);
            editor.report(format!("Loaded the build profile {name}"));
        }
        if ui.add_enabled(chosen, egui::Button::new("Save")).clicked()
            && let Some(name) = &selected
        {
            editor.document.save_build_profile(name);
        }
        if ui.button("Save As...").clicked() {
            let current = selected.clone().unwrap_or_default();
            settings.profile_name = Some(InputBox::new("Profile name", &current));
        }
        if ui.add_enabled(chosen, egui::Button::new("Delete")).clicked()
            && let Some(name) = selected
        {
            settings.profile_question = Some(ProfileQuestion::Delete(name));
        }
    });
}

/// `ShowProfile`'s box beside the list (`txtBuildProfile`, a text box),
/// as tall as the list's `height`: the profile's settings a line each. A
/// line longer than the box is cut off at its edge (no wrapping, and its
/// sideways scroll bar hidden); the box scrolls down when it must.
fn profile_summary(ui: &mut Ui, summary: &str, height: f32) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::TEXTBOX_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::TEXTBOX_BORDER));
    frame = frame.corner_radius(3.0);
    frame = frame.inner_margin(4.0);
    frame.show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let mut box_ui = tool_window::part(ui, "profile-summary", rect, Layout::top_down(Align::Min));
        scroll_bar_gutter(&mut box_ui);
        let mut scroll = egui::ScrollArea::vertical();
        scroll = scroll.auto_shrink([false, false]);
        scroll.show(&mut box_ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let font = theme::monospace(14.0);
            let color = theme::ACCENT_MUTED_GREEN;
            for line in summary.lines() {
                let galley = ui.painter().layout_no_wrap(line.to_string(), font.clone(), color);
                let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), galley.size().y), Sense::hover());
                let row_painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(row));
                row_painter.galley(row.min, galley, color);
            }
        });
    });
}

/// Save As's name and the tab's questions, over the window.
fn profile_dialogs(ctx: &egui::Context, editor: &mut Editor, settings: &mut BuildSettingsState) {
    if let Some(input) = settings.profile_name.as_mut() {
        match input.show(ctx, "profile-name") {
            Some(InputAnswer::Ok(name)) => {
                settings.profile_name = None;
                if editor.document.build_profiles().contains(&name) {
                    settings.profile_question = Some(ProfileQuestion::Overwrite(name));
                } else {
                    editor.document.save_build_profile(&name);
                    settings.profile = None;
                }
            }
            Some(InputAnswer::Cancel) => settings.profile_name = None,
            None => {}
        }
        return;
    }
    let Some(question) = settings.profile_question.clone() else {
        return;
    };
    let (caption_text, text) = match &question {
        ProfileQuestion::Overwrite(name) => (format!("Overwrite {name}"), "A profile with this name already exists. Do you want to overwrite it?"),
        ProfileQuestion::Delete(name) => (format!("Delete {name}"), "Are you sure you want to delete this profile?"),
    };
    let mut answer = None;
    let modal = TaskDialog::new(ctx, Id::new("profile-question"), 420.0);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, &caption_text, true, |ui| {
            ui.label(text);
        });
        task.buttons(ui, |ui| {
            if task_button(ui, "No").clicked() {
                answer = Some(false);
            }
            if task_button(ui, "Yes").clicked() {
                answer = Some(true);
            }
        });
    });
    if answer == Some(true) {
        match &question {
            ProfileQuestion::Overwrite(name) => editor.document.save_build_profile(name),
            ProfileQuestion::Delete(name) => editor.document.delete_build_profile(name),
        }
        settings.profile = None;
    }
    if answer.is_some() || response.should_close() {
        settings.profile_question = None;
    }
}

/// What Build Settings keeps in step with the resolution, the subdivision,
/// the output and the tile size, as the window does between
/// `GenerateBuckets`, `BucketChanged`, `GenerateTiles`, `TileChanged` and
/// `SaveBuildDefinition`: a subdivision the resolution rules out falls
/// back (to None, or to the next choice when the chosen one is merely
/// unavailable), the bucket size follows the subdivision, the tile size is
/// one Build Settings offers for it, and the build type and output type
/// follow the choices (Hybrid or Tiled only with a subdivision). With a
/// region chosen the build is a Region build of single images.
fn layout_members(resolution: usize, subdivision: Subdivision, tiled: bool, tile_resolution: usize, region: Option<&Region>) -> Vec<(&'static str, Value)> {
    let too_small = match subdivision {
        Subdivision::Slower => resolution < 4096,
        Subdivision::Balanced => resolution < 2048,
        Subdivision::Faster => resolution < 1024,
        Subdivision::None => false,
    };
    let mut subdivision = if too_small { Subdivision::None } else { subdivision };
    if !subdivision.available_at(resolution) {
        let index = Subdivision::ALL.iter().position(|choice| *choice == subdivision).unwrap_or(0);
        subdivision = Subdivision::ALL.get(index + 1).copied().filter(|next| next.available_at(resolution)).unwrap_or(Subdivision::None);
    }
    let bucket_resolution = subdivision.bucket_resolution(resolution);
    let sizes = tile_sizes(resolution, bucket_resolution);
    let tile_resolution = if sizes.contains(&tile_resolution) { tile_resolution } else { sizes.first().copied().unwrap_or(tile_resolution) };
    let build_type = match (subdivision, tiled) {
        _ if region.is_some() => "Region",
        (Subdivision::None, _) => "Standard",
        (_, false) => "Hybrid",
        (_, true) => "Tiled",
    };
    let output_type = if tiled && region.is_none() { OutputType::CroppedTiles } else { OutputType::SingleImage };
    vec![
        ("Type", Value::from(build_type)),
        ("Subdivision", Value::from(subdivision.name())),
        ("Resolution", json!(resolution)),
        ("TileResolution", json!(tile_resolution)),
        ("BucketResolution", json!(bucket_resolution)),
        ("OutputType", Value::from(output_type.name())),
        ("NumberOfTiles", json!(resolution / tile_resolution.max(1))),
    ]
}

/// `GenerateTiles`: from the bucket's size (half the terrain without a
/// subdivision) down by halves, larger than 64.
fn tile_sizes(resolution: usize, bucket_resolution: usize) -> Vec<usize> {
    let mut size = if bucket_resolution == resolution { resolution / 2 } else { bucket_resolution };
    let mut sizes = Vec::new();
    while size > 64 {
        sizes.push(size);
        size /= 2;
    }
    sizes
}

fn apply_layout(editor: &mut Editor, resolution: usize, subdivision: Subdivision, tiled: bool, tile_resolution: usize, region: Option<&Region>) {
    let members = layout_members(resolution, subdivision, tiled, tile_resolution, region);
    editor.document.set_build_settings(&members);
}

/// The region the Region box shows (`LoadProjectArea`): the terrain's
/// region the definition's `Portion` names; none (Whole Terrain) when it
/// names none the terrain has.
fn selected_region(editor: &Editor, definition: &BuildDefinition) -> Option<Region> {
    let portion = definition.portion.as_ref()?;
    editor.document.regions().into_iter().find(|region| region.name == portion.name)
}

/// `cboResolution`: one of the resolutions offered.
fn resolution_choice(ui: &mut Ui, current: usize) -> Option<usize> {
    caption(ui, "Resolution", theme::LABEL_DEFAULT);
    let current_name = RESOLUTIONS.iter().find(|(size, _)| *size == current).map(|(_, name)| *name).unwrap_or("Custom");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-resolution");
    combo = combo.selected_text(current_name);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for (size, name) in RESOLUTIONS {
            if selectable_label(ui, size == current, name).clicked() {
                chosen = Some(size);
            }
        }
    });
    chosen
}

/// `cboRegionSelector` (`ApplyRegionSelection`, then `SaveBuildDefinition`):
/// Whole Terrain, or a region to build on its own as single images.
fn region_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition, selected: Option<&Region>) {
    caption(ui, "Region", theme::LABEL_DEFAULT);
    let regions = editor.document.regions();
    let current = selected.map(|region| region.name.clone()).unwrap_or_else(|| "Whole Terrain".to_string());
    let mut chosen: Option<Option<Region>> = None;
    let mut combo = egui::ComboBox::from_id_salt("build-region");
    combo = combo.selected_text(current);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        if selectable_label(ui, selected.is_none(), "Whole Terrain").clicked() {
            chosen = Some(None);
        }
        for region in &regions {
            let is_selected = selected.is_some_and(|selected| selected.name == region.name);
            if selectable_label(ui, is_selected, region.name.as_str()).clicked() {
                chosen = Some(Some(region.clone()));
            }
        }
    });
    let Some(choice) = chosen else {
        return;
    };
    let tiled = definition.output_type == OutputType::CroppedTiles;
    let mut members = layout_members(definition.resolution, definition.subdivision, tiled, definition.tile_resolution, choice.as_ref());
    let portion = choice.as_ref().map(portion_json).unwrap_or(Value::Null);
    members.push(("Portion", portion));
    editor.document.set_build_settings(&members);
}

fn resolution_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Build Definition");
    let definition = editor.document.build_definition();
    let tiled = definition.output_type == OutputType::CroppedTiles;
    let selected = selected_region(editor, &definition);
    region_choice(ui, editor, &definition, selected.as_ref());
    ui.add_space(16.0);

    // With a region, the resolution is the region's size and there is no
    // subdivision or output to choose (`cboResolution_OnSelectionChanged`).
    if let Some(region) = selected {
        if let Some(size) = resolution_choice(ui, region.size.max(0) as usize) {
            let mut resized = region.clone();
            resized.size = size as i32;
            editor.document.update_region(&region.name, resized);
        }
        // `CheckRegionSelectBuildDefWarning`.
        if definition.resolution <= 2048 {
            ui.add_space(12.0);
            regions::warning(ui, "Regions require the base resolution of the terrain to be at least 4096.");
        }
        return;
    }
    if let Some(size) = resolution_choice(ui, definition.resolution) {
        apply_layout(editor, size, definition.subdivision, tiled, definition.tile_resolution, None);
    }

    ui.add_space(16.0);
    caption(ui, "Hybrid Subdivision", theme::LABEL_DEFAULT);
    let subdivisions: Vec<(&str, bool)> = Subdivision::ALL.iter().map(|choice| (choice.name(), choice.available_at(definition.resolution))).collect();
    let current_subdivision = Subdivision::ALL.iter().position(|choice| *choice == definition.subdivision).unwrap_or(0);
    if let Some(index) = options(ui, &subdivisions, current_subdivision) {
        apply_layout(editor, definition.resolution, Subdivision::ALL[index], tiled, definition.tile_resolution, None);
    }
    ui.add_space(16.0);
    caption(ui, "Output", theme::LABEL_DEFAULT);
    let outputs = [("Single Image", true), ("Tiled Images", true)];
    if let Some(index) = options(ui, &outputs, usize::from(tiled)) {
        apply_layout(editor, definition.resolution, definition.subdivision, index == 1, definition.tile_resolution, None);
    }

    if tiled {
        ui.add_space(16.0);
        tile_size_choice(ui, editor, &definition);
        if definition.subdivision != Subdivision::None {
            ui.add_space(16.0);
            blending_choice(ui, editor, &definition);
        }
    }

    // `UpdateTileLabels`.
    ui.add_space(8.0);
    let per_side = definition.resolution / definition.bucket_resolution.max(1);
    if per_side > 1 {
        let divisions = per_side * per_side;
        caption(ui, &format!("{divisions} divisions"), theme::MENU_SEPARATOR);
    }
    let tiles_per_side = definition.resolution / definition.tile_resolution.max(1);
    if tiled && tiles_per_side > 1 {
        let count = grouped(tiles_per_side * tiles_per_side);
        caption(ui, &format!("{tiles_per_side} x {tiles_per_side} ({count} tiles)"), theme::MENU_SEPARATOR);
    }

    // `CheckResolutionWarning`.
    if definition.resolution == 16384 && definition.subdivision == Subdivision::None {
        ui.add_space(12.0);
        caption(ui, "WARNING: 16K builds require an enormous amount of RAM.", theme::ACCENT_RED);
        caption(ui, "Complex builds may require over 128GB RAM, and GPU nodes may require over 12GB RAM.", theme::ACCENT_RED);
    }
}

/// `cboTile`: the sizes `GenerateTiles` offers.
fn tile_size_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition) {
    caption(ui, "Tile Size", theme::LABEL_DEFAULT);
    let sizes = tile_sizes(definition.resolution, definition.bucket_resolution);
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-size");
    combo = combo.selected_text(format!("{0} x {0}", definition.tile_resolution));
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for size in sizes {
            if selectable_label(ui, size == definition.tile_resolution, format!("{size} x {size}")).clicked() {
                chosen = Some(size);
            }
        }
    });
    if let Some(size) = chosen {
        apply_layout(editor, definition.resolution, definition.subdivision, true, size, None);
    }
}

/// `cboBlend`: `EdgeBlending`.
fn blending_choice(ui: &mut Ui, editor: &mut Editor, definition: &BuildDefinition) {
    caption(ui, "Blending", theme::LABEL_DEFAULT);
    let current = BLENDINGS.iter().find(|(share, _)| *share == definition.edge_blending).map(|(_, name)| *name).unwrap_or("");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-blending");
    combo = combo.selected_text(current);
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for (share, name) in BLENDINGS {
            if selectable_label(ui, share == definition.edge_blending, name).clicked() {
                chosen = Some(share);
            }
        }
    });
    if let Some(share) = chosen {
        editor.document.set_build_setting("EdgeBlending", json!(share));
    }
}

/// `###,###`.
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

/// The Tiles tab (`stackTiles`): how tiles are named and laid out.
fn tiles_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Tiled Output Settings");
    let definition = editor.document.build_definition();
    caption(ui, "Tile Suffix Pattern", theme::LABEL_DEFAULT);
    // Builds fall back to a default for an empty pattern; the editor must
    // preserve the empty string while the user replaces it.
    let saved_definition = editor.document.project.assets[0].extra.get("BuildDefinition");
    let saved_pattern = saved_definition.and_then(|value| value.get("TilePattern")).and_then(Value::as_str);
    let mut typed = saved_pattern.unwrap_or(&definition.tile_pattern).to_string();
    ui.horizontal(|ui| {
        let mut field = egui::TextEdit::singleline(&mut typed);
        field = field.id(Id::new(TILE_PATTERN_FIELD));
        field = field.font(theme::monospace(12.0));
        field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
        field = field.desired_width(220.0);
        let response = ui.add(field);
        if response.changed() {
            editor.document.set_build_setting_edited("TilePattern", Value::String(typed.clone()));
        }
        if response.lost_focus() {
            editor.document.finish_value_edit();
        }
        icon_menu(ui, "chevron-down", "Presets", None, |ui| {
            for preset in TILE_PATTERN_PRESETS {
                if menu_item(ui, None, None, preset, "").clicked() {
                    typed = preset.to_string();
                    editor.document.set_build_setting("TilePattern", Value::String(typed.clone()));
                    ui.close();
                }
            }
        });
    });
    // `UpdateTilePatternPreview`: the first tile's suffix as the boxes
    // stand.
    let definition = editor.document.build_definition();
    let pad = if definition.tile_pad_digits { 2 } else { 1 };
    let buckets = definition.resolution / definition.bucket_resolution.max(1);
    let suffix = tile_suffix(0, 0, &definition.tile_pattern, !definition.tile_zero_index, pad, buckets, true);
    let preview = RichText::new(format!("FileName{suffix}.ext")).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(preview);

    ui.add_space(8.0);
    let mut pad_digits = definition.tile_pad_digits;
    if ui.checkbox(&mut pad_digits, "Add Leading Zeroes").changed() {
        editor.document.set_build_setting("TilePadDigits", Value::Bool(pad_digits));
    }
    let mut from_one = !definition.tile_zero_index;
    if ui.checkbox(&mut from_one, "Start numbering from 1").changed() {
        editor.document.set_build_setting("TileZeroIndex", Value::Bool(!from_one));
    }

    ui.add_space(16.0);
    caption(ui, "Organization", theme::LABEL_DEFAULT);
    let current = ORGANIZATIONS.iter().find(|(organize, _)| *organize == definition.organize_files).map(|(_, name)| *name).unwrap_or("");
    let mut chosen = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-organization");
    combo = combo.selected_text(current);
    combo = combo.width(360.0);
    combo.show_ui(ui, |ui| {
        for (organize, name) in ORGANIZATIONS {
            if selectable_label(ui, organize == definition.organize_files, name).clicked() {
                chosen = Some(organize);
            }
        }
    });
    if let Some(organize) = chosen {
        editor.document.set_build_setting("OrganizeFiles", Value::from(organize.name()));
    }
    ui.separator();

    let mut flip = definition.flip_y;
    if ui.checkbox(&mut flip, "Flip Y axis").changed() {
        editor.document.set_build_setting("FlipY", Value::Bool(flip));
    }
    let mut preserve = definition.preserve_tiled_cache;
    if ui.checkbox(&mut preserve, "Preserve Cache in Build Folder").changed() {
        editor.document.set_build_setting("PreserveTiledCache", Value::Bool(preserve));
    }

    ui.add_space(16.0);
    caption(ui, "Overlap Pixels", theme::LABEL_DEFAULT);
    let mut chosen_overlap = None;
    let mut combo = egui::ComboBox::from_id_salt("build-tile-overlap");
    combo = combo.selected_text(definition.overlapping_pixels.to_string());
    combo = combo.width(360.0);
    combo.show_ui(ui, |ui| {
        for overlap in 0..=MOST_OVERLAP {
            if selectable_label(ui, overlap == definition.overlapping_pixels, overlap.to_string()).clicked() {
                chosen_overlap = Some(overlap);
            }
        }
    });
    if let Some(overlap) = chosen_overlap {
        editor.document.set_build_setting("OverlappingPixels", json!(overlap));
    }
}

fn build_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Export Settings");
    let definition = editor.document.build_definition();
    caption(ui, "Build Destination", theme::LABEL_DEFAULT);
    let mut typed = definition.destination.clone();
    ui.horizontal(|ui| {
        let mut field = egui::TextEdit::singleline(&mut typed);
        field = field.id(Id::new(DESTINATION_FIELD));
        field = field.font(theme::semibold(12.0));
        field = field.text_color(theme::TEXTBOX_FILE_FOREGROUND);
        field = field.hint_text(nerthus_export::destination::DEFAULT_TEMPLATE);
        field = field.desired_width(ui.available_width() - 70.0);
        let response = ui.add(field);
        if response.changed() {
            editor.document.set_build_setting_edited("Destination", Value::String(typed.clone()));
        }
        if response.lost_focus() {
            editor.document.finish_value_edit();
        }
        icon_menu(ui, "chevron-down", "Presets", None, |ui| {
            for preset in TEMPLATE_PRESETS {
                if menu_item(ui, None, None, preset, "").clicked() {
                    typed = preset.to_string();
                    editor.document.set_build_setting("Destination", Value::String(typed.clone()));
                    ui.close();
                }
            }
        });
    });
    let preview = match &editor.document.path {
        Some(path) => {
            let preferences = editor.user.preferences.export_preferences();
            let definition = editor.document.build_definition();
            preview_destination(path, &definition, &preferences).display().to_string()
        }
        None => "Save the project to see where it builds.".to_string(),
    };
    let preview_text = RichText::new(preview).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(preview_text);

    ui.add_space(8.0);
    caption(ui, "ColorSpace", theme::LABEL_DEFAULT);
    let mut chosen_space = None;
    let current_space = definition.color_space.clone();
    let mut combo = egui::ComboBox::from_id_salt("build-color-space");
    combo = combo.selected_text(current_space.as_str());
    combo = combo.width(260.0);
    combo.show_ui(ui, |ui| {
        for name in COLOR_SPACES {
            if selectable_label(ui, name == current_space, name).clicked() {
                chosen_space = Some(name);
            }
        }
    });
    if let Some(name) = chosen_space {
        editor.document.set_build_setting("ColorSpace", Value::String(name.to_string()));
    }
    caption(ui, "Applies to 16-bit and 8-bit RGB outputs only.", theme::MENU_SEPARATOR);

    ui.add_space(16.0);
    let flags = [
        ("MaintainPrimeFolder", "Maintain a static folder with the latest copy", definition.maintain_prime_folder, true),
        ("OpenFolder", "Open folder after build finishes", definition.open_folder, true),
        ("CopyTerrain", "Copy the .terrain file to the build folder", definition.copy_terrain, true),
        ("RemovePrimaryPortName", "Remove Primary port name in Build output", definition.remove_primary_port_name, true),
    ];
    for (member, text, value, enabled) in flags {
        let mut checked = value;
        let response = ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, text));
        if response.changed() {
            editor.document.set_build_setting(member, Value::Bool(checked));
        }
    }

    ui.add_space(16.0);
    caption(ui, "File overwrite mode", theme::LABEL_DEFAULT);
    let increment = definition.overwrite_mode == nerthus_export::build::OverwriteMode::Increment;
    let modes = [("Overwrite", true), ("Increment", true)];
    if let Some(choice) = options(ui, &modes, usize::from(increment)) {
        let name = if choice == 1 { "Increment" } else { "Overwrite" };
        editor.document.set_build_setting("OverwriteMode", Value::String(name.to_string()));
    }
    ui.add_space(8.0);
    let tokens = RichText::new(TOKEN_HELP).font(theme::monospace(11.0)).color(theme::MENU_SEPARATOR);
    ui.label(tokens);
}

/// `SavablesArea`: one row per marked node, by graph tab.
fn nodes_tab(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState, commands: &mut Vec<Command>) {
    section(ui, "Exportable Nodes");
    let marked: Vec<i32> = editor.document.terrain().nodes.values().filter(|node| ExportSettings::of(node).is_some()).map(|node| node.id).collect();
    if marked.is_empty() {
        caption(ui, "Mark nodes as Exportable (F3) to see them here.", theme::LABEL_DEFAULT);
        return;
    }
    ui.horizontal(|ui| {
        let response = ui.button("Change formats for all nodes");
        let mut popup = egui::Popup::menu(&response);
        popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        popup.show(|ui| crate::ui::widgets::menu_rows_below(ui, response.interact_rect, |ui| {
            for format in FileFormat::ALL {
                if menu_item(ui, None, None, format.name(), "").clicked() {
                    editor.document.set_all_export_formats(format);
                }
            }
        }));
        if editor.document.has_duplicate_export_names() {
            let warning = RichText::new("There are duplicate filenames! Click to fix them!").color(theme::ACCENT_RED);
            if ui.link(warning).clicked() {
                editor.document.fix_duplicate_export_names();
                settings.filenames.clear();
            }
        }
    });
    ui.add_space(4.0);
    for tab in editor.document.graph_tabs() {
        let in_tab: Vec<i32> = marked.iter().copied().filter(|id| editor.document.node(*id).is_some_and(|node| node_tab(node) == tab.index)).collect();
        let mut header = egui::CollapsingHeader::new(tab.name.as_str());
        header = header.id_salt(("export-tab", tab.index));
        header = header.default_open(true);
        header.show(ui, |ui| {
            for id in in_tab {
                savable(ui, editor, settings, id, commands);
            }
        });
    }
}

fn node_tab(node: &nerthus_engine::NodeData) -> usize {
    node.base.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// `Savable`: the node's toolbox colour, its ports menu, its file name
/// and its format.
fn savable(ui: &mut Ui, editor: &mut Editor, settings: &mut BuildSettingsState, id: i32, commands: &mut Vec<Command>) {
    let Some(node) = editor.document.node(id).cloned() else {
        return;
    };
    let Some(export) = ExportSettings::of(&node) else {
        return;
    };
    let catalogue = nerthus_engine::Catalogue::get();
    let category = catalogue.node(&node.type_name).and_then(|spec| spec.toolbox.clone()).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(2.0, 26.0), egui::Sense::hover());
        ui.painter().rect_filled(bar, 0.0, theme::toolbox_color(&category));
        {
            icon_menu(ui, "menu-deep", "Ports", None, |ui| {
                if ui.button(format!("Node: {}", node.name)).clicked() {
                    commands.push(Command::SelectNode(id));
                    ui.close();
                }
                menu_separator(ui);
                let mut enabled = export.enabled;
                if ui.checkbox(&mut enabled, "Enabled").changed() {
                    editor.document.edit_export(id, "Export enabled", |settings| settings.enabled = enabled);
                }
                menu_separator(ui);
                for port in node.ports.iter().filter(|port| port.is_output()) {
                    let mut exporting = port.is_exporting;
                    let response = ui.add_enabled(export.enabled, egui::Checkbox::new(&mut exporting, port.name.as_str()));
                    if response.changed() {
                        editor.document.set_port_exporting(id, &port.name, exporting);
                    }
                }
                menu_separator(ui);
                let outputs: Vec<String> = node.ports.iter().filter(|port| port.is_output()).map(|port| port.name.clone()).collect();
                if ui.add_enabled(export.enabled, egui::Button::new("Primary Only")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, name == "Out");
                    }
                }
                if ui.add_enabled(export.enabled, egui::Button::new("Mark all")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, true);
                    }
                }
                if ui.add_enabled(export.enabled, egui::Button::new("Mark none")).clicked() {
                    for name in &outputs {
                        editor.document.set_port_exporting(id, name, false);
                    }
                }
            });
        }

        // The file name, saved when focus leaves (empty: the node's name);
        // only Escape puts it back.
        let field_id = Id::new(("build-settings-filename", id));
        let editing_id = field_id.with("editing");
        let index = match settings.filenames.iter().position(|(node_id, _)| *node_id == id) {
            Some(index) => index,
            None => {
                // A fresh view may reuse a node id from the old document.
                ui.data_mut(|data| data.remove::<String>(editing_id));
                settings.filenames.push((id, export.filename.clone()));
                settings.filenames.len() - 1
            }
        };
        let was_focused = ui.memory(|memory| memory.has_focus(field_id));
        let name_width = (ui.available_width() - 120.0).max(80.0);
        let mut field = egui::TextEdit::singleline(&mut settings.filenames[index].1);
        field = field.id(field_id);
        field = field.font(theme::semibold(12.0));
        field = field.desired_width(name_width);
        let response = ui.add_enabled(export.enabled, field);
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if was_focused || response.has_focus() {
            ui.data_mut(|data| {
                data.get_temp_mut_or_insert_with(editing_id, || export.filename.clone());
            });
        }
        // egui can report the same focus loss again next frame. Finish
        // each edit once so that an immediate undo stays undone.
        let original = if response.lost_focus() {
            ui.data_mut(|data| data.remove_temp::<String>(editing_id))
        } else {
            None
        };
        let committed = original.as_deref() == Some(export.filename.as_str()) && !escaped;
        if committed {
            let typed = settings.filenames[index].1.trim().to_string();
            let filename = if typed.is_empty() { node.name.clone() } else { typed };
            settings.filenames[index].1 = filename.clone();
            if filename != export.filename {
                editor.document.edit_export(id, "Export filename", |settings| settings.filename = filename);
            }
        } else {
            // Escape, or the name changed elsewhere (undo, fixing
            // duplicates): show the saved name.
            let cancelled = original.is_some() && escaped;
            let stale = !response.has_focus() && settings.filenames[index].1 != export.filename;
            if cancelled || stale {
                settings.filenames[index].1 = export.filename.clone();
            }
        }

        let mut chosen = None;
        let mut combo = egui::ComboBox::from_id_salt(("export-format", id));
        combo = combo.selected_text(export.format.name());
        combo = combo.width(105.0);
        ui.add_enabled_ui(export.enabled, |ui| {
            combo.show_ui(ui, |ui| {
                for format in FileFormat::ALL {
                    let item = selectable_label(ui, format == export.format, format.name());
                    if item.tooltip(format.description()).clicked() {
                        chosen = Some(format);
                    }
                }
            });
        });
        if let Some(format) = chosen {
            editor.document.edit_export(id, "Export format", |settings| settings.format = format);
        }
    });
}

fn script_tab(ui: &mut Ui, editor: &mut Editor, commands: &mut Vec<Command>) {
    section(ui, "Post-Build Script");
    caption(ui, "Enter any command line instructions to execute after a successful build.", theme::LABEL_DEFAULT);
    caption(ui, "The working directory will be the current build path.", theme::LABEL_DEFAULT);
    ui.add_space(8.0);
    let mut script = editor.document.build_definition().post_build_script;
    let mut field = egui::TextEdit::multiline(&mut script);
    field = field.id(Id::new(SCRIPT_FIELD));
    field = field.font(theme::monospace(14.0));
    field = field.text_color(theme::ACCENT_MUTED_GREEN);
    field = field.desired_rows(12);
    field = field.desired_width(f32::INFINITY);
    let response = ui.add(field);
    if response.changed() {
        editor.document.set_build_setting_edited("PostBuildScript", Value::String(script));
    }
    if response.lost_focus() {
        editor.document.finish_value_edit();
    }
    // "Learn more" (`Button-Link`, no padding): the documentation on
    // post-build scripts.
    ui.add_space(8.0);
    if link_button(ui, "Learn more", theme::interface(12.0), Vec2::ZERO).clicked() {
        commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::POST_BUILD_SCRIPT.to_string())));
    }
}

fn terrain_tab(ui: &mut Ui, editor: &mut Editor) {
    section(ui, "Terrain Definition");
    let terrain = editor.document.terrain();
    let mut width = terrain.width;
    let mut height = terrain.height;
    ui.horizontal(|ui| {
        caption(ui, "Width", theme::LABEL_DEFAULT);
        let mut value = egui::DragValue::new(&mut width);
        value = value.range(100.0..=2_400_000.0);
        value = value.suffix(" m");
        let response = ui.add(value);
        if response.changed() {
            editor.document.set_terrain_size_edited("Width", width.round(), height.round());
            editor.changed();
        }
        if response.drag_stopped() || response.lost_focus() {
            editor.document.finish_value_edit();
        }
    });
    ui.horizontal(|ui| {
        caption(ui, "Height", theme::LABEL_DEFAULT);
        let mut value = egui::DragValue::new(&mut height);
        value = value.range(100.0..=10_000.0);
        value = value.suffix(" m");
        let response = ui.add(value);
        if response.changed() {
            editor.document.set_terrain_size_edited("Height", width.round(), height.round());
            editor.changed();
        }
        if response.drag_stopped() || response.lost_focus() {
            editor.document.finish_value_edit();
        }
    });
    let terrain = editor.document.terrain();
    let resolution = editor.document.build_definition().resolution;
    let meters_per_pixel = f64::from(terrain.width) / resolution as f64;
    let height_to_width = f64::from(terrain.height) / f64::from(terrain.width);
    let real_scale = format!("Real scale: {meters_per_pixel:.4} m/pixel at {resolution} × {resolution}");
    let ratio = format!("Height-to-width ratio: {height_to_width:.6} ({:.4}%)", height_to_width * 100.0);
    ui.add_space(12.0);
    caption(ui, &real_scale, theme::LABEL_EMPHASIS);
    caption(ui, &ratio, theme::LABEL_EMPHASIS);
    ui.add_space(12.0);
    caption(ui, "TIP:", theme::LABEL_EMPHASIS);
    caption(ui, "Think about your terrain size realistically. Most common mountains and hills tend to be quite smaller than what new users may anticipate. Precise size and height of your terrain will result in more realistic erosion and other simulations.", theme::LABEL_DEFAULT);
    ui.add_space(6.0);
    for line in ["Mt. Everest (Nepal): ~8800m tall", "Mt. Saint Helens (US): ~2500m tall", "Ben Nevis (Scotland): ~1300m tall", "Mt. Fuji (Japan): ~3700m tall"] {
        let text = RichText::new(line).font(theme::monospace(11.0)).color(theme::LABEL_DEFAULT);
        ui.label(text);
    }
    ui.add_space(6.0);
    caption(ui, "In practical terms, your meter/pixel ratio is more important in CG.", theme::LABEL_DEFAULT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::epaint::{ClippedShape, TextShape};
    use egui::{AreaState, Event, Key, Modifiers, MouseWheelUnit, Pos2, RawInput, Rect, Shape, TouchPhase};
    use nerthus_export::swarm::PROFILE_SEPARATOR;

    /// The screen, in points, of 1600 × 960 at the default 120%.
    const LARGE: Vec2 = Vec2::new(1333.0, 800.0);
    /// 800 × 600 and 640 × 480 at 120%.
    const SMALL_SCREENS: [Vec2; 2] = [Vec2::new(666.0, 500.0), Vec2::new(533.0, 400.0)];

    fn context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context
    }

    /// A frame at this screen size with these events. Returns what was
    /// painted and the window's rect.
    fn frame(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<BuildSettingsState>, events: Vec<Event>) -> (Vec<ClippedShape>, Rect) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            let (starts, finishes) = crate::ui::value_edit_boundaries(ui.ctx());
            if starts {
                editor.document.finish_value_edit();
            }
            show(ui.ctx(), editor, state, &mut Vec::new());
            if finishes {
                editor.document.finish_value_edit();
            }
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let area = AreaState::load(context, Id::new("build-settings")).filter(|area| area.size.is_some());
        (shapes, area.map(|area| area.rect()).unwrap_or(Rect::NOTHING))
    }

    /// Frames enough for egui's first-frame sizing to settle and the
    /// window to fade in.
    fn settle(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<BuildSettingsState>) -> (Vec<ClippedShape>, Rect) {
        for _ in 0..30 {
            frame(context, screen, editor, state, Vec::new());
        }
        frame(context, screen, editor, state, Vec::new())
    }

    fn texts(shapes: &[ClippedShape]) -> impl Iterator<Item = &TextShape> {
        shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) => Some(text),
            _ => None,
        })
    }

    fn key_event(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    fn focus_field(context: &egui::Context, editor: &mut Editor, state: &mut Option<BuildSettingsState>, field: &str) {
        context.memory_mut(|memory| memory.request_focus(Id::new(field)));
        frame(context, LARGE, editor, state, Vec::new());
        frame(context, LARGE, editor, state, vec![key_event(Key::End, true)]);
        frame(context, LARGE, editor, state, vec![key_event(Key::End, false)]);
    }

    fn type_character(context: &egui::Context, editor: &mut Editor, state: &mut Option<BuildSettingsState>, key: Key, text: &str) {
        let events = vec![key_event(key, true), Event::Text(text.to_string())];
        frame(context, LARGE, editor, state, events);
        frame(context, LARGE, editor, state, vec![key_event(key, false)]);
    }

    fn assert_field_text(context: &egui::Context, editor: &mut Editor, state: &mut Option<BuildSettingsState>, expected: &str) {
        let (shapes, _) = frame(context, LARGE, editor, state, Vec::new());
        assert!(texts(&shapes).any(|text| text.galley.text() == expected), "the field does not show {expected:?}");
    }

    #[test]
    fn opening_and_browsing_build_settings_preserves_the_saved_project() {
        let inconsistent = json!({
            "Type": "Tiled",
            "Subdivision": "Slower",
            "Resolution": 512,
            "TileResolution": 96,
            "BucketResolution": 77,
            "OutputType": "CroppedTiles",
            "NumberOfTiles": 13,
        });
        for definition in [None, Some(inconsistent)] {
            let context = context();
            let mut document = Document::new();
            if let Some(definition) = definition {
                document.project.assets[0].extra.insert("BuildDefinition".to_string(), definition);
            } else {
                assert_eq!(document.build_definition().number_of_tiles, 3);
            }
            let saved_project = nerthus_engine::to_terrain_json(&document.project);
            let mut editor = Editor::new(document);
            let mut state = Some(BuildSettingsState::new(SettingsTab::Resolution));
            settle(&context, LARGE, &mut editor, &mut state);

            for (tab, _, _, _) in TABS {
                state.as_mut().unwrap().tab = tab;
                frame(&context, LARGE, &mut editor, &mut state, Vec::new());
            }
            tool_window::request_close(&context, Id::new("build-settings"));
            frame(&context, LARGE, &mut editor, &mut state, Vec::new());
            assert!(state.is_none());
            state = Some(BuildSettingsState::new(SettingsTab::Resolution));
            frame(&context, LARGE, &mut editor, &mut state, Vec::new());

            assert_eq!(nerthus_engine::to_terrain_json(&editor.document.project), saved_project);
            assert!(!editor.document.dirty);
            assert!(editor.document.undo_labels().is_empty());
            assert!(editor.document.redo_labels().is_empty());

            apply_layout(&mut editor, 4096, Subdivision::Balanced, true, 1024, None);
            assert!(editor.document.dirty);
            assert_eq!(editor.document.undo_labels(), ["Build settings"]);
            assert_eq!(editor.document.build_definition().resolution, 4096);
        }
    }

    #[test]
    fn copying_the_build_command_uses_the_complete_command_without_editing() {
        use nerthus_engine::automation::{Variable, VariableValue};

        for saved_with_variables in [false, true] {
            let context = context();
            let mut document = Document::new();
            if saved_with_variables {
                document.path = Some(std::path::PathBuf::from("/projects/a 'quoted' terrain.terrain"));
                let variables = [
                    ("Height", VariableValue::Float(0.5)),
                    ("Style", VariableValue::Text("ridge 'north' \"south\"".to_string())),
                ];
                for (name, value) in variables {
                    let saved_variable = json!({"Name": name});
                    let mut variable = Variable::from_json(&saved_variable);
                    variable.value = value;
                    document.project.assets[0].automation.variables.insert(name.to_string(), variable);
                }
            }
            let saved_project = nerthus_engine::to_terrain_json(&document.project);
            let mut editor = Editor::new(document);
            let expected = crate::ui::command_line_window::build_command_line(&editor);
            let mut state = Some(BuildSettingsState::new(SettingsTab::Build));
            let (shapes, _) = settle(&context, LARGE, &mut editor, &mut state);
            let copy_button = texts(&shapes).find(|text| text.galley.text() == "Copy Command Line");
            let position = copy_button.expect("the copy button is painted").visual_bounding_rect().center();
            let press = Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
            frame(&context, LARGE, &mut editor, &mut state, vec![Event::PointerMoved(position), press]);

            let release = Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, LARGE)), events: vec![release], ..Default::default() };
            let output = context.run_ui(input, |ui| {
                let (starts, finishes) = crate::ui::value_edit_boundaries(ui.ctx());
                if starts {
                    editor.document.finish_value_edit();
                }
                show(ui.ctx(), &mut editor, &mut state, &mut Vec::new());
                if finishes {
                    editor.document.finish_value_edit();
                }
            });
            let copied: Vec<&str> = output.platform_output.commands.iter().filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            }).collect();
            assert_eq!(copied, [expected.as_str()]);
            output.drop_without_applying_deltas();

            assert_eq!(nerthus_engine::to_terrain_json(&editor.document.project), saved_project);
            assert!(!editor.document.dirty);
            assert!(editor.document.undo_labels().is_empty());
            assert!(editor.document.redo_labels().is_empty());
            assert!(state.is_some());
        }
    }

    #[test]
    fn build_text_typing_is_one_undo_step_across_key_presses_and_releases() {
        let fields = [
            (SettingsTab::Build, DESTINATION_FIELD, "Destination"),
            (SettingsTab::Tiles, TILE_PATTERN_FIELD, "TilePattern"),
            (SettingsTab::Script, SCRIPT_FIELD, "PostBuildScript"),
        ];
        for (tab, field, member) in fields {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let initial = [("OutputType", Value::from("CroppedTiles")), (member, Value::from("base"))];
            editor.document.set_build_settings(&initial);
            let history_length = editor.document.undo_labels().len();
            let mut state = Some(BuildSettingsState::new(tab));
            settle(&context, LARGE, &mut editor, &mut state);
            focus_field(&context, &mut editor, &mut state, field);

            let expected = if tab == SettingsTab::Tiles {
                let select_all = Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
                frame(&context, LARGE, &mut editor, &mut state, vec![select_all]);
                frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::A, false)]);
                frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Backspace, true)]);
                frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Backspace, false)]);
                frame(&context, LARGE, &mut editor, &mut state, Vec::new());
                let saved_pattern = &editor.document.project.assets[0].extra["BuildDefinition"]["TilePattern"];
                assert_eq!(saved_pattern, "");
                "abc"
            } else {
                "baseabc"
            };
            for (key, text) in [(Key::A, "a"), (Key::B, "b"), (Key::C, "c")] {
                type_character(&context, &mut editor, &mut state, key, text);
            }
            assert_field_text(&context, &mut editor, &mut state, expected);
            assert_eq!(editor.document.undo_labels().len(), history_length + 1, "typing {member} added extra undo steps");
            assert!(editor.document.undo());
            assert_field_text(&context, &mut editor, &mut state, "base");
            assert!(editor.document.redo());
            assert_field_text(&context, &mut editor, &mut state, expected);
        }
    }

    #[test]
    fn build_text_follows_undo_profiles_and_replacement_documents() {
        let fields = [
            (SettingsTab::Build, DESTINATION_FIELD, "Destination"),
            (SettingsTab::Tiles, TILE_PATTERN_FIELD, "TilePattern"),
        ];
        for (tab, field, member) in fields {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let initial = [("OutputType", Value::from("CroppedTiles")), (member, Value::from("profile"))];
            editor.document.set_build_settings(&initial);
            editor.document.save_build_profile("Saved");
            editor.document.set_build_setting(member, Value::from("base"));
            let mut state = Some(BuildSettingsState::new(tab));
            settle(&context, LARGE, &mut editor, &mut state);
            assert_field_text(&context, &mut editor, &mut state, "base");

            editor.document.set_build_setting(member, Value::from("changed"));
            assert_field_text(&context, &mut editor, &mut state, "changed");
            assert!(editor.document.undo());
            assert_field_text(&context, &mut editor, &mut state, "base");
            assert!(editor.document.redo());
            assert_field_text(&context, &mut editor, &mut state, "changed");
            assert!(editor.document.undo());
            focus_field(&context, &mut editor, &mut state, field);
            type_character(&context, &mut editor, &mut state, Key::D, "d");
            assert_field_text(&context, &mut editor, &mut state, "based");

            editor.document.load_build_profile("Saved");
            assert_field_text(&context, &mut editor, &mut state, "profile");
            let mut replacement = Document::new();
            let replacement_settings = [("OutputType", Value::from("CroppedTiles")), (member, Value::from("replacement"))];
            replacement.set_build_settings(&replacement_settings);
            editor.document = replacement;
            assert_field_text(&context, &mut editor, &mut state, "replacement");
        }
    }

    #[test]
    fn enter_finishes_destination_typing_and_continues_a_multiline_script() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(BuildSettingsState::new(SettingsTab::Build));
        settle(&context, LARGE, &mut editor, &mut state);
        focus_field(&context, &mut editor, &mut state, DESTINATION_FIELD);
        type_character(&context, &mut editor, &mut state, Key::A, "a");
        frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Enter, true)]);
        frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Enter, false)]);
        focus_field(&context, &mut editor, &mut state, DESTINATION_FIELD);
        type_character(&context, &mut editor, &mut state, Key::B, "b");
        assert_eq!(editor.document.undo_labels().len(), 2);
        assert!(editor.document.undo());
        assert_eq!(editor.document.build_definition().destination, "a");

        state.as_mut().unwrap().tab = SettingsTab::Script;
        frame(&context, LARGE, &mut editor, &mut state, Vec::new());
        focus_field(&context, &mut editor, &mut state, SCRIPT_FIELD);
        let history_length = editor.document.undo_labels().len();
        type_character(&context, &mut editor, &mut state, Key::A, "a");
        frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Enter, true)]);
        frame(&context, LARGE, &mut editor, &mut state, vec![key_event(Key::Enter, false)]);
        type_character(&context, &mut editor, &mut state, Key::B, "b");
        assert_eq!(editor.document.build_definition().post_build_script, "a\nb");
        assert_eq!(editor.document.undo_labels().len(), history_length + 1);
        assert!(editor.document.undo());
        assert_eq!(editor.document.build_definition().post_build_script, "");
    }

    #[test]
    fn terrain_dimension_drags_undo_once_per_gesture() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        editor.document.set_terrain_size(5000.0, 2500.0);
        let history_length = editor.document.undo_labels().len();
        let mut state = Some(BuildSettingsState::new(SettingsTab::Terrain));
        settle(&context, LARGE, &mut editor, &mut state);
        let pointer_button = |pos, pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };

        for (index, dimension) in ["Width", "Height"].into_iter().enumerate() {
            let terrain = editor.document.terrain();
            let initial_size = (terrain.width, terrain.height);
            let mut sizes = vec![initial_size];
            for gesture in 0..2 {
                let (shapes, _) = frame(&context, LARGE, &mut editor, &mut state, Vec::new());
                let field = texts(&shapes).filter(|text| text.galley.text().ends_with(" m")).nth(index);
                let field = field.unwrap_or_else(|| panic!("{dimension} is not painted"));
                let start = field.visual_bounding_rect().center();
                frame(&context, LARGE, &mut editor, &mut state, vec![Event::PointerMoved(start)]);
                frame(&context, LARGE, &mut editor, &mut state, vec![pointer_button(start, true)]);

                let mut changed_frames = 0;
                let mut previous_size = *sizes.last().unwrap();
                for distance in [20.0, 40.0, 60.0, 80.0] {
                    let position = start + Vec2::new(distance, 0.0);
                    frame(&context, LARGE, &mut editor, &mut state, vec![Event::PointerMoved(position)]);
                    let terrain = editor.document.terrain();
                    let current_size = (terrain.width, terrain.height);
                    changed_frames += usize::from(current_size != previous_size);
                    previous_size = current_size;
                }
                let end = start + Vec2::new(80.0, 0.0);
                frame(&context, LARGE, &mut editor, &mut state, vec![pointer_button(end, false)]);

                assert!(changed_frames >= 2, "{dimension} changed in only {changed_frames} drag frames");
                assert_eq!(editor.document.undo_labels().len(), history_length + gesture + 1, "{dimension} added extra undo steps");
                sizes.push(previous_size);
            }
            for expected in sizes[..2].iter().rev() {
                assert!(editor.document.undo());
                let terrain = editor.document.terrain();
                assert_eq!((terrain.width, terrain.height), *expected);
            }
        }
    }

    /// The Build tab at this screen size, scrolled to its end with the
    /// pointer on the page's scroll bar, which grows to its widest. Returns
    /// what was painted, the page's right edge (where the bar is) and the
    /// left edge of the widest bar.
    fn build_tab_scrolled(screen: Vec2) -> (Vec<ClippedShape>, f32, f32) {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(BuildSettingsState::new(SettingsTab::Build));
        let (_, window) = settle(&context, screen, &mut editor, &mut state);
        // The page reaches the window's content edge.
        let right = window.right() - egui::Frame::window(&context.global_style()).total_margin().right;
        let bar_left = right - context.global_style().spacing.scroll.bar_width;
        let on_bar = Pos2::new(right - 2.0, window.center().y);
        let wheel = Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: TouchPhase::Move, modifiers: Modifiers::NONE };
        frame(&context, screen, &mut editor, &mut state, vec![Event::PointerMoved(on_bar)]);
        frame(&context, screen, &mut editor, &mut state, vec![wheel]);
        for _ in 0..60 {
            frame(&context, screen, &mut editor, &mut state, Vec::new());
        }
        let (shapes, _) = frame(&context, screen, &mut editor, &mut state, Vec::new());
        (shapes, right, bar_left)
    }

    #[test]
    fn the_build_tab_ends_its_lines_before_the_scroll_bar() {
        // The small screens, and widths a pixel or two apart, so that the
        // token help's lines end at every distance from the page's edge.
        let widths = (0..30).map(|step| Vec2::new(600.0 + 2.0 * step as f32, 400.0));
        for screen in SMALL_SCREENS.into_iter().chain(widths) {
            let (shapes, right, bar_left) = build_tab_scrolled(screen);
            assert!(texts(&shapes).any(|text| text.galley.text() == TOKEN_HELP), "the token help does not show at {screen:?}");
            // The section bars are laid out to the page's width.
            let page = shapes.iter().find_map(|clipped| match &clipped.shape {
                Shape::Rect(rect) if rect.fill == theme::PANEL_BORDER => Some(rect.rect),
                _ => None,
            });
            let page = page.expect("the section bar is painted");
            assert!(page.right() <= bar_left, "the page is {page:?}, under the scroll bar from {bar_left} to {right}, at {screen:?}");
            // Everything above the footer ends before the bar.
            let footer = texts(&shapes).find(|text| text.galley.text() == "Close").map(|text| text.visual_bounding_rect().top());
            let footer = footer.expect("Close is painted");
            for text in texts(&shapes) {
                let rect = text.visual_bounding_rect();
                if rect.bottom() < footer {
                    assert!(rect.right() <= bar_left, "{:?} ends at {}, under the scroll bar from {bar_left}, at {screen:?}", text.galley.text(), rect.right());
                }
            }
        }
    }

    #[test]
    fn a_profile_summary_keeps_a_line_a_setting() {
        for screen in [LARGE, SMALL_SCREENS[0], SMALL_SCREENS[1]] {
            let context = context();
            let mut editor = Editor::new(Document::new());
            editor.document.save_build_profile("Narrow");
            let mut settings = BuildSettingsState::new(SettingsTab::Profiles);
            settings.profile = Some("Narrow".to_string());
            let mut state = Some(settings);
            let (shapes, _) = settle(&context, screen, &mut editor, &mut state);
            let definition = editor.document.build_profile("Narrow").expect("the profile is kept");
            let summary = nerthus_export::swarm::profile_summary(&definition);
            let font = theme::monospace(14.0);
            let painted: Vec<&TextShape> = texts(&shapes).filter(|text| text.galley.job.sections.iter().all(|section| section.format.font_id == font)).collect();
            // Every line whole on a row of its own, the dashed separators
            // too, cut off at the box's edge rather than wrapped.
            for line in summary.lines().filter(|line| !line.is_empty()) {
                let shown = painted.iter().find(|text| text.galley.text() == line);
                let shown = shown.unwrap_or_else(|| panic!("{line:?} is not a line of its own at {screen:?}"));
                assert_eq!(shown.galley.rows.len(), 1, "{line:?} wraps at {screen:?}");
            }
            assert!(summary.lines().any(|line| line == PROFILE_SEPARATOR));
        }
    }

    #[test]
    fn terrain_measurements_follow_dimensions_and_build_resolution() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(BuildSettingsState::new(SettingsTab::Terrain));
        editor.document.set_terrain_size(5000.0, 2600.0);
        editor.document.set_build_setting("Resolution", json!(2048));
        editor.preview_resolution = 512;

        let (shapes, _) = settle(&context, LARGE, &mut editor, &mut state);
        let painted: Vec<&str> = texts(&shapes).map(|text| text.galley.text()).collect();
        assert!(painted.contains(&"Real scale: 2.4414 m/pixel at 2048 × 2048"));
        assert!(painted.contains(&"Height-to-width ratio: 0.520000 (52.0000%)"));

        editor.document.set_terrain_size(8192.0, 1024.0);
        editor.document.set_build_setting("Resolution", json!(4096));
        let (shapes, _) = settle(&context, LARGE, &mut editor, &mut state);
        let painted: Vec<&str> = texts(&shapes).map(|text| text.galley.text()).collect();
        assert!(painted.contains(&"Real scale: 2.0000 m/pixel at 4096 × 4096"));
        assert!(painted.contains(&"Height-to-width ratio: 0.125000 (12.5000%)"));
    }
}

#[cfg(test)]
mod filename_tests {
    use super::*;
    use crate::document::Document;
    use egui::epaint::ClippedShape;
    use egui::{Event, Key, Modifiers, Pos2, Rect, Shape};

    const SAVED_NAME: &str = "terrain-output";

    struct FilenameWindow {
        context: egui::Context,
        editor: Editor,
        state: Option<BuildSettingsState>,
        node_id: i32,
    }

    impl FilenameWindow {
        fn new() -> Self {
            let context = egui::Context::default();
            theme::apply(&context);
            egui_extras::install_image_loaders(&context);
            let mut editor = Editor::new(Document::new());
            let node_id = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
            editor.document.toggle_export(&[node_id], FileFormat::Png16);
            editor.document.rename_export_file(node_id, SAVED_NAME);
            let state = Some(BuildSettingsState::new(SettingsTab::Nodes));
            let mut window = Self { context, editor, state, node_id };
            for _ in 0..30 {
                window.frame(Vec::new());
            }
            window
        }

        fn frame(&mut self, events: Vec<Event>) -> (Vec<ClippedShape>, Vec<Command>) {
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1333.0, 800.0));
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut commands = Vec::new();
            let mut output = self.context.run_ui(input, |ui| {
                show(ui.ctx(), &mut self.editor, &mut self.state, &mut commands);
            });
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            (shapes, commands)
        }

        fn text_rect(&mut self, label: &str) -> Rect {
            let (shapes, _) = self.frame(Vec::new());
            shapes.iter().find_map(|clipped| match &clipped.shape {
                Shape::Text(text) if text.galley.text() == label => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    Some(rect.intersect(clipped.clip_rect))
                }
                _ => None,
            }).unwrap_or_else(|| panic!("{label:?} was not painted"))
        }

        fn click(&mut self, at: Pos2) -> Vec<Command> {
            let press = Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
            self.frame(vec![Event::PointerMoved(at), press]);
            let release = Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
            let (_, commands) = self.frame(vec![release]);
            commands
        }

        fn type_filename(&mut self, filename: &str) {
            let saved = self.saved_filename();
            let rect = self.text_rect(&saved);
            self.click(rect.center());
            let select_all = key(Key::A, Modifiers::COMMAND);
            self.frame(vec![select_all, Event::Text(filename.to_string())]);
            assert_eq!(self.saved_filename(), saved);
            assert_eq!(self.state.as_ref().unwrap().filenames[0].1, filename);
        }

        fn click_blank(&mut self) {
            let area = egui::AreaState::load(&self.context, Id::new("build-settings")).unwrap();
            let at = area.rect().max - Vec2::new(40.0, 80.0);
            self.click(at);
        }

        fn saved_filename(&self) -> String {
            self.editor.document.export_settings(self.node_id).unwrap().filename
        }
    }

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn filename_close_and_execute_build_accept_pending_typing() {
        for button in ["Close", "Execute Build"] {
            let mut window = FilenameWindow::new();
            window.type_filename("  elevation  ");
            let rect = window.text_rect(button);
            let commands = window.click(rect.center());
            assert!(window.state.is_none(), "{button}");
            assert_eq!(window.saved_filename(), "elevation", "{button}");
            assert_eq!(commands.contains(&Command::BuildAndExport), button == "Execute Build");
        }
    }

    #[test]
    fn filename_mouse_focus_loss_saves_and_immediate_undo_resyncs() {
        let mut window = FilenameWindow::new();
        window.type_filename("elevation");
        window.click_blank();
        assert_eq!(window.saved_filename(), "elevation");

        assert!(window.editor.document.undo());
        window.frame(Vec::new());
        assert_eq!(window.saved_filename(), SAVED_NAME);
        assert_eq!(window.state.as_ref().unwrap().filenames[0].1, SAVED_NAME);
    }

    #[test]
    fn filename_escape_cancels_and_blank_typing_uses_the_node_name() {
        let mut window = FilenameWindow::new();
        window.type_filename("discarded");
        window.frame(vec![key(Key::Escape, Modifiers::NONE)]);
        assert_eq!(window.saved_filename(), SAVED_NAME);
        assert_eq!(window.state.as_ref().unwrap().filenames[0].1, SAVED_NAME);

        window.type_filename("   ");
        window.click_blank();
        let node_name = &window.editor.document.node(window.node_id).unwrap().name;
        assert_eq!(&window.saved_filename(), node_name);
    }

    #[test]
    fn filename_tab_switch_accepts_pending_typing() {
        let mut window = FilenameWindow::new();
        window.type_filename("elevation");
        let rect = window.text_rect("Build");
        window.click(rect.center());
        assert_eq!(window.state.as_ref().unwrap().tab, SettingsTab::Build);
        assert_eq!(window.saved_filename(), "elevation");
    }

    #[test]
    fn filename_external_close_accepts_except_on_escape() {
        for escaped in [false, true] {
            let mut window = FilenameWindow::new();
            window.type_filename("elevation");
            tool_window::request_close(&window.context, Id::new("build-settings"));
            let events = if escaped { vec![key(Key::Escape, Modifiers::NONE)] } else { Vec::new() };
            window.frame(events);
            assert!(window.state.is_none());
            let expected = if escaped { SAVED_NAME } else { "elevation" };
            assert_eq!(window.saved_filename(), expected);
        }
    }

    #[test]
    fn filename_tab_switch_keeps_a_saved_name_changed_elsewhere() {
        let mut window = FilenameWindow::new();
        window.type_filename("discarded");
        window.editor.document.rename_export_file(window.node_id, "changed-elsewhere");
        let rect = window.text_rect("Build");
        window.click(rect.center());
        assert_eq!(window.saved_filename(), "changed-elsewhere");
        assert_eq!(window.state.as_ref().unwrap().filenames[0].1, "changed-elsewhere");
    }

    #[test]
    fn resetting_views_discards_the_previous_documents_filename_draft() {
        let mut window = FilenameWindow::new();
        window.type_filename("discarded");
        let mut ui_state = crate::ui::UiState::default();
        ui_state.build_settings = window.state.take();
        ui_state.reset_views();
        window.state = ui_state.build_settings.take();
        assert!(window.state.as_ref().unwrap().filenames.is_empty());

        let mut document = Document::new();
        // Reuse the graph deliberately: new nodes receive random ids.
        document.project = window.editor.document.project.clone();
        document.rename_export_file(window.node_id, "new-document-output");
        window.editor.document = document;
        window.type_filename("new-elevation");
        let rect = window.text_rect("Close");
        window.click(rect.center());
        assert_eq!(window.saved_filename(), "new-elevation");
    }
}
