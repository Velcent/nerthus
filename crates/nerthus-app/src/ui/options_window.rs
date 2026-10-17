//! Options (`OptionsWindow`, Tools › Options…, Ctrl Alt O): the pages in
//! a list on the left (`TabList`) and the page on the right, with Reset
//! this page, Accept Changes and Cancel below; Esc is Cancel. In a body
//! smaller than the window it shrinks to fit, the list and the page
//! scrolling, so the title bar and the buttons stay in reach. The pages
//! other than Compute are in [`super::options_pages`].
//!
//! Compute (`ComputeTab`) lists the devices Nerthus can compute on, a group
//! per device with a check box per backend. Ticking a box activates the
//! backend at once (`Sycl.Device.TryActivate`) and unticking deactivates
//! it; Accept Changes keeps the active ones as the preferences'
//! `Compute.Backends`, which load as Nerthus starts. The CPU is always on.
//! In CPU-only mode the page offers to leave it on the next start.

use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Align, Color32, Id, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, Ui, Vec2};
use nerthus_compute::{DeviceInfo, DeviceType, ProfileError, Scores};

use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::file_dialogs::FileDialogs;
use crate::ui::options_pages::{self, OptionsCopy, PageRequest};
use crate::ui::tool_window;
use crate::ui::widgets::Tooltips;
use crate::user;

/// `OptionsWindow`'s pages (Network Builds stays collapsed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsTab {
    General,
    Paths,
    Toolbox,
    Graph,
    Node,
    DataView,
    Build,
    Cache,
    Viewport,
    Navigation,
    LazyMenu,
    Compute,
    Experimental,
}

/// Each page, its header, and the brush that marks it selected (its
/// group's); groups start 24 below the one before.
const TABS: [(OptionsTab, &str, Color32, bool); 13] = [
    (OptionsTab::General, "General", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::Paths, "Paths", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::Toolbox, "Toolbox", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::Graph, "Graph", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::Node, "Node", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::DataView, "Data View", theme::ACCENT_MUTED_PURPLE, false),
    (OptionsTab::Build, "Build", theme::ACCENT_TRANSPARENT_GREEN, true),
    (OptionsTab::Cache, "Cache", theme::ACCENT_TRANSPARENT_GREEN, false),
    (OptionsTab::Viewport, "Viewport", theme::ACCENT_TRANSPARENT_BLUE, true),
    (OptionsTab::Navigation, "Navigation", theme::ACCENT_TRANSPARENT_BLUE, false),
    (OptionsTab::LazyMenu, "Lazy Menu", theme::ACCENT_TRANSPARENT_BLUE, false),
    (OptionsTab::Compute, "Compute", theme::ACCENT_TRANSPARENT_TEAL, true),
    (OptionsTab::Experimental, "Experimental", theme::ACCENT_TRANSPARENT_TEAL, false),
];

impl OptionsTab {
    /// The page whose header this is, ignoring case and spaces (`--show
    /// options:data-view`).
    pub fn from_name(name: &str) -> Option<OptionsTab> {
        let wanted: String = name.chars().filter(|character| character.is_alphanumeric()).collect();
        let found = TABS.iter().find(|(_, header, _, _)| header.replace(' ', "").eq_ignore_ascii_case(&wanted));
        found.map(|(tab, _, _, _)| *tab)
    }
}

/// `OptionsWindow`'s size before the interface scale, its title bar and
/// frame included. It gets as much of it as the shell's body has room
/// for, and the page list and the page scroll in what is left.
const WINDOW_SIZE: Vec2 = Vec2::new(854.0, 686.0);
/// `TabList` headers' `MinWidth`.
const TAB_WIDTH: f32 = 128.0;
/// The footer under the page list and the page: the gap above its
/// buttons, the buttons, and the gap below.
const FOOTER_HEIGHT: f32 = 44.0;

/// A message box (`MBox`) the page raised.
struct Notice {
    title: &'static str,
    text: String,
}

/// What the benchmark's worker reports.
enum BenchmarkReport {
    Line(String),
    Finished,
}

/// The Compute page while the window is open.
struct ComputePage {
    /// The devices the libraries report (`Sycl.GetAvailableDevices`),
    /// none until found.
    devices: Option<Vec<DeviceInfo>>,
    finding: Option<Receiver<(Vec<DeviceInfo>, String)>>,
    scores: Scores,
    /// `consoleXPU`.
    console: String,
    benchmark: Option<Receiver<BenchmarkReport>>,
    /// A documentation page asked for.
    docs: Option<&'static str>,
    /// "Enforce Optimal Settings" making the NVIDIA profile.
    enforcing: Option<Receiver<Result<Vec<String>, ProfileError>>>,
}

impl ComputePage {
    fn open() -> ComputePage {
        let mut page = ComputePage {
            devices: None,
            finding: None,
            scores: Scores::new(),
            console: String::new(),
            benchmark: None,
            docs: None,
            enforcing: None,
        };
        page.load();
        page
    }

    /// `LoadBackends`: the devices (found on a worker)
    /// and their saved scores.
    fn load(&mut self) {
        self.scores = user::data_folder().map(|folder| nerthus_compute::read_scores(&folder)).unwrap_or_default();
        if nerthus_compute::is_cpu_only() {
            return;
        }
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("find devices".to_string()).spawn(move || {
            let _ = sender.send(nerthus_compute::available_devices());
        });
        match spawned {
            Ok(_) => self.finding = Some(receiver),
            Err(error) => self.console = format!("Failed to get CPU and/or GPU devices: {error}"),
        }
    }

