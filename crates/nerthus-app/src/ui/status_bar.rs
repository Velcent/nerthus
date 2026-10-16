//! The status bar (`Shell` `StatusBar`, padding 8, 0, 8, 2): on the
//! right, docked first, what the engine is doing, the compute device and
//! memory (`Button-IconText`, monospace); the latest message takes what
//! they leave on the left, cut off at its edge. The device button
//! (`btnCPU`) shows only in CPU-only mode, or when the benchmark's fastest
//! device is off; it opens Options. The memory button opens Clean Memory
//! and lists the memory statistics on hover.

use egui::{Align, Layout, Pos2, Rect, RichText, Sense, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::help_windows;
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::Tooltips;

pub const HEIGHT: f32 = 20.0;

/// `BrushName.Teal`: the memory readout, once a version has started for
/// the first time.
const MEMORY_COLOR: egui::Color32 = egui::Color32::from_rgb(0x47, 0xb4, 0xb3);

/// Whether `eco.dat` was in the data folder as Nerthus started.
static ECO_MARKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `SetupInterface`: notes whether a version has started for the first
/// time before (`eco.dat`), as the interface is set up and before this
/// start's What's New check can write it.
pub fn note_eco_mark() {
    ECO_MARKED.get_or_init(|| crate::user::data_folder().is_some_and(|folder| folder.join(help_windows::ECO_FILE).exists()));
}

/// The memory readout is teal once a version has started for the first
/// time before this start (set on the button itself, so its hover tint
/// leaves it), the label colour until then.
fn memory_colour() -> Option<egui::Color32> {
    note_eco_mark();
    let marked = ECO_MARKED.get().copied().unwrap_or(false);
    marked.then_some(MEMORY_COLOR)
}

/// `process_gib` is the process's memory (`SystemInformationDiagnosticsPlugin`).
pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, process_gib: Option<f64>, commands: &mut Vec<Command>) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);
    let content = Rect::from_min_max(rect.min + Vec2::new(8.0, 0.0), rect.max - Vec2::new(8.0, 2.0));
    // The right-hand group is docked first, from the right edge, so the
    // message has only the width it leaves.
    let builder = UiBuilder::new().max_rect(content).layout(Layout::right_to_left(Align::Center));
    let mut bar = ui.new_child(builder);
    bar.spacing_mut().item_spacing.x = 0.0;
    readout(&mut bar, editor, process_gib, commands);

    // `StatusText` (margin 0, -1, 0, 0) fills the rest, cut off where the
    // engine's state begins.
    let message_right = bar.min_rect().left();
    let message = editor.messages.last().cloned().unwrap_or_default();
    let galley = ui.painter().layout_no_wrap(message, theme::interface(10.0), theme::LABEL_DEFAULT);
    let room = Rect::from_x_y_ranges(content.left()..=message_right.max(content.left()), content.y_range());
    let position = Pos2::new(room.left(), room.center().y - galley.size().y / 2.0 - 1.0);
    ui.painter().with_clip_rect(room.intersect(ui.clip_rect())).galley(position, galley, theme::LABEL_DEFAULT);
}

/// The right-hand group, right to left: memory, the device, the Nahimic
/// warning and what the engine is doing.
fn readout(ui: &mut Ui, editor: &Editor, process_gib: Option<f64>, commands: &mut Vec<Command>) {
    // `btnMemory`: the cached maps against the process, as Humanizer's
    // "0.0 GB" writes them.
    let gigabytes = |bytes: f64| bytes / (1024.0 * 1024.0 * 1024.0);
    let cached = gigabytes(editor.cache_bytes as f64);
    let memory = match process_gib {
        Some(process) => format!("{cached:.1} GB / {process:.1} GB"),
        None => format!("{cached:.1} GB"),
    };
    let memory_button = icon_text(ui, "binary", &memory, 11.0, memory_colour());
    if memory_button.tooltip(memory_tooltip(cached, process_gib)).clicked() {
        commands.push(Command::CleanMemory);
    }
    let device = if editor.compute.cpu_only { Some("CPU Only") } else { editor.compute.fastest_disabled.as_deref() };
    // `btnCPU` (`ShowOptionsDialog`): Options on the page it was last left
    // at.
    if let Some(device) = device
        && icon_text(ui, "cpu-2", device, 10.0, None).clicked()
    {
        commands.push(Command::ShowOptions(None));
    }
    // `NahimicWarning`: shows the warning again.
    if crate::nahimic::detected() {
        ui.add_space(8.0);
        let warning_text = RichText::new("⚠ Nahimic Detected!").font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
        let warning = ui.add(egui::Label::new(warning_text).sense(Sense::click()));
        let warning = warning.on_hover_cursor(egui::CursorIcon::PointingHand);
        if warning.clicked() {
            commands.push(Command::Tools(ToolCommand::ShowNahimicWarning));
        }
    }
    let state = if editor.cancelling && editor.building {
        "Cancelling"
    } else if !editor.engine_active {
        "Engine suspended"
    } else if editor.building {
        "Building"
    } else {
        "Idle"
    };
    ui.add_space(8.0);
    let state_text = RichText::new(state).font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
    ui.label(state_text);
}

