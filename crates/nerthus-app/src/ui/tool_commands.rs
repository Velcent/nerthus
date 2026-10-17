//! Tools' and Help's commands (the Console, Verbose Logging, the Artifact
//! Explorer, Run Diagnostics, the help windows, the documentation and
//! update checks), the
//! keys the shell gives them (`Shell.OnPreviewKeyDown`: F12, backtick,
//! Esc and M), and drawing their windows and the first-run setup.

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::editor::Editor;
use crate::ui::artifact_explorer::{self, ArtifactExplorer};
use crate::ui::commands::Command;
use crate::ui::console_window::{self, ConsoleWindow};
use crate::ui::consume_shortcut;
use crate::ui::diagnostics_window::{self, DiagnosticsRequest, DiagnosticsWindow};
use crate::ui::docs_window::{self, DocsWindow};
use crate::ui::file_windows;
use crate::ui::help_windows::{self, UpdateDialog};
use crate::ui::{math_editor, painter, viewport_area};
use crate::ui::nahimic_warning::{self, NahimicWarning};
use crate::ui::onboard::{self, Onboard};
use crate::ui::prompt::Prompt;
use crate::ui::status_bar;
use crate::ui::tool_window;
use crate::ui::UiState;
use crate::updates::{DownloadOutcome, ProxySettings};
use crate::user::Preferences;

#[derive(Clone, Debug, PartialEq)]
pub enum ToolCommand {
    /// Tools › Console (F12, `ShowDebugConsole`).
    ShowConsole,
    /// Tools › Artifact Explorer….
    ShowArtifactExplorer,
    /// Tools › Run Diagnostics… and Help › Diagnostics….
    ShowDiagnostics,
    /// Tools › Verbose Logging, asked first; and its Yes.
    ToggleVerboseLogging,
    SetVerboseLogging(bool),
    /// Tools › MCP Server: on or off, now and for the next start.
    ToggleMcpServer,
    /// Help › Shortcut Reference, What's new in this version?, Check for
    /// updates and About Nerthus (and the title bar's version and Update
    /// buttons).
    ShowShortcuts,
    ShowWhatsNew,
    CheckForUpdates,
    ShowAbout,
    /// Help › User Guide and Node Reference, and the help links around
    /// the editor: the documentation at a page (`path#heading`).
    ShowDocs(String),
    /// The property area's Show Help for this Node: the selected node's
    /// page (`MiHelp_OnClick`).
    ShowNodeHelp,
    /// The downloaded update's installer run; with `close`, Nerthus
    /// closes at once (Yes to "Close Nerthus before proceeding?").
    InstallUpdate { path: std::path::PathBuf, close: bool },
    /// Esc: the preview build stops; with Ctrl the
    /// cancellation is reset.
    CancelPreviewBuild,
    ResetCancellation,
    /// "Nahimic Detected!" on the Start dialog or the status bar: the
    /// Nahimic warning.
    ShowNahimicWarning,
}

/// M asks the property area to open its Add Modifiers menu, as a flag in
/// egui's memory for the frame.
const ADD_MODIFIERS_ID: &str = "open-add-modifiers";

