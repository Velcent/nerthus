//! The first-run setup (`Onboard`): shown when the data folder has no
//! `onboard.dat`, or with `--onboard`, before the editor is used. Three
//! pages: where documents, projects and builds go; the navigation and
//! toolbox styles; and what the compute devices said. "Let's get
//! started!" keeps the choices, takes them at once and writes
//! `onboard.dat`. `--skip-onboard` writes the file without asking, and
//! `--screenshot` runs skip the window.
//!
//! On a screen too small for the whole window it shrinks: its margins
//! narrow, the pages scroll above the footer, and the choices wrap or go
//! one over the other, so the Next button and the note below the pages
//! never cover them.

use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Align, Color32, FontId, Galley, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::{json, Value};

use crate::editor::Editor;
use crate::theme;
use crate::ui::file_dialogs::{DialogKind, DialogRequest, FileDialogs};
use crate::ui::help_windows::{self, SideTab, ACCENT_ORANGE};
use crate::user::ToolboxStyle;

/// The file whose presence says the setup is done.
pub const ONBOARD_FILE: &str = "onboard.dat";

const TABS: [SideTab; 3] = [
    SideTab { title: "Project Paths", brush: theme::ACCENT_PRIMARY },
    SideTab { title: "Preferences", brush: theme::ACCENT_MUTED_TEAL },
    SideTab { title: "GPU Compute", brush: ACCENT_ORANGE },
];

/// The window at its full size.
const WINDOW_SIZE: Vec2 = Vec2::new(1240.0, 720.0);
/// The tab headers' width.
const HEADERS_WIDTH: f32 = 200.0;
/// The bottom of the window, kept for the note and the button.
const FOOTER_HEIGHT: f32 = 90.0;
/// The note beside the button.
const NOTE: &str = "You can change any choices in the Options dialog later.";

/// A `ListBox-OptionsX` frame's margin and border, both sides together.
const LIST_FRAME: Vec2 = Vec2::new(6.0, 4.0);
/// The gap between a list's items.
const LIST_GAP: f32 = 4.0;
/// The narrowest a navigation style goes beside the other.
const MIN_CHOICE_WIDTH: f32 = 160.0;
/// A toolbox style's card at its full size.
const CARD_SIZE: Vec2 = Vec2::new(340.0, 200.0);
/// The narrowest a toolbox style's card goes beside the other.
const MIN_CARD_WIDTH: f32 = 220.0;
/// The shortest a toolbox style's card gets before the page scrolls.
const MIN_CARD_HEIGHT: f32 = 160.0;

/// The folders the first page can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folder {
    Documents,
    Projects,
    Builds,
}

impl Folder {
    fn member(self) -> &'static str {
        match self {
            Folder::Documents => "Documents",
            Folder::Projects => "Projects",
            Folder::Builds => "Builds",
        }
    }

    fn dialog_key(self) -> String {
        format!("onboard-{}", self.member())
    }

    /// The folder dialog's description.
    fn description(self) -> &'static str {
        match self {
            Folder::Documents => "Select a folder to store Nerthus User Files.",
            Folder::Projects => "Select a folder to store your projects.",
            Folder::Builds => "Select a folder to store your Build Output.",
        }
    }
}

/// What the compute devices said (`Devices.txt`).
enum GpuCheck {
    Running(Receiver<String>),
    Done { usable: bool, text: String },
}

/// The window while it shows.
pub struct Onboard {
    tab: usize,
    /// `btnLetsGo`'s text.
    next_label: &'static str,
    /// "Place PROJECTS and BUILDS in different locations."
    separate_folders: bool,
    /// `lstNavStyle`: Nerthus Defaults or Industry Standard (chosen at
    /// first).
    navigation: usize,
    /// `lstToolboxStyle`: Docked Panel or Compact Buttons (chosen at
    /// first).
    toolbox: usize,
    gpu: GpuCheck,
    /// A folder under Nerthus's own, refused.
    refused: bool,
}

/// Whether the setup shows as Nerthus starts; with `--skip-onboard` it is
/// marked done instead (`App.ShowOnboarding`).
pub fn should_show(forced: bool, skipped: bool, screenshot: bool) -> bool {
    let marker = crate::user::data_folder().map(|folder| folder.join(ONBOARD_FILE));
    let done = marker.as_ref().is_some_and(|marker| marker.exists());
    if skipped {
        if !done {
            mark_done();
        }
        return false;
    }
    if forced {
        return true;
    }
    !done && !screenshot
}

/// Writes `onboard.dat`.
fn mark_done() {
    let Some(folder) = crate::user::data_folder() else {
        return;
    };
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(ONBOARD_FILE), "!"));
    if let Err(error) = written {
        log::warn!("cannot write {ONBOARD_FILE}: {error}");
    }
}

impl Onboard {
    /// `Onboard_Loaded`: the devices checked in the background.
    pub fn open(cpu_only: bool) -> Onboard {
        let gpu = if cpu_only {
            let text = "Nerthus is in CPU-only mode. You can turn it off in Options › Compute.".to_string();
            GpuCheck::Done { usable: false, text }
        } else {
            start_gpu_check()
        };
        Onboard {
            tab: 0,
            next_label: "Next: Select your Preferences",
            separate_folders: false,
            navigation: 1,
            toolbox: 1,
            gpu,
            refused: false,
        }
    }