    /// Takes in what the workers sent since the last frame.
    fn poll(&mut self) {
        if let Some(finding) = &self.finding {
            match finding.try_recv() {
                Ok((devices, problems)) => {
                    if !problems.is_empty() {
                        self.console = problems;
                    }
                    self.devices = Some(devices);
                    self.finding = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.finding = None,
            }
        }
        let Some(benchmark) = &self.benchmark else {
            return;
        };
        let mut finished = false;
        for report in benchmark.try_iter() {
            match report {
                BenchmarkReport::Line(line) => {
                    self.console.push_str(&line);
                    self.console.push('\n');
                }
                BenchmarkReport::Finished => finished = true,
            }
        }
        if finished {
            self.benchmark = None;
            self.console.push_str("Benchmark completed.");
            self.load();
        }
    }

    /// `BtnEnforce_OnClick`: the NVIDIA profile made on a worker.
    fn start_enforcing(&mut self) {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("nvidia profile".to_string()).spawn(move || {
            let _ = sender.send(nerthus_compute::enforce_optimal_settings());
        });
        match spawned {
            Ok(_) => self.enforcing = Some(receiver),
            Err(error) => log::error!("Cannot make the NVIDIA profile: {error}"),
        }
    }

    /// What making the NVIDIA profile came to, once it has.
    fn poll_enforcing(&mut self) -> Option<Notice> {
        let receiver = self.enforcing.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(ProfileError::NoDriver),
        };
        self.enforcing = None;
        let notice = match result {
            Ok(_) => {
                let text = "An optimal settings profile has been created for Nerthus and NVidia devices. You may need to restart Nerthus for it to take effect.";
                Notice { title: "Optimal Settings Enforced", text: text.to_string() }
            }
            Err(_) => {
                let text = "Optimal settings could not set. Please try again later, or contact Tech Support.";
                Notice { title: "Nerthus", text: text.to_string() }
            }
        };
        Some(notice)
    }

    /// `BtnBenchmark_OnClick`: scores every device on a worker and saves
    /// them.
    fn start_benchmark(&mut self) {
        self.console = "Starting benchmark...\n".to_string();
        let (sender, receiver) = crossbeam_channel::unbounded();
        let spawned = std::thread::Builder::new().name("benchmark".to_string()).spawn(move || {
            let report = |line: String| {
                let _ = sender.send(BenchmarkReport::Line(line));
            };
            let scores = nerthus_compute::scan(report);
            if let Some(folder) = user::data_folder()
                && let Err(error) = nerthus_compute::write_scores(&folder, &scores)
            {
                let _ = sender.send(BenchmarkReport::Line(format!("Cannot save the scores: {error}")));
            }
            let _ = sender.send(BenchmarkReport::Finished);
        });
        match spawned {
            Ok(_) => self.benchmark = Some(receiver),
            Err(error) => self.console.push_str(&format!("Cannot start the benchmark: {error}")),
        }
    }
}

/// The window's state while it is open.
pub struct OptionsState {
    pub tab: OptionsTab,
    /// The selection last shown: reveal a newly opened or externally
    /// selected page once, while leaving manual scrolling alone.
    tab_seen: Option<OptionsTab>,
    compute: ComputePage,
    notice: Option<Notice>,
    /// The other pages' copies of the settings, taken as the window first
    /// shows (`LoadSettings`).
    copy: Option<OptionsCopy>,
    /// Reset this page waiting for Yes or No.
    resetting: bool,
}

impl OptionsState {
    pub fn new(tab: OptionsTab) -> OptionsState {
        OptionsState {
            tab,
            tab_seen: None,
            compute: ComputePage::open(),
            notice: None,
            copy: None,
            resetting: false,
        }
    }

    /// A factory reset supersedes edits made before it. The next frame
    /// loads the defaults and then follows the first-run setup's choices.
    pub fn reload_settings(&mut self) {
        self.copy = None;
        self.resetting = false;
    }
}

/// What the window did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OptionsOutcome {
    /// Accept Changes kept the pages' settings.
    pub accepted: bool,
    /// General's Run Diagnostics or Factory Reset.
    pub run_diagnostics: bool,
    /// A documentation page to open.
    pub docs: Option<&'static str>,
}

/// The page list: headers right-aligned, the selected one on its group's
/// brush.
fn tab_list(ui: &mut Ui, selected: &mut OptionsTab, reveal_selected: bool) {
    ui.vertical(|ui| {
        ui.set_width(TAB_WIDTH);
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut group_gap = false;
        for (tab, name, brush, starts_group) in TABS {
            group_gap |= starts_group;
            // The web build has no folders, and computes on the CPU alone.
            if cfg!(target_arch = "wasm32") && matches!(tab, OptionsTab::Paths | OptionsTab::Compute) {
                continue;
            }
            if std::mem::take(&mut group_gap) {
                ui.add_space(24.0);
            }
            let response = crate::ui::widgets::tab_list_item(ui, name, *selected == tab, true, brush);
            if reveal_selected && *selected == tab {
                response.scroll_to_me(None);
                let options_id = Id::new("options");
                let (focused, modal) = ui.memory(|memory| (memory.focused(), memory.top_modal_layer()));
                let focused_here = focused.and_then(|id| ui.ctx().read_response(id)).is_some_and(|focused| focused.layer_id.id == options_id);
                let modal_over = modal.is_some_and(|layer| layer.id != options_id);
                if !focused_here && !modal_over {
                    response.request_focus();
                }
            }
            if response.has_focus() {
                let stroke = Stroke::new(1.0, theme::TEXTBOX_FOCUSED_BORDER);
                ui.painter().rect_stroke(response.rect.shrink(2.0), 3.0, stroke, egui::StrokeKind::Inside);
            }
            if response.gained_focus() {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                *selected = tab;
            }
        }
    });
}

/// A plain button (`Button`), or with `important` the accented one
/// (`Button-Important`).
fn button(ui: &mut Ui, text: &str, important: bool, enabled: bool) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(100.0, 28.0));
    ui.add_enabled(enabled, widget)
}