pub fn run(command: ToolCommand, editor: &mut Editor, ui_state: &mut UiState) {
    match command {
        ToolCommand::ShowConsole => {
            if ui_state.console.is_none() {
                ui_state.console = Some(ConsoleWindow::open());
            }
        }
        ToolCommand::ShowArtifactExplorer => {
            if ui_state.artifact_explorer.is_none() {
                ui_state.artifact_explorer = Some(ArtifactExplorer::open(editor));
            }
        }
        ToolCommand::ShowDiagnostics => {
            if ui_state.diagnostics.is_none() {
                ui_state.diagnostics = Some(DiagnosticsWindow::open());
            }
        }
        // `BtnVerbose_OnChecked` and `OnUnchecked`.
        ToolCommand::ToggleVerboseLogging => {
            let prompt = if crate::logging::verbose() {
                let text = "Are you sure you want to disable Verbose logging? This will create a new log file for this session.";
                Prompt::new("Switch to Standard logging", text, Command::Tools(ToolCommand::SetVerboseLogging(false)))
            } else {
                let text = "Are you sure you want to enable Verbose logging? This will create a new log file for this session.\n\nThis is mostly useful if you need to provide detailed technical logs to Tech Support.";
                Prompt::new("Switch to Verbose logging", text, Command::Tools(ToolCommand::SetVerboseLogging(true)))
            };
            ui_state.prompt = Some(prompt);
        }
        ToolCommand::SetVerboseLogging(verbose) => crate::logging::switch_verbose(verbose),
        ToolCommand::ToggleMcpServer => editor.mcp.toggle_requested = true,
        ToolCommand::ShowShortcuts => ui_state.help.shortcuts = true,
        ToolCommand::ShowWhatsNew => {
            if ui_state.help.whats_new.is_none() {
                ui_state.help.whats_new = Some(0);
            }
        }
        ToolCommand::ShowAbout => ui_state.help.about = true,
        ToolCommand::ShowDocs(target) => open_docs(ui_state, &target),
        ToolCommand::ShowNodeHelp => {
            let Some(node) = editor.active_node().and_then(|id| editor.document.node(id)) else {
                return;
            };
            log::info!("Open help for {}", node.type_name);
            let library = crate::docs::library();
            let target = library.node_page(&node.type_name).unwrap_or(crate::docs::NODE_REFERENCE).to_string();
            open_docs(ui_state, &target);
        }
        ToolCommand::InstallUpdate { path, close } => {
            crate::updates::run_installer(&path);
            // Closing does not ask about unsaved changes: the question
            // said they would be lost.
            if close {
                editor.files.exit_now = true;
            }
        }
        ToolCommand::CheckForUpdates => {
            let proxy = proxy_settings(&editor.user.preferences);
            match ui_state.updates.ask(proxy) {
                Some(answer) => ui_state.help.update = Some(UpdateDialog::new(answer)),
                None => editor.report("Checking for updates..."),
            }
        }
        ToolCommand::CancelPreviewBuild => {
            if editor.building {
                editor.cancel_build = true;
            }
        }
        ToolCommand::ResetCancellation => {
            log::info!("Manual reset of Cancellation Token.");
            editor.cancel_build = false;
            editor.cancelling = false;
        }
        ToolCommand::ShowNahimicWarning => {
            if ui_state.nahimic_warning.is_none() {
                ui_state.nahimic_warning = Some(NahimicWarning::default());
            }
        }
    }
}

/// Download Update's download: started when asked for; once it is in,
/// "Close Nerthus before proceeding?" (Yes closes Nerthus and runs the
/// installer, No only runs it).
fn update_download(editor: &mut Editor, ui_state: &mut UiState) {
    if let Some(token) = ui_state.help.download_requested.take() {
        let proxy = proxy_settings(&editor.user.preferences);
        ui_state.updates.start_download(&token, proxy);
    }
    match ui_state.updates.poll_download() {
        Some(DownloadOutcome::Finished(path)) => {
            let text = "Close Nerthus before proceeding? Unsaved work will be lost!\n\nOr click NO, save your work, close Nerthus, and then proceed with the installer.";
            let yes = Command::Tools(ToolCommand::InstallUpdate { path: path.clone(), close: true });
            let no = Command::Tools(ToolCommand::InstallUpdate { path, close: false });
            let mut prompt = Prompt::new("Close Nerthus?", text, yes);
            prompt.no = Some(no.clone());
            prompt.dismissed = Some(no);
            ui_state.prompt = Some(prompt);
        }
        Some(DownloadOutcome::Failed(error)) => editor.report(format!("The update could not be downloaded. {error}")),
        None => {}
    }
}

/// The documentation at `target`, in its window (opened if need be).
fn open_docs(ui_state: &mut UiState, target: &str) {
    match ui_state.docs.as_mut() {
        Some(window) => window.go(target),
        None => ui_state.docs = Some(DocsWindow::open(target)),
    }
}

/// Whether M asked for the Add Modifiers menu this frame (the property
/// area opens it when it shows a node, `btnAddModifier.IsSubmenuOpen`).
pub fn add_modifiers_wanted(ctx: &egui::Context) -> bool {
    let id = egui::Id::new(ADD_MODIFIERS_ID);
    ctx.data_mut(|data| data.remove_temp::<bool>(id)).unwrap_or(false)
}

