//! The message boxes: with Yes and No (`MBox(text, caption,
//! MessageBoxButton.YesNo)`, the command Yes runs), with Yes, No and
//! Cancel (`MboxFull(…, YesNoCancel)`, the save question), with OK
//! (`MessageBox.Show`), or with a task dialog's own button and Close. A
//! task dialog's footer shows under the buttons.
//!
//! Each is laid out as a task dialog is at 96 dpi: an upper panel with the
//! icon, the main instruction (the caption) and the text, then a line and
//! a strip with the buttons at its right, then the footer.

use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Sense, Ui, Vec2};

use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::widgets::DialogFit;
use crate::ui::UiState;

/// The window's width inside its border.
const WIDTH: f32 = 350.0;
/// Round the upper panel.
const PANEL_MARGIN: i8 = 10;
/// The icon at the panel's top left, and the gap between it and the words.
const ICON_SIZE: f32 = 32.0;
const ICON_GAP: f32 = 10.0;
/// The main instruction's font size and line pitch, and the gap under it.
const HEADING_SIZE: f32 = 16.0;
const HEADING_LINE: f32 = 21.0;
const HEADING_GAP: f32 = 10.0;
/// The text's (and the footer's) font size and line pitch, and the space
/// kept above and below the text.
const TEXT_SIZE: f32 = 12.0;
const TEXT_LINE: f32 = 15.0;
const TEXT_MARGIN: f32 = 5.0;
/// Round the buttons in their strip.
const STRIP_MARGIN: Margin = Margin { left: 10, right: 11, top: 9, bottom: 10 };
/// A button's least size, the space either side of its words, and the gap
/// between buttons.
const BUTTON_MIN_SIZE: Vec2 = Vec2::new(70.0, 21.0);
const BUTTON_PADDING: f32 = 16.0;
const BUTTON_GAP: f32 = 8.0;
/// Round the footer's text.
const FOOTER_MARGIN: Margin = Margin { left: 10, right: 10, top: 8, bottom: 8 };

/// A task dialog with custom content or actions. Its instruction, body,
/// button strip and optional footer share the message boxes' layout.
pub struct TaskDialog {
    id: egui::Id,
    width: f32,
    frame: egui::Frame,
    fit: DialogFit,
    lower_radius: u8,
}

impl TaskDialog {
    pub fn new(ctx: &egui::Context, id: egui::Id, width: f32) -> Self {
        let mut frame = egui::Frame::popup(&ctx.global_style());
        frame.inner_margin = Margin::ZERO;
        let border = frame.stroke.width as u8;
        let lower_radius = frame.corner_radius.sw.saturating_sub(border);
        let fit = DialogFit::new(ctx, id, &frame);
        Self { id, width, frame, fit, lower_radius }
    }

    pub fn show<R>(&self, ctx: &egui::Context, content: impl FnOnce(&mut Ui, &Self) -> R) -> egui::ModalResponse<R> {
        let modal = egui::Modal::new(self.id).frame(self.frame);
        let response = modal.show(ctx, |ui| {
            ui.set_width(self.fit.width(self.width));
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            content(ui, self)
        });
        self.fit.end(ctx, response.response.rect);
        response
    }

    /// The instruction and content scroll together when space is short.
    pub fn body<R>(&self, ui: &mut Ui, heading: &str, warning: bool, content: impl FnOnce(&mut Ui) -> R) -> R {
        self.fit.body(ui, |ui| instruction_panel(ui, heading, warning, content))
    }

    /// The separated button strip. Add the rightmost action first.
    pub fn buttons<R>(&self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        self.button_panel(ui, None::<fn(&mut Ui)>, content)
    }

    /// A verification control stays above the actions when the body scrolls.
    pub fn buttons_with_verification<R>(&self, ui: &mut Ui, verification: impl FnOnce(&mut Ui), content: impl FnOnce(&mut Ui) -> R) -> R {
        self.button_panel(ui, Some(verification), content)
    }

