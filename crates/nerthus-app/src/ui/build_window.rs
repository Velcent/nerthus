//! Build and Export: the confirmation (`B.VerifyAndBuild`'s task dialog),
//! the window that follows a build (the build's console), and Save cached
//! build's progress window.

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Ui, Vec2};
use nerthus_engine::StateOverride;
use nerthus_export::build::BuildType;

use crate::editor::Editor;
use crate::export_build::{ExportKind, LogLine};
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::memory_dialogs;
use crate::ui::tool_window;
use crate::ui::widgets;
use crate::ui::UiState;

/// The progress window's size when it first opens.
const DEFAULT_SIZE: Vec2 = Vec2::new(560.0, 380.0);
/// The smallest it can be made: room for the status, the footer and a few
/// lines of the log.
const MIN_SIZE: Vec2 = Vec2::new(360.0, 200.0);
/// The height of the footer under the log.
const FOOTER_HEIGHT: f32 = 20.0;

/// The confirmation's state while it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildConfirm {
    /// "Open folder after build finishes".
    pub open_folder: bool,
}

/// A command link (`TaskDialogButtonStyle.CommandLinks`): an arrow, the
/// title and a note under it.
fn command_link(ui: &mut Ui, title: &str, note: &str) -> bool {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 46.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::MENU_BACKGROUND_HOVER);
    }
    let arrow_color = if response.hovered() { theme::ACCENT_BLUE } else { theme::ACCENT_MUTED_BLUE };
    let arrow = egui::Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::splat(16.0));
    icons::paint(ui, arrow, "chevron-right", arrow_color);
    let title_position = rect.min + Vec2::new(32.0, 6.0);
    let title_galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(13.0), theme::LABEL_EMPHASIS);
    ui.painter().galley(title_position, title_galley, theme::LABEL_EMPHASIS);
    let note_position = rect.min + Vec2::new(32.0, 26.0);
    let note_galley = ui.painter().layout_no_wrap(note.to_string(), theme::interface(11.0), theme::LABEL_DEFAULT);
    ui.painter().galley(note_position, note_galley, theme::LABEL_DEFAULT);
    response.clicked()
}

/// "Build and Export?": what will be built, a link to the build settings,
/// Start Build, Close Nerthus and Build, or Cancel. A choice that waited
/// for Save As goes on from here once the project is saved.
pub fn confirm(ctx: &egui::Context, editor: &Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    if let Some(choice) = editor.resume_build {
        commands.push(Command::StartBuild { open_folder: choice.open_folder, close: choice.close });
    }
    let Some(confirm) = ui_state.build_confirm.as_mut() else {
        return;
    };
    let definition = editor.document.build_definition();
    let terrain = editor.document.terrain();
    // `Total nodes to build`: the nodes neither baked nor bypassed.
    let unlocked = terrain.nodes.values().filter(|node| StateOverride::of(node) == StateOverride::None).count();
    let mut start = false;
    let mut close = false;
    let mut cancel = false;
    let modal = widgets::dialog(ctx, egui::Id::new("build-and-export"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(460.0);
        let window_title = RichText::new("Build and Export?");
        ui.label(window_title.font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        ui.add_space(6.0);
        // A Region build builds the region at its size.
        let region = definition.portion.as_ref().filter(|_| definition.build_type == BuildType::Region);
        let what = if region.is_some() { "Region" } else { "Terrain" };
        let size = region.map(|region| region.size.max(0) as usize).unwrap_or(definition.resolution);
        let instruction = RichText::new(format!("Build {what} at {size} x {size}?"));
        ui.label(instruction.font(theme::semibold(16.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(8.0);
        ui.label(format!("Would you like to build this {what}? The build will export any files marked for save or output nodes."));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label("Adjust your ");
            if ui.link("build settings").clicked() {
                commands.push(Command::BuildSettings);
            }
            ui.label(", export file formats, and other details before proceeding. You can also access them in the Project menu.");
        });
        ui.add_space(6.0);
        ui.label(format!("Total nodes to build: {unlocked:02}"));
        ui.add_space(10.0);
        if command_link(ui, "Start Build", "Build the saved project and export its marked nodes") {
            start = true;
        }
        if command_link(ui, "Close Nerthus and Build", "Save RAM and GPU memory by closing the Nerthus interface") {
            close = true;
        }
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut confirm.open_folder, "Open folder after build finishes");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        // The footer, with `TaskDialogIcon.Warning`.
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::splat(14.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            icons::paint(ui, rect, "alert-triangle", theme::ACCENT_YELLOW);
            let footer = RichText::new("Close the Nerthus interface during a build to conserve resources and speed up your builds.");
            ui.label(footer.font(theme::interface(11.0)).color(theme::LABEL_DEFAULT));
        });
    });
    if start || close {
        let open_folder = confirm.open_folder;
        commands.push(Command::StartBuild { open_folder, close });
        ui_state.build_confirm = None;
        return;
    }
    if cancel || response.should_close() {
        ui_state.build_confirm = None;
    }
}