/// `Button-IconText-Bordered` in red with the `alert-circle` icon.
fn alert_button(ui: &mut Ui, text: &str) -> egui::Response {
    let available = ui.available_width();
    let text_width = (available - 38.0).max(1.0);
    let galley = ui.painter().layout(text.to_string(), theme::interface(11.0), theme::ACCENT_RED, text_width);
    let size = Vec2::new((38.0 + galley.size().x).min(available), (galley.size().y + 10.0).max(26.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    let border = if response.has_focus() { ui.visuals().selection.stroke } else { Stroke::new(1.0, theme::BUTTON_BORDER) };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, border, egui::StrokeKind::Inside);
    let icon_rect = Rect::from_min_size(egui::Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(ui, icon_rect, "alert-circle", theme::ACCENT_RED);
    let text_position = egui::Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, theme::ACCENT_RED);
    response
}

/// A section's title bar (`TextBlock` on `Panel-Border`, semibold).
fn section(ui: &mut Ui, title: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let galley = ui.painter().layout_no_wrap(title.to_string(), theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let position = egui::Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_EMPHASIS);
    ui.add_space(16.0);
}

/// `GetBG`: the brush a device type's boxes are bordered with.
fn type_brush(device_type: DeviceType) -> Color32 {
    match device_type {
        DeviceType::Cpu | DeviceType::Accelerator => theme::PARAMS_A[0],
        DeviceType::CpuAccelerated => theme::PARAMS_A[1],
        DeviceType::GpuIntegrated => theme::PARAMS_A[2],
        DeviceType::GpuDiscrete | DeviceType::Unknown => theme::PARAMS_A[3],
    }
}

/// A backend's check box: bordered in its type's brush, the id as its
/// tooltip. Returns the new state when clicked.
fn backend_box(ui: &mut Ui, device: &DeviceInfo, label: &str, checked: bool, enabled: bool) -> Option<bool> {
    let font = if device.is_fallback_device { theme::semibold(12.0) } else { theme::interface(12.0) };
    let color = if enabled { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
    let available = ui.available_width();
    let text_width = (available - 24.0).max(1.0);
    let galley = ui.painter().layout(label.to_string(), font, color, text_width);
    let size = Vec2::new((24.0 + galley.size().x).min(available), galley.size().y.max(22.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if response.has_focus() {
        ui.painter().rect_stroke(rect, 2.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
    let box_rect = Rect::from_min_size(egui::Pos2::new(rect.left(), rect.center().y - 8.0), Vec2::splat(16.0));
    let mut border = type_brush(device.device_type);
    if !enabled {
        border = border.gamma_multiply(0.5);
    }
    ui.painter().rect_filled(box_rect, 2.0, theme::TEXTBOX_BACKGROUND);
    ui.painter().rect_stroke(box_rect, 2.0, Stroke::new(1.5, border), egui::StrokeKind::Inside);
    if checked {
        icons::paint(ui, box_rect.shrink(2.0), "check", border);
    }
    let text_position = egui::Pos2::new(box_rect.right() + 8.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, color);
    let response = response.tooltip(&device.id);
    if response.clicked() {
        return Some(!checked);
    }
    None
}

/// The devices grouped: CPUs first, one group per
/// device (name and type), its backends by name.
fn device_groups(devices: &[DeviceInfo]) -> Vec<Vec<&DeviceInfo>> {
    let mut ordered: Vec<&DeviceInfo> = devices.iter().collect();
    ordered.sort_by_key(|device| device.device_type != DeviceType::Cpu);
    let mut groups: Vec<Vec<&DeviceInfo>> = Vec::new();
    for device in ordered {
        let same = groups.iter_mut().find(|group| group[0].device_name == device.device_name && group[0].device_type == device.device_type);
        match same {
            Some(group) => group.push(device),
            None => groups.push(vec![device]),
        }
    }
    for group in groups.iter_mut() {
        group.sort_by(|left, right| left.backend_name.cmp(&right.backend_name));
    }
    groups
}

/// The name shown for a device: trademarks as symbols.
fn display_name(name: &str) -> String {
    name.replace("(TM)", "™").replace("(R)", "®")
}

fn device_group(ui: &mut Ui, group: &[&DeviceInfo], page: &mut ComputePage, editor: &mut Editor) {
    let first = group[0];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let name = RichText::new(display_name(&first.device_name)).font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS);
        ui.add(egui::Label::new(name).wrap());
        let kind = RichText::new(format!(" ({})", first.device_type.human_name())).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT);
        ui.label(kind);
    });
    let active: Vec<String> = nerthus_compute::active_devices().into_iter().map(|device| device.id).collect();
    for &device in group {
        let score = match page.scores.get(&device.id) {
            Some(score) => format!(" (Score: {score:.0})"),
            None => " (Score: Unknown)".to_string(),
        };
        let label = format!("{}{score}", device.backend_name);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if device.is_fallback_device {
                backend_box(ui, device, &label, true, false);
                return;
            }
            if device.requires_driver_update {
                backend_box(ui, device, &label, false, false);
                return;
            }
            let checked = active.contains(&device.id);
            let Some(wanted) = backend_box(ui, device, &label, checked, true) else {
                return;
            };
            ui.ctx().set_cursor_icon(egui::CursorIcon::Wait);
            if wanted {
                log::info!("Activating: {}", device.id);
                if !nerthus_compute::try_activate(&device.id) {
                    page.console = format!("{} ({}) could not be activated.", device.device_name, device.backend_name);
                }
            } else {
                log::info!("Deactivating: {}", device.id);
                nerthus_compute::try_deactivate(&device.id);
            }
            editor.compute.check_fastest();
        });
        if device.requires_driver_update {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                // `B_Click`: the documentation on GPU drivers.
                if alert_button(ui, "Requires driver update").clicked() {
                    page.docs = Some(crate::docs::DRIVER_UPDATE);
                }
            });
        }
    }
}

/// The Compute page (`compute.xaml`): the devices at the top, the console
/// and the buttons docked at the bottom.
fn compute_page(ui: &mut Ui, editor: &mut Editor, page: &mut ComputePage, notice: &mut Option<Notice>) {
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let width = ui.available_width();
        let stacked = width < 600.0;
        let rows = if stacked && !editor.compute.cpu_only { 3.0 } else { 1.0 };
        let buttons_height = rows * 28.0 + (rows - 1.0) * ui.spacing().item_spacing.y;
        let buttons_layout = if stacked { Layout::top_down(Align::Min) } else { Layout::left_to_right(Align::Center) };
        // Give these top-down children their final height before placing
        // them from the bottom. A wrapping row can otherwise grow below
        // the page, and ScrollArea always starts at its parent's top.
        ui.allocate_ui_with_layout(Vec2::new(width, buttons_height), buttons_layout, |ui| {
            compute_buttons(ui, editor, page, notice);
        });
        ui.add_space(8.0);
        if !editor.compute.cpu_only {
            let console_height = (ui.available_height() - 72.0).clamp(0.0, 90.0);
            if console_height > 0.0 {
                ui.allocate_ui_with_layout(Vec2::new(width, console_height), Layout::top_down(Align::Min), |ui| {
                    compute_console(ui, page);
                });
                ui.add_space(8.0);
            }
        }
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            let mut devices = egui::ScrollArea::vertical();
            devices = devices.id_salt("compute-devices");
            devices = devices.min_scrolled_height(0.0);
            devices = devices.auto_shrink([false, false]);
            devices.show(ui, |ui| compute_devices(ui, editor, page, notice));
        });
    });
}

