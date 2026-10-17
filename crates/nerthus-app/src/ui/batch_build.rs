//! Project › Execute Batch Build… (`BatchBuildWindow`, "Batch Build
//! Manager"): a script of builds, one `nerthus-build` command line a
//! build, with a build profile, its region, a mutation and the project's
//! variables. Add Profile adds the profile chosen, Add All Profiles one
//! build each (the current settings first, mutations reset), Add Mutation
//! the chosen profile as the next mutation. The script can be edited,
//! saved, and started: it runs on its own while the editor carries on.
//! Esc closes it while it is the window being worked in.
//!
//! The script, its file and the mutation count last until Nerthus closes,
//! whichever project is open.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use egui::{Align, Id, Key, LayerId, Layout, Modifiers, Order, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use nerthus_engine::region;
use nerthus_export::swarm;

use crate::editor::Editor;
use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};
use crate::ui::tool_window;
use crate::ui::widgets::{selectable_value, Tooltips};

/// `noProfile`.
const NO_PROFILE: &str = "Current Settings (No Profile)";

/// The key Save Batch File's dialog answers under.
const SAVE_KEY: &str = "batch-build-save";

/// The window's size when it first opens, title bar and frame included.
const DEFAULT_SIZE: Vec2 = Vec2::new(750.0, 450.0);
/// The smallest it can be made: room for the rows of buttons and a few
/// lines of the script.
const MIN_SIZE: Vec2 = Vec2::new(480.0, 280.0);
/// The Build Profile label's height.
const LABEL_HEIGHT: f32 = 16.0;
/// The height of the profile list and of each row of buttons.
const ROW_HEIGHT: f32 = 20.0;

/// The script's editor, which Esc leaves before it closes the window.
fn script_id() -> Id {
    Id::new("batch-script-text")
}

/// What lasts between openings of the window.
#[derive(Default)]
pub struct BatchSession {
    /// `Seed`: the last mutation added.
    seed: i32,
    /// `BatchFile`: where the script was saved, if it was.
    file: Option<PathBuf>,
    /// `BatchFileContents`: the script as last kept.
    contents: String,
    /// A batch running: its end, and its file.
    running: Option<Receiver<Result<(), String>>>,
}

/// The window while it is open.
pub struct BatchWindow {
    /// `cboProfiles`: 0 for the current settings, else the profile's
    /// index from 1.
    profile: usize,
    /// The script as shown (`batch`).
    text: String,
    /// Whether it is the window being worked in, the one Esc closes: from
    /// when it opens until the shell or another window is pressed, and
    /// again once it is pressed.
    active: bool,
}

/// The first line of a script (`REM Nerthus Batch Build Version …`, with
/// `#` for `REM` outside Windows).
fn script_header() -> String {
    let comment = if cfg!(windows) { "REM" } else { "#" };
    format!("{comment} Nerthus Batch Build Version {}", env!("CARGO_PKG_VERSION"))
}

/// `nerthus-build` beside the editor (where cargo builds both).
fn build_program() -> PathBuf {
    let name = if cfg!(windows) { "nerthus-build.exe" } else { "nerthus-build" };
    let current = std::env::current_exe().unwrap_or_default();
    current.with_file_name(name)
}

/// In quotes.
fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

/// In quotes when it holds a letter.
fn quoted_if_text(text: &str) -> String {
    if text.chars().any(char::is_alphabetic) { quoted(text) } else { text.to_string() }
}

impl BatchSession {
    /// Whether a batch is running.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// `ReloadBatchQueue`: the script, or its first line when there is
    /// none.
    fn reload(&mut self) -> String {
        if self.contents.is_empty() {
            self.contents = script_header();
        }
        self.contents.clone()
    }

    /// Takes in a running batch's end.
    pub fn poll(&mut self, editor: &mut Editor) {
        let Some(running) = &self.running else {
            return;
        };
        let Ok(result) = running.try_recv() else {
            return;
        };
        self.running = None;
        match result {
            Ok(()) => editor.report("Batch Build completed."),
            Err(problem) => editor.report(format!("Batch Build failed: {problem}")),
        }
    }
}