    fn button_panel<R>(&self, ui: &mut Ui, verification: Option<impl FnOnce(&mut Ui)>, content: impl FnOnce(&mut Ui) -> R) -> R {
        divider(ui);
        let corners = CornerRadius { nw: 0, ne: 0, sw: self.lower_radius, se: self.lower_radius };
        let panel = egui::Frame::new().fill(theme::WINDOW_BACKGROUND).corner_radius(corners);
        let shown = panel.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let strip = egui::Frame::new().inner_margin(STRIP_MARGIN);
            strip.show(ui, |ui| {
                if let Some(verification) = verification {
                    verification(ui);
                    ui.add_space(BUTTON_GAP);
                }
                ui.spacing_mut().item_spacing.x = BUTTON_GAP;
                ui.spacing_mut().button_padding.x = BUTTON_PADDING;
                ui.with_layout(Layout::right_to_left(Align::Min), content).inner
            }).inner
        });
        shown.inner
    }

    pub fn footer<R>(&self, ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
        divider(ui);
        let corners = CornerRadius { nw: 0, ne: 0, sw: self.lower_radius, se: self.lower_radius };
        let panel = egui::Frame::new().fill(theme::WINDOW_BACKGROUND).corner_radius(corners).inner_margin(FOOTER_MARGIN);
        panel.show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        }).inner
    }
}

/// A button shared by custom task dialogs and ordinary message boxes.
pub fn task_button(ui: &mut Ui, text: &str) -> egui::Response {
    let label = styled(text, TEXT_SIZE, TEXT_LINE, theme::LABEL_EMPHASIS);
    let button = egui::Button::new(label).min_size(BUTTON_MIN_SIZE);
    ui.add(button)
}

/// A command link whose instruction and explanation wrap together.
pub fn task_link(ui: &mut Ui, title: &str, note: &str) -> egui::Response {
    let width = ui.available_width();
    let text_width = (width - 40.0).max(0.0);
    let title_galley = ui.painter().layout(title.to_string(), theme::semibold(13.0), theme::LABEL_EMPHASIS, text_width);
    let note_galley = ui.painter().layout(note.to_string(), theme::interface(11.0), theme::LABEL_DEFAULT, text_width);
    let title_height = title_galley.size().y;
    let height = (title_height + note_galley.size().y + 14.0).max(44.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 3.0, theme::BUTTON_HOVER);
        ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
    }
    let arrow = egui::Rect::from_min_size(rect.min + Vec2::new(8.0, 6.0), Vec2::splat(16.0));
    icons::paint(ui, arrow, "arrow-right-square", theme::ACCENT_GREEN);
    let text_position = rect.min + Vec2::new(32.0, 5.0);
    ui.painter().galley(text_position, title_galley, theme::LABEL_EMPHASIS);
    ui.painter().galley(text_position + Vec2::new(0.0, title_height + 3.0), note_galley, theme::LABEL_DEFAULT);
    response
}

/// A question waiting for Yes or No, or a message waiting for OK.
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub caption: String,
    pub text: String,
    /// What Yes runs; none for a message.
    pub yes: Option<Command>,
    /// What No runs, when it does more than close the box.
    pub no: Option<Command>,
    /// What closing the box without an answer (Escape, a click outside)
    /// runs, when anything.
    pub dismissed: Option<Command>,
    /// Whether there is a Cancel button (Yes, No and Cancel).
    pub cancel: bool,
    /// The task dialog's footer (`MBox`'s footnote).
    pub footnote: Option<String>,
    /// What the Yes and No buttons say.
    pub yes_text: String,
    pub no_text: String,
    /// Whether the warning icon shows beside the main instruction
    /// (`TaskDialogIcon.Warning`).
    pub warning: bool,
}

impl Prompt {
    pub fn new(caption: &str, text: &str, yes: Command) -> Self {
        let mut prompt = Self::message(caption, text);
        prompt.yes = Some(yes);
        prompt
    }

    pub fn message(caption: &str, text: &str) -> Self {
        Self {
            caption: caption.to_string(),
            text: text.to_string(),
            yes: None,
            no: None,
            dismissed: None,
            cancel: false,
            footnote: None,
            yes_text: "Yes".to_string(),
            no_text: "No".to_string(),
            warning: false,
        }
    }

    /// Yes, No and Cancel, each with its own command but Cancel (which
    /// only closes the box).
    pub fn yes_no_cancel(caption: &str, text: &str, yes: Command, no: Command) -> Self {
        let mut prompt = Self::new(caption, text, yes);
        prompt.no = Some(no);
        prompt.cancel = true;
        prompt
    }