/// `consoleXPU`: what the libraries and the benchmark had to say.
fn compute_console(ui: &mut Ui, page: &ComputePage) {
    let mut console = page.console.clone();
    let mut field = egui::TextEdit::multiline(&mut console);
    field = field.font(theme::monospace(11.0));
    field = field.desired_rows(4);
    field = field.desired_width(f32::INFINITY);
    field = field.interactive(false);
    let mut scroll = egui::ScrollArea::vertical();
    scroll = scroll.id_salt("compute-console");
    scroll = scroll.max_height(ui.available_height().min(90.0));
    scroll = scroll.min_scrolled_height(0.0);
    scroll = scroll.auto_shrink([false, false]);
    scroll.show(ui, |ui| ui.add(field));
}

/// The page's title and its devices (or, in CPU-only mode, the way out).
fn compute_devices(ui: &mut Ui, editor: &mut Editor, page: &mut ComputePage, notice: &mut Option<Notice>) {
    section(ui, "Available Devices");
    if editor.compute.cpu_only {
        let text = "Nerthus is in CPU-only mode to provide maximum stability. You can turn it off to use additional devices such as GPU for faster processing.\n\nIf you face any issues, you can go back to CPU-only mode here, or by starting Nerthus with --cpu-only.";
        ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
        ui.add_space(8.0);
        if alert_button(ui, "Restart Nerthus with all devices enabled!").clicked() {
            if let Some(folder) = user::data_folder() {
                let _ = std::fs::remove_file(folder.join(nerthus_compute::CPU_ONLY_FILE));
            }
            *notice = Some(Notice { title: "Restart Nerthus", text: "Restart Nerthus for all devices to be loaded.".to_string() });
        }
        return;
    }

    match &page.devices {
        None => {
            ui.label(RichText::new("Finding devices...").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
        }
        Some(devices) => {
            // `UniformGrid` with two columns: the groups two to a row.
            let devices = devices.clone();
            let groups = device_groups(&devices);
            if ui.available_width() < 480.0 {
                for group in &groups {
                    device_group(ui, group, page, editor);
                    ui.add_space(16.0);
                }
                return;
            }
            for pair in groups.chunks(2) {
                ui.columns(2, |columns| {
                    for (column, group) in columns.iter_mut().zip(pair) {
                        device_group(column, group, page, editor);
                    }
                });
                ui.add_space(16.0);
            }
        }
    }
}

/// The page's buttons (`btnBenchmark`, `btnEnforce`), and the way back to
/// CPU-only mode (writing `cpuonly.dat`).
fn compute_buttons(ui: &mut Ui, editor: &Editor, page: &mut ComputePage, notice: &mut Option<Notice>) {
    if !editor.compute.cpu_only {
        let idle = page.benchmark.is_none();
        if button(ui, "Scan Hardware and Determine Speeds", true, idle).clicked() {
            page.start_benchmark();
        }
    }
    // `btnEnforce`: the NVIDIA profile, which only Windows has.
    let can_enforce = cfg!(windows) && page.enforcing.is_none();
    let enforce = button(ui, "Enforce Optimal Settings", false, can_enforce);
    if enforce.clicked() {
        page.start_enforcing();
    }
    if !cfg!(windows) {
        enforce.disabled_tooltip("NVIDIA settings profiles are made on Windows.");
    }
    if !editor.compute.cpu_only && button(ui, "Use CPU-only Mode", false, true).clicked() {
        let written = user::data_folder().map(|folder| std::fs::create_dir_all(&folder).and_then(|_| std::fs::write(folder.join(nerthus_compute::CPU_ONLY_FILE), "!")));
        let text = match written {
            Some(Ok(())) => "Restart Nerthus to compute on the CPU only.".to_string(),
            Some(Err(error)) => format!("CPU-only mode could not be set: {error}"),
            None => "CPU-only mode could not be set: Nerthus has no data folder.".to_string(),
        };
        *notice = Some(Notice { title: "CPU-only Mode", text });
    }
}

/// `btnAcceptChanges_Click`: `SaveSettings` for each page.
fn accept(editor: &mut Editor, copy: Option<&OptionsCopy>) {
    if let Some(copy) = copy {
        let user = &mut editor.user;
        copy.save(&mut user.options, &mut user.preferences);
    }
    if editor.compute.cpu_only {
        return;
    }
    let active: Vec<String> = nerthus_compute::active_devices().into_iter().map(|device| device.id).collect();
    editor.user.preferences.set_compute_backends(&active);
}

/// Shows the message box the page raised, until dismissed.
fn show_notice(ctx: &egui::Context, notice: &mut Option<Notice>) {
    let Some(shown) = notice.as_ref() else {
        return;
    };
    let mut dismissed = false;
    let modal = crate::ui::prompt::TaskDialog::new(ctx, Id::new("options-notice"), 360.0);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, shown.title, false, |ui| {
            ui.add(egui::Label::new(&shown.text).wrap());
        });
        task.buttons(ui, |ui| {
            if crate::ui::prompt::task_button(ui, "OK").clicked() {
                dismissed = true;
            }
        });
    });
    let enter = response.is_top_modal && !response.any_popup_open && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    if dismissed || response.should_close() || enter {
        *notice = None;
    }
}

/// `BtnReset_OnClick`'s question.
fn reset_question(ctx: &egui::Context, options: &mut OptionsState) {
    if !options.resetting {
        return;
    }
    let mut answer = None;
    let modal = crate::ui::prompt::TaskDialog::new(ctx, Id::new("options-reset"), 380.0);
    let response = modal.show(ctx, |ui, task| {
        task.body(ui, "Reset Preferences", false, |ui| {
            ui.add(egui::Label::new("Are you sure you want to reset preferences in this tab to their default values?").wrap());
        });
        task.buttons(ui, |ui| {
            if crate::ui::prompt::task_button(ui, "No").clicked() {
                answer = Some(false);
            }
            if crate::ui::prompt::task_button(ui, "Yes").clicked() {
                answer = Some(true);
            }
        });
    });
    if answer == Some(true)
        && let Some(copy) = options.copy.as_mut()
    {
        copy.reset(options.tab);
    }
    if answer.is_some() || response.should_close() {
        options.resetting = false;
    }
}