    /// `--show onboard:<page>`: the setup on the page named (`paths`,
    /// `preferences` or `gpu`).
    pub fn open_at(page: &str) -> Onboard {
        let mut onboard = Onboard::open(nerthus_compute::is_cpu_only());
        onboard.tab = match page {
            "preferences" => 1,
            "gpu" => 2,
            _ => 0,
        };
        onboard.next_label = next_label(onboard.tab);
        onboard
    }

    fn poll_gpu(&mut self) {
        let GpuCheck::Running(receiver) = &self.gpu else {
            return;
        };
        let lines = match receiver.try_recv() {
            Ok(lines) => lines,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => String::new(),
        };
        self.gpu = gpu_answer(&lines);
    }
}

/// On a worker thread: on Windows, the NVIDIA profile when the NVIDIA
/// driver is there; then "Evaluating devices...": the devices listed. It
/// sends what they had to say about the ones they cannot use.
fn start_gpu_check() -> GpuCheck {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("onboard devices".to_string()).spawn(move || {
        if cfg!(windows) {
            log::info!("Creating NVidia profiles...");
            if let Err(nerthus_compute::ProfileError::Failed(_)) = nerthus_compute::enforce_optimal_settings() {
                log::error!("NV Failed.");
            }
        }
        log::info!("Evaluating devices...");
        let (_, problems) = nerthus_compute::available_devices();
        let _ = sender.send(problems);
    });
    match spawned {
        Ok(_) => GpuCheck::Running(receiver),
        Err(error) => GpuCheck::Done { usable: false, text: error.to_string() },
    }
}

/// No problems: every device can compute. Otherwise the last device that
/// had one is named.
fn gpu_answer(problems: &str) -> GpuCheck {
    let Some(last) = problems.lines().rev().find(|line| !line.trim().is_empty()) else {
        let text = "All devices enabled for Compute. You can edit them anytime in Options.".to_string();
        return GpuCheck::Done { usable: true, text };
    };
    let device = last.split_once(": ").map(|(device, _)| device).unwrap_or(last).trim();
    let text = format!("Your CPU or GPU drivers may be out of date. We recommend updating them before starting Nerthus.\n\nPossible driver issue in: {device}");
    GpuCheck::Done { usable: false, text }
}

/// `Tabs_OnSelectionChanged`: what the button says on each page.
fn next_label(tab: usize) -> &'static str {
    match tab {
        0 => "Next: Select your preferences",
        1 => "Next: Select your GPU options",
        _ => "Let's get started!",
    }
}

/// The window. Returns whether it is still open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) -> bool {
    onboard.poll_gpu();
    if matches!(onboard.gpu, GpuCheck::Running(_)) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    take_folder_answers(editor, onboard, dialogs);
    let mut finished = false;
    // The full window covers the editor, which waits behind it.
    let screen = ctx.content_rect();
    let mut veil = egui::Area::new(egui::Id::new("onboard-veil"));
    veil = veil.order(egui::Order::Middle);
    veil = veil.fixed_pos(screen.min);
    veil.show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(160));
    });
    let window = help_windows::dialog_window(ctx, "onboard", WINDOW_SIZE);
    window.show(ctx, |ui| {
        let area = ui.max_rect();
        ui.set_min_size(area.size());
        paint_backdrop(ui, area);
        let parts = parts(area);

        let mut welcome_ui = region(ui, "onboard-welcome", parts.welcome, Layout::top_down(Align::Min));
        welcome_ui.label(RichText::new("Welcome to Nerthus!").font(theme::interface(32.0)).color(Color32::WHITE));
        let subtitle = "Before you get started, you can quickly set up your preferred workspace.";
        welcome_ui.label(RichText::new(subtitle).font(theme::interface(14.0)).color(theme::LABEL_DEFAULT));

        // The headers and the page scroll where the window is too short
        // for them, so that they stay above the footer.
        let mut headers_ui = region(ui, "onboard-headers", parts.headers, Layout::top_down(Align::Min));
        let mut headers = egui::ScrollArea::vertical();
        headers = headers.id_salt("onboard-headers");
        headers = headers.auto_shrink([false, false]);
        if headers.show(&mut headers_ui, |ui| help_windows::side_tabs(ui, &TABS, &mut onboard.tab, HEADERS_WIDTH)).inner {
            onboard.next_label = next_label(onboard.tab);
        }
        let mut page_ui = region(ui, "onboard-page", parts.page, Layout::top_down(Align::Min));
        let mut page = egui::ScrollArea::vertical();
        // Each page keeps its own place.
        page = page.id_salt(onboard.tab);
        page = page.auto_shrink([false, false]);
        page.show(&mut page_ui, |ui| match onboard.tab {
            0 => paths_page(ui, editor, onboard, dialogs),
            1 => preferences_page(ui, onboard),
            _ => gpu_page(ui, onboard),
        });

        // The note (margin 32, 0, 0, 32) and the button (margin 0, 0, 64,
        // 48), in the footer: the note is below the button's row, so they
        // stay apart however narrow the window.
        let note_galley = ui.painter().layout_no_wrap(NOTE.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
        let note_position = egui::pos2(area.left() + 32.0, area.bottom() - 32.0 - note_galley.size().y);
        ui.painter().galley(note_position, note_galley, theme::LABEL_DEFAULT);
        let button_corner = area.right_bottom() - Vec2::new(64.0, 48.0);
        let button_rect = Rect::from_min_max(button_corner - Vec2::new(400.0, 48.0), button_corner);
        let mut button_ui = region(ui, "onboard-next", button_rect, Layout::right_to_left(Align::Center));
        let next = help_windows::important_button(&mut button_ui, onboard.next_label, 14.0, 0.0, Vec2::new(24.0, 12.0));
        if next.clicked() {
            if onboard.tab + 1 < TABS.len() {
                onboard.tab += 1;
                onboard.next_label = next_label(onboard.tab);
            } else {
                finished = true;
            }
        }
    });
    refusal(ctx, onboard);
    if finished {
        finish(editor, onboard);
        return false;
    }
    true
}

