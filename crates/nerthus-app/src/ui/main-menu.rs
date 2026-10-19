//! Main menu contents shared by the native macOS menu bar and egui.

use egui::{Color32, Ui};
use nerthus_engine::StateOverride;

use crate::document::{CombineKind, PortalState, TabTarget};
use crate::editor::{Editor, PREVIEW_RESOLUTIONS, RenderMode};
use crate::files::FileCommand;
use crate::theme;
use crate::ui::UiState;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::{node_color, node_icon};
use crate::ui::graph_commands::GraphCommand;
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::{Tooltips, check_item, menu_item_enabled, menu_separator, menu_style, submenu};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MainMenu {
    pub title: String,
    pub entries: Vec<MenuEntry>,
    pub icon: Option<String>,
    pub accent: Option<Color32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum MenuEntry {
    Action {
        label: String,
        shortcut: &'static str,
        enabled: bool,
        checked: Option<bool>,
        command: Option<Command>,
        icon: Option<String>,
        accent: Option<Color32>,
        tooltip: Option<String>,
        secondary_command: Option<Command>,
    },
    Separator,
    Submenu(MainMenu),
}

impl MainMenu {
    fn new(title: impl Into<String>, entries: Vec<MenuEntry>) -> Self {
        Self {
            title: title.into(),
            entries,
            icon: None,
            accent: None,
        }
    }
}

impl MenuEntry {
    fn set_appearance(&mut self, shown_icon: impl Into<String>, shown_accent: Option<Color32>) {
        match self {
            Self::Action { icon, accent, .. } => {
                *icon = Some(shown_icon.into());
                *accent = shown_accent;
            }
            Self::Submenu(menu) => {
                menu.icon = Some(shown_icon.into());
                menu.accent = shown_accent;
            }
            Self::Separator => {}
        }
    }

    fn set_enabled(&mut self, value: bool) {
        if let Self::Action { enabled, .. } = self {
            *enabled = value;
        }
    }

    fn set_tooltip(&mut self, value: String) {
        if let Self::Action { tooltip, .. } = self {
            *tooltip = Some(value);
        }
    }
}

fn action(label: impl Into<String>, shortcut: &'static str, command: Command) -> MenuEntry {
    MenuEntry::Action {
        label: label.into(),
        shortcut,
        enabled: true,
        checked: None,
        command: Some(command),
        icon: None,
        accent: None,
        tooltip: None,
        secondary_command: None,
    }
}

fn styled_action(icon: &str, accent: Option<Color32>, label: impl Into<String>, shortcut: &'static str, command: Command) -> MenuEntry {
    let mut entry = action(label, shortcut, command);
    entry.set_appearance(icon, accent);
    entry
}

fn enabled_action(label: impl Into<String>, shortcut: &'static str, command: Command, enabled: bool) -> MenuEntry {
    let mut entry = action(label, shortcut, command);
    entry.set_enabled(enabled);
    entry
}

fn checked_action(label: impl Into<String>, shortcut: &'static str, command: Command, selected: bool) -> MenuEntry {
    let mut entry = action(label, shortcut, command);
    if let MenuEntry::Action { checked, .. } = &mut entry {
        *checked = Some(selected);
    }
    entry
}

fn caption(label: &str) -> MenuEntry {
    MenuEntry::Action {
        label: label.to_string(),
        shortcut: "",
        enabled: false,
        checked: None,
        command: None,
        icon: None,
        accent: None,
        tooltip: None,
        secondary_command: None,
    }
}

fn submenu_entry(title: impl Into<String>, entries: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry::Submenu(MainMenu::new(title, entries))
}

/// Build from the current document so enabled states, checks and dynamic
/// submenus agree across both platform presentations.
pub(super) fn build_menus(editor: &Editor, ui_state: &UiState) -> Vec<MainMenu> {
    vec![
        file_menu(editor),
        edit_menu(editor, ui_state),
        graph_menu(editor, ui_state),
        node_menu(editor, ui_state.graph.tab),
        preview_menu(editor),
        project_menu(),
        tools_menu(editor),
        help_menu(),
    ]
}

pub(super) fn show_entries(ui: &mut Ui, entries: &[MenuEntry], commands: &mut Vec<Command>) {
    menu_style(ui);
    for entry in entries {
        match entry {
            MenuEntry::Separator => menu_separator(ui),
            MenuEntry::Submenu(menu) => {
                submenu(ui, menu.icon.as_deref(), menu.accent, &menu.title, |ui| {
                    show_entries(ui, &menu.entries, commands);
                });
            }
            MenuEntry::Action {
                label,
                shortcut,
                enabled,
                checked,
                command,
                icon,
                accent,
                tooltip,
                secondary_command,
            } => {
                let response = match checked {
                    Some(selected) if icon.is_none() => {
                        let scope = ui.add_enabled_ui(*enabled, |ui| check_item(ui, *selected, label, shortcut));
                        scope.inner
                    }
                    _ => menu_item_enabled(ui, icon.as_deref(), *accent, label, shortcut, *enabled),
                };
                let response = match tooltip {
                    Some(text) => response.tooltip(text),
                    None => response,
                };
                let requested = if response.secondary_clicked() {
                    secondary_command.as_ref()
                } else if response.clicked() {
                    command.as_ref()
                } else {
                    None
                };
                if let Some(command) = requested {
                    commands.push(command.clone());
                }
            }
        }
    }
}

pub(super) fn file_menu(editor: &Editor) -> MainMenu {
    let recent_entries = recent_entries(editor);
    let mut entries = vec![
        styled_action("file-broken", Some(theme::ACCENT_PRIMARY), "New", "Ctrl N", Command::NewProject),
        styled_action("folder", None, "Open...", "Ctrl O", Command::Open),
        submenu_entry("Open Recent", recent_entries),
        action("Open Examples...", "", Command::File(FileCommand::OpenExamples)),
        styled_action("input-spark", Some(theme::ACCENT_YELLOW), "Start Dialog", "", Command::File(FileCommand::StartDialog)),
        MenuEntry::Separator,
        styled_action("device-floppy", Some(theme::ACCENT_BLUE), "Save...", "Ctrl S", Command::Save),
        styled_action("device-floppy", None, "Save as...", "Ctrl Shift S", Command::SaveAs),
        styled_action("copy-plus", None, "Save incremental", "", Command::File(FileCommand::SaveIncremental)),
        MenuEntry::Separator,
    ];
    let web = cfg!(target_arch = "wasm32");
    if !web {
        entries.push(action("Repair File...", "", Command::File(FileCommand::RepairFile)));
        let imports = [
            ("Macro...", FileCommand::ImportMacro),
            ("Graph...", FileCommand::ImportGraph),
            ("TORLET...", FileCommand::ImportTorlet),
            ("Image...", FileCommand::ImportImage),
        ];
        let mut import_entries = Vec::new();
        for (label, command) in imports {
            import_entries.push(action(label, "", Command::File(command)));
        }
        entries.push(submenu_entry("Import", import_entries));
    }
    let mut export_entries = Vec::new();
    if !web {
        export_entries.push(action("Macro...", "", Command::File(FileCommand::ExportMacro)));
    }
    export_entries.push(action("TORLET...", "", Command::File(FileCommand::ExportTorlet)));
    entries.push(submenu_entry("Export", export_entries));
    let mut share = caption("Share...");
    share.set_appearance("file-barcode", None);
    entries.push(share);
    if !web {
        entries.push(MenuEntry::Separator);
        let mut exit = action("Exit", "Alt F4", Command::Exit);
        if let MenuEntry::Action { accent, .. } = &mut exit {
            *accent = Some(theme::ACCENT_RED);
        }
        entries.push(exit);
    }
    MainMenu::new("File", entries)
}

fn recent_entries(editor: &Editor) -> Vec<MenuEntry> {
    let mut entries = vec![caption("Recent Files")];
    let existing = editor.user.recent.files().iter().filter(|file| file.is_file());
    for (index, file) in existing.enumerate() {
        let name = file.file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        let mut entry = action(name, "", Command::OpenRecent(file.clone()));
        entry.set_appearance(format!("number-{}-small", index + 1), None);
        let modified = file.metadata().and_then(|metadata| metadata.modified()).ok();
        let modified_text = modified
            .map(|time| chrono::DateTime::<chrono::Local>::from(time).format("%-m/%-d/%Y %-I:%M:%S %p").to_string())
            .unwrap_or_default();
        entry.set_tooltip(format!("{}\nModified: {modified_text}", file.display()));
        entries.push(entry);
    }
    entries.push(MenuEntry::Separator);
    entries.push(action("Open Autosave...", "", Command::File(FileCommand::OpenAutosave)));
    entries
}

pub(super) fn edit_menu(editor: &Editor, ui_state: &UiState) -> MainMenu {
    let has_selection = !editor.selection.is_empty();
    let graph = &ui_state.graph;
    let can_rename = has_selection || !graph.selected_groups.is_empty() || graph.selected_note.is_some();
    let can_delete = can_rename || !graph.selected_connections.is_empty();
    let active = editor.active_node().and_then(|id| editor.document.node(id));
    let copied_type = editor.copied_node.as_ref().map(|copied| copied.type_name.as_str());
    let can_paste_properties = active.is_some_and(|node| copied_type == Some(node.type_name.as_str()));
    let mut undo = styled_action("arrow-back-up", None, "Undo", "Ctrl Z", Command::Undo);
    undo.set_enabled(editor.document.can_undo());
    let mut redo = styled_action("arrow-forward-up", None, "Redo", "Ctrl Y", Command::Redo);
    redo.set_enabled(editor.document.can_redo());
    let mut paste = styled_action("clipboard-copy", None, "Paste", "Ctrl V", Command::Graph(GraphCommand::Paste));
    paste.set_enabled(editor.node_clipboard.is_some());
    let mut duplicate = styled_action("stack", None, "Duplicate", "Ctrl D", Command::Duplicate);
    duplicate.set_enabled(has_selection);
    let mut delete = styled_action("square-rounded-x", Some(theme::ACCENT_RED), "Delete", "Del", Command::Delete);
    delete.set_enabled(can_delete);
    let entries = vec![
        undo,
        redo,
        MenuEntry::Separator,
        enabled_action("Cut", "Ctrl X", Command::Graph(GraphCommand::Cut), has_selection),
        enabled_action("Copy", "Ctrl C", Command::Graph(GraphCommand::Copy), has_selection),
        paste,
        MenuEntry::Separator,
        enabled_action("Copy Properties", "", Command::CopyProperties, active.is_some()),
        enabled_action("Paste Properties", "", Command::Graph(GraphCommand::PastePropertiesToActive), can_paste_properties),
        MenuEntry::Separator,
        enabled_action("Rename", "F2", Command::Rename, can_rename),
        duplicate,
        MenuEntry::Separator,
        delete,
    ];
    MainMenu::new("Edit", entries)
}

pub(super) fn graph_menu(editor: &Editor, ui_state: &UiState) -> MainMenu {
    let mut entries = graph_command_entries(editor, ui_state);
    entries.push(MenuEntry::Separator);
    if !cfg!(target_arch = "wasm32") {
        entries.push(MenuEntry::Submenu(bake_menu(editor, false)));
    }
    entries.push(MenuEntry::Separator);
    let bookmark_entries = bookmark_entries(editor, ui_state);
    entries.push(submenu_entry("Bookmarks", bookmark_entries));
    MainMenu::new("Graph", entries)
}

pub(super) fn graph_command_entries(editor: &Editor, ui_state: &UiState) -> Vec<MenuEntry> {
    vec![
        styled_action("square-rounded-plus-2", Some(theme::ACCENT_BLUE), "New Graph Tab", "", Command::NewGraphTab),
        styled_action("note", Some(theme::ACCENT_STRONG_YELLOW), "Add Note", "", Command::AddNote),
        MenuEntry::Separator,
        caption("Zoom"),
        styled_action("zoom-scan", None, "Extents", "Ctrl 0", Command::ZoomExtents),
        action("Reset Zoom", "Ctrl 1", Command::ZoomReset),
        action("Selection", "Ctrl 2", Command::ZoomSelection),
        MenuEntry::Separator,
        caption("Graph Surface"),
        checked_action("Show Grid", "", Command::ToggleGrid, ui_state.graph.show_grid),
        checked_action("Snap to Grid", "", Command::ToggleSnapToGrid, ui_state.graph.snap_to_grid),
        checked_action("Snap to Items", "", Command::Graph(GraphCommand::ToggleSnapToItems), editor.user.options.snap_to_items()),
    ]
}

pub(super) fn bookmark_entries(editor: &Editor, ui_state: &UiState) -> Vec<MenuEntry> {
    let mut entries = vec![styled_action("bookmark-plus", None, "Add bookmark", "", Command::AddBookmark)];
    let bookmarks = editor.document.bookmarks();
    let mut remove_entries = Vec::new();
    if !bookmarks.is_empty() {
        entries.push(MenuEntry::Separator);
    }
    for id in bookmarks {
        let Some(node) = editor.document.node(id) else { continue };
        let removing = ui_state.graph.bookmark_removing == Some(id);
        let label = if removing { "Remove?" } else { &node.name };
        let command = if removing { Command::RemoveBookmark(id) } else { Command::GoToBookmark(id) };
        let mut entry = action(label, "", command);
        entry.set_appearance(node_icon(&node.type_name), Some(node_color(node)));
        if let MenuEntry::Action { secondary_command, .. } = &mut entry {
            *secondary_command = Some(Command::ArmBookmarkRemoval(id));
        }
        entries.push(entry);
        remove_entries.push(action(node.name.clone(), "", Command::RemoveBookmark(id)));
    }
    if cfg!(target_os = "macos") && !remove_entries.is_empty() {
        entries.push(MenuEntry::Separator);
        entries.push(submenu_entry("Remove Bookmark", remove_entries));
    }
    entries
}

pub(super) fn bake_menu(editor: &Editor, reload_item: bool) -> MainMenu {
    let selected = editor.active_node().and_then(|id| editor.document.node(id));
    let selected_baked = selected.is_some_and(|node| StateOverride::of(node) == StateOverride::Baked);
    let any_baked = !editor.baked_nodes().is_empty();
    let resolution = editor.document.build_definition().bake_resolution;
    let can_bake = selected.is_some() && !selected_baked;
    let label = match selected {
        Some(node) if can_bake => super::menus::bake_estimate(editor, node.id, resolution),
        _ => "Bake Selected".to_string(),
    };
    let mut bake_selected = styled_action("matchstick", None, label, "", Command::BakeSelected);
    bake_selected.set_enabled(can_bake);
    let mut unbake_selected = styled_action("fire-extinguisher", None, "Unbake Selected", "", Command::UnbakeSelected);
    unbake_selected.set_enabled(selected_baked);
    let mut unbake_all = styled_action("fire-hydrant", Some(theme::ACCENT_MUTED_RED), "Unbake All", "", Command::UnbakeAll);
    unbake_all.set_enabled(any_baked);
    let mut entries = vec![
        bake_selected,
        styled_action("file-shred", None, "Bake Required Nodes", "", Command::BakeRequiredNodes),
        enabled_action("Force load baked cache", "", Command::ForceLoadBakedCache, any_baked),
        MenuEntry::Separator,
        unbake_selected,
        unbake_all,
        MenuEntry::Separator,
        caption("Resolution"),
    ];
    for size in [1024, 2048, 4096, 8192, 16384] {
        let label = format!("{size} x {size}");
        entries.push(checked_action(label, "", Command::SetBakeResolution(size), size == resolution));
    }
    entries.push(MenuEntry::Separator);
    if reload_item {
        entries.push(action("Reload SmartCache to RAM", "", Command::ReloadSmartCache));
        entries.push(MenuEntry::Separator);
    }
    entries.push(caption("Cache to Disk"));
    entries.push(checked_action("When closing file", "", Command::TogglePersistOnSave, editor.persist_on_save()));
    entries.push(checked_action("When Idle", "", Command::ToggleFreezeWhenIdle, crate::disk_cache::freeze_when_idle()));
    entries.push(styled_action("trash-x", Some(theme::ACCENT_MUTED_RED), "Delete Cache", "", Command::DeleteDiskCache));
    MainMenu::new("Bake", entries)
}

pub(super) fn node_menu(editor: &Editor, current_tab: usize) -> MainMenu {
    let has_selection = !editor.selection.is_empty();
    let exporting = editor
        .active_node()
        .and_then(|id| editor.document.export_settings(id))
        .is_some_and(|settings| settings.enabled);
    let export_accent = exporting.then_some(theme::ACCENT_PRIMARY);
    let mut refresh = styled_action("rotate-360", Some(theme::ACCENT_GREEN), "Refresh", "F5", Command::Refresh);
    refresh.set_enabled(has_selection);
    let mut bypass = styled_action("road-sign", None, "Bypass", "B", Command::Bypass);
    bypass.set_enabled(has_selection);
    let mut export = styled_action("device-floppy", export_accent, "Mark for Export", "F3", Command::MarkForExport);
    export.set_enabled(has_selection);
    if let MenuEntry::Action { checked, .. } = &mut export {
        *checked = Some(exporting);
    }
    let mut viewport = styled_action(
        "picture-in-picture",
        Some(theme::ACCENT_MUTED_BLUE),
        "Locked 2D Viewport",
        "Ctrl F6",
        Command::ShowLocked2DViewport,
    );
    viewport.set_enabled(has_selection);
    let combine_entries = vec![
        action("Combine as Inputs", "", Command::Graph(GraphCommand::Combine(CombineKind::Inputs))),
        action("Mixer as Color Layers", "", Command::Graph(GraphCommand::Combine(CombineKind::Mixer))),
        action("Layers as Normal Layers", "", Command::Graph(GraphCommand::Combine(CombineKind::Layers))),
    ];
    let mut combine = submenu_entry("Combine nodes", combine_entries);
    combine.set_appearance("arrows-join-2", Some(theme::ACCENT_MUTED_ORANGE));
    let portal_entries = portal_entries(editor);
    let mut portals = submenu_entry("Convert port to portals", portal_entries);
    portals.set_appearance("keyframe", None);
    let tab_entries = tab_entries(editor, current_tab);
    let preset_entries = preset_entries(editor);
    let entries = vec![
        refresh,
        MenuEntry::Separator,
        bypass,
        enabled_action("Lock Preview to Node", "F", Command::LockPreview, has_selection),
        export,
        enabled_action("Reset", "Ctrl R", Command::ResetNode, has_selection),
        enabled_action("Mutate", "", Command::Graph(GraphCommand::Mutate), has_selection),
        MenuEntry::Separator,
        viewport,
        MenuEntry::Separator,
        styled_action("arrow-left-rhombus", Some(theme::ACCENT_PURPLE), "Select Upsteam", "", Command::SelectUpstream),
        styled_action("arrow-right-rhombus", Some(theme::ACCENT_PURPLE), "Select Downstream", "", Command::SelectDownstream),
        MenuEntry::Separator,
        styled_action("layers-linked", None, "Group / Ungroup", "", Command::Graph(GraphCommand::GroupUngroup)),
        combine,
        MenuEntry::Separator,
        portals,
        MenuEntry::Separator,
        submenu_entry("Move to Tab", tab_entries),
        MenuEntry::Separator,
        submenu_entry("Presets", preset_entries),
    ];
    MainMenu::new("Node", entries)
}

pub(super) fn portal_entries(editor: &Editor) -> Vec<MenuEntry> {
    let Some(node) = editor.active_node().and_then(|id| editor.document.node(id)) else {
        return Vec::new();
    };
    let mut outputs: Vec<_> = node.ports.iter().filter(|port| port.is_output()).collect();
    outputs.sort_by_key(|port| (!port.is_primary(), port.name.clone()));
    let mut entries = Vec::new();
    for port in outputs {
        let command = GraphCommand::TogglePortPortal {
            node: node.id,
            port: port.name.clone(),
        };
        let mut entry = action(port.name.clone(), "", Command::Graph(command));
        if PortalState::of(port).is_portal() {
            entry.set_appearance("square-rounded-x", Some(theme::ACCENT_RED));
        }
        entries.push(entry);
    }
    entries
}

pub(super) fn tab_entries(editor: &Editor, current_tab: usize) -> Vec<MenuEntry> {
    let tabs = editor.document.graph_tabs();
    let mut entries = Vec::new();
    for tab in &tabs {
        if tab.index != current_tab {
            let command = GraphCommand::MoveToTab(TabTarget::Existing(tab.index));
            entries.push(action(tab.name.clone(), "", Command::Graph(command)));
        }
    }
    if tabs.len() > 1 {
        entries.push(MenuEntry::Separator);
    }
    entries.push(action("New Graph", "", Command::Graph(GraphCommand::MoveToTab(TabTarget::New))));
    entries
}

pub(super) fn preset_entries(editor: &Editor) -> Vec<MenuEntry> {
    let node = editor.active_node().and_then(|id| editor.document.node(id));
    let spec = node.and_then(|node| nerthus_engine::Catalogue::get().node(&node.type_name));
    let node_type = spec.map(|spec| spec.name.as_str()).unwrap_or("none");
    let create = match node {
        Some(node) => action("Create Preset", "", Command::CreatePreset(node.id)),
        None => {
            let mut entry = caption("Create Preset");
            entry.set_enabled(true);
            entry
        }
    };
    let mut entries = vec![create];
    let presets = editor.presets.of_type(node_type);
    if presets.is_empty() {
        entries.push(caption("No presets found..."));
        return entries;
    }
    entries.push(MenuEntry::Separator);
    for preset in presets {
        let mut entry = action(preset.name.clone(), "", Command::ApplyPreset(preset.name.clone()));
        if let Some(description) = &preset.description {
            if !description.is_empty() {
                entry.set_tooltip(description.clone());
            }
        }
        entries.push(entry);
    }
    entries
}

pub(super) fn preview_menu(editor: &Editor) -> MainMenu {
    let mut entries = vec![caption("Resolution")];
    for size in PREVIEW_RESOLUTIONS {
        let label = format!("{size} x {size}");
        entries.push(checked_action(label, "", Command::SetPreviewResolution(size), editor.preview_resolution == size));
    }
    entries.push(MenuEntry::Separator);
    let web = cfg!(target_arch = "wasm32");
    if !web {
        entries.push(caption("Viewport"));
        entries.extend(viewport_entries(editor));
        entries.push(MenuEntry::Separator);
    }
    if super::regions::menus_visible(editor) {
        let region_entries = region_entries(editor, true);
        entries.push(submenu_entry("Regions", region_entries));
        entries.push(MenuEntry::Separator);
    }
    let engine_text = if editor.engine_active { "Engine Active" } else { "Engine Suspended" };
    entries.push(styled_action("engine", Some(theme::ACCENT_GREEN), engine_text, "", Command::ToggleEngine));
    entries.push(styled_action("hexagon-off", Some(theme::ACCENT_BLUE), "Clean Memory", "", Command::CleanMemory));
    entries.push(action("Clear Cache", "", Command::ClearCache));
    entries.push(MenuEntry::Separator);
    entries.push(action("Rebuild All", "", Command::RefreshAll));
    entries.push(MenuEntry::Separator);
    if !web {
        entries.push(caption("Screenshot"));
        entries.push(styled_action("camera", Some(theme::ACCENT_STRONG_GREEN), "Save Screenshot", "F9", Command::SaveScreenshot));
        let screenshot_entries = screenshot_entries(editor);
        entries.push(submenu_entry("Screenshot Options", screenshot_entries));
        entries.push(MenuEntry::Separator);
        entries.push(styled_action("view-360-number", None, "Record Turntable Animation", "", Command::ShowTurntable));
        entries.push(MenuEntry::Separator);
    }
    entries.push(caption("Preview Renderer"));
    let modes = [
        (RenderMode::Data, "Visualize Data (Render Off)"),
        (RenderMode::Clay, "Clay Render"),
        (RenderMode::Realistic, "Realistic Render"),
    ];
    for (mode, label) in modes {
        entries.push(checked_action(label, "", Command::SetRenderMode(mode), editor.render_mode == mode));
    }
    MainMenu::new("Preview", entries)
}

pub(super) fn viewport_entries(editor: &Editor) -> Vec<MenuEntry> {
    let floating = editor.user.options.viewport_floating();
    let mut entries = Vec::new();
    for (label, value) in [("Embedded", false), ("Floating", true)] {
        let mut entry = checked_action(label, "", Command::SetViewportFloating(value), floating == value);
        if floating == value {
            if let MenuEntry::Action { command, .. } = &mut entry {
                *command = None;
            }
        }
        entries.push(entry);
    }
    entries
}

pub(super) fn region_entries(editor: &Editor, edit_regions: bool) -> Vec<MenuEntry> {
    let previewed = editor.preview_region.as_deref();
    let mut entries = vec![
        checked_action("Whole Terrain", "", Command::PreviewRegion(None), previewed.is_none()),
        MenuEntry::Separator,
        caption("Regions"),
    ];
    if editor.preview_node.is_none() {
        return entries;
    }
    for region in editor.document.regions() {
        let selected = previewed == Some(region.name.as_str());
        let command = Command::PreviewRegion(Some(region.name.clone()));
        entries.push(checked_action(region.name, "", command, selected));
    }
    entries.push(MenuEntry::Separator);
    entries.push(action("Save Current Node preview", "", Command::SaveRegionReference));
    if edit_regions {
        entries.push(action("Edit Regions", "", Command::EditRegions));
    }
    entries
}

pub(super) fn screenshot_entries(editor: &Editor) -> Vec<MenuEntry> {
    let mut entries = vec![
        action("Save Graph Screenshot as...", "", Command::SaveGraphScreenshotAs),
        action("Save Viewport Screenshot as...", "Ctrl F9", Command::SaveViewportScreenshotAs),
        action("Open Screenshots Folder", "", Command::OpenScreenshotsFolder),
        MenuEntry::Separator,
        caption("Resolution"),
    ];
    for (scale, label) in [(1, "1x"), (2, "2x"), (4, "4x")] {
        entries.push(checked_action(label, "", Command::SetScreenshotScale(scale), editor.screenshot_supersample == scale));
    }
    entries
}

pub(super) fn project_menu() -> MainMenu {
    let mut entries = Vec::new();
    let web = cfg!(target_arch = "wasm32");
    if !web {
        entries.push(styled_action(
            "atom2",
            Some(theme::ACCENT_GREEN),
            "Build and Export...",
            "Ctrl Shift B",
            Command::BuildAndExport,
        ));
        let mut batch = action("Execute Batch Build...", "", Command::BatchBuild);
        if let MenuEntry::Action { accent, .. } = &mut batch {
            *accent = Some(theme::ACCENT_MAGENTA);
        }
        entries.push(batch);
        let mut network = caption("Execute Network Build...");
        network.set_appearance("timeline-event-plus", Some(theme::ACCENT_MAGENTA));
        entries.push(network);
        entries.push(MenuEntry::Separator);
    }
    entries.push(styled_action("settings", Some(theme::ACCENT_BLUE), "Build Settings...", "Ctrl B", Command::BuildSettings));
    entries.push(MenuEntry::Separator);
    if !web {
        entries.push(caption("Build Stack"));
        entries.push(action("Open Build Folder", "", Command::OpenBuildFolder));
        entries.push(MenuEntry::Separator);
    }
    entries.push(caption("Reports"));
    entries.push(styled_action("histogram", None, "Statistics", "", Command::ShowBuildStatistics));
    MainMenu::new("Project", entries)
}

const UI_SCALES: [(&str, f32); 8] = [
    ("80%", 0.8),
    ("100%", 1.0),
    ("110%", 1.1),
    ("120%", 1.2),
    ("125%", 1.25),
    ("130%", 1.3),
    ("150%", 1.5),
    ("175%", 1.75),
];

pub(super) fn tools_menu(editor: &Editor) -> MainMenu {
    let percent = (editor.user.options.ui_scale() * 100.0).round();
    let title = format!("Interface Scale ({percent}%)");
    let mut scale_entries = Vec::new();
    for (label, scale) in UI_SCALES {
        scale_entries.push(action(label, "", Command::SetUiScale(scale)));
    }
    let mut entries = vec![submenu_entry(title, scale_entries), MenuEntry::Separator];
    let web = cfg!(target_arch = "wasm32");
    if !web {
        entries.push(styled_action(
            "report-medical",
            None,
            "Run Diagnostics...",
            "",
            Command::Tools(ToolCommand::ShowDiagnostics),
        ));
        entries.push(action("Artifact Explorer...", "", Command::Tools(ToolCommand::ShowArtifactExplorer)));
        entries.push(MenuEntry::Separator);
    }
    entries.push(styled_action(
        "terminal-2",
        Some(theme::ACCENT_MUTED_GREEN),
        "Console",
        "F12",
        Command::Tools(ToolCommand::ShowConsole),
    ));
    if !web {
        entries.push(checked_action(
            "Verbose Logging",
            "",
            Command::Tools(ToolCommand::ToggleVerboseLogging),
            crate::logging::verbose(),
        ));
        let hint = match &editor.mcp.address {
            Some(address) => format!("Listening on {address}"),
            None => "Lets AI assistants (MCP clients such as Claude Code) edit and build the open project".to_string(),
        };
        let mut mcp = checked_action("MCP Server", "", Command::Tools(ToolCommand::ToggleMcpServer), editor.mcp.address.is_some());
        mcp.set_tooltip(hint);
        entries.push(mcp);
    }
    entries.push(MenuEntry::Separator);
    entries.push(styled_action(
        "settings-2",
        Some(theme::ACCENT_MUTED_PURPLE),
        "Options...",
        "Ctrl Alt O",
        Command::ShowOptions(None),
    ));
    MainMenu::new("Tools", entries)
}

pub(super) fn help_menu() -> MainMenu {
    let guide = Command::Tools(ToolCommand::ShowDocs(crate::docs::HOME.to_string()));
    let reference = Command::Tools(ToolCommand::ShowDocs(crate::docs::NODE_REFERENCE.to_string()));
    let mut entries = vec![
        styled_action("help-square-rounded", Some(theme::ACCENT_BLUE), "User Guide", "", guide),
        styled_action("info-square-rounded", None, "Node Reference", "", reference),
        styled_action("keyboard", None, "Shortcut Reference", "", Command::Tools(ToolCommand::ShowShortcuts)),
        MenuEntry::Separator,
        action("What's new in this version?", "", Command::Tools(ToolCommand::ShowWhatsNew)),
    ];
    if !cfg!(target_arch = "wasm32") {
        entries.push(action("Check for updates", "", Command::Tools(ToolCommand::CheckForUpdates)));
        entries.push(MenuEntry::Separator);
        entries.push(styled_action("report-medical", None, "Diagnostics...", "", Command::Tools(ToolCommand::ShowDiagnostics)));
    }
    entries.push(MenuEntry::Separator);
    entries.push(styled_action("versions", None, "About Nerthus", "", Command::Tools(ToolCommand::ShowAbout)));
    MainMenu::new("Help", entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn find_action<'a>(menu: &'a MainMenu, wanted: &str) -> &'a MenuEntry {
        menu.entries
            .iter()
            .find(|entry| matches!(entry, MenuEntry::Action { label, .. } if label == wanted))
            .expect("the requested action exists")
    }

    fn find_submenu<'a>(menu: &'a MainMenu, wanted: &str) -> &'a MainMenu {
        menu.entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu(menu) if menu.title == wanted => Some(menu),
                _ => None,
            })
            .expect("the requested submenu exists")
    }

    #[test]
    fn menu_order_and_selection_availability_match_the_editor() {
        let mut editor = Editor::new(Document::new());
        let mut ui_state = UiState::default();
        let menus = build_menus(&editor, &ui_state);
        let titles: Vec<_> = menus.iter().map(|menu| menu.title.as_str()).collect();
        assert_eq!(titles, ["File", "Edit", "Graph", "Node", "Preview", "Project", "Tools", "Help"]);
        let empty_edit = edit_menu(&editor, &ui_state);
        assert!(matches!(find_action(&empty_edit, "Delete"), MenuEntry::Action { enabled: false, .. }));
        let node = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        ui_state.graph.selected_connections = vec![(node, "In".to_string())];
        let connection_edit = edit_menu(&editor, &ui_state);
        assert!(matches!(find_action(&connection_edit, "Delete"), MenuEntry::Action { enabled: true, .. }));
        assert!(matches!(find_action(&connection_edit, "Copy"), MenuEntry::Action { enabled: false, .. }));
        editor.select(Some(node));
        let selected_edit = edit_menu(&editor, &ui_state);
        assert!(matches!(find_action(&selected_edit, "Copy"), MenuEntry::Action { enabled: true, .. }));
    }

    #[test]
    fn dynamic_submenus_keep_their_command_payloads() {
        let mut editor = Editor::new(Document::new());
        let ui_state = UiState::default();
        let node = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        editor.select(Some(node));
        editor.document.add_bookmark(node);
        let graph = graph_menu(&editor, &ui_state);
        let bookmarks = find_submenu(&graph, "Bookmarks");
        let name = &editor.document.node(node).unwrap().name;
        assert!(matches!(find_action(bookmarks, name), MenuEntry::Action { command: Some(Command::GoToBookmark(id)), .. } if *id == node));
        if cfg!(target_os = "macos") {
            let remove = find_submenu(bookmarks, "Remove Bookmark");
            assert!(matches!(find_action(remove, name), MenuEntry::Action { command: Some(Command::RemoveBookmark(id)), .. } if *id == node));
        }
        let node_menu = node_menu(&editor, 0);
        let presets = find_submenu(&node_menu, "Presets");
        assert!(matches!(find_action(presets, "Create Preset"), MenuEntry::Action { command: Some(Command::CreatePreset(id)), .. } if *id == node));
        let region = editor.document.add_region([0.0, 0.0]);
        editor.preview_node = Some(node);
        editor.preview_region = Some(region.clone());
        let preview = preview_menu(&editor);
        let regions = find_submenu(&preview, "Regions");
        assert!(matches!(find_action(regions, &region), MenuEntry::Action { checked: Some(true), command: Some(Command::PreviewRegion(Some(name))), .. } if *name == region));
    }

    #[test]
    fn viewport_checkmarks_do_not_repeat_the_active_layout_command() {
        let mut editor = Editor::new(Document::new());
        for floating in [false, true] {
            editor.user.options.set_viewport_floating(floating);
            let preview = preview_menu(&editor);
            for (label, wanted) in [("Embedded", false), ("Floating", true)] {
                let MenuEntry::Action { checked, command, .. } = find_action(&preview, label) else {
                    unreachable!()
                };
                assert_eq!(*checked, Some(floating == wanted));
                let expected = (floating != wanted).then_some(Command::SetViewportFloating(wanted));
                assert_eq!(*command, expected);
            }
        }
    }
}