/// Esc is Cancel (`IsCancel`) while the window shows: taken before the
/// shell's keys, so that it does not stop a build as well. A message box,
/// the reset question or a list dropped down on the page takes it instead.
pub fn take_escape(ctx: &egui::Context, state: &mut Option<OptionsState>, graph_popup_open: bool) {
    let Some(options) = state.as_ref() else {
        return;
    };
    if options.notice.is_some() || options.resetting || graph_popup_open || egui::Popup::is_any_open(ctx) {
        return;
    }
    // egui retains a modal's layer for the frame after it closes. The
    // state above already guards our own questions while they are open.
    let own_layers = [Id::new("options"), Id::new("options-notice"), Id::new("options-reset")];
    let modal_over = ctx.memory(|memory| memory.top_modal_layer().is_some_and(|layer| !own_layers.contains(&layer.id)));
    if modal_over {
        return;
    }
    if tool_window::on_top(ctx) != Some(Id::new("options")) {
        return;
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        *state = None;
    }
}

/// The window. `last_tab` keeps the page shown last (`LastOptionsTab`).
pub fn show(ctx: &egui::Context, editor: &mut Editor, state: &mut Option<OptionsState>, last_tab: &mut OptionsTab, dialogs: &mut FileDialogs) -> OptionsOutcome {
    let mut outcome = OptionsOutcome::default();
    let Some(options) = state.as_mut() else {
        return outcome;
    };
    if options.copy.is_none() {
        options.copy = Some(OptionsCopy::load(&editor.user.options, &editor.user.preferences));
    }
    if let Some(copy) = options.copy.as_mut() {
        copy.refresh(&editor.user.options, &editor.user.preferences);
    }
    options.compute.poll();
    if let Some(notice) = options.compute.poll_enforcing() {
        options.notice = Some(notice);
    }
    let working = options.compute.benchmark.is_some() || options.compute.finding.is_some() || options.compute.enforcing.is_some();
    if working {
        ctx.request_repaint();
    }
    let mut open = true;
    let mut close = false;
    let mut window = egui::Window::new("Options");
    window = window.collapsible(false);
    window = window.resizable(false);
    // egui keeps the size to the body the window is constrained to.
    window = window.fixed_size(WINDOW_SIZE);
    window = tool_window::place(ctx, window, Id::new("options"), WINDOW_SIZE);
    window = window.open(&mut open);
    tool_window::focus_scope(ctx, Id::new("options"));
    window.show(ctx, |ui| {
        // The room inside, split between the parts whatever they hold, so
        // that the footer stays in the window and the window in the body.
        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let body = rect.with_max_y((rect.bottom() - FOOTER_HEIGHT).max(rect.top()));
        let list = body.with_max_x((body.left() + TAB_WIDTH).min(body.right()));
        let page = body.with_min_x((list.right() + 8.0).min(body.right()));
        let footer = rect.with_min_y((body.bottom() + 12.0).min(rect.bottom()));

        let mut list_ui = tool_window::part(ui, "options-list", list, Layout::top_down(Align::Min));
        let mut tabs = egui::ScrollArea::vertical();
        tabs = tabs.id_salt("options-tabs");
        tabs = tabs.auto_shrink([false, false]);
        tabs = tabs.animated(false);
        let reveal_selected = options.tab_seen != Some(options.tab);
        tabs.show(&mut list_ui, |ui| tab_list(ui, &mut options.tab, reveal_selected));
        options.tab_seen = Some(options.tab);

        // The page's border goes round its place, and the page is kept
        // inside it, whatever the page holds.
        let border = Stroke::new(1.5, theme::ACCENT_MUTED_PURPLE);
        ui.painter().rect_stroke(page, 3.0, border, egui::StrokeKind::Inside);
        let mut page_ui = tool_window::part(ui, "options-page", page.shrink(border.width), Layout::top_down(Align::Min));
        egui::Frame::new().inner_margin(8.0).show(&mut page_ui, |ui| {
            // The page's size less the frame's margin and border.
            let inside = (page.size() - Vec2::splat(19.0)).max(Vec2::ZERO);
            ui.set_min_size(inside);
            ui.set_max_size(inside);
            match options.tab {
                OptionsTab::Compute => {
                    compute_page(ui, editor, &mut options.compute, &mut options.notice);
                    outcome.docs = options.compute.docs.take();
                }
                tab => {
                    if let Some(copy) = options.copy.as_mut()
                        && options_pages::page(ui, tab, copy, dialogs) == Some(PageRequest::RunDiagnostics)
                    {
                        outcome.run_diagnostics = true;
                    }
                }
            }
        });

        // The footer (margin 146, 8, 8, 0): Reset on the left, Accept
        // Changes and Cancel on the right.
        let mut footer_ui = tool_window::part(ui, "options-footer", footer, Layout::left_to_right(Align::Min));
        footer_ui.add_space(146.0);
        if button(&mut footer_ui, "Reset this page", false, true).clicked() {
            if options.tab == OptionsTab::Compute {
                let text = "Settings for this page cannot be reset.";
                options.notice = Some(Notice { title: "Nerthus", text: text.to_string() });
            } else {
                options.resetting = true;
            }
        }
        footer_ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.add_space(8.0);
            if button(ui, "Cancel", false, true).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if button(ui, "Accept Changes", true, true).clicked() {
                accept(editor, options.copy.as_ref());
                outcome.accepted = true;
                close = true;
            }
        });
    });
    show_notice(ctx, &mut options.notice);
    reset_question(ctx, options);
    *last_tab = options.tab;
    if close || !open {
        *state = None;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::{AreaState, Event, PointerButton, Pos2, RawInput};

    /// The screen, in points, of 1366 × 768, 1280 × 720 (or 1920 × 1080
    /// at 150%) and 640 × 480 at the default 120%, and 1024 × 768 at 175%.
    const SMALL_SCREENS: [Vec2; 4] = [Vec2::new(1138.0, 640.0), Vec2::new(1066.0, 600.0), Vec2::new(533.0, 400.0), Vec2::new(585.0, 438.0)];

    fn context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context
    }

    /// A frame at this screen size with these events, Esc taken before the
    /// window as the shell does. Returns the window's rect and what it did.
    fn painted_frame(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<OptionsState>, events: Vec<Event>) -> (Rect, OptionsOutcome, Vec<egui::epaint::ClippedShape>) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
        let mut last_tab = OptionsTab::General;
        let mut dialogs = FileDialogs::default();
        let mut outcome = OptionsOutcome::default();
        let mut output = context.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            take_escape(&ctx, state, false);
            outcome = show(&ctx, editor, state, &mut last_tab, &mut dialogs);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let rect = AreaState::load(context, Id::new("options")).filter(|area| area.size.is_some()).map(|area| area.rect());
        (rect.unwrap_or(Rect::NOTHING), outcome, shapes)
    }

    fn frame(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<OptionsState>, events: Vec<Event>) -> (Rect, OptionsOutcome) {
        let (rect, outcome, _) = painted_frame(context, screen, editor, state, events);
        (rect, outcome)
    }

    /// Frames enough for egui's first-frame sizing to settle.
    fn settle(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<OptionsState>) -> Rect {
        for _ in 0..4 {
            frame(context, screen, editor, state, Vec::new());
        }
        frame(context, screen, editor, state, Vec::new()).0
    }

    /// A click at `position`; what the window did on the release.
    fn click(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<OptionsState>, position: Pos2) -> OptionsOutcome {
        let button = |pressed| Event::PointerButton { pos: position, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(context, screen, editor, state, vec![Event::PointerMoved(position)]);
        frame(context, screen, editor, state, vec![button(true)]);
        frame(context, screen, editor, state, vec![button(false)]).1
    }

    fn escape() -> Event {
        Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    fn visible_text(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<Rect> {
        shapes.iter().find_map(|clipped| {
            let egui::Shape::Text(text) = &clipped.shape else {
                return None;
            };
            let rect = text.visual_bounding_rect();
            (text.galley.text() == wanted && clipped.clip_rect.contains_rect(rect)).then_some(rect)
        })
    }

    #[test]
    fn selected_headers_are_revealed_on_open_and_external_selection() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let screen = Vec2::new(533.0, 400.0);
        let mut state = Some(OptionsState::new(OptionsTab::Experimental));
        settle(&context, screen, &mut editor, &mut state);
        let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
        assert!(visible_text(&shapes, "Experimental").is_some(), "the initially selected header is clipped");

        state.as_mut().unwrap().tab = OptionsTab::General;
        settle(&context, screen, &mut editor, &mut state);
        let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
        assert!(visible_text(&shapes, "General").is_some(), "the externally selected header is clipped");

        state.as_mut().unwrap().tab = OptionsTab::Experimental;
        settle(&context, screen, &mut editor, &mut state);
        let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
        let header = visible_text(&shapes, "Experimental").expect("the newly selected header is visible");
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 10_000.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        frame(&context, screen, &mut editor, &mut state, vec![Event::PointerMoved(header.center()), wheel]);
        settle(&context, screen, &mut editor, &mut state);
        let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
        assert_eq!(state.as_ref().unwrap().tab, OptionsTab::Experimental);
        assert!(visible_text(&shapes, "General").is_some(), "manual scrolling did not reach the first header");
        assert!(visible_text(&shapes, "Experimental").is_none(), "the selection pinned the manually scrolled list");
    }

    #[test]
    fn notices_close_with_ok_escape_enter_and_an_outside_click() {
        let screen = Vec2::new(1066.0, 600.0);
        for dismissal in ["OK", "Escape", "Enter", "outside"] {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let mut options = OptionsState::new(OptionsTab::General);
            options.notice = Some(Notice { title: "Notice", text: "Settings changed.".to_string() });
            let mut state = Some(options);
            settle(&context, screen, &mut editor, &mut state);
            match dismissal {
                "OK" => {
                    let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
                    let button = visible_text(&shapes, "OK").expect("the notice has an OK button");
                    click(&context, screen, &mut editor, &mut state, button.center());
                }
                "Escape" => {
                    frame(&context, screen, &mut editor, &mut state, vec![escape()]);
                }
                "Enter" => {
                    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
                    frame(&context, screen, &mut editor, &mut state, vec![enter]);
                }
                _ => {
                    click(&context, screen, &mut editor, &mut state, Pos2::new(1.0, 1.0));
                }
            }
            assert!(state.as_ref().is_some_and(|options| options.notice.is_none()), "{dismissal} did not dismiss just the notice");
        }
    }

    #[test]
    fn a_notice_under_another_modal_leaves_enter_for_it() {
        let context = context();
        let mut notice = Some(Notice { title: "Notice", text: "Settings changed.".to_string() });
        let mut enter_seen = false;
        for step in 0..4 {
            let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
            let events = if step == 3 { vec![enter] } else { Vec::new() };
            let input = RawInput { events, ..Default::default() };
            let output = context.run_ui(input, |ui| {
                show_notice(ui.ctx(), &mut notice);
                egui::Modal::new(Id::new("over-options-notice")).show(ui.ctx(), |ui| {
                    ui.label("Another question");
                });
                enter_seen = ui.input(|input| input.key_pressed(Key::Enter));
            });
            output.drop_without_applying_deltas();
        }
        assert!(notice.is_some());
        assert!(enter_seen, "the covered notice consumed Enter");
    }

    #[test]
    fn an_open_bool_graph_menu_takes_escape_before_options() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let screen = Vec2::new(1066.0, 600.0);
        let mut state = Some(OptionsState::new(OptionsTab::General));
        settle(&context, screen, &mut editor, &mut state);
        let mut graph = crate::ui::graph_canvas::GraphState::default();
        graph.popup = Some(crate::ui::graph_popups::GraphPopup::Lazy { at: Pos2::new(50.0, 50.0) });
        for step in 0..4 {
            let events = if step == 3 { vec![escape()] } else { Vec::new() };
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
            let mut commands = Vec::new();
            let output = context.run_ui(input, |ui| {
                take_escape(ui.ctx(), &mut state, graph.popup.is_some());
                crate::ui::graph_popups::show(ui, &mut editor, &mut graph, &mut commands);
            });
            output.drop_without_applying_deltas();
        }
        assert!(state.is_some(), "Options took the graph menu's Escape");
        assert!(graph.popup.is_none(), "Escape did not close the Lazy Menu");
    }

    #[test]
    fn factory_reset_reloads_options_before_accepting_first_run_choices() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        editor.user.options.set_value("GridSize", serde_json::json!(50));
        editor.user.options.set_value("HideBookmarkShortcuts", serde_json::json!(true));
        let screen = Vec2::new(1066.0, 600.0);
        let mut state = Some(OptionsState::new(OptionsTab::Graph));
        settle(&context, screen, &mut editor, &mut state);
        assert!(state.as_ref().unwrap().copy.is_some());

        editor.user.options = crate::user::UserOptions::new();
        editor.user.preferences = crate::user::Preferences::new();
        state.as_mut().unwrap().reload_settings();
        assert!(state.as_ref().unwrap().copy.is_none());
        for name in ["AltPan", "AltZoom", "AltRotate"] {
            editor.user.options.set_value(name, serde_json::json!(true));
        }
        settle(&context, screen, &mut editor, &mut state);
        let (_, _, shapes) = painted_frame(&context, screen, &mut editor, &mut state, Vec::new());
        let accept = visible_text(&shapes, "Accept Changes").expect("Accept Changes is visible");
        let outcome = click(&context, screen, &mut editor, &mut state, accept.center());

        assert!(state.is_none() && outcome.accepted);
        let defaults = crate::user::UserOptions::new();
        for name in ["GridSize", "HideBookmarkShortcuts"] {
            assert_eq!(editor.user.options.value(name), defaults.value(name), "Accept restored the old {name}");
        }
        for name in ["AltPan", "AltZoom", "AltRotate"] {
            assert_eq!(editor.user.options.value(name), serde_json::json!(true), "Accept discarded the first-run {name} choice");
        }
    }

    #[test]
    fn keyboard_starts_on_the_selected_header_and_stays_in_options() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(OptionsState::new(OptionsTab::Graph));
        let screen = Vec2::new(1066.0, 600.0);
        let mut shell_clicked = false;
        let mut keyboard_frame = |events: Vec<Event>, request_shell: bool| {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
            let mut last_tab = OptionsTab::General;
            let mut dialogs = FileDialogs::default();
            let mut focused = None;
            let mut output = context.run_ui(input, |ui| {
                let shell = ui.button("Shell action");
                shell_clicked |= shell.clicked();
                if request_shell {
                    shell.request_focus();
                }
                show(ui.ctx(), &mut editor, &mut state, &mut last_tab, &mut dialogs);
                focused = ui.memory(|memory| memory.focused()).and_then(|id| ui.ctx().read_response(id));
            });
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            (focused, shapes)
        };
        keyboard_frame(Vec::new(), true);
        for _ in 0..4 {
            keyboard_frame(Vec::new(), false);
        }
        let (focused, shapes) = keyboard_frame(Vec::new(), false);
        let header = visible_text(&shapes, "Graph").expect("the selected Graph header is visible");
        let focused = focused.expect("the selected header has keyboard focus");
        assert_eq!(focused.layer_id.id, Id::new("options"));
        assert!(focused.rect.contains_rect(header), "the initial focus is not on the selected header");

        for key in [Key::Space, Key::Enter] {
            let event = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
            keyboard_frame(vec![event], false);
        }
        for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
            for _ in 0..40 {
                let event = Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers };
                let (focused, _) = keyboard_frame(vec![event], false);
                if let Some(focused) = focused {
                    assert_eq!(focused.layer_id.id, Id::new("options"), "Tab traversal escaped Options");
                }
                // egui wraps at either end on the following frame.
                let (focused, _) = keyboard_frame(Vec::new(), false);
                let focused = focused.expect("Tab traversal restores focus within Options after wrapping");
                assert_eq!(focused.layer_id.id, Id::new("options"), "Tab traversal escaped Options");
            }
        }
        assert!(!shell_clicked, "Space or Enter activated the shell behind Options");
    }

    #[test]
    fn the_window_keeps_its_size_where_there_is_room() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(OptionsState::new(OptionsTab::General));
        // 1600 × 960 at 120%.
        let screen = Vec2::new(1333.0, 800.0);
        let rect = settle(&context, screen, &mut editor, &mut state);
        let body = tool_window::body(&context);
        assert!((rect.size() - WINDOW_SIZE).abs().max_elem() <= 1.0, "{rect:?} is not {WINDOW_SIZE:?}");
        assert!((rect.center() - body.center()).length() <= 1.0, "{rect:?} is not centred in {body:?}");
    }

    #[test]
    fn every_page_leaves_the_window_its_size_and_place() {
        // egui draws the title bar as wide as the window's size, and grows
        // the frame round a page too big for it: the frame would then stick
        // out past the title bar, and the window would jump as such a page
        // came and went (Compute, with its devices, console and buttons).
        // 1600 × 960 at 120% and 100%, and at 175%, where it is squeezed.
        for screen in [Vec2::new(1333.0, 800.0), Vec2::new(1600.0, 960.0), Vec2::new(914.0, 548.0)] {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let mut state = Some(OptionsState::new(OptionsTab::General));
            let first = settle(&context, screen, &mut editor, &mut state);
            let body = tool_window::body(&context);
            let wanted = WINDOW_SIZE.min(body.size());
            assert!((first.size() - wanted).abs().max_elem() <= 1.0, "General at {screen:?}: {first:?} is not {wanted:?}");
            for (tab, ..) in TABS.into_iter().chain([TABS[0]]) {
                state.as_mut().unwrap().tab = tab;
                let rect = settle(&context, screen, &mut editor, &mut state);
                assert!((rect.min - first.min).abs().max_elem() <= 0.5 && (rect.max - first.max).abs().max_elem() <= 0.5, "{tab:?} at {screen:?}: {rect:?} is not {first:?}");
            }
        }
    }

    #[test]
    fn every_page_fits_small_bodies() {
        for screen in SMALL_SCREENS {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let mut state = Some(OptionsState::new(OptionsTab::General));
            for (tab, ..) in TABS {
                state.as_mut().unwrap().tab = tab;
                let rect = settle(&context, screen, &mut editor, &mut state);
                let body = tool_window::body(&context);
                assert!(body.expand(0.5).contains_rect(rect), "{tab:?} at {screen:?}: {rect:?} is outside {body:?}");
            }
        }
    }

    #[test]
    fn accept_and_cancel_are_in_reach_in_small_bodies() {
        // The footer's buttons from the window's bottom right corner: past
        // the frame (7), Cancel 8 in and 100 wide, Accept Changes 8 to its
        // left, in the row 4 above the bottom.
        let row = |rect: Rect| rect.bottom() - 7.0 - 4.0 - 14.0;
        let cancel = |rect: Rect| Pos2::new(rect.right() - 7.0 - 8.0 - 50.0, row(rect));
        let accept = |rect: Rect| Pos2::new(rect.right() - 7.0 - 8.0 - 100.0 - 8.0 - 40.0, row(rect));
        for screen in SMALL_SCREENS {
            let context = context();
            let mut editor = Editor::new(Document::new());
            let mut state = Some(OptionsState::new(OptionsTab::Viewport));
            let rect = settle(&context, screen, &mut editor, &mut state);
            let outcome = click(&context, screen, &mut editor, &mut state, cancel(rect));
            assert!(state.is_none() && !outcome.accepted, "Cancel at {screen:?} did not close the window");

            let mut state = Some(OptionsState::new(OptionsTab::Viewport));
            let rect = settle(&context, screen, &mut editor, &mut state);
            let outcome = click(&context, screen, &mut editor, &mut state, accept(rect));
            assert!(state.is_none() && outcome.accepted, "Accept Changes at {screen:?} did not accept");
        }
    }

    #[test]
    fn escape_cancels_unless_the_reset_question_is_up() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let screen = Vec2::new(1066.0, 600.0);
        let mut state = Some(OptionsState::new(OptionsTab::Graph));
        settle(&context, screen, &mut editor, &mut state);
        state.as_mut().unwrap().resetting = true;
        settle(&context, screen, &mut editor, &mut state);
        // Esc answers the question, and the window stays.
        frame(&context, screen, &mut editor, &mut state, vec![escape()]);
        assert!(state.as_ref().is_some_and(|options| !options.resetting));
        let (_, outcome) = frame(&context, screen, &mut editor, &mut state, vec![escape()]);
        assert!(state.is_none() && !outcome.accepted);
    }

    #[test]
    fn the_shell_does_not_see_the_escape_the_window_takes() {
        let context = context();
        let mut editor = Editor::new(Document::new());
        let mut state = Some(OptionsState::new(OptionsTab::General));
        settle(&context, Vec2::new(1066.0, 600.0), &mut editor, &mut state);
        let input = RawInput { events: vec![escape()], ..Default::default() };
        let mut seen = true;
        let output = context.run_ui(input, |ui| {
            take_escape(ui.ctx(), &mut state, false);
            seen = ui.input(|input| input.key_pressed(Key::Escape));
        });
        output.drop_without_applying_deltas();
        assert!(state.is_none() && !seen);
    }
}