/// The memory button's tooltip (`ReadOut_Memory`'s figures): the cache,
/// all of it hot, the rest of the process and the total.
fn memory_tooltip(cached: f64, process_gib: Option<f64>) -> String {
    let process = process_gib.unwrap_or(cached);
    let other = (process - cached).max(0.0);
    format!("Memory Statistics\n\nTotal Cache: {cached:.1} GB\nHot Memory: {cached:.1} GB\nWarm Memory: 0.0 GB\nOther Uses: {other:.1} GB\nTotal Memory: {process:.1} GB")
}

/// `Button-IconText` (padding 8, 2; the icon as wide as `MinWidth` 16;
/// the text 6 past it in monospace, hand cursor). Under the pointer it
/// takes `Button-Hover` (the bar's own colour) and the icon and text turn
/// `Accent-Muted-Gray`, unless the button has a colour of its own
/// (`color`), which the hover leaves as it is.
fn icon_text(ui: &mut Ui, icon: &str, text: &str, size: f32, color: Option<egui::Color32>) -> egui::Response {
    // Laid out uncoloured, so the text takes the tint the icon does.
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::monospace(size), egui::Color32::PLACEHOLDER);
    let width = 8.0 + 16.0 + 6.0 + galley.size().x + 8.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, rect_height(ui)), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::BUTTON_HOVER);
    }
    let tint = match color {
        Some(color) => color,
        None if response.hovered() => theme::ACCENT_MUTED_GRAY,
        None => theme::LABEL_DEFAULT,
    };
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, icon, tint);
    let text_position = Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0 + 0.5);
    ui.painter().galley(text_position, galley, tint);
    response
}