/// The frame is over: an unanswered M is dropped.
pub fn forget_add_modifiers(ctx: &egui::Context) {
    let id = egui::Id::new(ADD_MODIFIERS_ID);
    ctx.data_mut(|data| data.remove::<bool>(id));
}

/// `Locations.UseProxy` and `Locations.Proxy`.
pub fn proxy_settings(preferences: &Preferences) -> ProxySettings {
    let use_proxy = preferences.member("Locations", "UseProxy").as_bool().unwrap_or(false);
    let proxy = preferences.member("Locations", "Proxy");
    let proxy = proxy.as_str().unwrap_or_default().to_string();
    ProxySettings { use_proxy, proxy }
}

/// The shell's keys for these commands (`Shell.OnPreviewKeyDown`); none
/// while a text field has the keyboard.
pub fn shortcuts(ctx: &egui::Context, pending: &mut Vec<Command>) {
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
        return;
    }
    // Windows take Esc before these shortcuts. In the shell it stops the
    // preview build and is also left for the graph to end a drag.
    let (escape, command_held) = ctx.input(|input| (input.key_pressed(Key::Escape), input.modifiers.command));
    if escape && !tool_window::shell_has_keyboard(ctx) {
        let command = if command_held { ToolCommand::ResetCancellation } else { ToolCommand::CancelPreviewBuild };
        pending.push(Command::Tools(command));
    }
    let console = KeyboardShortcut::new(Modifiers::NONE, Key::F12);
    if ctx.input_mut(|input| consume_shortcut(input, &console)) {
        pending.push(Command::Tools(ToolCommand::ShowConsole));
    }
    // The backtick (`Key.Oem3`): suspend or resume.
    let backtick = KeyboardShortcut::new(Modifiers::NONE, Key::Backtick);
    if ctx.input_mut(|input| consume_shortcut(input, &backtick)) {
        pending.push(Command::ToggleEngine);
    }
    let add_modifiers = KeyboardShortcut::new(Modifiers::NONE, Key::M);
    if ctx.input_mut(|input| consume_shortcut(input, &add_modifiers)) {
        let id = egui::Id::new(ADD_MODIFIERS_ID);
        ctx.data_mut(|data| data.insert_temp(id, true));
    }
}

/// The first frame's work: the memory readout's colour noted, the
/// first-run setup, and the update check made at start.
pub fn start(editor: &Editor, ui_state: &mut UiState) {
    status_bar::note_eco_mark();
    if ui_state.onboard_at_launch {
        ui_state.onboard = Some(Onboard::open(editor.compute.cpu_only));
    }
    if ui_state.check_updates_at_launch {
        let proxy = proxy_settings(&editor.user.preferences);
        ui_state.updates.start(proxy, false);
    }
}