#[cfg(test)]
mod compute_layout_tests {
    use super::*;
    use crate::document::Document;
    use egui::{Pos2, RawInput, Shape};

    type Drawn = (String, Rect, Rect);

    fn frame(context: &egui::Context, editor: &mut Editor, page: &mut ComputePage, place: Rect) -> Vec<Drawn> {
        let screen = Rect::from_min_size(Pos2::ZERO, place.max.to_vec2() + Vec2::splat(20.0));
        let input = RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut notice = None;
        let output = context.run_ui(input, |ui| {
            let builder = egui::UiBuilder::new().max_rect(place).layout(Layout::top_down(Align::Min));
            let mut page_ui = ui.new_child(builder);
            page_ui.set_clip_rect(place);
            compute_page(&mut page_ui, editor, page, &mut notice);
        });
        let mut texts = Vec::new();
        let mut shapes: Vec<(Shape, Rect)> = output.shapes.iter().map(|clipped| (clipped.shape.clone(), clipped.clip_rect)).collect();
        while let Some((shape, clip)) = shapes.pop() {
            match shape {
                Shape::Vec(inner) => shapes.extend(inner.into_iter().map(|shape| (shape, clip))),
                Shape::Text(text) => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    texts.push((text.galley.text().to_string(), rect, clip));
                }
                _ => {}
            }
        }
        output.drop_without_applying_deltas();
        texts
    }

    fn page() -> ComputePage {
        let device = DeviceInfo {
            library_id: "layout-test".to_string(),
            device_type: DeviceType::GpuDiscrete,
            backend_name: "A particularly long compute backend name for a device with an outdated driver".to_string(),
            vendor_name: "Layout tests".to_string(),
            device_name: "A particularly long graphics processor name with a full manufacturer and model number".to_string(),
            id: "layout-test-gpu".to_string(),
            is_fallback_device: false,
            requires_driver_update: true,
        };
        let console = "A long benchmark status line that needs to wrap inside the console without widening the Options page.\n".repeat(30);
        ComputePage {
            devices: Some(vec![device]),
            finding: None,
            scores: Scores::new(),
            console,
            benchmark: None,
            docs: None,
            enforcing: None,
        }
    }

    #[test]
    fn compute_labels_and_buttons_fit_narrow_pages_with_long_console_output() {
        for width in [300.0, 380.0, 685.0] {
            for cpu_only in [false, true] {
                let context = egui::Context::default();
                theme::apply(&context);
                let mut editor = Editor::new(Document::new());
                editor.compute.cpu_only = cpu_only;
                let mut page = page();
                let place = Rect::from_min_size(Pos2::new(160.0, 100.0), Vec2::new(width, 420.0));
                frame(&context, &mut editor, &mut page, place);
                let texts = frame(&context, &mut editor, &mut page, place);
                for (text, rect, clip) in &texts {
                    let right_edge = place.right().min(clip.right());
                    assert!(rect.right() <= right_edge + 0.5, "Compute at {width}, CPU-only {cpu_only}: {text:?} extends beyond {right_edge}: {rect:?}");
                }

                let mut visible = vec!["Available Devices", "Enforce Optimal Settings"];
                if cpu_only {
                    visible.push("Restart Nerthus with all devices enabled!");
                } else {
                    visible.push("Scan Hardware and Determine Speeds");
                    visible.push("Use CPU-only Mode");
                }
                for wanted in visible {
                    let drawn = texts.iter().find(|(text, ..)| text == wanted);
                    let (_, rect, clip) = drawn.unwrap_or_else(|| panic!("Compute at {width}: {wanted:?} is not drawn"));
                    assert!(clip.expand(0.5).contains_rect(*rect), "Compute at {width}: {wanted:?} is clipped at {clip:?}: {rect:?}");
                }
            }
        }
    }

    #[test]
    fn short_compute_pages_keep_the_device_heading_and_toolbar_visible() {
        for width in [300.0, 380.0, 685.0] {
            let context = egui::Context::default();
            theme::apply(&context);
            let mut editor = Editor::new(Document::new());
            let mut page = page();
            let place = Rect::from_min_size(Pos2::new(160.0, 100.0), Vec2::new(width, 220.0));
            frame(&context, &mut editor, &mut page, place);
            let texts = frame(&context, &mut editor, &mut page, place);
            let visible = ["Available Devices", "Scan Hardware and Determine Speeds", "Enforce Optimal Settings", "Use CPU-only Mode"];
            for wanted in visible {
                let drawn = texts.iter().find(|(text, ..)| text == wanted);
                let (_, rect, clip) = drawn.unwrap_or_else(|| panic!("Compute at {width}: {wanted:?} is not drawn"));
                assert!(clip.expand(0.5).contains_rect(*rect), "Compute at {width}: {wanted:?} is clipped at {clip:?}: {rect:?}");
            }
        }
    }
}