fn rect_height(ui: &Ui) -> f32 {
    ui.available_height().max(16.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::epaint::{ClippedShape, Shape, TextShape};
    use egui::{Color32, Event, Modifiers, PointerButton};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1000.0, 600.0));
    const BAR: Rect = Rect::from_min_max(Pos2::new(0.0, 580.0), Pos2::new(1000.0, 600.0));
    const PROCESS_GIB: f64 = 1.3;

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set, and the icons' loaders.
    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        egui_extras::install_image_loaders(&context);
        context.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        context
    }

    /// One frame of the bar with these events: what it drew and the
    /// commands it gave.
    fn frame(context: &egui::Context, editor: &Editor, events: Vec<Event>) -> (Vec<ClippedShape>, Vec<Command>) {
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let mut commands = Vec::new();
        let mut output = context.run_ui(input, |ui| show(ui, BAR, editor, Some(PROCESS_GIB), &mut commands));
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (shapes, commands)
    }

    fn text_saying<'a>(shapes: &'a [ClippedShape], text: &str) -> Option<&'a TextShape> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Text(shape) if shape.galley.text() == text => Some(shape),
            _ => None,
        })
    }

    /// A fill under `point` that stands out from the bar's.
    fn hover_fill(shapes: &[ClippedShape], point: Pos2) -> Option<Color32> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Rect(shape) if shape.rect.contains(point) && BAR.contains_rect(shape.rect) && shape.rect != BAR => Some(shape.fill),
            _ => None,
        })
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    /// The button saying `label`, `color` at rest, takes `Button-Hover` and
    /// `hovered` under the pointer, says `tooltip` (if any) once the
    /// pointer rests on it, and gives `command` when clicked.
    fn check_button(editor: &Editor, label: &str, color: Color32, hovered: Color32, tooltip: Option<&str>, command: Command) {
        let context = themed_context();
        let (shapes, _) = frame(&context, editor, Vec::new());
        let resting = text_saying(&shapes, label).unwrap_or_else(|| panic!("{label} shows")).clone();
        assert_eq!(resting.fallback_color, color, "{label} at rest");
        let point = resting.visual_bounding_rect().center();
        assert_eq!(hover_fill(&shapes, point), None, "{label} has no fill at rest");

        let (shapes, _) = frame(&context, editor, vec![Event::PointerMoved(point)]);
        let shown = text_saying(&shapes, label).unwrap_or_else(|| panic!("{label} shows hovered"));
        assert_eq!(shown.fallback_color, hovered, "{label} hovered");
        assert_eq!(hover_fill(&shapes, point), Some(theme::BUTTON_HOVER), "{label}'s fill hovered");

        let said = |text: &str| (0..120).any(|_| text_saying(&frame(&context, editor, Vec::new()).0, text).is_some());
        if let Some(tooltip) = tooltip {
            assert!(said(tooltip), "{label} says {tooltip:?} on hover");
        }

        frame(&context, editor, vec![button(point, true)]);
        let (_, commands) = frame(&context, editor, vec![button(point, false)]);
        assert_eq!(commands, vec![command], "{label} clicked");
    }

    #[test]
    fn the_memory_readout_lists_the_statistics_and_opens_clean_memory() {
        let editor = Editor::new(Document::new());
        let tooltip = memory_tooltip(0.0, Some(PROCESS_GIB));
        assert_eq!(tooltip, "Memory Statistics\n\nTotal Cache: 0.0 GB\nHot Memory: 0.0 GB\nWarm Memory: 0.0 GB\nOther Uses: 1.3 GB\nTotal Memory: 1.3 GB");
        // Teal is the button's own colour, which the hover leaves.
        let (rest, hovered) = match memory_colour() {
            Some(color) => (color, color),
            None => (theme::LABEL_DEFAULT, theme::ACCENT_MUTED_GRAY),
        };
        check_button(&editor, "0.0 GB / 1.3 GB", rest, hovered, Some(&tooltip), Command::CleanMemory);
    }

    #[test]
    fn the_device_readout_opens_options() {
        let mut editor = Editor::new(Document::new());
        editor.compute.cpu_only = true;
        check_button(&editor, "CPU Only", theme::LABEL_DEFAULT, theme::ACCENT_MUTED_GRAY, None, Command::ShowOptions(None));
    }

    #[test]
    fn a_long_message_is_cut_off_where_the_readout_begins() {
        let mut editor = Editor::new(Document::new());
        editor.compute.cpu_only = true;
        let folder = "/a very long directory name for testing recent paths overflow in nerthus/another nested folder level two/and a third";
        let message = format!("Opened {folder}/Alpine_Valley_with_an_extremely_long_file_name_used_to_test_overflow.terrain");
        editor.messages.push(message.clone());
        let context = themed_context();
        let (shapes, _) = frame(&context, &editor, Vec::new());

        let shown = shapes.iter().find(|clipped| matches!(&clipped.shape, Shape::Text(shape) if shape.galley.text() == message)).expect("the message shows");
        let Shape::Text(text) = &shown.shape else { unreachable!() };
        assert!(!text.galley.elided, "the message is laid out whole");
        assert!(text.visual_bounding_rect().left() >= BAR.left() + 8.0, "the message starts inside the bar");
        let state = text_saying(&shapes, "Idle").expect("the engine's state shows").visual_bounding_rect();
        assert!((shown.clip_rect.right() - state.left()).abs() < 1.0, "the message is cut off at {} rather than where the state begins, {}", shown.clip_rect.right(), state.left());
    }

    #[test]
    fn a_short_message_shows_whole() {
        let mut editor = Editor::new(Document::new());
        let message = "Opened Alpine Valley.terrain";
        editor.messages.push(message.to_string());
        let (shapes, _) = frame(&themed_context(), &editor, Vec::new());
        let shown = text_saying(&shapes, message).expect("the message shows");
        assert!(!shown.galley.elided, "the message is whole");
    }
}
