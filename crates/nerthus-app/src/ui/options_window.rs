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
            compute: ComputePage::open(),
            notice: None,
            copy: None,
            resetting: false,
        }
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
fn tab_list(ui: &mut Ui, selected: &mut OptionsTab) {
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
            let (rect, response) = ui.allocate_exact_size(Vec2::new(TAB_WIDTH, 26.0), Sense::click());
            let chosen = *selected == tab;
            if chosen {
                ui.painter().rect_filled(rect.shrink(2.0), 3.0, brush);
            }
            let color = if chosen || response.hovered() { theme::LABEL_EMPHASIS } else { theme::LABEL_DEFAULT };
            let galley = ui.painter().layout_no_wrap(name.to_string(), theme::interface(12.0), color);
            let position = egui::Pos2::new(rect.right() - 10.0 - galley.size().x, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(position, galley, color);
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
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::interface(11.0), theme::ACCENT_RED);
    let size = Vec2::new(8.0 + 16.0 + 6.0 + galley.size().x + 8.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = if response.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON_BACKGROUND };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER), egui::StrokeKind::Inside);
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
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, color);
    let size = Vec2::new(16.0 + 8.0 + galley.size().x, 22.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
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
    let response = response.on_hover_text(&device.id);
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let name = RichText::new(display_name(&first.device_name)).font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS);
        ui.label(name);
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
        ui.horizontal(|ui| compute_buttons(ui, editor, page, notice));
        ui.add_space(8.0);
        if !editor.compute.cpu_only {
            compute_console(ui, page);
            ui.add_space(8.0);
        }
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            let mut devices = egui::ScrollArea::vertical();
            devices = devices.id_salt("compute-devices");
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
    ui.add(field);
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
        enforce.on_disabled_hover_text("NVIDIA settings profiles are made on Windows.");
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
    let modal = egui::Modal::new(Id::new("options-notice"));
    modal.show(ctx, |ui| {
        ui.set_width(360.0);
        ui.label(RichText::new(shown.title).font(theme::semibold(14.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(8.0);
        ui.label(&shown.text);
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if button(ui, "OK", false, true).clicked() {
                dismissed = true;
            }
        });
    });
    if dismissed {
        *notice = None;
    }
}

/// `BtnReset_OnClick`'s question.
fn reset_question(ctx: &egui::Context, options: &mut OptionsState) {
    if !options.resetting {
        return;
    }
    let mut answer = None;
    let modal = egui::Modal::new(Id::new("options-reset"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(RichText::new("Reset Preferences").font(theme::semibold(14.0)).color(theme::ACCENT_MUTED_BLUE));
        ui.add_space(8.0);
        ui.label("Are you sure you want to reset preferences in this tab to their default values?");
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if button(ui, "No", false, true).clicked() {
                answer = Some(false);
            }
            if button(ui, "Yes", true, true).clicked() {
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
pub fn take_escape(ctx: &egui::Context, state: &mut Option<OptionsState>) {
    let Some(options) = state.as_ref() else {
        return;
    };
    if options.notice.is_some() || options.resetting || egui::Popup::is_any_open(ctx) {
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
        tabs.show(&mut list_ui, |ui| tab_list(ui, &mut options.tab));

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
    fn frame(context: &egui::Context, screen: Vec2, editor: &mut Editor, state: &mut Option<OptionsState>, events: Vec<Event>) -> (Rect, OptionsOutcome) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
        let mut last_tab = OptionsTab::General;
        let mut dialogs = FileDialogs::default();
        let mut outcome = OptionsOutcome::default();
        let output = context.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            take_escape(&ctx, state);
            outcome = show(&ctx, editor, state, &mut last_tab, &mut dialogs);
        });
        output.drop_without_applying_deltas();
        let rect = AreaState::load(context, Id::new("options")).filter(|area| area.size.is_some()).map(|area| area.rect());
        (rect.unwrap_or(Rect::NOTHING), outcome)
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
        let mut state = Some(OptionsState::new(OptionsTab::General));
        let input = RawInput { events: vec![escape()], ..Default::default() };
        let mut seen = true;
        let output = context.run_ui(input, |ui| {
            take_escape(ui.ctx(), &mut state);
            seen = ui.input(|input| input.key_pressed(Key::Escape));
        });
        output.drop_without_applying_deltas();
        assert!(state.is_none() && !seen);
    }
}