    /// A task dialog with a button of its own, which runs `command`, and
    /// Close.
    pub fn with_button(caption: &str, text: &str, button: &str, command: Command) -> Self {
        let mut prompt = Self::new(caption, text, command);
        prompt.yes_text = button.to_string();
        prompt.no_text = "Close".to_string();
        prompt
    }
}

/// The prompt, while one is open; Yes (and No, when it has a command)
/// queues its command.
pub fn show(ctx: &egui::Context, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(prompt) = ui_state.prompt.as_ref() else {
        return;
    };
    let mut frame = egui::Frame::popup(&ctx.global_style());
    frame.inner_margin = Margin::ZERO;
    // The lower panel's corners follow the window's, inside its border.
    let border = frame.stroke.width as u8;
    let lower_radius = frame.corner_radius.sw.saturating_sub(border);

    // The window keeps the size the last box left it, and is centred by
    // it: a box other than the last one shown is measured before it is
    // drawn.
    let id = modal_id();
    let measure = ctx.data_mut(|data| {
        let shown = data.get_temp_mut_or_default::<Option<Prompt>>(id.with("shown"));
        let new = shown.as_ref() != Some(prompt);
        if new {
            *shown = Some(prompt.clone());
        }
        new
    });
    let area = egui::Modal::default_area(id).sizing_pass(measure);
    let modal = egui::Modal::new(id).area(area).frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        upper_panel(ui, prompt);
        lower_panel(ui, prompt, lower_radius)
    });

    // Asked once: asking takes the Escape press.
    let closing = response.should_close();
    let answer = response.inner;
    match answer {
        Some(Answer::Yes) => commands.extend(prompt.yes.clone()),
        Some(Answer::No) => commands.extend(prompt.no.clone()),
        Some(Answer::Cancel) => {}
        None if closing => commands.extend(prompt.dismissed.clone()),
        None => {}
    }
    if answer.is_some() || closing {
        ui_state.prompt = None;
    }
}

/// The message box's window: the one for every prompt.
fn modal_id() -> egui::Id {
    egui::Id::new("message-box")
}

/// The icon, when there is one, beside the main instruction over the text.
/// The panel ends 10 under the icon or 15 under the text, whichever is
/// lower.
fn upper_panel(ui: &mut Ui, prompt: &Prompt) {
    instruction_panel(ui, &prompt.caption, prompt.warning, |ui| {
        if !prompt.text.is_empty() {
            let text = styled(&prompt.text, TEXT_SIZE, TEXT_LINE, theme::LABEL_DEFAULT);
            ui.add(egui::Label::new(text).wrap());
        }
    });
}

fn instruction_panel<R>(ui: &mut Ui, heading: &str, warning: bool, content: impl FnOnce(&mut Ui) -> R) -> R {
    let panel = egui::Frame::new().inner_margin(PANEL_MARGIN);
    let shown = panel.show(ui, |ui| {
        let row = ui.horizontal_top(|ui| {
            if warning {
                let icon_size = Vec2::splat(ICON_SIZE);
                let (icon_rect, _) = ui.allocate_exact_size(icon_size, Sense::hover());
                icons::paint(ui, icon_rect, "alert-triangle", theme::ACCENT_YELLOW);
                ui.add_space(ICON_GAP);
            }
            let column = ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.style_mut().override_font_id = Some(theme::interface(TEXT_SIZE));
                ui.visuals_mut().override_text_color = Some(theme::LABEL_DEFAULT);
                if !heading.is_empty() {
                    let heading = styled(heading, HEADING_SIZE, HEADING_LINE, theme::ACCENT_MUTED_BLUE);
                    ui.add(egui::Label::new(heading).wrap());
                    ui.add_space(HEADING_GAP);
                }
                ui.add_space(TEXT_MARGIN);
                let result = content(ui);
                ui.add_space(TEXT_MARGIN);
                result
            });
            column.inner
        });
        row.inner
    });
    shown.inner
}

/// Under a line, the strip with the buttons, then the footer under a line
/// of its own. Returns the button clicked.
fn lower_panel(ui: &mut Ui, prompt: &Prompt, corner_radius: u8) -> Option<Answer> {
    divider(ui);
    let corners = CornerRadius { nw: 0, ne: 0, sw: corner_radius, se: corner_radius };
    let lower = egui::Frame::new().fill(theme::WINDOW_BACKGROUND).corner_radius(corners);
    let shown = lower.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let strip = egui::Frame::new().inner_margin(STRIP_MARGIN);
        let clicked = strip.show(ui, |ui| buttons(ui, prompt));
        if let Some(footnote) = &prompt.footnote {
            divider(ui);
            let footer = egui::Frame::new().inner_margin(FOOTER_MARGIN);
            let text = styled(footnote, TEXT_SIZE, TEXT_LINE, theme::ACCENT_YELLOW);
            footer.show(ui, |ui| ui.add(egui::Label::new(text).wrap()));
        }
        clicked.inner
    });
    shown.inner
}

