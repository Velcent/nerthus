//! File commands with their questions. New, Open, a recent or dropped
//! file, Repair File, Open Examples and Exit first ask whether to save
//! changes (`SavePrompt`: only with unsaved changes and an undo history;
//! Exit asks whenever there are unsaved changes, `Shell_Closing`), and
//! Open, New and Repair whether to close while nodes build
//! (`PreventCloseWhileBuilding`; Open asks it first, New and Repair after
//! the save question, in `SetupNewFile`). Repair then closes the project
//! for a new one and shows its file chooser (`RepairFile`).

use std::path::Path;

use crate::editor::Editor;
use crate::files::{self, OpenMode, ExportForm, FileAction, FileCommand, FileDialog, Pending};
use crate::ui::commands::Command;
use crate::ui::graph_canvas::ZoomRequest;
use crate::ui::prompt::Prompt;
use crate::ui::UiState;

pub fn run(command: FileCommand, editor: &mut Editor, ui_state: &mut UiState) {
    match command {
        FileCommand::New => request(FileAction::New, editor, ui_state),
        FileCommand::Open => request(FileAction::Open, editor, ui_state),
        FileCommand::OpenRecent(path) => {
            let action = FileAction::OpenPath { path, mode: OpenMode::Normal };
            request(action, editor, ui_state);
        }
        FileCommand::OpenLatest => {
            let latest = editor.user.recent.files().first().cloned();
            if let Some(path) = latest {
                let action = FileAction::OpenPath { path, mode: OpenMode::Normal };
                request(action, editor, ui_state);
            }
        }
        // `Shell_Drop`: the save question, the file, and the Start dialog
        // closed.
        FileCommand::Dropped(path) => {
            editor.files.start_dialog_open = false;
            let action = FileAction::OpenPath { path, mode: OpenMode::Normal };
            request(action, editor, ui_state);
        }
        FileCommand::SaveIncremental => save_incremental(editor),
        FileCommand::SetAutosaveInterval(minutes) => {
            editor.files.autosave.set_interval_minutes(minutes);
            editor.user.options.set_autosave_interval(editor.files.autosave.interval);
        }
        FileCommand::ResetAutosaveTimer => editor.files.autosave.reset_timer(),
        // `OpenAutosaveEntry` and `OpenAutoSave` open without asking.
        FileCommand::OpenAutosaveEntry(path) => {
            if path.is_file() {
                files::open_project(editor, &path);
            }
        }
        FileCommand::OpenAutosave => editor.files.dialog = Some(FileDialog::Open { folder: files::autosaves_folder() }),
        FileCommand::RevertToSaved => revert_to_saved(editor, ui_state),
        FileCommand::RevertConfirmed => {
            if let Some(path) = editor.document.path.clone() {
                files::open_project(editor, &path);
            }
        }
        FileCommand::RepairFile => request(FileAction::Repair, editor, ui_state),
        FileCommand::OpenExamples => request(FileAction::Examples, editor, ui_state),
        FileCommand::StartDialog => editor.files.start_dialog_open = true,
        FileCommand::ImportMacro => editor.files.dialog = Some(FileDialog::InstallMacro),
        FileCommand::ImportGraph => editor.files.dialog = Some(FileDialog::ImportGraph),
        FileCommand::ImportTorlet => editor.files.dialog = Some(FileDialog::ImportTorlet),
        FileCommand::ImportImage => {
            let tab = ui_state.graph.tab;
            let position = ui_state.graph.drop_position();
            editor.files.dialog = Some(FileDialog::ImportImage { tab, position });
        }
        // `Shell.ExportMacro`: the project saved, then the dialog.
        FileCommand::ExportMacro => {
            if let Some(path) = editor.document.path.clone() {
                editor.save_to(&path);
            }
            editor.files.export_macro = Some(ExportForm::for_project(editor));
        }
        FileCommand::ExportTorlet => {
            let tab = ui_state.graph.tab;
            editor.files.dialog = Some(FileDialog::ExportTorlet { tab });
        }
        FileCommand::Exit => request(FileAction::Exit, editor, ui_state),
        FileCommand::Restart(arguments) => request(FileAction::Restart(arguments), editor, ui_state),
        FileCommand::SaveFirst => save_first(editor, ui_state),
        // No to the save question, or a Save As it asked for went through.
        FileCommand::DiscardChanges => {
            if let Some(pending) = editor.files.pending.as_mut() {
                pending.save_answered = true;
            }
            advance(editor, ui_state);
        }
        FileCommand::ContinueWhileBuilding => {
            if let Some(pending) = editor.files.pending.as_mut() {
                pending.building_answered = true;
            }
            advance(editor, ui_state);
        }
        FileCommand::InstallMacro(path) => files::install(editor, &path),
        FileCommand::AddMacro(id) => {
            let position = ui_state.graph.drop_position();
            let tab = ui_state.graph.tab;
            files::add_macro(editor, &id, position, tab);
        }
        // Start and Examples stay editable over the project, so each
        // replacement checks for unsaved work when the action is chosen.
        FileCommand::StartNew => {
            editor.files.start_dialog_open = false;
            request(FileAction::New, editor, ui_state);
        }
        FileCommand::StartOpen => {
            editor.files.start_dialog_open = false;
            request(FileAction::Open, editor, ui_state);
        }
        FileCommand::OpenDirect { path, mode } => {
            editor.files.start_dialog_open = false;
            let action = FileAction::OpenPath { path, mode };
            request(action, editor, ui_state);
        }
        FileCommand::OpenExample { path, mode } => {
            let action = FileAction::OpenExample { path, mode };
            request(action, editor, ui_state);
        }
        FileCommand::ExportMacroChecked => export_macro_checked(editor),
        // `RecoverOrphans` runs before the graph is made, so a saved tab
        // that was one of the orphans' is the one shown.
        FileCommand::RecoverOrphans => {
            editor.document.recover_orphans();
            ui_state.graph.tab = editor.document.opening_tab();
        }
        FileCommand::DeleteOrphans => {
            editor.document.delete_orphans();
            editor.changed();
        }
        FileCommand::DeleteObsoleteBakes(folder) => files::delete_obsolete_bakes(editor, &folder),
    }
}

