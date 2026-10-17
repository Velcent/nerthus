//! `Shell.ShowCommandLine` (Vars › Show Build Command Line): a task dialog
//! with the command that builds the project from a prompt, one argument
//! to a line: `nerthus-build`, the project, and every variable as
//! `-v name=value`. Copy to Clipboard copies it as one line; Close only
//! closes.

use std::path::PathBuf;

use egui::{Id, RichText, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::build_command::{CommandKind, quote_argument, quote_program, variable_argument};
use crate::ui::prompt::{task_button, TaskDialog};

/// `TaskDialog.Width` (400 dialog units) in points, roughly, where there
/// is room.
const WIDTH: f32 = 600.0;

/// The dialog while it shows: the command as shown and as copied.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandLineWindow {
    shown: String,
    copied: String,
}

/// `nerthus-build` beside the running program.
fn builder_path() -> PathBuf {
    let name = if cfg!(windows) { "nerthus-build.exe" } else { "nerthus-build" };
    let program = std::env::current_exe().unwrap_or_default();
    let folder = program.parent().map(PathBuf::from).unwrap_or_default();
    folder.join(name)
}

impl CommandLineWindow {
    pub fn new(editor: &Editor) -> Self {
        let builder = builder_path().display().to_string();
        let project = editor.document.path.as_ref().map(|path| path.display().to_string()).unwrap_or_default();
        let mut variables = Vec::new();
        for (name, variable) in &editor.document.automation().variables {
            let value = variable.value.to_text();
            variables.push(variable_argument(name, &value, CommandKind::Prompt));
        }
        Self::from_parts(&builder, &project, &variables)
    }

    /// `content` (indented lines) and `text` (one line, what is copied).
    fn from_parts(builder: &str, project: &str, variables: &[String]) -> Self {
        let program = quote_program(builder, CommandKind::Prompt);
        let project_argument = quote_argument(project, CommandKind::Prompt);
        let mut shown = format!("{program}\n    build {project_argument}");
        let mut copied = format!("{program} build {project_argument}");
        for variable in variables {
            shown.push_str("\n    ");
            shown.push_str(variable);
            copied.push(' ');
            copied.push_str(variable);
        }
        Self { shown, copied }
    }
}

/// The dialog; Copy to Clipboard copies the command and closes it, as
/// Close and Escape do without copying. The command scrolls when the
/// window is too small for it, the buttons and footer staying below it.
pub fn show(ctx: &egui::Context, window: &mut Option<CommandLineWindow>) {
    let Some(dialog) = window.as_ref() else {
        return;
    };
    let mut copy = false;
    let mut close = false;
    let id = Id::new("build-command-line");
    let modal = TaskDialog::new(ctx, id, WIDTH);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, "Build Command Line", false, |ui| {
            let content = RichText::new(dialog.shown.as_str()).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
            ui.add(egui::Label::new(content).wrap());
        });
        task.buttons(ui, |ui| {
            if task_button(ui, "Close").clicked() {
                close = true;
            }
            if task_button(ui, "Copy to Clipboard").clicked() {
                copy = true;
            }
        });
        // `Footer` with `TaskDialogIcon.Information`.
        task.footer(ui, |ui| {
            ui.horizontal(|ui| {
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(icons::DEFAULT_SIZE), egui::Sense::hover());
                icons::paint(ui, icon_rect, "info-square-rounded", theme::ACCENT_BLUE);
                let footer = RichText::new("You can use this command line directly from any command prompt.").font(theme::interface(11.0)).color(theme::LABEL_DEFAULT);
                ui.add(egui::Label::new(footer).wrap());
            });
        });
    });
    if copy {
        ctx.copy_text(dialog.copied.clone());
    }
    if copy || close || response.should_close() {
        *window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_shown_a_line_an_argument_and_copied_as_one() {
        let variables = vec![variable_argument("Height", "0.5", CommandKind::Prompt), variable_argument("Style", "Ridge", CommandKind::Prompt)];
        let window = CommandLineWindow::from_parts("/bin/nerthus-build", "/p/a b.terrain", &variables);
        let program = quote_program("/bin/nerthus-build", CommandKind::Prompt);
        let project = quote_argument("/p/a b.terrain", CommandKind::Prompt);
        assert_eq!(window.shown, format!("{program}\n    build {project}\n    {}\n    {}", variables[0], variables[1]));
        assert_eq!(window.copied, format!("{program} build {project} {} {}", variables[0], variables[1]));
    }

    #[cfg(unix)]
    #[test]
    fn the_copied_command_preserves_ranges_and_literal_text() {
        use crate::document::Document;
        use nerthus_engine::automation::{Variable, VariableValue};

        let project = "/p/a 'quoted' $(printf expanded).terrain";
        let mut document = Document::new();
        document.path = Some(PathBuf::from(project));
        let values = [
            ("Range", VariableValue::Float2([0.0, 1.0])),
            ("Number text", VariableValue::Text("123 456".to_string())),
            ("Empty", VariableValue::Null),
            ("Literal", VariableValue::Text("a'b\"c &|; $HOME `printf expanded` \\".to_string())),
        ];
        for (name, value) in values {
            let json = serde_json::json!({"Name": name});
            let mut variable = Variable::from_json(&json);
            variable.value = value;
            document.project.assets[0].automation.variables.insert(name.to_string(), variable);
        }
        let editor = Editor::new(document);
        let window = CommandLineWindow::new(&editor);
        // Record argv in place of the real builder, retaining the exact
        // project and variable arguments generated by the dialog.
        let program = quote_program(&builder_path().to_string_lossy(), CommandKind::Prompt);
        let script = window.copied.replacen(&program, "printf '%s\\0'", 1);
        let output = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = format!("build\0{project}\0-v\0Range=0, 1\0-v\0Number text=123 456\0-v\0Empty=\0-v\0Literal=a'b\"c &|; $HOME `printf expanded` \\\0");
        assert_eq!(output.stdout, expected.as_bytes());
    }

    #[test]
    fn the_dialog_fits_small_windows() {
        use crate::ui::widgets::fit_checks;
        let context = fit_checks::context();
        let variables: Vec<String> = (0..24).map(|index| format!("-v Variable{index}={index}")).collect();
        let builder = "/home/someone/Applications/Nerthus/a rather long folder name/nerthus-build";
        let mut window = Some(CommandLineWindow::from_parts(builder, "/home/someone/Documents/Nerthus/Projects/Alpine Valley.terrain", &variables));
        for screen in fit_checks::SMALL_SCREENS {
            let words = fit_checks::settle(&context, screen, |ctx| show(ctx, &mut window));
            let expected = ["Build Command Line", "Copy to Clipboard", "Close", "You can use this command line"];
            fit_checks::assert_fits(&context, screen, Id::new("build-command-line"), &words, &expected);
        }
    }
}