/// The buttons from the right: OK alone for a message, else Cancel (when
/// there is one), No and Yes. The row is aligned to its top: centred, it
/// would take all the height left in the window.
fn buttons(ui: &mut Ui, prompt: &Prompt) -> Option<Answer> {
    ui.spacing_mut().item_spacing.x = BUTTON_GAP;
    ui.spacing_mut().button_padding.x = BUTTON_PADDING;
    let row = ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        if prompt.yes.is_none() {
            return button(ui, "OK").then_some(Answer::No);
        }
        let mut answer = None;
        if prompt.cancel && button(ui, "Cancel") {
            answer = Some(Answer::Cancel);
        }
        if button(ui, &prompt.no_text) {
            answer = Some(Answer::No);
        }
        if button(ui, &prompt.yes_text) {
            answer = Some(Answer::Yes);
        }
        answer
    });
    row.inner
}

/// A button at least 70 by 21; whether it was clicked.
fn button(ui: &mut Ui, text: &str) -> bool {
    task_button(ui, text).clicked()
}

/// A line across the window, between its panels.
fn divider(ui: &mut Ui) {
    let size = Vec2::new(ui.available_width(), 1.0);
    let (line, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(line, 0.0, theme::MENU_BORDER);
}

/// `text` in the interface font at `size`, its lines `line` apart.
fn styled(text: &str, size: f32, line: f32, color: Color32) -> RichText {
    let rich = RichText::new(text).font(theme::interface(size)).color(color);
    rich.line_height(Some(line))
}

#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Yes,
    No,
    Cancel,
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Shape};

    use super::*;
    use crate::files::FileCommand;

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));

    /// One frame of the prompt with `events`; the commands it queued, and
    /// whether the box was painted.
    fn painted_frame(context: &egui::Context, ui_state: &mut UiState, events: Vec<Event>) -> (Vec<Command>, bool) {
        let input = RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let mut commands = Vec::new();
        let mut output = context.run_ui(input, |ui| show(ui.ctx(), ui_state, &mut commands));
        // A pass that only measures the box adds no-ops in its place.
        let painted = std::mem::take(&mut output.shapes).iter().any(|clipped| !matches!(clipped.shape, Shape::Noop));
        output.drop_without_applying_deltas();
        (commands, painted)
    }

    /// One frame of the prompt with `events`; the commands it queued.
    fn prompt_frame(context: &egui::Context, ui_state: &mut UiState, events: Vec<Event>) -> Vec<Command> {
        painted_frame(context, ui_state, events).0
    }

    fn escape() -> Event {
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// The box where the last frame left it.
    fn box_rect(context: &egui::Context) -> Rect {
        context.memory(|memory| memory.area_rect(modal_id())).expect("the box was shown")
    }

    /// "Save cached build?", with its footnote.
    fn save_cached_build() -> Prompt {
        let text = "All nodes marked for export seem to be built. Would you like to try to save the current build directly?";
        let mut question = Prompt::new("Save cached build?", text, Command::RunBuild { close: false });
        question.no = Some(Command::RunBuild { close: false });
        question.footnote = Some("Please note, some advanced features such as Prime Folders may not work.".to_string());
        question
    }

    #[test]
    fn a_box_with_a_footnote_keeps_its_size() {
        let context = egui::Context::default();
        let mut ui_state = UiState { prompt: Some(save_cached_build()), ..Default::default() };

        let mut painted = Vec::new();
        for _ in 0..60 {
            if painted_frame(&context, &mut ui_state, Vec::new()).1 {
                painted.push(box_rect(&context));
            }
        }
        let first = painted[0];
        assert!(painted.iter().all(|rect| *rect == first), "{painted:?}");
        // The caption, the text, the buttons and the footnote, no more.
        assert!(first.height() < 250.0, "{first:?}");
        assert!(SCREEN.contains_rect(first), "{first:?}");

        prompt_frame(&context, &mut ui_state, vec![escape()]);
        assert!(ui_state.prompt.is_none());
    }

    #[test]
    fn a_new_box_is_measured_afresh() {
        let context = egui::Context::default();
        let long_message = Prompt::message("Long", &"A long message. ".repeat(60));
        let mut ui_state = UiState { prompt: Some(long_message), ..Default::default() };
        for _ in 0..3 {
            prompt_frame(&context, &mut ui_state, Vec::new());
        }
        let long = box_rect(&context);

        // Put in its place, a shorter box is not drawn at the last one's
        // size: from its first painted frame it is where it stays.
        ui_state.prompt = Some(save_cached_build());
        let mut painted = Vec::new();
        for _ in 0..3 {
            if painted_frame(&context, &mut ui_state, Vec::new()).1 {
                painted.push(box_rect(&context));
            }
        }
        let settled = box_rect(&context);
        assert!(settled.height() < long.height(), "{settled:?} {long:?}");
        assert!(!painted.is_empty());
        assert!(painted.iter().all(|rect| *rect == settled), "{painted:?} {settled:?}");
        assert!((settled.center().y - SCREEN.center().y).abs() < 1.0, "{settled:?}");
    }

    #[test]
    fn escape_closes_the_box_and_runs_what_dismissing_it_runs() {
        let context = egui::Context::default();
        let mut ui_state = UiState::default();
        let mut message = Prompt::message("Caption", "Text");
        message.dismissed = Some(Command::File(FileCommand::DiscardChanges));
        ui_state.prompt = Some(message);

        // The box is the top one from its second frame on.
        let first_frame = prompt_frame(&context, &mut ui_state, Vec::new());
        assert!(first_frame.is_empty());
        let second_frame = prompt_frame(&context, &mut ui_state, vec![escape()]);

        assert_eq!(second_frame, vec![Command::File(FileCommand::DiscardChanges)]);
        assert!(ui_state.prompt.is_none());
    }

    #[test]
    fn custom_task_dialogs_keep_their_actions_and_footer_below_a_scrolling_body() {
        use crate::ui::widgets::fit_checks;
        let context = fit_checks::context();
        let id = egui::Id::new("custom-task-dialog");
        let text = "Long explanatory content that must scroll rather than pushing the actions out of the window. ".repeat(50);
        for screen in fit_checks::SMALL_SCREENS {
            let mut body_clip = Rect::NOTHING;
            let mut close = Rect::NOTHING;
            let mut action = Rect::NOTHING;
            let mut footer = Rect::NOTHING;
            let mut draw = |ctx: &egui::Context| {
                let dialog = TaskDialog::new(ctx, id, 460.0);
                dialog.show(ctx, |ui, task| {
                    task.body(ui, "Task dialog instruction", true, |ui| {
                        body_clip = ui.clip_rect();
                        ui.add(egui::Label::new(&text).wrap());
                        task_link(ui, "A command with a long instruction", "Its explanation also wraps to remain inside the dialog.");
                    });
                    task.buttons(ui, |ui| {
                        close = task_button(ui, "Close").rect;
                        action = task_button(ui, "Perform action").rect;
                    });
                    task.footer(ui, |ui| {
                        footer = ui.label("Task dialog footer").rect;
                    });
                });
            };
            let words = fit_checks::settle(&context, screen, &mut draw);
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
            let output = context.run_ui(input, |ui| draw(ui.ctx()));
            let instruction = output.shapes.iter().find_map(|clipped| match &clipped.shape {
                Shape::Text(shape) if shape.galley.text() == "Task dialog instruction" => Some(shape),
                _ => None,
            }).expect("the main instruction is painted");
            for section in &instruction.galley.job.sections {
                assert_eq!(section.format.font_id.size, HEADING_SIZE);
                assert_eq!(section.format.color, theme::ACCENT_MUTED_BLUE);
            }
            output.drop_without_applying_deltas();
            let expected = ["Task dialog instruction", "Close", "Perform action", "Task dialog footer"];
            fit_checks::assert_fits(&context, screen, id, &words, &expected);
            assert!(body_clip.bottom() <= close.top(), "the actions cover the scrolling body");
            assert!(action.right() < close.left(), "the actions are not aligned from the right");
            assert!(footer.top() > close.bottom(), "the footer is not below the action strip");
        }
    }
}
