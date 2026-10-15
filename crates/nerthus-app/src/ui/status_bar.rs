//! The status bar (`Shell` `StatusBar`, padding 8, 0, 8, 2): the latest
//! message on the left, cut short with "…" before the right-hand group
//! and shown in full on hover; on the right, what the engine is doing, the
//! compute device and memory (`Button-IconText`, monospace). The device
//! button (`btnCPU`) shows only in CPU-only mode, or when the benchmark's
//! fastest device is off; it opens Options at Compute. The memory button
//! opens Clean Memory. Each says on hover what it shows and does.

use egui::{Align, Layout, Pos2, Rect, RichText, Sense, Ui, UiBuilder, Vec2};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::help_windows;
use crate::ui::options_window::OptionsTab;
use crate::ui::tool_commands::ToolCommand;

pub const HEIGHT: f32 = 20.0;

/// The space between the end of the message and the right-hand group.
const MESSAGE_GAP: f32 = 16.0;

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
/// time before this start, the label colour until then.
fn memory_colour() -> egui::Color32 {
    note_eco_mark();
    let marked = ECO_MARKED.get().copied().unwrap_or(false);
    if marked { MEMORY_COLOR } else { theme::LABEL_DEFAULT }
}

/// `process_gib` is the process's memory (`SystemInformationDiagnosticsPlugin`).
pub fn show(ui: &mut Ui, rect: Rect, editor: &Editor, process_gib: Option<f64>, commands: &mut Vec<Command>) {
    ui.painter().rect_filled(rect, 0.0, theme::WINDOW_BACKGROUND);
    let content = Rect::from_min_max(rect.min + Vec2::new(8.0, 0.0), rect.max - Vec2::new(8.0, 2.0));
    // The right-hand group is laid out first, from the right edge, so the
    // message has only the width it leaves.
    let builder = UiBuilder::new().max_rect(content).layout(Layout::right_to_left(Align::Center));
    let mut bar = ui.new_child(builder);
    bar.spacing_mut().item_spacing.x = 0.0;
    readout(&mut bar, editor, process_gib, commands);

    bar.add_space(MESSAGE_GAP);
    let message = editor.messages.last().cloned().unwrap_or_default();
    let message_text = RichText::new(message).font(theme::interface(10.0)).color(theme::LABEL_DEFAULT);
    bar.with_layout(Layout::left_to_right(Align::Center), |ui| {
        ui.add(egui::Label::new(message_text).truncate());
    });
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
    if memory_button.on_hover_text(memory_tooltip(cached, process_gib)).clicked() {
        commands.push(Command::CleanMemory);
    }
    let device = if editor.compute.cpu_only { Some("CPU Only") } else { editor.compute.fastest_disabled.as_deref() };
    if let Some(device) = device {
        let device_button = icon_text(ui, "cpu-2", device, 10.0, theme::LABEL_DEFAULT);
        if device_button.on_hover_text(device_tooltip(editor.compute.cpu_only)).clicked() {
            commands.push(Command::ShowOptions(Some(OptionsTab::Compute)));
        }
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
    let state = if !editor.engine_active {
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

/// The memory button's tooltip: what its two figures are, and what a
/// click does.
fn memory_tooltip(cached: f64, process_gib: Option<f64>) -> String {
    let mut lines = vec![format!("Cache: {cached:.1} GB, the maps the nodes have built")];
    if let Some(process) = process_gib {
        lines.push(format!("Total: {process:.1} GB, all the memory Nerthus uses"));
    }
    lines.push(String::new());
    lines.push("Click to clean memory.".to_string());
    lines.join("\n")
}

/// The device button's tooltip: why it shows, and what a click does.
fn device_tooltip(cpu_only: bool) -> &'static str {
    if cpu_only {
        "Nodes build on the CPU alone.\n\nClick to choose the compute devices in Options."
    } else {
        "The fastest device in the benchmark is not in use.\n\nClick to choose the compute devices in Options."
    }
}

/// `Button-IconText` (padding 8, 2; the icon as wide as `MinWidth` 16;
/// the text 6 past it in monospace). Under the pointer it lights up: a
/// fill that stands out from the bar, and the icon and text brighter.
fn icon_text(ui: &mut Ui, icon: &str, text: &str, size: f32, color: egui::Color32) -> egui::Response {
    // Laid out uncoloured, so the text takes the tint the icon does.
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::monospace(size), egui::Color32::PLACEHOLDER);
    let width = 8.0 + 16.0 + 6.0 + galley.size().x + 8.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, rect_height(ui)), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::MENU_BACKGROUND_HOVER);
    }
    let tint = if response.hovered() { color.lerp_to_gamma(egui::Color32::WHITE, 0.5) } else { color };
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

    fn brightness(color: Color32) -> u32 {
        u32::from(color.r()) + u32::from(color.g()) + u32::from(color.b())
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    /// The button saying `label` lights up under the pointer, says
    /// `tooltip` once the pointer rests on it, and gives `command` when
    /// clicked.
    fn check_button(editor: &Editor, label: &str, color: Color32, tooltip: &str, command: Command) {
        let context = themed_context();
        let (shapes, _) = frame(&context, editor, Vec::new());
        let resting = text_saying(&shapes, label).unwrap_or_else(|| panic!("{label} shows")).clone();
        assert_eq!(resting.fallback_color, color, "{label} at rest");
        let point = resting.visual_bounding_rect().center();
        assert_eq!(hover_fill(&shapes, point), None, "{label} has no fill at rest");

        let (shapes, _) = frame(&context, editor, vec![Event::PointerMoved(point)]);
        let hovered = text_saying(&shapes, label).unwrap_or_else(|| panic!("{label} shows hovered"));
        assert!(brightness(hovered.fallback_color) > brightness(color), "{label} brightens, not {:?}", hovered.fallback_color);
        let fill = hover_fill(&shapes, point).unwrap_or_else(|| panic!("{label} is filled hovered"));
        assert_ne!(fill, theme::WINDOW_BACKGROUND, "{label}'s fill shows against the bar");
        assert!(brightness(fill) > brightness(theme::WINDOW_BACKGROUND), "{label}'s fill is lighter than the bar");

        let shown = (0..120).any(|_| text_saying(&frame(&context, editor, Vec::new()).0, tooltip).is_some());
        assert!(shown, "{label} says {tooltip:?} on hover");

        frame(&context, editor, vec![button(point, true)]);
        let (_, commands) = frame(&context, editor, vec![button(point, false)]);
        assert_eq!(commands, vec![command], "{label} clicked");
    }

    #[test]
    fn the_memory_readout_explains_itself_and_opens_clean_memory() {
        let editor = Editor::new(Document::new());
        let tooltip = memory_tooltip(0.0, Some(PROCESS_GIB));
        assert!(tooltip.contains("Cache: 0.0 GB") && tooltip.contains("Total: 1.3 GB"), "{tooltip}");
        check_button(&editor, "0.0 GB / 1.3 GB", memory_colour(), &tooltip, Command::CleanMemory);
    }

    #[test]
    fn the_device_readout_explains_itself_and_opens_compute() {
        let mut editor = Editor::new(Document::new());
        editor.compute.cpu_only = true;
        let command = Command::ShowOptions(Some(OptionsTab::Compute));
        check_button(&editor, "CPU Only", theme::LABEL_DEFAULT, device_tooltip(true), command);
    }

    #[test]
    fn a_long_message_stops_short_of_the_readout_and_shows_in_full_on_hover() {
        let mut editor = Editor::new(Document::new());
        editor.compute.cpu_only = true;
        let folder = "/a very long directory name for testing recent paths overflow in nerthus/another nested folder level two/and a third";
        let message = format!("Opened {folder}/Alpine_Valley_with_an_extremely_long_file_name_used_to_test_overflow.terrain");
        editor.messages.push(message.clone());
        let context = themed_context();
        let (shapes, _) = frame(&context, &editor, Vec::new());

        let shown = text_saying(&shapes, &message).expect("the message shows");
        assert!(shown.galley.elided, "the message is cut short");
        let message_rect = shown.visual_bounding_rect();
        assert!(message_rect.left() >= BAR.left() + 8.0, "the message starts inside the bar: {message_rect:?}");
        let state = text_saying(&shapes, "Idle").expect("the engine's state shows").visual_bounding_rect();
        assert!(message_rect.right() + MESSAGE_GAP <= state.left(), "the message {message_rect:?} stops short of the state {state:?}");
        for readout in ["CPU Only", "0.0 GB / 1.3 GB"] {
            let readout_rect = text_saying(&shapes, readout).unwrap_or_else(|| panic!("{readout} shows")).visual_bounding_rect();
            assert!(!message_rect.intersects(readout_rect), "the message {message_rect:?} clears {readout} {readout_rect:?}");
        }

        let point = message_rect.center();
        frame(&context, &editor, vec![Event::PointerMoved(point)]);
        let in_full = (0..120).any(|_| {
            let (shapes, _) = frame(&context, &editor, Vec::new());
            shapes.iter().any(|clipped| match &clipped.shape {
                Shape::Text(shape) => shape.galley.text() == message && !shape.galley.elided,
                _ => false,
            })
        });
        assert!(in_full, "the whole message shows on hover");
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