/// The build's progress: nodes built, files written, problems; Cancel
/// while it runs, Open Folder and Close after. Save cached build has a
/// progress window instead, which goes once the export is over; if it
/// failed, a normal build is offered.
pub fn progress(ctx: &egui::Context, editor: &Editor, commands: &mut Vec<Command>) {
    let Some(build) = editor.export_build.as_ref() else {
        return;
    };
    if let ExportKind::Cached { close } = build.kind {
        if !build.finished {
            let text = build.exporting_text();
            memory_dialogs::progress_window(ctx, "cached-export", "Exporting Cached Build", &text, build.progress(), false);
        } else if build.succeeded() {
            commands.push(Command::CloseBuildWindow);
        } else {
            commands.push(Command::CachedExportFailed { close });
        }
        return;
    }
    let title = format!("Building {} at {} x {}", build.project_name, build.resolution, build.resolution);
    let mut open = true;
    let mut window = egui::Window::new(title);
    window = window.collapsible(false);
    window = window.resizable(true);
    // Before `place`, which can fix the size for a frame.
    window = window.min_size(MIN_SIZE);
    window = tool_window::place(ctx, window, egui::Id::new("build-progress"), DEFAULT_SIZE);
    window = window.open(&mut open);
    window.show(ctx, |ui| {
            let status = if let Some(refusal) = &build.refusal {
                RichText::new(refusal.as_str()).color(theme::ACCENT_RED)
            } else if build.cancelled {
                RichText::new("Build cancelled").color(theme::ACCENT_YELLOW)
            } else if build.finished {
                let summary = format!("Built in {:.1} s: {} files written, {} problems", build.seconds(), build.files, build.failures);
                let color = if build.failures == 0 { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_YELLOW };
                RichText::new(summary).color(color)
            } else if let (None, Some(stage)) = (&build.current, &build.stage) {
                RichText::new(stage.as_str()).color(theme::LABEL_EMPHASIS)
            } else {
                let current = build.current.clone().unwrap_or_else(|| "Starting".to_string());
                let position = (build.built + 1).min(build.total.max(1));
                let bucket = build.bucket.map(|(index, count)| format!("Bucket {index} of {count}, ")).unwrap_or_default();
                RichText::new(format!("{bucket}Node {position} of {}: {current}", build.total)).color(theme::LABEL_EMPHASIS)
            };
            ui.label(status);
            if let Some(folder) = &build.folder {
                let location = RichText::new(folder.display().to_string());
                ui.label(location.font(theme::monospace(11.0)).color(theme::TEXTBOX_FILE_FOREGROUND));
            }
            ui.add_space(4.0);
            progress_bar(ui, build.progress());
            ui.add_space(6.0);

            // The room left, split between the log and the footer under
            // it: the footer keeps its height and the log gets the rest, so
            // that making the window shorter shrinks the log rather than
            // cutting off the footer.
            let (rest, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
            let footer = rest.with_min_y((rest.bottom() - FOOTER_HEIGHT).max(rest.top()));
            // Between them a gap, with the separator's line along its middle.
            let gap = 8.0;
            let log_rect = rest.with_max_y((footer.top() - gap).max(rest.top()));
            let mut log_ui = tool_window::part(ui, "build-log", log_rect, Layout::top_down(Align::Min));
            widgets::scroll_bar_gutter(&mut log_ui);
            let mut log = ScrollArea::vertical();
            log = log.stick_to_bottom(true);
            log = log.auto_shrink([false, false]);
            log.show(&mut log_ui, |ui| {
                for line in &build.log {
                    log_line(ui, line);
                }
            });

            let separator = ui.visuals().widgets.noninteractive.bg_stroke;
            ui.painter().hline(rest.x_range(), footer.top() - gap / 2.0, separator);
            tool_window::part(ui, "build-footer", footer, Layout::right_to_left(Align::Center)).scope(|ui| {
                if build.finished {
                    if ui.button("Close").clicked() {
                        commands.push(Command::CloseBuildWindow);
                    }
                    let can_open = build.folder.as_ref().is_some_and(|folder| folder.is_dir());
                    if ui.add_enabled(can_open, egui::Button::new("Open Folder")).clicked()
                        && let Some(folder) = &build.folder
                    {
                        commands.push(Command::OpenFolder(folder.clone()));
                    }
                } else if ui.button("Cancel").clicked() {
                    commands.push(Command::CancelBuild);
                }
                let elapsed = RichText::new(format!("{:.0} s", build.seconds()));
                ui.label(elapsed.font(theme::monospace(11.0)).color(theme::LABEL_DEFAULT));
            });
            if !build.finished {
                ui.ctx().request_repaint();
            }
        });
    if !open {
        if build.finished {
            commands.push(Command::CloseBuildWindow);
        } else {
            commands.push(Command::CancelBuild);
        }
    }
}

fn progress_bar(ui: &mut Ui, fraction: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 4.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PROGRESS_EMPTY);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(filled, 0.0, theme::ACCENT_PRIMARY);
}