/// Where the window's parts go.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    welcome: Rect,
    headers: Rect,
    page: Rect,
}

/// The parts in `area`. In the full window they are where `Onboard` has
/// them: the welcome at margin 72, 64, and the tabs at margin 72, 160, 20,
/// 90 with the page past the headers (the tab's margin 24, the page's 8
/// and padding 8). A shorter window takes the top margin in, from 64 at
/// 700 tall to 24 at 600; one too narrow for the toolbox styles side by
/// side takes the left margin and the gap past the headers in, from 72
/// and 40 to 24 and 16 at 640 wide. The bottom 90 are the footer's.
fn parts(area: Rect) -> Parts {
    let short = ((700.0 - area.height()) / 100.0).clamp(0.0, 1.0);
    let narrow = ((1040.0 - area.width()) / 400.0).clamp(0.0, 1.0);
    let top = 64.0 - 40.0 * short;
    let left = 72.0 - 48.0 * narrow;
    let welcome = Rect::from_min_size(area.min + Vec2::new(left, top), Vec2::new((area.width() - 2.0 * left).max(0.0), 80.0));
    let body_min = egui::pos2(area.left() + left, welcome.bottom() + 16.0);
    let body_max = (area.max - Vec2::new(20.0, FOOTER_HEIGHT)).max(body_min);
    let headers = Rect::from_min_size(body_min, Vec2::new(HEADERS_WIDTH, body_max.y - body_min.y));
    let page_min = egui::pos2(headers.right() + 40.0 - 24.0 * narrow, body_min.y + 8.0);
    let page = Rect::from_min_max(page_min, (body_max - Vec2::splat(8.0)).max(page_min));
    Parts { welcome, headers, page }
}

/// A part of the window laid out on its own and kept to its place: what
/// does not fit is cut off at its edge.
fn region(ui: &mut Ui, id: &str, rect: Rect, layout: Layout) -> Ui {
    let builder = egui::UiBuilder::new().id_salt(id).max_rect(rect).layout(layout);
    let mut region = ui.new_child(builder);
    region.set_clip_rect(rect.intersect(ui.clip_rect()));
    region
}

/// The faint logo behind the pages (at 5% opacity, running off the bottom
/// right).
fn paint_backdrop(ui: &Ui, area: Rect) {
    let logo_corner = area.min + Vec2::new(area.width() * 0.45, 95.0);
    let logo_rect = Rect::from_min_size(logo_corner, Vec2::splat(area.height() * 1.1));
    let logo = egui::Image::new(egui::include_image!("../../assets/nerthus.svg"));
    let faded = logo.tint(Color32::from_white_alpha(13));
    faded.paint_at(ui, logo_rect);
}

/// A path as the link buttons show it (`Button-Link`, monospace 16).
fn folder_link(ui: &mut Ui, path: &Path, color: Color32) -> bool {
    let text = RichText::new(path.display().to_string()).font(theme::monospace(16.0)).color(color);
    let response = ui.add(egui::Label::new(text).sense(Sense::click()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A paragraph under a setting (13 px, margin 24, 12, 360, 0), at least
/// 240 wide where the page has room for it.
fn paragraph(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        let width = (ui.available_width() - 360.0).max(240.0).min(ui.available_width());
        ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.add(egui::Label::new(RichText::new(text).font(theme::interface(13.0)).color(theme::LABEL_DEFAULT)).wrap());
        });
    });
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::interface(14.0)).color(theme::LABEL_DEFAULT));
}

/// Project Paths.
fn paths_page(ui: &mut Ui, editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) {
    let preferences = &editor.user.preferences;
    let documents = preferences.documents_folder();
    let projects = preferences.projects_folder();
    let builds = preferences.builds_folder();
    heading(ui, "Nerthus Storage Folder");
    if folder_link(ui, &documents, theme::ACCENT_MUTED_GREEN) {
        ask_folder(dialogs, Folder::Documents, &documents);
    }
    paragraph(ui, "This is where all Presets, Screenshots, and other Nerthus documents are stored.");
    ui.add_space(24.0);
    let label = RichText::new("Place PROJECTS and BUILDS in different locations.").font(theme::interface(14.0));
    ui.checkbox(&mut onboard.separate_folders, label);
    if onboard.separate_folders {
        ui.add_space(36.0);
        heading(ui, "Projects");
        if folder_link(ui, &projects, theme::ACCENT_MUTED_BLUE) {
            ask_folder(dialogs, Folder::Projects, &projects);
        }
        paragraph(ui, "This is where all Nerthus project files are stored by default.");
        ui.add_space(36.0);
        heading(ui, "Build Output");
        if folder_link(ui, &builds, theme::ACCENT_GRAY) {
            ask_folder(dialogs, Folder::Builds, &builds);
        }
    }
    paragraph(ui, "Projects are usually small, lightweight files. However, Builds are very large and heavy. Keeping them separate is recommended if you frequently backup either folder.");
}