/// Esc closes the window on top before the shell can cancel a preview
/// build. Each window runs its own close action, including Math's Cancel
/// rollback, the Painter's cleanup, and a running export's cancellation.
/// The Artifact Explorer and console keep their source behavior and stay.
pub fn take_escape(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState) {
    let build_progress = editor.export_build.as_ref().is_some_and(|build| matches!(build.kind, crate::export_build::ExportKind::Build));
    let shown = [
        (ui_state.diagnostics.is_some(), diagnostics_window::window_id()),
        (ui_state.docs.is_some(), docs_window::window_id()),
        (ui_state.console.is_some(), console_window::window_id()),
        (ui_state.artifact_explorer.is_some(), artifact_explorer::window_id()),
        (editor.files.export_macro.is_some(), file_windows::export_macro_id()),
        (editor.files.start_dialog_open, egui::Id::new("start-dialog")),
        (editor.files.examples_open, egui::Id::new("examples-window")),
        (ui_state.build_settings.is_some(), egui::Id::new("build-settings")),
        (ui_state.statistics.is_some(), egui::Id::new("statistics-window")),
        (build_progress, egui::Id::new("build-progress")),
        (ui_state.help.shortcuts, egui::Id::new("shortcut-guide")),
        (ui_state.help.whats_new.is_some(), egui::Id::new("whats-new")),
        (ui_state.help.about, egui::Id::new("about-nerthus")),
        (ui_state.sun_open, viewport_area::lighting_panel_id()),
        (math_editor::is_open(ctx), math_editor::area_id()),
        (painter::is_open(ctx), painter::area_id()),
    ];
    let mut windows: Vec<egui::Id> = shown.into_iter().filter_map(|(open, id)| open.then_some(id)).collect();
    if let Some(constructor) = ui_state.constructor.as_ref() {
        windows.push(egui::Id::new(("constructor", constructor.node())));
    }
    windows.extend(ui_state.viewport_2d.window_ids());
    let Some(closed) = tool_window::take_escape(ctx, &windows) else {
        return;
    };
    if closed == diagnostics_window::window_id() {
        ui_state.diagnostics = None;
    } else if closed == docs_window::window_id() {
        ui_state.docs = None;
    } else if closed == file_windows::export_macro_id() {
        file_windows::close_export_macro(editor);
    } else if closed == viewport_area::lighting_panel_id() {
        ui_state.sun_open = false;
    } else if closed == console_window::window_id() || closed == artifact_explorer::window_id() {
        // These windows have no Cancel action. Esc stays in the window
        // and cannot cancel an unrelated preview build in the shell.
    } else {
        tool_window::request_close(ctx, closed);
    }
}

/// Whether a window that keeps the editor's keys to itself is up: the
/// first-run setup, the Artifact Explorer, Help's windows, the Nahimic
/// warning, or any `egui::Modal` (Build and Export?, Save terrain?, an
/// input box). egui's modals keep the pointer from what is under them but
/// not the keys. The shortcuts are read before this frame's windows are
/// drawn, so the modal shown last frame counts: the one on screen.
pub fn modal_open(ctx: &egui::Context, ui_state: &UiState) -> bool {
    let egui_modal = ctx.memory(|memory| memory.top_modal_layer().is_some());
    egui_modal || ui_state.onboard.is_some() || ui_state.artifact_explorer.is_some() || ui_state.help.any_open() || ui_state.nahimic_warning.is_some()
}

/// Draws the windows open, and an update check's answer once it is in.
/// Returns whether Run Diagnostics put the settings back to their
/// defaults, for the interface to take them in.
pub fn show_windows(ctx: &egui::Context, editor: &mut Editor, ui_state: &mut UiState, pending: &mut Vec<Command>) -> bool {
    let mut settings_reset = false;
    for command in ui_state.take_tools_at_launch() {
        run(command, editor, ui_state);
    }
    if ui_state.whats_new_at_launch && ui_state.onboard.is_none() {
        ui_state.whats_new_at_launch = false;
        if help_windows::version_first_run() {
            ui_state.help.whats_new = Some(0);
            ui_state.help.whats_new_first_run = true;
        }
    }
    // The check made as Nerthus starts found Nahimic's service running.
    if crate::nahimic::take_warning_request() && ui_state.nahimic_warning.is_none() {
        ui_state.nahimic_warning = Some(NahimicWarning::default());
    }
    if let Some(answer) = ui_state.updates.poll() {
        ui_state.help.update = Some(UpdateDialog::new(answer));
    }
    update_download(editor, ui_state);
    if ui_state.updates.checking() || ui_state.updates.downloading() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    if let Some(console) = ui_state.console.as_mut()
        && !console_window::show(ctx, console)
    {
        ui_state.console = None;
    }
    if let Some(explorer) = ui_state.artifact_explorer.as_mut()
        && !artifact_explorer::show(ctx, editor, explorer)
    {
        ui_state.artifact_explorer = None;
    }
    if let Some(diagnostics) = ui_state.diagnostics.as_mut() {
        let known_update = ui_state.updates.update_available();
        let (open, request) = diagnostics_window::show(ctx, editor, diagnostics, known_update);
        match request {
            Some(DiagnosticsRequest::Restart(arguments)) => pending.push(Command::File(crate::files::FileCommand::Restart(arguments))),
            Some(DiagnosticsRequest::SettingsReset) => {
                settings_reset = true;
                ui_state.onboard = Some(Onboard::open(editor.compute.cpu_only));
            }
            None => {}
        }
        if !open {
            ui_state.diagnostics = None;
        }
    }
    help_windows::show(ctx, &mut ui_state.help);
    if let Some(window) = ui_state.docs.as_mut()
        && !docs_window::show(ctx, window)
    {
        ui_state.docs = None;
    }
    if let Some(setup) = ui_state.onboard.as_mut()
        && !onboard::show(ctx, editor, setup, &mut ui_state.file_dialogs)
    {
        ui_state.onboard = None;
    }
    if let Some(warning) = ui_state.nahimic_warning.as_mut()
        && !nahimic_warning::show(ctx, warning)
    {
        ui_state.nahimic_warning = None;
    }
    settings_reset
}