fn log_line(ui: &mut Ui, line: &LogLine) {
    let (text, color): (String, Color32) = match line {
        LogLine::Built { node, milliseconds } => (format!("{node}  {milliseconds} ms"), theme::LABEL_DEFAULT),
        LogLine::Failed { node, error } => (format!("{node} failed: {error}"), theme::ACCENT_RED),
        LogLine::Wrote(path) => {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            (format!("  wrote {name}"), theme::ACCENT_MUTED_GREEN)
        }
        LogLine::Problem(problem) => (problem.clone(), theme::ACCENT_YELLOW),
        LogLine::Step(step) => (step.clone(), theme::ACCENT_MUTED_BLUE),
    };
    let line_text = RichText::new(text);
    ui.label(line_text.font(theme::monospace(11.0)).color(color));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::editor::NodeStatus;
    use crate::export_build::{BuildChoice, ExportBuild};
    use crate::ui::commands;
    use crate::worker::{CachedPort, NodeCache};
    use bevy::prelude::Window;
    use nerthus_engine::{NodeState, RenderIntent};

    fn temporary_project(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_nanos()).unwrap_or(0);
        std::env::temp_dir().join(format!("nerthus-{name}-{nanos}.terrain"))
    }

    /// A saved project whose Perlin, marked for export, the editor holds
    /// at the build's 1024.
    fn holding_editor(path: &std::path::Path) -> Editor {
        let mut document = Document::new();
        let perlin = document.add_node("Perlin", [26000.0, 26000.0]).unwrap();
        document.toggle_export(&[perlin], nerthus_export::FileFormat::Png16);
        document.set_build_setting("Resolution", serde_json::json!(1024));
        let mut editor = Editor::new(document);
        editor.save_to(path);
        editor.preview_resolution = 1024;
        let status = NodeStatus { state: NodeState::Built, ..NodeStatus::default() };
        editor.statuses.insert(perlin, status);
        let port = CachedPort { name: "Out".to_string(), width: 1024, height: 1024, channels: 1, intent: RenderIntent::Default };
        let cache = NodeCache { ports: vec![port], ..NodeCache::default() };
        editor.node_caches.insert(perlin, cache);
        editor
    }

    #[test]
    fn a_build_the_editor_holds_is_offered_as_a_cached_export() {
        let path = temporary_project("cached-question");
        let mut editor = holding_editor(&path);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        let chosen = vec![Command::StartBuild { open_folder: false, close: true }];
        commands::run(chosen, &mut editor, &mut ui_state, &mut window);
        let question = ui_state.prompt.take().expect("Save cached build? is asked");
        assert_eq!(question.caption, "Save cached build?");
        assert_eq!(question.yes, Some(Command::ExportCachedBuild { close: true }));
        assert_eq!(question.no, Some(Command::RunBuild { close: true }));
        assert!(question.footnote.is_some());

        // Yes: the export goes to the build thread with the next request.
        commands::run(vec![Command::ExportCachedBuild { close: true }], &mut editor, &mut ui_state, &mut window);
        let kind = editor.export_build.as_ref().map(|build| build.kind);
        assert_eq!(kind, Some(ExportKind::Cached { close: true }));
        assert!(editor.needs_build);

        // A failed export offers the normal build of the same choice.
        commands::run(vec![Command::CachedExportFailed { close: true }], &mut editor, &mut ui_state, &mut window);
        assert!(editor.export_build.is_none());
        let retry = ui_state.prompt.take().expect("a normal build is offered");
        assert_eq!(retry.caption, "Try a normal build?");
        assert_eq!(retry.yes, Some(Command::RunBuild { close: true }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_build_the_checks_refuse_never_starts() {
        let path = temporary_project("refused-build");
        let mut editor = Editor::new(Document::new());
        editor.save_to(&path);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        let chosen = vec![Command::StartBuild { open_folder: false, close: true }];
        commands::run(chosen, &mut editor, &mut ui_state, &mut window);
        let refusal = ui_state.prompt.take().expect("the refusal shows");
        assert_eq!(refusal.caption, "Validation failed");
        assert_eq!(refusal.text, "There are no nodes marked for export");
        assert!(editor.export_build.is_none());
        assert!(editor.files.requests.is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_choice_waiting_for_save_as_goes_on_once() {
        let path = temporary_project("resumed-build");
        let mut editor = holding_editor(&path);
        let mut ui_state = UiState::default();
        let mut window = Window::default();
        editor.resume_build = Some(BuildChoice { open_folder: false, close: false });
        let mut pending = Vec::new();
        confirm(&egui::Context::default(), &editor, &mut ui_state, &mut pending);
        assert_eq!(pending, vec![Command::StartBuild { open_folder: false, close: false }]);
        commands::run(pending, &mut editor, &mut ui_state, &mut window);
        assert_eq!(editor.resume_build, None);
        assert_eq!(ui_state.prompt.as_ref().map(|prompt| prompt.caption.as_str()), Some("Save cached build?"));
        std::fs::remove_file(&path).ok();
    }

    /// A frame of the progress window with these events. Returns what was
    /// painted and the window's rect.
    fn progress_frame(context: &egui::Context, editor: &Editor, events: Vec<egui::Event>) -> (Vec<egui::epaint::ClippedShape>, egui::Rect) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 960.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            progress(ui.ctx(), editor, &mut Vec::new());
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let area = egui::AreaState::load(context, egui::Id::new("build-progress")).filter(|area| area.size.is_some());
        (shapes, area.map(|area| area.rect()).unwrap_or(egui::Rect::NOTHING))
    }

    #[test]
    fn log_lines_end_before_the_scroll_bar() {
        let mut editor = Editor::new(Document::new());
        let (_sender, messages) = crossbeam_channel::unbounded();
        let mut build = ExportBuild::following("Gutter".to_string(), 1024, false, messages);
        build.finished = true;
        // Enough lines to scroll, the last for a node renamed to a long
        // name without spaces.
        build.log = (0..100).map(|index| LogLine::Step(format!("Step {index}"))).collect();
        build.log.push(LogLine::Wrote(format!("{}_Out.png", "ABCD".repeat(30)).into()));
        editor.export_build = Some(build);
        let context = egui::Context::default();
        theme::apply(&context);
        // Once it has faded in.
        let mut shapes = Vec::new();
        for _ in 0..30 {
            shapes = progress_frame(&context, &editor, Vec::new()).0;
        }
        // The log is as wide as the progress bar above it.
        let progress = shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill == theme::PROGRESS_EMPTY => Some(rect.rect),
            _ => None,
        });
        let progress = progress.expect("the progress bar is painted");
        // The pointer on the log's scroll bar, which grows to its widest.
        let on_bar = egui::pos2(progress.right() - 2.0, progress.bottom() + 80.0);
        progress_frame(&context, &editor, vec![egui::Event::PointerMoved(on_bar)]);
        for _ in 0..20 {
            progress_frame(&context, &editor, Vec::new());
        }
        let shapes = progress_frame(&context, &editor, Vec::new()).0;
        let bar_left = progress.right() - context.global_style().spacing.scroll.bar_width;
        let lines: Vec<(&str, egui::Rect)> = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("Step ") || text.galley.text().contains("wrote") => Some((text.galley.text(), text.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        assert!(lines.iter().any(|(text, _)| text.contains("ABCD_Out.png")), "the long line is not painted");
        for (text, rect) in lines {
            assert!(rect.right() <= bar_left, "{text:?} ends at {}, under the scroll bar from {bar_left}", rect.right());
        }
    }

    #[test]
    fn a_shorter_progress_window_keeps_its_footer() {
        let mut editor = Editor::new(Document::new());
        let (_sender, messages) = crossbeam_channel::unbounded();
        let mut build = ExportBuild::following("Footer".to_string(), 1024, false, messages);
        build.finished = true;
        build.log = (0..100).map(|index| LogLine::Step(format!("Step {index}"))).collect();
        editor.export_build = Some(build);
        let context = egui::Context::default();
        let mut opened = egui::Rect::NOTHING;
        for _ in 0..5 {
            opened = progress_frame(&context, &editor, Vec::new()).1;
        }
        // The bottom right grip, dragged up past the smallest it can be.
        let grip = opened.max - Vec2::splat(2.0);
        let to = egui::pos2(grip.x, opened.top() + 60.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        progress_frame(&context, &editor, vec![egui::Event::PointerMoved(grip)]);
        progress_frame(&context, &editor, vec![button(grip, true)]);
        for step in 1..=8 {
            let pos = grip + (to - grip) * (step as f32 / 8.0);
            progress_frame(&context, &editor, vec![egui::Event::PointerMoved(pos)]);
        }
        progress_frame(&context, &editor, vec![button(to, false)]);
        for _ in 0..4 {
            progress_frame(&context, &editor, Vec::new());
        }
        let (shapes, shorter) = progress_frame(&context, &editor, Vec::new());
        assert!(shorter.height() < opened.height() - 100.0, "{shorter:?} was not made shorter than {opened:?}");
        assert!((shorter.min - opened.min).length() <= 1.0, "the window's top left moved from {opened:?} to {shorter:?}");
        for label in ["Close", "Open Folder"] {
            let painted = shapes.iter().find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some((clipped.clip_rect, text.visual_bounding_rect())),
                _ => None,
            });
            let (clip, rect) = painted.unwrap_or_else(|| panic!("{label} is not painted"));
            assert!(clip.expand(0.5).contains_rect(rect), "{label} at {rect:?} is cut off by {clip:?}");
            assert!(shorter.contains_rect(rect), "{label} at {rect:?} is outside the window {shorter:?}");
        }
    }
}