fn ask_folder(dialogs: &mut FileDialogs, folder: Folder, current: &Path) {
    let request = DialogRequest {
        key: folder.dialog_key(),
        title: folder.description().to_string(),
        kind: DialogKind::PickFolder,
        directory: Some(current.to_path_buf()),
    };
    dialogs.ask(request);
}

/// The folders picked: refused under Nerthus's own folder; the storage
/// folder takes `Projects` and `Builds` with it unless they are kept apart
/// (`BtnFolder_OnClick`).
fn take_folder_answers(editor: &mut Editor, onboard: &mut Onboard, dialogs: &mut FileDialogs) {
    for folder in [Folder::Documents, Folder::Projects, Folder::Builds] {
        let Some(Some(picked)) = dialogs.take(&folder.dialog_key()) else {
            continue;
        };
        if under_installation(&picked) {
            onboard.refused = true;
            continue;
        }
        let preferences = &mut editor.user.preferences;
        preferences.set_member("Locations", folder.member(), path_value(&picked));
        if folder == Folder::Documents && !onboard.separate_folders {
            preferences.set_member("Locations", "Projects", path_value(&picked.join("Projects")));
            preferences.set_member("Locations", "Builds", path_value(&picked.join("Builds")));
        }
    }
}

fn path_value(path: &Path) -> Value {
    json!(path.to_string_lossy())
}

/// Whether the folder is in Nerthus's installation folder (the
/// executable's), compared without case.
fn under_installation(picked: &Path) -> bool {
    let Ok(program) = std::env::current_exe() else {
        return false;
    };
    let Some(installation) = program.parent() else {
        return false;
    };
    let picked_text = picked.to_string_lossy().to_lowercase();
    let installation_text = installation.to_string_lossy().to_lowercase();
    picked_text.starts_with(&installation_text)
}

/// "Invalid Location".
fn refusal(ctx: &egui::Context, onboard: &mut Onboard) {
    if !onboard.refused {
        return;
    }
    let modal = crate::ui::prompt::TaskDialog::new(ctx, egui::Id::new("onboard-invalid-location"), 380.0);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, "Invalid Location", false, |ui| {
            ui.add(egui::Label::new("You cannot place Nerthus Projects or User Files under the Nerthus Installation Folder.").wrap());
        });
        task.buttons(ui, |ui| {
            if crate::ui::prompt::task_button(ui, "OK").clicked() {
                onboard.refused = false;
            }
        });
    });
    if response.should_close() {
        onboard.refused = false;
    }
}

/// A `ListBox-OptionsX` item `width` wide: a bold first line and a plain
/// second, centred and wrapped in it, the selected one on the selection
/// brush. It is 56 tall, or taller for lines that wrap.
fn option_item(ui: &mut Ui, width: f32, title: &str, detail: &str, selected: bool) -> egui::Response {
    let wrap_width = (width - 16.0).max(0.0);
    let title_galley = centred_text(ui, title, theme::semibold(14.0), wrap_width);
    let detail_galley = centred_text(ui, detail, theme::interface(14.0), wrap_width);
    let height = title_galley.size().y + detail_galley.size().y;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, (height + 16.0).max(56.0)), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
    }
    let color = if selected || response.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
    let top = rect.center().y - height / 2.0;
    let title_height = title_galley.size().y;
    ui.painter().galley(egui::pos2(rect.center().x, top), title_galley, color);
    ui.painter().galley(egui::pos2(rect.center().x, top + title_height), detail_galley, color);
    response
}

/// Text wrapped at `width` with each line centred on where it is drawn,
/// in the colour it is drawn with.
fn centred_text(ui: &Ui, text: &str, font: FontId, width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, Color32::PLACEHOLDER, width);
    job.halign = Align::Center;
    ui.painter().layout_job(job)
}