/// Starts an action that may need answers first.
fn request(action: FileAction, editor: &mut Editor, ui_state: &mut UiState) {
    let pending = Pending { action, save_answered: false, building_answered: false };
    editor.files.pending = Some(pending);
    advance(editor, ui_state);
}

/// Asks the next question the pending action needs, or does it.
fn advance(editor: &mut Editor, ui_state: &mut UiState) {
    let Some(pending) = editor.files.pending.clone() else {
        return;
    };
    let building = editor.building || editor.baking;
    let asks_building_first = pending.action == FileAction::Open;
    let asks_building_after = matches!(pending.action, FileAction::New | FileAction::Repair);
    if asks_building_first && building && !pending.building_answered {
        ask_building(ui_state);
        return;
    }
    if !pending.save_answered && needs_save_question(&pending.action, editor) {
        ui_state.prompt = Some(save_question());
        return;
    }
    if asks_building_after && building && !pending.building_answered {
        ask_building(ui_state);
        return;
    }
    editor.files.pending = None;
    perform(pending.action, editor);
}

/// The save question (`MboxFull(…, YesNoCancel, TaskDialogIcon.Warning)`):
/// Yes saves first, No goes on without saving.
pub fn save_question() -> Prompt {
    let text = "There are unsaved changes in the current terrain. Do you want to save the current terrain before proceeding? ";
    let yes = Command::File(FileCommand::SaveFirst);
    let no = Command::File(FileCommand::DiscardChanges);
    let mut question = Prompt::yes_no_cancel("Save terrain?", text, yes, no);
    question.warning = true;
    question
}

/// `SavePrompt`'s condition (unsaved changes and something to undo), or
/// for Exit `Shell_Closing`'s (unsaved changes).
fn needs_save_question(action: &FileAction, editor: &Editor) -> bool {
    if matches!(action, FileAction::Exit | FileAction::Restart(_)) {
        return editor.document.dirty;
    }
    editor.document.dirty && editor.document.can_undo()
}

/// `PreventCloseWhileBuilding`.
fn ask_building(ui_state: &mut UiState) {
    let text = "Nodes are building, are you sure you want close the existing project?";
    ui_state.prompt = Some(Prompt::new("Nodes are building?", text, Command::File(FileCommand::ContinueWhileBuilding)));
}