#[cfg(test)]
mod tests {
    use egui::{Event, Pos2, RawInput, Rect, Vec2};

    use super::*;
    use crate::document::Document;
    use crate::files::ExportForm;
    use crate::theme;
    use crate::ui::{batch_build, build_settings, constructor, options_window, statistics_window, viewport_2d};

    fn escape() -> Event {
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// A frame of the windows with `events`, Esc taken first as the shell
    /// takes it. Returns whether what was drawn after saw the Esc.
    fn frame(context: &egui::Context, editor: &mut Editor, ui_state: &mut UiState, events: Vec<Event>) -> bool {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1333.0, 800.0))), events, ..Default::default() };
        let mut seen = false;
        let mut pending = Vec::new();
        let output = context.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            take_escape(&ctx, editor, ui_state);
            if !modal_open(&ctx, ui_state) {
                options_window::take_escape(&ctx, &mut ui_state.options);
                batch_build::take_escape(&ctx, &mut ui_state.batch_build);
                shortcuts(&ctx, &mut pending);
            }
            seen = ctx.input(|input| input.key_pressed(Key::Escape));
            for command in std::mem::take(&mut pending) {
                if let Command::Tools(command) = command {
                    run(command, editor, ui_state);
                }
            }
            build_settings::show(&ctx, editor, &mut ui_state.build_settings, &mut pending);
            statistics_window::show(&ctx, &mut ui_state.statistics, &mut pending);
            if let Some(window) = ui_state.constructor.as_mut()
                && !constructor::show(&ctx, editor, window)
            {
                ui_state.constructor = None;
            }
            math_editor::show(&ctx, editor);
            painter::show(&ctx, editor);
            viewport_2d::show_floating(&ctx, editor, &mut ui_state.viewport_2d, &[], &mut pending);
            file_windows::show(&ctx, editor, ui_state, &mut pending);
            options_window::show(&ctx, editor, &mut ui_state.options, &mut ui_state.last_options_tab, &mut ui_state.file_dialogs);
            if let Some(window) = ui_state.batch_build.as_mut()
                && !batch_build::show(&ctx, editor, window, &mut ui_state.batch_session, &mut ui_state.file_dialogs)
            {
                ui_state.batch_build = None;
            }
            show_windows(&ctx, editor, ui_state, &mut pending);
            tool_window::note_keyboard(&ctx);
        });
        output.drop_without_applying_deltas();
        seen
    }

    fn settle(context: &egui::Context, editor: &mut Editor, ui_state: &mut UiState) {
        for _ in 0..3 {
            frame(context, editor, ui_state, Vec::new());
        }
    }

    fn shell() -> (egui::Context, Editor, UiState) {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        let empty_documents = std::env::temp_dir().join(format!("nerthus-dialog-test-no-projects-{}", std::process::id()));
        editor.user.preferences.set_member("Locations", "Documents", serde_json::json!(empty_documents));
        (context, editor, UiState::default())
    }

    /// Which of the documentation, Diagnostics and Export Macro are open.
    fn open(editor: &Editor, ui_state: &UiState) -> Vec<bool> {
        vec![ui_state.docs.is_some(), ui_state.diagnostics.is_some(), editor.files.export_macro.is_some()]
    }

    #[test]
    fn escape_closes_the_tool_windows_from_the_top_down() {
        let (context, mut editor, mut ui_state) = shell();
        // Opened one after another, each over the last.
        let opening: [fn(&mut Editor, &mut UiState); 3] = [
            |_, ui_state| ui_state.docs = Some(DocsWindow::open(crate::docs::HOME)),
            |_, ui_state| ui_state.diagnostics = Some(DiagnosticsWindow::open()),
            |editor, _| editor.files.export_macro = Some(ExportForm::for_project(editor)),
        ];
        for open_window in opening {
            open_window(&mut editor, &mut ui_state);
            settle(&context, &mut editor, &mut ui_state);
        }
        assert_eq!(open(&editor, &ui_state), [true; 3]);
        // Each Esc closes the one on top, and does not stop a build too.
        for left in (0..3).rev() {
            let seen = frame(&context, &mut editor, &mut ui_state, vec![escape()]);
            assert!(!seen, "the shell saw the Esc that closed a window");
            let expected: Vec<bool> = (0..3).map(|window| window < left).collect();
            assert_eq!(open(&editor, &ui_state), expected);
            settle(&context, &mut editor, &mut ui_state);
        }
        // With none left, Esc is the shell's.
        assert!(frame(&context, &mut editor, &mut ui_state, vec![escape()]));
    }

    fn open_app_windows(editor: &Editor, ui_state: &UiState) -> Vec<bool> {
        vec![
            editor.files.start_dialog_open,
            ui_state.build_settings.is_some(),
            ui_state.statistics.is_some(),
            editor.files.examples_open,
            ui_state.constructor.is_some(),
            ui_state.help.shortcuts,
            ui_state.help.whats_new.is_some(),
            ui_state.help.about,
        ]
    }

    #[test]
    fn escape_closes_each_app_window_without_cancelling_a_preview_build() {
        let (context, mut editor, mut ui_state) = shell();
        editor.building = true;
        let opening: [fn(&mut Editor, &mut UiState); 8] = [
            |editor, _| editor.files.start_dialog_open = true,
            |_, ui_state| ui_state.build_settings = Some(build_settings::BuildSettingsState::new(build_settings::SettingsTab::Resolution)),
            |editor, ui_state| ui_state.statistics = Some(statistics_window::StatisticsWindow::new(editor)),
            |editor, _| editor.files.examples_open = true,
            |editor, ui_state| {
                let node = editor.document.add_node("Construction", [0.0, 0.0]).unwrap();
                ui_state.constructor = Some(constructor::ConstructorWindow::new(node));
            },
            |_, ui_state| ui_state.help.shortcuts = true,
            |_, ui_state| ui_state.help.whats_new = Some(0),
            |_, ui_state| ui_state.help.about = true,
        ];
        for open_window in opening {
            open_window(&mut editor, &mut ui_state);
            settle(&context, &mut editor, &mut ui_state);
        }
        assert_eq!(open_app_windows(&editor, &ui_state), [true; 8]);
        for left in (0..8).rev() {
            let seen = frame(&context, &mut editor, &mut ui_state, vec![escape()]);
            assert!(!seen && !editor.cancel_build, "closing a window also cancelled the preview");
            let expected: Vec<bool> = (0..8).map(|window| window < left).collect();
            assert_eq!(open_app_windows(&editor, &ui_state), expected);
        }
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(editor.cancel_build, "Esc in the shell still cancels its preview build");
    }

    #[test]
    fn options_does_not_take_escape_from_a_newer_window() {
        let (context, mut editor, mut ui_state) = shell();
        ui_state.options = Some(options_window::OptionsState::new(options_window::OptionsTab::General));
        settle(&context, &mut editor, &mut ui_state);
        ui_state.statistics = Some(statistics_window::StatisticsWindow::new(&editor));
        settle(&context, &mut editor, &mut ui_state);
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(ui_state.statistics.is_none() && ui_state.options.is_some());
        settle(&context, &mut editor, &mut ui_state);
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(ui_state.options.is_none());
    }

    #[test]
    fn escape_closes_one_map_viewer_at_a_time() {
        let (context, mut editor, mut ui_state) = shell();
        let first = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let second = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let following = egui::Id::new("map-viewer-following");
        let first_window = egui::Id::new(("map-viewer", first));
        let second_window = egui::Id::new(("map-viewer", second));
        ui_state.viewport_2d.show_2d_view(&editor);
        settle(&context, &mut editor, &mut ui_state);
        ui_state.viewport_2d.open_floating(first);
        settle(&context, &mut editor, &mut ui_state);
        ui_state.viewport_2d.open_floating(second);
        settle(&context, &mut editor, &mut ui_state);
        assert_eq!(ui_state.viewport_2d.window_ids(), [first_window, second_window, following]);
        for expected in [vec![first_window, following], vec![following], Vec::new()] {
            frame(&context, &mut editor, &mut ui_state, vec![escape()]);
            assert_eq!(ui_state.viewport_2d.window_ids(), expected);
        }
    }

    #[test]
    fn consecutive_escapes_close_math_and_painter_without_leaving_a_close_request() {
        let (context, mut editor, mut ui_state) = shell();
        editor.preview_resolution = 32;
        editor.building = true;
        let math_node = editor.document.add_node("MathX", [0.0, 0.0]).unwrap();
        let draw_node = editor.document.add_node("Draw", [0.0, 0.0]).unwrap();
        ui_state.docs = Some(DocsWindow::open(crate::docs::HOME));
        settle(&context, &mut editor, &mut ui_state);
        math_editor::open(&context, &editor, math_node);
        settle(&context, &mut editor, &mut ui_state);
        painter::open(&context, &editor, draw_node);
        settle(&context, &mut editor, &mut ui_state);
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(!painter::is_open(&context) && math_editor::is_open(&context));
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(!math_editor::is_open(&context) && ui_state.docs.is_some());
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(ui_state.docs.is_none() && !editor.cancel_build);
        math_editor::open(&context, &editor, math_node);
        painter::open(&context, &editor, draw_node);
        settle(&context, &mut editor, &mut ui_state);
        assert!(math_editor::is_open(&context) && painter::is_open(&context));
    }

    #[test]
    fn escape_closes_the_lighting_panel_before_a_preview_build() {
        let (context, mut editor, mut ui_state) = shell();
        editor.building = true;
        ui_state.sun_open = true;
        let mut settings = nerthus_render::ViewportSettings::default();
        let overlays = viewport_area::ViewOverlays { north_angle: 0.0, navigation_hint: None };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1333.0, 800.0));
        let area = Rect::from_min_size(Pos2::new(20.0, 60.0), Vec2::new(900.0, 600.0));
        for step in 0..4 {
            let events = if step == 3 { vec![escape()] } else { Vec::new() };
            let input = RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut pending = Vec::new();
            let output = context.run_ui(input, |ui| {
                take_escape(ui.ctx(), &mut editor, &mut ui_state);
                shortcuts(ui.ctx(), &mut pending);
                viewport_area::show(ui, area, &editor, &mut settings, &mut ui_state.sun_open, &mut ui_state.viewport_2d, &overlays, &mut pending);
                tool_window::note_keyboard(ui.ctx());
            });
            output.drop_without_applying_deltas();
            for command in pending {
                if let Command::Tools(command) = command {
                    run(command, &mut editor, &mut ui_state);
                }
            }
        }
        assert!(!ui_state.sun_open && !editor.cancel_build);
    }

    #[test]
    fn the_console_and_the_artifact_explorer_stay_on_escape() {
        let (context, mut editor, mut ui_state) = shell();
        editor.building = true;
        ui_state.console = Some(ConsoleWindow::open());
        settle(&context, &mut editor, &mut ui_state);
        // About over the console closes; the console, with no Cancel
        // button, stays.
        ui_state.help.about = true;
        settle(&context, &mut editor, &mut ui_state);
        frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(!ui_state.help.about && ui_state.console.is_some());
        settle(&context, &mut editor, &mut ui_state);
        let seen = frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(ui_state.console.is_some() && !seen && !editor.cancel_build);
        // So does the Artifact Explorer.
        ui_state.artifact_explorer = Some(ArtifactExplorer::open(&editor));
        settle(&context, &mut editor, &mut ui_state);
        let seen = frame(&context, &mut editor, &mut ui_state, vec![escape()]);
        assert!(ui_state.artifact_explorer.is_some() && !seen && !editor.cancel_build);
    }
}