impl BatchWindow {
    /// `BatchBuildWindow_Loaded`: the current settings chosen, the script
    /// shown.
    pub fn open(session: &mut BatchSession) -> BatchWindow {
        BatchWindow { profile: 0, text: session.reload(), active: true }
    }

    /// The profile chosen, by name (`None` for the current settings).
    fn chosen_profile(&self, names: &[String]) -> Option<String> {
        if self.profile == 0 {
            return None;
        }
        names.get(self.profile - 1).cloned()
    }

    /// `AddProfileToQueue`: the script as typed kept, and a build of the
    /// profile added.
    fn add(&mut self, session: &mut BatchSession, editor: &Editor, profile: Option<&str>) {
        session.contents = self.text.clone();
        let file = editor.document.path.as_ref().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
        let mut line = format!("\n{} build {}", quoted(&build_program().to_string_lossy()), quoted(&file));
        let definition = profile.and_then(|name| editor.document.build_profile(name));
        if let (Some(name), Some(_)) = (profile, &definition) {
            line.push_str(&format!(" -p {}", quoted(name)));
        }
        let profile_json = profile.and_then(|name| swarm::profile_json(&editor.document.project.assets[0], name));
        if let Some(portion) = profile_json.and_then(region::portion) {
            line.push_str(&format!(" -r {}", quoted(&portion.name)));
        }
        if session.seed > 0 {
            line.push_str(&format!(" --seed {}", session.seed));
        }
        // Every variable with its value.
        let mut variables = String::new();
        for (name, variable) in &editor.document.project.assets[0].automation.variables {
            variables.push_str(&format!("-v {name}={} ", quoted_if_text(&variable.value.to_text())));
        }
        let variables = format!(" {variables}");
        line.push_str(variables.trim_end());
        session.contents.push_str(&line);
        self.text = session.contents.clone();
    }

    fn clear(&mut self, session: &mut BatchSession) {
        session.contents.clear();
        self.text = session.reload();
    }
}

/// Starts the script (`BtnStartBatchBuild_OnClick`): written to its file,
/// or a new temporary one, and run.
fn start(session: &mut BatchSession, text: &str, editor: &mut Editor) {
    editor.report("Batch Build started.");
    let extension = if cfg!(windows) { "bat" } else { "sh" };
    let file = match &session.file {
        Some(file) => file.clone(),
        None => {
            let name = format!("{}.{extension}", uuid_name());
            let file = std::env::temp_dir().join(name);
            session.file = Some(file.clone());
            file
        }
    };
    if let Err(error) = std::fs::write(&file, text) {
        editor.report(format!("Batch Build failed: cannot write {}: {error}", file.display()));
        return;
    }
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("batch build".to_string()).spawn(move || {
        let _ = sender.send(run_script(&file));
    });
    match spawned {
        Ok(_) => session.running = Some(receiver),
        Err(error) => editor.report(format!("Batch Build failed: {error}")),
    }
}

/// A name no other temporary script has.
fn uuid_name() -> String {
    let nanoseconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
    format!("nerthus-batch-{}-{nanoseconds:x}", std::process::id())
}

/// Runs a script to its end.
fn run_script(file: &Path) -> Result<(), String> {
    let mut command = if cfg!(windows) { std::process::Command::new("cmd") } else { std::process::Command::new("sh") };
    if cfg!(windows) {
        command.arg("/C");
    }
    command.arg(file);
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("the script ended with {status}")) }
}

/// Esc closes the window while it is the one being worked in: taken
/// before the shell's keys, so that it does not stop a build as well. Esc
/// in the script only leaves it, as in any text field, and a list dropped
/// down takes it instead.
pub fn take_escape(ctx: &egui::Context, window: &mut Option<BatchWindow>) {
    if !window.as_ref().is_some_and(|shown| shown.active) || egui::Popup::is_any_open(ctx) {
        return;
    }
    if tool_window::on_top(ctx) != Some(Id::new("batch-build")) {
        return;
    }
    // egui has already taken the keyboard from the script for this Esc.
    let typing = ctx.memory(|memory| memory.had_focus_last_frame(script_id()));
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) && !typing {
        *window = None;
    }
}