/// The save question's Yes (`SaveFile`): saved, then on; an untitled
/// project asks where first. When that save does not happen, nothing
/// goes on.
fn save_first(editor: &mut Editor, ui_state: &mut UiState) {
    let Some(path) = editor.document.path.clone() else {
        editor.files.dialog = Some(FileDialog::SaveAsThenContinue);
        return;
    };
    editor.save_to(&path);
    if editor.document.dirty {
        editor.files.pending = None;
        return;
    }
    if let Some(pending) = editor.files.pending.as_mut() {
        pending.save_answered = true;
    }
    advance(editor, ui_state);
}

/// What each action does once answered.
fn perform(action: FileAction, editor: &mut Editor) {
    match action {
        // `CloseFile`, `SetupNewFile`.
        FileAction::New => files::new_project(editor),
        FileAction::Open => editor.files.dialog = Some(FileDialog::Open { folder: None }),
        FileAction::OpenPath { path, mode } => {
            if files::open_project(editor, &path) {
                apply_open_mode(editor, mode);
            }
        }
        FileAction::OpenExample { path, mode } => open_example(editor, &path, mode),
        // `CloseFile`, `SetupNewFile`, then the chooser.
        FileAction::Repair => {
            files::new_project(editor);
            editor.files.dialog = Some(FileDialog::Repair);
        }
        FileAction::Examples => editor.files.examples_open = true,
        // `CloseFile` as Nerthus closes: its previews written first when
        // the project keeps them.
        FileAction::Exit => {
            if !editor.exit_when_frozen() {
                editor.files.exit_now = true;
            }
        }
        FileAction::Restart(arguments) => {
            if let Err(error) = crate::ui::diagnostics_window::prepare_restart(arguments) {
                let message = files::Message::new("Cannot restart Nerthus", &error.to_string());
                editor.files.messages.push(message);
                return;
            }
            if !editor.exit_when_frozen() {
                editor.files.exit_now = true;
            }
        }
    }
}

/// `Shell.SaveIncremental`: a saved project is saved again under the next
/// numbered name (`Increment`), its bakes copied along; an unsaved one is
/// saved as usual.
fn save_incremental(editor: &mut Editor) {
    let saved = editor.document.path.clone().filter(|path| path.is_file());
    let Some(path) = saved else {
        editor.save();
        return;
    };
    let next = files::incremented_path(&path);
    editor.save_to(&next);
}

/// `Shell.RevertToSaved`.
fn revert_to_saved(editor: &mut Editor, ui_state: &mut UiState) {
    let saved = editor.document.path.as_deref().is_some_and(Path::is_file);
    if !saved {
        let text = "This file was not saved, so it cannot be reverted to the saved state.";
        ui_state.prompt = Some(Prompt::message("Cannot revert", text));
        return;
    }
    let text = "Are you sure you want to undo all changes and revert to the last saved state?";
    ui_state.prompt = Some(Prompt::new("Revert to saved state?", text, Command::File(FileCommand::RevertConfirmed)));
}

/// An example opened from the examples (`ExamplesWindow.B_Click` and its
/// menu): as an untitled copy named after it, kept out of the recent
/// files; at a 512 preview, or with the engine suspended, when asked.
fn open_example(editor: &mut Editor, path: &Path, mode: OpenMode) {
    editor.files.examples_open = false;
    editor.files.start_dialog_open = false;
    if files::open_example(editor, path) {
        apply_open_mode(editor, mode);
    }
}

/// `LoadingMechanism`: ForceLowResolution previews at 512,
/// SuspendEngine leaves the engine suspended.
fn apply_open_mode(editor: &mut Editor, mode: OpenMode) {
    match mode {
        OpenMode::Normal => {}
        OpenMode::LowResolution => editor.set_preview_resolution(512),
        OpenMode::SuspendEngine => editor.engine_active = false,
    }
}

/// Export Macro, its questions answered: the save dialog when it saves a
/// file (`ExportMacroDialog.BtnOK_OnClick`), else straight into the
/// collection.
fn export_macro_checked(editor: &mut Editor) {
    let Some(form) = editor.files.export_macro.as_ref() else {
        return;
    };
    if form.save_file {
        let id = form.macro_id();
        editor.files.dialog = Some(FileDialog::ExportMacro { id });
    } else {
        files::export_to(editor, None);
    }
}