/// Preferences: the navigation and toolbox styles.
fn preferences_page(ui: &mut Ui, onboard: &mut Onboard) {
    heading(ui, "Navigation Style");
    ui.add_space(8.0);
    let navigation_choices = [
        ("Nerthus Defaults", "LMB Orbit / MMB Pan / RMB Zoom"),
        ("Industry Standard", "Alt+LMB Orbit / Alt+MMB Pan / Alt+RMB Zoom"),
    ];
    let (beside, width) = list_items(ui.available_width().min(840.0), MIN_CHOICE_WIDTH, f32::INFINITY);
    list_frame(ui, beside, |ui| {
        for (index, (title, detail)) in navigation_choices.iter().enumerate() {
            if option_item(ui, width, title, detail, onboard.navigation == index).clicked() {
                onboard.navigation = index;
            }
        }
    });
    ui.add_space(24.0);
    heading(ui, "Toolbox Style");
    ui.add_space(8.0);
    let (beside, width) = list_items(ui.available_width(), MIN_CARD_WIDTH, CARD_SIZE.x);
    // In a page a little too short for them the cards give up some of
    // their height before it has to scroll. What is above them measures
    // the same wherever the page is scrolled to.
    let above = ui.cursor().top() - ui.min_rect().top();
    let rows = if beside { 1.0 } else { 2.0 };
    let room = (ui.clip_rect().height() - above - LIST_FRAME.y - LIST_GAP * (rows - 1.0)) / rows;
    let size = Vec2::new(width, room.clamp(MIN_CARD_HEIGHT, CARD_SIZE.y));
    list_frame(ui, beside, |ui| {
        let styles = [
            ("Docked Panel", "Recommended for Beginners", ToolboxStyle::Panel),
            ("Compact Buttons", "For experts and reduced UI space", ToolboxStyle::Menus),
        ];
        for (index, (title, detail, style)) in styles.into_iter().enumerate() {
            if toolbox_item(ui, size, title, detail, style, onboard.toolbox == index).clicked() {
                onboard.toolbox = index;
            }
        }
    });
}

/// How two items share a list `width` wide: whether they go side by side,
/// which they do if each gets at least `min`, or one over the other; and
/// how wide each is, at most `max`.
fn list_items(width: f32, min: f32, max: f32) -> (bool, f32) {
    let inside = (width - LIST_FRAME.x).max(0.0);
    let half = (inside - LIST_GAP) / 2.0;
    if half >= min { (true, half.min(max)) } else { (false, inside.min(max)) }
}

/// `ListBox-OptionsX`'s box: its items in a row, or one over the other.
fn list_frame(ui: &mut Ui, beside: bool, contents: impl FnOnce(&mut Ui)) {
    let mut frame = egui::Frame::new();
    frame = frame.fill(theme::OPTIONS_BACKGROUND);
    frame = frame.stroke(Stroke::new(1.0, theme::OPTIONS_BORDER));
    frame = frame.corner_radius(4.0);
    frame = frame.inner_margin(egui::Margin::symmetric(2, 1));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(LIST_GAP);
        if beside {
            ui.horizontal(contents);
        } else {
            ui.vertical(contents);
        }
    });
}

/// A toolbox style's card (340 × 200 at most): its name over a sketch of
/// it, bordered in teal when chosen. The name wraps in a narrow card,
/// pushing the sketch down.
fn toolbox_item(ui: &mut Ui, size: Vec2, title: &str, detail: &str, style: ToolboxStyle, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let border = if selected { theme::ACCENT_TEAL } else { Color32::TRANSPARENT };
    if response.hovered() && !selected {
        ui.painter().rect_filled(rect, 4.0, theme::OPTIONS_SELECTION.gamma_multiply(0.4));
    }
    ui.painter().rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, border), egui::StrokeKind::Inside);
    let color = if selected || response.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
    let wrap_width = (size.x - 20.0).max(0.0);
    let title_galley = ui.painter().layout(title.to_string(), theme::semibold(14.0), Color32::PLACEHOLDER, wrap_width);
    let detail_galley = ui.painter().layout(detail.to_string(), theme::interface(14.0), Color32::PLACEHOLDER, wrap_width);
    let text_left = rect.left() + 10.0;
    let detail_top = rect.top() + 10.0 + title_galley.size().y;
    let text_bottom = detail_top + detail_galley.size().y;
    ui.painter().galley(egui::pos2(text_left, rect.top() + 10.0), title_galley, color);
    ui.painter().galley(egui::pos2(text_left, detail_top), detail_galley, color);
    let sketch_top = (rect.top() + 60.0).max(text_bottom + 16.0);
    let sketch = Rect::from_min_max(egui::pos2(rect.left() + 10.0, sketch_top), rect.max - Vec2::splat(10.0));
    paint_toolbox_sketch(ui, sketch, style);
    response
}

/// A sketch of the graph area with the toolbox docked as a panel of node
/// rows, or as a strip of buttons.
fn paint_toolbox_sketch(ui: &Ui, rect: Rect, style: ToolboxStyle) {
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::WINDOW_BACKGROUND);
    let line = Stroke::new(1.0, theme::BUTTON_BORDER);
    match style {
        ToolboxStyle::Panel => {
            let panel = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(90.0, rect.height() - 12.0));
            painter.rect_filled(panel, 2.0, theme::PANEL_BACKGROUND);
            for row in 0..8 {
                let top = panel.top() + 6.0 + row as f32 * 15.0;
                if top + 9.0 > panel.bottom() {
                    break;
                }
                let swatch = Rect::from_min_size(egui::pos2(panel.left() + 6.0, top), Vec2::splat(9.0));
                painter.rect_filled(swatch, 2.0, theme::toolbox_color(TOOLBOX_SKETCH[row % TOOLBOX_SKETCH.len()]));
                let bar = Rect::from_min_size(egui::pos2(swatch.right() + 6.0, top + 2.0), Vec2::new(54.0, 5.0));
                painter.rect_filled(bar, 2.0, theme::LABEL_DEFAULT.gamma_multiply(0.5));
            }
            let node = Rect::from_center_size(egui::pos2(rect.center().x + 50.0, rect.center().y), Vec2::new(60.0, 36.0));
            painter.rect_stroke(node, 3.0, line, egui::StrokeKind::Inside);
        }
        ToolboxStyle::Menus => {
            let strip = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(rect.width() - 12.0, 22.0));
            painter.rect_filled(strip, 2.0, theme::PANEL_BACKGROUND);
            for button in 0..10 {
                let left = strip.left() + 6.0 + button as f32 * 20.0;
                if left + 14.0 > strip.right() {
                    break;
                }
                let icon = Rect::from_min_size(egui::pos2(left, strip.top() + 4.0), Vec2::splat(14.0));
                painter.rect_filled(icon, 3.0, theme::toolbox_color(TOOLBOX_SKETCH[button % TOOLBOX_SKETCH.len()]));
            }
            let node = Rect::from_center_size(egui::pos2(rect.center().x, rect.center().y + 14.0), Vec2::new(60.0, 36.0));
            painter.rect_stroke(node, 3.0, line, egui::StrokeKind::Inside);
        }
    }
}