/// A press in the window makes it the one being worked in, and a press in
/// the shell or another window stops it being; a list or menu dropped
/// down changes nothing.
fn note_press(ctx: &egui::Context, window: &mut BatchWindow, layer: LayerId) {
    let pressed = ctx.input(|input| if input.pointer.any_pressed() { input.pointer.press_origin() } else { None });
    let Some(position) = pressed else {
        return;
    };
    // The shell draws under every window, so a press on no window is on it.
    let under = ctx.layer_id_at(position);
    if under.is_none_or(|under| matches!(under.order, Order::Background | Order::Middle)) {
        window.active = under == Some(layer);
    }
}

/// The window. Returns false once closed.
pub fn show(ctx: &egui::Context, editor: &mut Editor, window: &mut BatchWindow, session: &mut BatchSession, dialogs: &mut FileDialogs) -> bool {
    if let Some(answer) = dialogs.take(SAVE_KEY)
        && let Some(path) = answer
    {
        match std::fs::write(&path, &window.text) {
            Ok(()) => {
                session.file = Some(path);
                editor.report("Batch Build file saved.");
            }
            Err(error) => editor.report(format!("Cannot save {}: {error}", path.display())),
        }
    }
    let names = editor.document.build_profiles();
    if window.profile > names.len() {
        window.profile = 0;
    }
    let mut open = true;
    let mut close = false;
    let mut frame = egui::Window::new("Batch Build Manager");
    frame = frame.collapsible(false);
    frame = frame.resizable(true);
    // Before `place`, which can fix the size for a frame.
    frame = frame.min_size(MIN_SIZE);
    frame = tool_window::place(ctx, frame, Id::new("batch-build"), DEFAULT_SIZE);
    frame = frame.open(&mut open);
    let shown = frame.show(ctx, |ui| {
        // The room inside, split between the parts whatever they hold: the
        // label and the rows keep their heights and the script gets the
        // rest, so that nothing makes the window larger than it is.
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let row = |top: f32, height: f32| Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), height));
        let label = row(rect.top() + 8.0, LABEL_HEIGHT);
        let profiles = row(label.bottom() + 4.0, ROW_HEIGHT);
        let adds = row(profiles.bottom() + 8.0, ROW_HEIGHT);
        let script_top = adds.bottom() + 16.0;
        let footer = row((rect.bottom() - ROW_HEIGHT).max(script_top + 8.0), ROW_HEIGHT);
        let script_rect = Rect::from_min_max(Pos2::new(rect.left(), script_top), Pos2::new(rect.right(), footer.top() - 8.0));

        let mut label_ui = tool_window::part(ui, "batch-label", label, Layout::left_to_right(Align::Center));
        label_ui.label(RichText::new("Build Profile").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        let mut profiles_ui = tool_window::part(ui, "batch-profiles", profiles, Layout::left_to_right(Align::Center));
        let current = if window.profile == 0 { NO_PROFILE.to_string() } else { names[window.profile - 1].clone() };
        let mut combo = egui::ComboBox::from_id_salt("batch-profile");
        combo = combo.selected_text(current);
        combo = combo.width(profiles.width());
        combo.show_ui(&mut profiles_ui, |ui| {
            selectable_value(ui, &mut window.profile, 0, NO_PROFILE);
            for (index, name) in names.iter().enumerate() {
                selectable_value(ui, &mut window.profile, index + 1, name.as_str());
            }
        });
        tool_window::part(ui, "batch-adds", adds, Layout::right_to_left(Align::Center)).scope(|ui| {
            let can_clear = session.contents != script_header();
            if ui.add_enabled(can_clear, egui::Button::new("Clear Batch Build")).tooltip("Clears the below Batch Build list.").clicked() {
                window.clear(session);
            }
            if ui.button("Add All Profiles").tooltip("Adds all profiles to batch build.").clicked() {
                // `BtnAddAllProfiles_OnClick`: a fresh script, mutations
                // from the start, the current settings then every profile.
                window.clear(session);
                session.seed = 0;
                window.add(session, editor, None);
                for name in &names {
                    window.add(session, editor, Some(name));
                }
            }
            if ui.button("Add Profile").tooltip("Adds the selected profile to batch build.").clicked() {
                let profile = window.chosen_profile(&names);
                window.add(session, editor, profile.as_deref());
            }
            if ui.button("Add Mutation").tooltip("Adds a mutation to the batch.").clicked() {
                session.seed += 1;
                let profile = window.chosen_profile(&names);
                window.add(session, editor, profile.as_deref());
            }
        });
        let mut script = egui::TextEdit::multiline(&mut window.text);
        script = script.id(script_id());
        script = script.font(theme::monospace(12.0));
        script = script.code_editor();
        script = script.desired_width(f32::INFINITY);
        let mut script_ui = tool_window::part(ui, "batch-script", script_rect, Layout::top_down(Align::Min));
        egui::ScrollArea::both().id_salt("batch-script").auto_shrink([false, false]).show(&mut script_ui, |ui| {
            ui.add_sized(ui.available_size(), script);
        });
        tool_window::part(ui, "batch-footer", footer, Layout::right_to_left(Align::Center)).scope(|ui| {
            if ui.button("Close").clicked() {
                close = true;
            }
            let label = RichText::new("Start Batch Build").color(theme::LABEL_EMPHASIS);
            let mut start_button = egui::Button::new(label);
            start_button = start_button.stroke(Stroke::new(1.0, theme::BUTTON_IMPORTANT_BORDER));
            if ui.add_enabled(!session.is_running(), start_button).clicked() {
                start(session, &window.text.clone(), editor);
            }
            ui.add_space(28.0);
            if ui.add_enabled(!dialogs.pending(SAVE_KEY), egui::Button::new("Save Batch File")).clicked() {
                let (filter, extension) = if cfg!(windows) { ("Batch File (*.bat)", "bat") } else { ("Shell Script (*.sh)", "sh") };
                // `BtnSaveBatchFile_OnClick` starts where the last save went.
                let directory = editor.user.locations.folder(crate::user::locations::SAVE);
                let request = DialogRequest {
                    key: SAVE_KEY.to_string(),
                    title: "Save Batch File".to_string(),
                    kind: DialogKind::SaveFile { filter: filter.to_string(), extensions: vec![extension.to_string()], file_name: String::new() },
                    directory,
                };
                dialogs.ask(request);
            }
        });
    });
    if let Some(shown) = shown {
        note_press(ctx, window, shown.response.layer_id);
    }
    !(close || !open)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::{AreaState, Event, PointerButton, RawInput};

    /// The screen, in points, of 1600 × 960 at the default 120%.
    const LARGE: Vec2 = Vec2::new(1333.0, 800.0);
    /// 800 × 600 and 640 × 480 at 120%.
    const SMALL_SCREENS: [Vec2; 2] = [Vec2::new(666.0, 500.0), Vec2::new(533.0, 400.0)];

    /// What the shell keeps for the window.
    struct Shell {
        context: egui::Context,
        editor: Editor,
        session: BatchSession,
        window: Option<BatchWindow>,
        dialogs: FileDialogs,
    }

    impl Shell {
        /// Project › Execute Batch Build… chosen.
        fn open() -> Shell {
            let context = egui::Context::default();
            theme::apply(&context);
            let mut session = BatchSession::default();
            let window = Some(BatchWindow::open(&mut session));
            Shell { context, editor: Editor::new(Document::new()), session, window, dialogs: FileDialogs::default() }
        }

        /// A frame at this screen size with these events, Esc taken before
        /// the window as the shell does. Returns the window's rect, and
        /// whether the shell saw an Esc.
        fn frame_seen(&mut self, screen: Vec2, events: Vec<Event>) -> (Rect, bool) {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
            let context = self.context.clone();
            let mut seen = false;
            let output = context.run_ui(input, |ui| {
                let ctx = ui.ctx().clone();
                take_escape(&ctx, &mut self.window);
                seen = ctx.input(|input| input.key_pressed(Key::Escape));
                if let Some(window) = self.window.as_mut()
                    && !show(&ctx, &mut self.editor, window, &mut self.session, &mut self.dialogs)
                {
                    self.window = None;
                }
            });
            output.drop_without_applying_deltas();
            let area = AreaState::load(&self.context, Id::new("batch-build")).filter(|area| area.size.is_some());
            (area.map(|area| area.rect()).unwrap_or(Rect::NOTHING), seen)
        }

        fn frame(&mut self, screen: Vec2, events: Vec<Event>) -> Rect {
            self.frame_seen(screen, events).0
        }

        /// Frames enough for egui's first-frame sizing to settle.
        fn settle(&mut self, screen: Vec2) -> Rect {
            for _ in 0..4 {
                self.frame(screen, Vec::new());
            }
            self.frame(screen, Vec::new())
        }

        /// A click at `position`.
        fn click(&mut self, screen: Vec2, position: Pos2) {
            let button = |pressed| Event::PointerButton { pos: position, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            self.frame(screen, vec![Event::PointerMoved(position)]);
            self.frame(screen, vec![button(true)]);
            self.frame(screen, vec![button(false)]);
        }

        /// Whether the script has the keyboard.
        fn typing(&self) -> bool {
            self.context.memory(|memory| memory.has_focus(script_id()))
        }
    }

    fn escape() -> Event {
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    fn close(a: Rect, b: Rect) -> bool {
        (a.min - b.min).abs().max_elem() <= 1.0 && (a.max - b.max).abs().max_elem() <= 1.0
    }

    #[test]
    fn the_window_opens_at_its_size_and_keeps_it() {
        let mut shell = Shell::open();
        let opened = shell.settle(LARGE);
        let body = tool_window::body(&shell.context);
        assert!((opened.size() - DEFAULT_SIZE).abs().max_elem() <= 1.0, "{opened:?} is not {DEFAULT_SIZE:?}");
        assert!((opened.center() - body.center()).length() <= 1.0, "{opened:?} is not centred in {body:?}");
        // It grew every frame, off the top and bottom within seconds.
        for _ in 0..240 {
            shell.frame(LARGE, Vec::new());
        }
        let later = shell.frame(LARGE, Vec::new());
        assert!(close(later, opened), "{later:?} grew from {opened:?}");
    }

    #[test]
    fn the_window_stays_in_small_bodies() {
        for screen in SMALL_SCREENS {
            let mut shell = Shell::open();
            let opened = shell.settle(screen);
            for _ in 0..240 {
                shell.frame(screen, Vec::new());
            }
            let later = shell.frame(screen, Vec::new());
            let body = tool_window::body(&shell.context);
            assert!(body.expand(0.5).contains_rect(later), "at {screen:?}: {later:?} is outside {body:?}");
            assert!(close(later, opened), "at {screen:?}: {later:?} grew from {opened:?}");
        }
    }

    #[test]
    fn close_is_in_reach() {
        // Close is the footer's last button, in the window's bottom right
        // corner past the frame (7).
        let at = |rect: Rect| Pos2::new(rect.right() - 7.0 - 12.0, rect.bottom() - 7.0 - ROW_HEIGHT / 2.0);
        for screen in [LARGE, SMALL_SCREENS[0], SMALL_SCREENS[1]] {
            let mut shell = Shell::open();
            let rect = shell.settle(screen);
            shell.click(screen, at(rect));
            assert!(shell.window.is_none(), "Close at {screen:?} did not close the window");
        }
    }

    #[test]
    fn escape_closes_the_window_and_not_the_build() {
        let mut shell = Shell::open();
        shell.settle(LARGE);
        let (_, seen) = shell.frame_seen(LARGE, vec![escape()]);
        assert!(shell.window.is_none() && !seen);
    }

    #[test]
    fn escape_in_the_script_only_leaves_it() {
        let mut shell = Shell::open();
        let rect = shell.settle(LARGE);
        shell.click(LARGE, rect.center());
        assert!(shell.typing());
        shell.frame(LARGE, vec![escape()]);
        assert!(shell.window.is_some() && !shell.typing());
        shell.frame(LARGE, vec![escape()]);
        assert!(shell.window.is_none());
    }

    #[test]
    fn escape_is_the_shells_once_it_is_pressed() {
        let mut shell = Shell::open();
        let rect = shell.settle(LARGE);
        let body = tool_window::body(&shell.context);
        shell.click(LARGE, body.left_top() + Vec2::splat(20.0));
        let (_, seen) = shell.frame_seen(LARGE, vec![escape()]);
        assert!(shell.window.is_some() && seen);
        // Pressed again, by its title bar.
        shell.click(LARGE, rect.center_top() + Vec2::new(0.0, 12.0));
        shell.frame(LARGE, vec![escape()]);
        assert!(shell.window.is_none());
    }
}