/// What the interface does each frame for files: commands from outside it
/// run, a message waiting shows when no other is open, a tab an import
/// added comes into view, and an alert raised shows.
pub fn take_requests(editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    for request in std::mem::take(&mut editor.files.requests) {
        commands.push(Command::File(request));
    }
    if ui_state.prompt.is_none() && !editor.files.messages.is_empty() {
        let message = editor.files.messages.remove(0);
        let mut prompt = Prompt::message(&message.caption, &message.text);
        prompt.yes = message.yes.map(Command::File);
        prompt.no = message.no.map(Command::File);
        prompt.dismissed = message.dismissed.map(Command::File);
        prompt.footnote = message.footnote;
        ui_state.prompt = Some(prompt);
    }
    if let Some((tab, frame)) = editor.files.show_tab.take() {
        ui_state.graph.tab = tab;
        if frame {
            ui_state.graph.zoom_request = Some(ZoomRequest::Extents);
        }
    }
    if let Some(alert) = editor.files.alert.take() {
        ui_state.alert = Some(alert);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn test_folder(name: &str) -> std::path::PathBuf {
        let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
        let folder = std::env::temp_dir().join(format!("nerthus-unsaved-{name}-{}-{}", std::process::id(), elapsed.as_nanos()));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn edited_project() -> Editor {
        let mut document = Document::new();
        document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.preview_resolution = 2048;
        editor
    }

    fn replacement_commands(path: &Path) -> Vec<FileCommand> {
        let mut commands = vec![FileCommand::StartNew, FileCommand::StartOpen, FileCommand::OpenExamples];
        for mode in [OpenMode::Normal, OpenMode::LowResolution, OpenMode::SuspendEngine] {
            commands.push(FileCommand::OpenDirect { path: path.to_path_buf(), mode });
            commands.push(FileCommand::OpenExample { path: path.to_path_buf(), mode });
        }
        commands
    }

    #[test]
    fn start_and_example_actions_ask_before_replacing_unsaved_work() {
        let path = Path::new("unopened-project.terrain");
        for command in replacement_commands(path) {
            for escape in [false, true] {
                let mut editor = edited_project();
                let serial = editor.document_serial;
                let mut ui_state = UiState::default();
                run(command.clone(), &mut editor, &mut ui_state);
                assert_eq!(ui_state.prompt.as_ref().map(|prompt| prompt.caption.as_str()), Some("Save terrain?"), "{command:?}");
                assert!(editor.files.dialog.is_none() && !editor.files.examples_open);

                // Exercise both Cancel and Escape through the actual prompt.
                let context = egui::Context::default();
                let keys = if escape { vec![egui::Key::Escape] } else { vec![egui::Key::Tab, egui::Key::Tab, egui::Key::Enter] };
                let frames = std::iter::once(None).chain(keys.into_iter().map(Some));
                let mut pending = Vec::new();
                for key in frames {
                    let events = key.map(|key| egui::Event::Key {
                        key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
                    }).into_iter().collect();
                    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
                    let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                    let output = context.run_ui(input, |ui| crate::ui::prompt::show(ui.ctx(), &mut ui_state, &mut pending));
                    output.drop_without_applying_deltas();
                }
                assert!(ui_state.prompt.is_none() && pending.is_empty(), "{command:?}, Escape: {escape}");
                assert_eq!(editor.document_serial, serial);
                assert!(editor.document.dirty && editor.document.can_undo());
                assert_eq!(editor.document.terrain().nodes.len(), 1);
                assert_eq!(editor.preview_resolution, 2048);
                assert!(editor.engine_active && editor.files.dialog.is_none() && !editor.files.examples_open);
            }
        }
    }

    #[test]
    fn confirmed_start_and_example_actions_keep_their_open_modes_and_copy_semantics() {
        let folder = test_folder("open-modes");
        let path = folder.join("Example.terrain");
        let mut target = Document::new();
        target.add_node("Perlin", [0.0, 0.0]).unwrap();
        target.save(&path).unwrap();
        let target_resolution = Editor::new(Document::open(&path).unwrap()).preview_resolution;
        for command in replacement_commands(&path) {
            // The same actions also work at clean startup without a question.
            for dirty in [false, true] {
                let mut editor = edited_project();
                editor.document.dirty = dirty;
                let serial = editor.document_serial;
                let mut ui_state = UiState::default();
                run(command.clone(), &mut editor, &mut ui_state);
                assert_eq!(ui_state.prompt.is_some(), dirty, "{command:?}");
                if dirty {
                    ui_state.prompt = None;
                    run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
                }
                assert!(editor.files.pending.is_none());
                match &command {
                    FileCommand::StartNew => {
                        assert!(editor.document.terrain().nodes.is_empty());
                        assert!(!editor.document.dirty);
                        assert_eq!(editor.document_serial, serial + 1);
                    }
                    FileCommand::StartOpen => {
                        assert_eq!(editor.files.dialog, Some(FileDialog::Open { folder: None }));
                        assert_eq!(editor.document_serial, serial);
                    }
                    FileCommand::OpenExamples => {
                        assert!(editor.files.examples_open);
                        assert_eq!(editor.document_serial, serial);
                    }
                    FileCommand::OpenDirect { mode, .. } | FileCommand::OpenExample { mode, .. } => {
                        assert_eq!(editor.document_serial, serial + 1);
                        assert_eq!(editor.document.terrain().nodes.values().next().unwrap().type_name, "Perlin");
                        assert!(!editor.document.dirty && !editor.document.can_undo());
                        let resolution = if *mode == OpenMode::LowResolution { 512 } else { target_resolution };
                        assert_eq!(editor.preview_resolution, resolution);
                        assert_eq!(editor.engine_active, *mode != OpenMode::SuspendEngine);
                        if matches!(command, FileCommand::OpenExample { .. }) {
                            assert!(editor.document.path.is_none());
                            assert_eq!(editor.document.untitled_name.as_deref(), Some("Example"));
                            assert!(editor.user.recent.files().is_empty());
                        } else {
                            assert_eq!(editor.document.path.as_ref(), Some(&path));
                            assert_eq!(editor.user.recent.files(), &[path.clone()]);
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_example_card_rechecks_edits_made_after_the_browser_opened() {
        let mut editor = Editor::new(Document::new());
        let mut ui_state = UiState::default();
        run(FileCommand::OpenExamples, &mut editor, &mut ui_state);
        assert!(editor.files.examples_open && ui_state.prompt.is_none());
        let mountain = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let serial = editor.document_serial;
        let command = FileCommand::OpenExample { path: "unopened-example.terrain".into(), mode: OpenMode::Normal };
        run(command, &mut editor, &mut ui_state);
        assert_eq!(ui_state.prompt.as_ref().unwrap().caption, "Save terrain?");
        assert!(editor.files.examples_open && editor.document.dirty);
        assert!(editor.document.node(mountain).is_some());
        assert_eq!(editor.document_serial, serial);
    }

    #[test]
    fn saving_before_replacement_writes_the_current_project_first() {
        let folder = test_folder("save-first");
        let saved = folder.join("Current.terrain");
        let example = folder.join("Example.terrain");
        let mut target = Document::new();
        target.save(&example).unwrap();
        let mut editor = edited_project();
        editor.document.path = Some(saved.clone());
        let mut ui_state = UiState::default();
        let command = FileCommand::OpenExample { path: example, mode: OpenMode::LowResolution };
        run(command, &mut editor, &mut ui_state);
        assert!(!saved.exists());
        ui_state.prompt = None;
        run(FileCommand::SaveFirst, &mut editor, &mut ui_state);
        let original = Document::open(&saved).unwrap();
        assert_eq!(original.terrain().nodes.values().next().unwrap().type_name, "Mountain");
        assert!(editor.document.terrain().nodes.is_empty() && editor.document.path.is_none());
        assert_eq!(editor.preview_resolution, 512);
        assert!(editor.files.pending.is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_untitled_save_keeps_the_selected_example_pending_until_saved() {
        let folder = test_folder("save-as");
        let saved = folder.join("Current.terrain");
        let example = folder.join("Example.terrain");
        let mut target = Document::new();
        target.save(&example).unwrap();
        let mut editor = edited_project();
        let serial = editor.document_serial;
        let mut ui_state = UiState::default();
        let command = FileCommand::OpenExample { path: example.clone(), mode: OpenMode::SuspendEngine };
        run(command, &mut editor, &mut ui_state);
        ui_state.prompt = None;
        run(FileCommand::SaveFirst, &mut editor, &mut ui_state);
        assert_eq!(editor.files.dialog.take(), Some(FileDialog::SaveAsThenContinue));
        let action = FileAction::OpenExample { path: example, mode: OpenMode::SuspendEngine };
        assert_eq!(editor.files.pending.as_ref().unwrap().action, action);
        assert!(editor.document.dirty && editor.engine_active);
        assert_eq!(editor.document_serial, serial);
        // The save dialog queues DiscardChanges only after a successful save.
        editor.save_to(&saved);
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert!(saved.is_file() && editor.document.path.is_none());
        assert_eq!(editor.document_serial, serial + 1);
        assert!(!editor.engine_active && editor.files.pending.is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn failed_saves_and_opens_keep_the_current_project_and_open_mode() {
        let folder = test_folder("failures");
        for example in [false, true] {
            for save_first in [false, true] {
                let mut editor = edited_project();
                let serial = editor.document_serial;
                if save_first {
                    editor.document.path = Some(folder.join("missing-parent").join("Current.terrain"));
                }
                let path = folder.join("Missing.terrain");
                let mode = OpenMode::SuspendEngine;
                let command = if example { FileCommand::OpenExample { path, mode } } else { FileCommand::OpenDirect { path, mode } };
                let mut ui_state = UiState::default();
                run(command, &mut editor, &mut ui_state);
                ui_state.prompt = None;
                let answer = if save_first { FileCommand::SaveFirst } else { FileCommand::DiscardChanges };
                run(answer, &mut editor, &mut ui_state);
                assert_eq!(editor.document_serial, serial);
                assert!(editor.document.dirty && editor.document.can_undo());
                assert_eq!(editor.document.terrain().nodes.len(), 1);
                assert!(editor.engine_active && editor.files.pending.is_none());
                assert_eq!(editor.preview_resolution, 2048);
            }
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn canceled_or_displaced_restarts_do_not_arm_a_later_ordinary_exit() {
        use crate::ui::diagnostics_window::take_restart;
        let mut editor = Editor::new(Document::new());
        let mut ui_state = UiState::default();
        editor.document.dirty = true;
        assert!(!editor.document.can_undo());
        let arguments = vec!["--safe-mode".to_string()];
        run(FileCommand::Restart(arguments.clone()), &mut editor, &mut ui_state);
        assert!(ui_state.prompt.is_some(), "restarts ask to save even without an undo history");
        let pending = editor.files.pending.as_ref().unwrap();
        assert_eq!(pending.action, FileAction::Restart(arguments.clone()));
        assert_eq!(take_restart(), None, "an unanswered restart must not be armed");

        // Cancel dismisses the save prompt. An ordinary exit replaces
        // that pending action and must only close, even after No.
        ui_state.prompt = None;
        run(FileCommand::Exit, &mut editor, &mut ui_state);
        ui_state.prompt = None;
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert!(editor.files.exit_now);
        assert_eq!(take_restart(), None);

        editor.files.exit_now = false;
        run(FileCommand::Restart(arguments.clone()), &mut editor, &mut ui_state);
        run(FileCommand::OpenExamples, &mut editor, &mut ui_state);
        ui_state.prompt = None;
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert!(editor.files.examples_open);
        assert_eq!(take_restart(), None, "a displaced restart must not be armed");
        run(FileCommand::Exit, &mut editor, &mut ui_state);
        ui_state.prompt = None;
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert!(editor.files.exit_now);
        assert_eq!(take_restart(), None);

        // Confirming the restart itself transfers its arguments to the
        // exit handler, after the save flow has finished.
        editor.files.exit_now = false;
        run(FileCommand::Restart(arguments.clone()), &mut editor, &mut ui_state);
        ui_state.prompt = None;
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert!(editor.files.exit_now);
        assert_eq!(take_restart(), Some(arguments));
    }

    #[test]
    fn repair_closes_the_project_for_a_new_one_before_its_chooser() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut ui_state = UiState::default();
        // With unsaved changes, No to the save question goes on: the
        // project closes and a new one comes up under the chooser.
        run(FileCommand::RepairFile, &mut editor, &mut ui_state);
        assert!(ui_state.prompt.take().is_some());
        assert_eq!(editor.files.dialog, None);
        assert!(editor.document.terrain().nodes.contains_key(&mountain));
        run(FileCommand::DiscardChanges, &mut editor, &mut ui_state);
        assert_eq!(editor.files.dialog.take(), Some(FileDialog::Repair));
        assert!(!editor.document.terrain().nodes.contains_key(&mountain));
        // Without, straight away.
        let mountain = editor.document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        editor.document.dirty = false;
        run(FileCommand::RepairFile, &mut editor, &mut ui_state);
        assert!(ui_state.prompt.is_none());
        assert_eq!(editor.files.dialog, Some(FileDialog::Repair));
        assert!(!editor.document.terrain().nodes.contains_key(&mountain));
    }
}