/// Toolbox categories whose colours the sketches use.
const TOOLBOX_SKETCH: [&str; 5] = ["Primitive", "Terrain", "Simulate", "Surface", "Data"];

/// GPU Compute: what the devices said; the box is for show.
fn gpu_page(ui: &mut Ui, onboard: &Onboard) {
    let (usable, text) = match &onboard.gpu {
        GpuCheck::Running(_) => (false, "Determining hardware..."),
        GpuCheck::Done { usable, text } => (*usable, text.as_str()),
    };
    let mut checked = usable;
    let label = RichText::new("Use GPU for compute").font(theme::interface(14.0));
    let checkbox = egui::Checkbox::new(&mut checked, label);
    ui.add_enabled(false, checkbox);
    ui.add_space(24.0);
    ui.add(egui::Label::new(RichText::new(text).font(theme::interface(14.0)).color(theme::ACCENT_GRAY)).wrap());
}

/// `btnLetsGo_Click` on the last page: Industry Standard turns on the Alt
/// navigation, Docked Panel the panel toolbox; the settings are saved and
/// `onboard.dat` written.
fn finish(editor: &mut Editor, onboard: &Onboard) {
    let options = &mut editor.user.options;
    if onboard.navigation == 1 {
        options.set_value("AltPan", Value::Bool(true));
        options.set_value("AltZoom", Value::Bool(true));
        options.set_value("AltRotate", Value::Bool(true));
    }
    if onboard.toolbox == 0 {
        options.set_toolbox_style(ToolboxStyle::Panel);
    }
    options.mark_changed();
    editor.user.preferences.mark_changed();
    editor.user.save_if_changed();
    mark_done();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::ui::{status_bar, title_bar};
    use egui::epaint::ClippedShape;
    use egui::{AreaState, Event, Id, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, Shape, TouchPhase};

    #[test]
    fn devices_without_problems_are_all_usable() {
        let answer = gpu_answer("");
        assert!(matches!(answer, GpuCheck::Done { usable: true, .. }));
    }

    #[test]
    fn the_last_problem_names_its_device() {
        let answer = gpu_answer("A (Vulkan): too old\nB (Metal): no float atomics\n");
        let GpuCheck::Done { usable, text } = answer else {
            panic!("still running");
        };
        assert!(!usable);
        assert!(text.ends_with("Possible driver issue in: B (Metal)"));
    }

    #[test]
    fn the_button_reads_as_the_page_needs() {
        assert_eq!(next_label(0), "Next: Select your preferences");
        assert_eq!(next_label(2), "Let's get started!");
    }

    /// The screen, in points, of 1366 × 768, 1280 × 720 (or 1920 × 1080
    /// at 150%), 1024 × 768, 800 × 600 and 640 × 480 at the default 120%.
    const SMALL_SCREENS: [Vec2; 5] = [
        Vec2::new(1138.0, 640.0),
        Vec2::new(1066.0, 600.0),
        Vec2::new(853.0, 640.0),
        Vec2::new(666.0, 500.0),
        Vec2::new(533.0, 400.0),
    ];
    /// 1600 × 960 at 120%, where the window has its full size.
    const FULL_SCREEN: Vec2 = Vec2::new(1333.0, 800.0);
    /// A folder about 110 characters long.
    const LONG_FOLDER: &str = "/home/someone/Documents/Studio Work/Clients/Northern Range Survey/Terrain Sources/Heightfield Tiles/2026 Q4";

    fn context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        egui_extras::install_image_loaders(&context);
        context
    }

    /// The setup on this page, with PROJECTS and BUILDS apart in long
    /// folders when `long_folders`.
    fn setup(tab: usize, long_folders: bool) -> (Editor, Onboard) {
        let mut editor = Editor::new(Document::new());
        let mut onboard = Onboard::open(true);
        onboard.tab = tab;
        onboard.next_label = next_label(tab);
        if long_folders {
            onboard.separate_folders = true;
            let preferences = &mut editor.user.preferences;
            preferences.set_member("Locations", "Projects", json!(format!("{LONG_FOLDER}/Projects")));
            preferences.set_member("Locations", "Builds", json!(format!("{LONG_FOLDER}/Builds")));
        }
        (editor, onboard)
    }

    /// A frame at this screen size with these events. Returns what was
    /// painted and the window's rect.
    fn frame(context: &egui::Context, screen: Vec2, editor: &mut Editor, onboard: &mut Onboard, events: Vec<Event>) -> (Vec<ClippedShape>, Rect) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
        let mut dialogs = FileDialogs::default();
        let mut output = context.run_ui(input, |ui| {
            show(ui.ctx(), editor, onboard, &mut dialogs);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let rect = AreaState::load(context, Id::new("onboard")).filter(|area| area.size.is_some()).map(|area| area.rect());
        (shapes, rect.unwrap_or(Rect::NOTHING))
    }

    /// Frames enough for egui's first-frame sizing to settle and the
    /// window to fade in.
    fn settle(context: &egui::Context, screen: Vec2, editor: &mut Editor, onboard: &mut Onboard) -> (Vec<ClippedShape>, Rect) {
        for _ in 0..20 {
            frame(context, screen, editor, onboard, Vec::new());
        }
        frame(context, screen, editor, onboard, Vec::new())
    }

    /// Each shape painted, nested ones on their own, with the part of it
    /// that shows. Shapes that do not show are left out.
    fn shown(shapes: &[ClippedShape]) -> Vec<(Rect, &Shape)> {
        let mut shown = Vec::new();
        for clipped in shapes {
            let mut stack = vec![&clipped.shape];
            while let Some(shape) = stack.pop() {
                if let Shape::Vec(inner) = shape {
                    stack.extend(inner);
                    continue;
                }
                let rect = shape.visual_bounding_rect().intersect(clipped.clip_rect);
                if rect.is_positive() {
                    shown.push((rect, shape));
                }
            }
        }
        shown
    }

    fn text(shape: &Shape) -> Option<&str> {
        match shape {
            Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        }
    }

    /// Whether two rects share more than a sliver.
    fn overlap(a: Rect, b: Rect) -> bool {
        a.intersect(b).shrink(0.5).is_positive()
    }

    /// The Next button's rect: the smallest painted round its label.
    fn button(shown: &[(Rect, &Shape)], label: &str, window: Rect) -> Rect {
        let label_rect = shown.iter().find(|(_, shape)| text(shape) == Some(label)).map(|(rect, _)| *rect).expect("no Next button");
        let around = shown.iter().filter_map(|(_, shape)| match shape {
            Shape::Rect(rect) if rect.rect.contains_rect(label_rect) && !rect.rect.contains_rect(window.shrink(2.0)) => Some(rect.rect),
            _ => None,
        });
        around.min_by(|a, b| a.area().total_cmp(&b.area())).expect("no button round the Next label")
    }

    /// What the window shows stays inside it, the button and the note
    /// cover nothing else, and no text runs into other text.
    fn assert_laid_out(shapes: &[ClippedShape], window: Rect, label: &str, what: &str) {
        let shown = shown(shapes);
        let button = button(&shown, label, window);
        let note = shown.iter().find(|(_, shape)| text(shape) == Some(NOTE)).map(|(rect, _)| *rect).expect("no note");
        // The backdrop is an image; the veil, the shadow and the frame
        // cover the window.
        let image = |shape: &Shape| matches!(shape, Shape::Mesh(_)) || matches!(shape, Shape::Rect(rect) if rect.brush.is_some());
        let contents = shown.iter().filter(|(rect, shape)| !image(shape) && !rect.contains_rect(window.shrink(2.0)));
        let contents: Vec<_> = contents.collect();
        for (rect, shape) in &contents {
            assert!(window.expand(0.5).contains_rect(*rect), "{what}: {shape:?} at {rect:?} is outside the window {window:?}");
            let footer = text(shape) == Some(NOTE) || text(shape) == Some(label) || matches!(shape, Shape::Rect(own) if own.rect == button);
            if !footer {
                assert!(!overlap(*rect, button), "{what}: {shape:?} at {rect:?} is under the Next button {button:?}");
                assert!(!overlap(*rect, note), "{what}: {shape:?} at {rect:?} is under the note {note:?}");
            }
        }
        let texts: Vec<_> = contents.iter().filter_map(|(rect, shape)| text(shape).map(|text| (*rect, text))).collect();
        for (index, (rect, text)) in texts.iter().enumerate() {
            for (other_rect, other) in &texts[index + 1..] {
                assert!(!overlap(*rect, *other_rect), "{what}: {text:?} at {rect:?} runs into {other:?} at {other_rect:?}");
            }
        }
    }

    /// The page in the window (inside its border, 1 wide).
    fn page(window: Rect) -> Rect {
        parts(window.shrink(1.0)).page
    }

    /// Whether the page's last item shows whole: the paragraph under the
    /// folders, the Compact Buttons sketch (the last sketch drawn), or what
    /// the devices said.
    fn end_shows(shapes: &[ClippedShape], window: Rect, tab: usize) -> bool {
        let page = page(window);
        let held = shapes.iter().filter(|clipped| page.expand(1.0).contains_rect(clipped.clip_rect));
        let mut last = held.filter(|clipped| match (&clipped.shape, tab) {
            (Shape::Text(text), 0) => text.galley.text().starts_with("Projects are usually small"),
            (Shape::Rect(rect), 1) => rect.fill == theme::WINDOW_BACKGROUND,
            (Shape::Text(text), 2) => text.galley.text().starts_with("Nerthus is in CPU-only mode"),
            _ => false,
        });
        last.next_back().is_some_and(|clipped| clipped.clip_rect.expand(0.5).contains_rect(clipped.shape.visual_bounding_rect()))
    }

    #[test]
    fn the_full_window_keeps_its_layout() {
        // Inside the window's border.
        let area = Rect::from_min_size(Pos2::ZERO, WINDOW_SIZE - Vec2::splat(2.0));
        let parts = parts(area);
        assert_eq!(parts.welcome, Rect::from_min_size(Pos2::new(72.0, 64.0), Vec2::new(1094.0, 80.0)));
        assert_eq!(parts.headers, Rect::from_min_max(Pos2::new(72.0, 160.0), Pos2::new(272.0, 628.0)));
        assert_eq!(parts.page, Rect::from_min_max(Pos2::new(312.0, 168.0), Pos2::new(1210.0, 620.0)));
    }

    #[test]
    fn the_window_keeps_clear_of_the_title_bar() {
        for screen in SMALL_SCREENS.into_iter().chain([FULL_SCREEN]) {
            let context = context();
            let (mut editor, mut onboard) = setup(0, false);
            let (_, window) = settle(&context, screen, &mut editor, &mut onboard);
            let body = Rect::from_min_max(Pos2::new(0.0, title_bar::HEIGHT), Pos2::new(screen.x, screen.y - status_bar::HEIGHT));
            assert!(body.expand(0.5).contains_rect(window), "the window at {window:?} leaves the body {body:?} at {screen:?}");
        }
    }

    #[test]
    fn nothing_runs_under_the_footer_or_out_of_the_window() {
        let pages = [(0, false), (0, true), (1, false), (2, false)];
        for screen in SMALL_SCREENS.into_iter().chain([FULL_SCREEN]) {
            for (tab, long_folders) in pages {
                let context = context();
                let (mut editor, mut onboard) = setup(tab, long_folders);
                let (shapes, window) = settle(&context, screen, &mut editor, &mut onboard);
                assert!(window.is_positive(), "no window at {screen:?}");
                let what = format!("page {tab} (long folders {long_folders}) at {screen:?}");
                assert_laid_out(&shapes, window, onboard.next_label, &what);
            }
        }
    }

    #[test]
    fn laptop_screens_show_every_page_whole() {
        // 1366 × 768 and 1280 × 720.
        for screen in &SMALL_SCREENS[..2] {
            for tab in 0..TABS.len() {
                let context = context();
                let (mut editor, mut onboard) = setup(tab, false);
                let (shapes, window) = settle(&context, *screen, &mut editor, &mut onboard);
                assert!(end_shows(&shapes, window, tab), "page {tab} at {screen:?} has to scroll");
            }
            // The styles side by side.
            let context = context();
            let (mut editor, mut onboard) = setup(1, false);
            let (shapes, _) = settle(&context, *screen, &mut editor, &mut onboard);
            let shown = shown(&shapes);
            let top = |name: &str| shown.iter().find(|(_, shape)| text(shape) == Some(name)).map(|(rect, _)| rect.top()).expect(name);
            assert!((top("Docked Panel") - top("Compact Buttons")).abs() < 1.0);
            assert!((top("Nerthus Defaults") - top("Industry Standard")).abs() < 1.0);
        }
    }

    #[test]
    fn next_is_in_reach_on_small_screens() {
        for screen in SMALL_SCREENS {
            let context = context();
            let (mut editor, mut onboard) = setup(0, false);
            // Only the first two pages: on the last, Next finishes and
            // writes the settings.
            for expected in [1, 2] {
                let (shapes, window) = settle(&context, screen, &mut editor, &mut onboard);
                let position = button(&shown(&shapes), onboard.next_label, window).center();
                let press = |pressed| Event::PointerButton { pos: position, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
                frame(&context, screen, &mut editor, &mut onboard, vec![Event::PointerMoved(position)]);
                frame(&context, screen, &mut editor, &mut onboard, vec![press(true)]);
                frame(&context, screen, &mut editor, &mut onboard, vec![press(false)]);
                assert_eq!(onboard.tab, expected, "Next at {screen:?}");
            }
        }
    }

    #[test]
    fn each_page_scrolls_to_its_end_on_the_smallest_screen() {
        let screen = SMALL_SCREENS[4];
        for tab in 0..TABS.len() {
            let context = context();
            let (mut editor, mut onboard) = setup(tab, true);
            let (shapes, window) = settle(&context, screen, &mut editor, &mut onboard);
            assert!(!end_shows(&shapes, window, tab) || tab == 2, "page {tab} fits at {screen:?} without scrolling");
            let page = page(window);
            let wheel = Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: TouchPhase::Move, modifiers: Modifiers::NONE };
            frame(&context, screen, &mut editor, &mut onboard, vec![Event::PointerMoved(page.center())]);
            frame(&context, screen, &mut editor, &mut onboard, vec![wheel]);
            for _ in 0..60 {
                frame(&context, screen, &mut editor, &mut onboard, Vec::new());
            }
            let (shapes, _) = frame(&context, screen, &mut editor, &mut onboard, Vec::new());
            assert!(end_shows(&shapes, window, tab), "page {tab} does not scroll to its end");
            assert_laid_out(&shapes, window, onboard.next_label, &format!("page {tab} scrolled"));
        }
    }
}
