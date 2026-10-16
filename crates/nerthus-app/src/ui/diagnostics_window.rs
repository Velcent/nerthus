//! Tools › Run Diagnostics… ("Watson"): troubleshooting tasks to choose
//! from, run with Start.
//!
//! - Create a diagnostics package: the logs, the settings and what Nerthus
//!   runs on, zipped into the documents folder.
//! - Run Nerthus in Verbose Mode, or with CPU only: Nerthus starts again
//!   that way.
//! - Run Nerthus in Safe Mode, using the default settings for one session.
//! - Reset to factory defaults: the settings go back to their defaults
//!   (backed up first if asked), and the first-run setup shows again.
//! - Clean up autosaves, caches and logs.
//! - Run Benchmark: every compute device scored.
//!
//! There are no licence or Houdini tasks.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Id, RichText, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::widgets::{dialog_button, dialog_button_row, DialogFit};

#[path = "diagnostics-reports.rs"]
mod reports;

/// The tasks, in Watson's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Task {
    Package,
    Verbose,
    SafeMode,
    CpuOnly,
    FactoryReset,
    Cleanup,
    Benchmark(usize),
}

/// The window's content, where there is room.
const WIDTH: f32 = 540.0;

const TASKS: [(Task, &str); 9] = [
    (Task::Package, "Create a diagnostics package (required for technical support)"),
    (Task::Verbose, "Run Nerthus in Verbose Mode"),
    (Task::SafeMode, "Run Nerthus in Safe Mode"),
    (Task::CpuOnly, "Run Nerthus with CPU only (Recommend for crashes)"),
    (Task::FactoryReset, "Reset to factory defaults (Clean install)"),
    (Task::Cleanup, "Cleanup autosaves, caches, and logs"),
    (Task::Benchmark(10), "Run Benchmark (10 passes)"),
    (Task::Benchmark(20), "Run Benchmark — High Accuracy (20 passes)"),
    (Task::Benchmark(50), "Run Benchmark — Very High Accuracy (50 passes)"),
];

/// The command line Nerthus starts again with once it has closed.
static RESTART: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// The window while it is open.
pub struct DiagnosticsWindow {
    task: Task,
    /// "Backup current settings" (factory reset).
    backup: bool,
    /// What the tasks have said.
    output: String,
    benchmark: Option<Receiver<Option<String>>>,
    package_job: Option<Receiver<Result<DiagnosticsPackage, String>>>,
    package: Option<DiagnosticsPackage>,
    viewing_package: bool,
}

/// The exact files written to the archive, retained for the package viewer.
struct DiagnosticsPackage {
    path: PathBuf,
    entries: Vec<(String, Vec<u8>)>,
    selected: usize,
    copy_status: String,
}

impl DiagnosticsWindow {
    pub fn open() -> DiagnosticsWindow {
        DiagnosticsWindow {
            task: Task::Package,
            backup: true,
            output: String::new(),
            benchmark: None,
            package_job: None,
            package: None,
            viewing_package: false,
        }
    }

    fn say(&mut self, line: impl AsRef<str>) {
        self.output.push_str(line.as_ref());
        self.output.push('\n');
    }
}

/// What the window asks of the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticsRequest {
    /// Close Nerthus (it then starts again as set up).
    Restart(Vec<String>),
    /// The settings were reset: the interface takes them in.
    SettingsReset,
}

/// The window's id.
pub fn window_id() -> Id {
    Id::new("diagnostics")
}

/// The window: its text, the tasks and the output scroll when the window
/// is too small for them, Start and Close staying below them. Returns
/// whether it is still open, and a request.
pub fn show(ctx: &egui::Context, editor: &mut Editor, window: &mut DiagnosticsWindow, known_update: bool) -> (bool, Option<DiagnosticsRequest>) {
    poll_benchmark(window);
    poll_package(window);
    if window.benchmark.is_some() || window.package_job.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if window.viewing_package {
        return (show_package(ctx, window), None);
    }
    let mut request = None;
    let mut open = true;
    let mut close = false;
    let id = window_id();
    let fit = DialogFit::new(ctx, id, &egui::Frame::window(&ctx.global_style()));
    let mut frame = egui::Window::new("Nerthus Diagnostics");
    frame = frame.id(id);
    frame = frame.collapsible(false);
    frame = frame.resizable(false);
    frame = fit.window(frame, WIDTH);
    frame = frame.anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
    frame = frame.open(&mut open);
    let shown = frame.show(ctx, |ui| {
        ui.set_width(fit.width(WIDTH));
        fit.body(ui, |ui| {
            let text = "This program is designed to create diagnostics logs for technical support, find and fix potential issues in your Nerthus installation, and perform other tasks such as benchmarks.";
            ui.add(egui::Label::new(RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_DEFAULT)).wrap());
            if known_update {
                ui.add_space(8.0);
                let warning = "A newer version of Nerthus is available. Update before reporting a problem or comparing benchmark results.";
                ui.add(egui::Label::new(RichText::new(warning).color(theme::LABEL_EMPHASIS)).wrap());
                ui.hyperlink_to("View available updates", crate::updates::releases_page());
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Troubleshooting").font(theme::semibold(12.0)).color(theme::LABEL_EMPHASIS));
            ui.add_space(4.0);
            for (task, label) in TASKS {
                ui.radio_value(&mut window.task, task, RichText::new(label).font(theme::interface(12.0)));
                if task == Task::FactoryReset && window.task == Task::FactoryReset {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.checkbox(&mut window.backup, "Backup current settings");
                    });
                }
            }
            ui.add_space(12.0);
            let mut output = window.output.as_str();
            let mut field = egui::TextEdit::multiline(&mut output);
            field = field.font(theme::monospace(11.0));
            field = field.desired_rows(8);
            field = field.desired_width(f32::INFINITY);
            egui::ScrollArea::vertical().max_height(180.0).stick_to_bottom(true).show(ui, |ui| ui.add(field));
            if window.package.is_some() {
                ui.add_space(8.0);
                if dialog_button(ui, "View Package", false).clicked() {
                    window.viewing_package = true;
                }
            }
        });
        ui.add_space(8.0);
        dialog_button_row(ui, |ui| {
            if dialog_button(ui, "Close", false).clicked() {
                close = true;
            }
            let busy = window.benchmark.is_some() || window.package_job.is_some();
            let started = ui.add_enabled_ui(!busy, |ui| dialog_button(ui, "Start", true));
            if started.inner.clicked() {
                request = start(editor, window);
            }
        });
    });
    if let Some(shown) = shown {
        fit.end(ctx, shown.response.rect);
    }
    (open && !close, request)
}

/// The package's actual text files, before the user shares its archive.
fn show_package(ctx: &egui::Context, window: &mut DiagnosticsWindow) -> bool {
    let Some(package) = window.package.as_mut() else {
        window.viewing_package = false;
        return true;
    };
    let mut open = true;
    let mut close = false;
    let id = window_id();
    let fit = DialogFit::new(ctx, id.with("package-view"), &egui::Frame::window(&ctx.global_style()));
    let mut frame = egui::Window::new("Diagnostics Package");
    frame = frame.id(id);
    frame = frame.collapsible(false);
    frame = frame.resizable(false);
    frame = fit.window(frame, 700.0);
    frame = frame.anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
    frame = frame.open(&mut open);
    let shown = frame.show(ctx, |ui| {
        ui.set_width(fit.width(700.0));
        fit.body(ui, |ui| {
            ui.add(egui::Label::new("Review the package's files before sharing it with technical support.").wrap());
            let path = RichText::new(package.path.display().to_string()).font(theme::monospace(11.0));
            ui.add(egui::Label::new(path).wrap());
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if dialog_button(ui, "Open Folder", false).clicked()
                    && let Some(folder) = package.path.parent()
                {
                    crate::export_build::open_folder(folder);
                }
                if dialog_button(ui, "Copy to Desktop", false).clicked() {
                    package.copy_status = match reports::copy_to_desktop(&package.path) {
                        Ok(path) => format!("Copied to {}", path.display()),
                        Err(error) => format!("Could not copy the package: {error}"),
                    };
                }
            });
            if !package.copy_status.is_empty() {
                ui.add(egui::Label::new(&package.copy_status).wrap());
            }
            ui.add_space(8.0);
            if !package.entries.is_empty() {
                let mut combo = egui::ComboBox::from_id_salt("diagnostics-package-file");
                combo = combo.selected_text(&package.entries[package.selected].0);
                combo = combo.width(ui.available_width());
                combo = combo.truncate();
                combo.show_ui(ui, |ui| {
                    for (index, (name, _)) in package.entries.iter().enumerate() {
                        ui.selectable_value(&mut package.selected, index, name);
                    }
                });
                let contents = String::from_utf8_lossy(&package.entries[package.selected].1);
                let mut text = contents.as_ref();
                let mut field = egui::TextEdit::multiline(&mut text);
                field = field.font(theme::monospace(11.0));
                field = field.desired_rows(18);
                field = field.desired_width(f32::INFINITY);
                let mut scroll = egui::ScrollArea::vertical();
                scroll = scroll.id_salt(("diagnostics-package-contents", package.selected));
                scroll = scroll.max_height(360.0);
                scroll.show(ui, |ui| ui.add(field));
            }
        });
        ui.add_space(8.0);
        dialog_button_row(ui, |ui| {
            if dialog_button(ui, "Close", false).clicked() {
                close = true;
            }
            if dialog_button(ui, "Back", true).clicked() {
                window.viewing_package = false;
            }
        });
    });
    if let Some(shown) = shown {
        fit.end(ctx, shown.response.rect);
    }
    open && !close
}

fn poll_package(window: &mut DiagnosticsWindow) {
    let Some(receiver) = window.package_job.as_ref() else {
        return;
    };
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err("The diagnostics package worker stopped.".to_string()),
    };
    window.package_job = None;
    match result {
        Ok(package) => {
            window.say(format!("Diagnostics package created: {}", package.path.display()));
            window.package = Some(package);
            window.viewing_package = true;
        }
        Err(error) => window.say(format!("Error: {error}")),
    }
}

/// Start: the chosen task.
fn start(editor: &mut Editor, window: &mut DiagnosticsWindow) -> Option<DiagnosticsRequest> {
    match window.task {
        Task::Package => {
            let summary = summary(editor);
            let folder = editor.user.preferences.documents_folder();
            let (sender, receiver) = crossbeam_channel::bounded(1);
            window.say("Creating diagnostics package...");
            let spawned = std::thread::Builder::new().name("diagnostics package".to_string()).spawn(move || {
                let result = create_package(summary, &folder).map_err(|error| error.to_string());
                let _ = sender.send(result);
            });
            match spawned {
                Ok(_) => window.package_job = Some(receiver),
                Err(error) => window.say(format!("Error: {error}")),
            }
            None
        }
        Task::Verbose => {
            Some(restart_with(&["--verbose"]))
        }
        Task::SafeMode => {
            Some(restart_with(&["--safe-mode"]))
        }
        Task::CpuOnly => {
            Some(restart_with(&["--cpu-only"]))
        }
        Task::FactoryReset => match factory_reset(editor, window.backup) {
            Ok(backup) => {
                match backup {
                    Some(folder) => window.say(format!("Settings reset. The old settings are in {}", folder.display())),
                    None => window.say("Settings reset."),
                }
                Some(DiagnosticsRequest::SettingsReset)
            }
            Err(error) => {
                let folder = crate::user::data_folder().unwrap_or_default();
                window.say(format!("Settings could not be reset. Error: {error}\n\nManually delete the folder: {}", folder.display()));
                None
            }
        },
        Task::Cleanup => {
            let removed = cleanup();
            window.say(format!("Cleanup finished: {removed} files deleted."));
            None
        }
        Task::Benchmark(passes) => {
            window.say(format!("Starting benchmark ({passes} passes per device)..."));
            let (sender, receiver) = crossbeam_channel::unbounded();
            let spawned = std::thread::Builder::new().name("benchmark".to_string()).spawn(move || {
                let report = |line: String| {
                    let _ = sender.send(Some(line));
                };
                let scores = nerthus_compute::scan_with_passes(passes, report);
                if let Some(folder) = crate::user::data_folder() {
                    let _ = nerthus_compute::write_scores(&folder, &scores);
                }
                let _ = sender.send(None);
            });
            match spawned {
                Ok(_) => window.benchmark = Some(receiver),
                Err(error) => window.say(format!("Error: {error}")),
            }
            None
        }
    }
}

/// The benchmark's lines as they come; at its end, the scores.
fn poll_benchmark(window: &mut DiagnosticsWindow) {
    let Some(benchmark) = &window.benchmark else {
        return;
    };
    let mut lines = Vec::new();
    let mut finished = false;
    let mut stopped = false;
    loop {
        match benchmark.try_recv() {
            Ok(Some(line)) => lines.push(line),
            Ok(None) => {
                finished = true;
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                stopped = true;
                break;
            }
        }
    }
    for line in lines {
        window.say(line);
    }
    if stopped {
        window.benchmark = None;
        window.say("The benchmark worker stopped before completing. You can start it again.");
        return;
    }
    if finished {
        window.benchmark = None;
        window.say("\nBenchmark Results");
        let scores = crate::user::data_folder().map(|folder| nerthus_compute::read_scores(&folder)).unwrap_or_default();
        for (device, score) in scores.iter() {
            window.say(format!("{device}: {score:.0}"));
        }
        window.say("The higher the score, the better the performance.");
    }
}

/// Starts Nerthus again with these options once it has closed.
fn restart_with(extra: &[&str]) -> DiagnosticsRequest {
    let arguments = restart_arguments(std::env::args().skip(1), extra);
    DiagnosticsRequest::Restart(arguments)
}

/// Arms a restart only after the pending file action's save question has
/// been answered. CPU-only diagnostics also enables the persistent mode.
pub fn prepare_restart(arguments: Vec<String>) -> std::io::Result<()> {
    if arguments.iter().any(|argument| argument == "--cpu-only")
        && let Some(folder) = crate::user::data_folder()
    {
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join(nerthus_compute::CPU_ONLY_FILE), "!")?;
    }
    let mut restart = RESTART.lock().map_err(|error| std::io::Error::other(error.to_string()))?;
    *restart = Some(arguments);
    Ok(())
}

pub(super) fn take_restart() -> Option<Vec<String>> {
    RESTART.lock().ok().and_then(|mut restart| restart.take())
}

/// Replace restart switches in any spelling accepted by the launcher.
/// Keep the project and other windows, but do not reopen Diagnostics.
fn restart_arguments(arguments: impl Iterator<Item = String>, extra: &[&str]) -> Vec<String> {
    let mut arguments = arguments;
    let mut restarted = Vec::new();
    while let Some(argument) = arguments.next() {
        let option = argument.strip_prefix("--").or_else(|| argument.strip_prefix('-'));
        let option = option.or_else(|| if cfg!(target_os = "windows") { argument.strip_prefix('/') } else { None });
        let Some(option) = option else {
            restarted.push(argument);
            continue;
        };
        let (name, written_value) = match option.split_once([':', '=']) {
            Some((name, value)) => (name, Some(value)),
            None => (option, None),
        };
        let name = name.replace(['-', '_'], "").to_ascii_lowercase();
        match name.as_str() {
            "verbose" | "cpuonly" | "safemode" | "onboard" | "diagnostics" => {}
            "show" => {
                let panels = written_value.map(str::to_string).or_else(|| arguments.next());
                if let Some(panels) = panels {
                    let panels: Vec<&str> = panels.split(',').filter(|panel| !panel.eq_ignore_ascii_case("diagnostics")).collect();
                    if !panels.is_empty() {
                        restarted.push("--show".to_string());
                        restarted.push(panels.join(","));
                    }
                }
            }
            _ => restarted.push(argument),
        }
    }
    for argument in extra {
        restarted.push(argument.to_string());
    }
    restarted
}

/// Starts the new Nerthus as this one closes.
pub fn restart_on_exit(mut exits: bevy::prelude::MessageReader<bevy::prelude::AppExit>) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(arguments) = take_restart() else {
        return;
    };
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    if let Err(error) = std::process::Command::new(program).args(arguments).spawn() {
        log::error!("Cannot start Nerthus again: {error}");
    }
}

/// The diagnostics package: `Nerthus_Diagnostics_<time>.zip` in the
/// documents folder.
fn create_package(summary: String, folder: &Path) -> std::io::Result<DiagnosticsPackage> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    entries.push(("summary.json".to_string(), summary.into_bytes()));
    entries.push(("Devices.txt".to_string(), devices().into_bytes()));
    entries.push(("Hardware.txt".to_string(), reports::hardware().into_bytes()));
    entries.push(("ProgramFiles.txt".to_string(), reports::program_files().into_bytes()));
    entries.push(("Processes.txt".to_string(), reports::processes().into_bytes()));
    entries.push(("SessionLog.txt".to_string(), crate::logging::session_text().into_bytes()));
    if let Some(folder) = crate::user::data_folder() {
        let settings = ["user.options", nerthus_engine::paths::PREFERENCES_FILE, "recent-files.mru", nerthus_compute::BENCHMARK_FILE];
        for name in settings {
            if let Ok(bytes) = std::fs::read(folder.join(name)) {
                entries.push((format!("Settings/{name}"), bytes));
            }
        }
        // The ten newest logs.
        let logs = folder.join("Logs");
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&logs).into_iter().flatten().flatten() {
            let modified = entry.metadata().and_then(|metadata| metadata.modified()).unwrap_or(std::time::UNIX_EPOCH);
            files.push((modified, entry.path()));
        }
        files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        for (_, path) in files.into_iter().take(10) {
            if let (Some(name), Ok(bytes)) = (path.file_name(), std::fs::read(&path)) {
                entries.push((format!("Logs/{}", name.to_string_lossy()), bytes));
            }
        }
    }
    std::fs::create_dir_all(folder)?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S_%f");
    let path = folder.join(format!("Nerthus_Diagnostics_{stamp}.zip"));
    let borrowed: Vec<(&str, &[u8])> = entries.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    std::fs::write(&path, stored_zip(&borrowed))?;
    Ok(DiagnosticsPackage { path, entries, selected: 0, copy_status: String::new() })
}

/// `summary.json`: the version and the machine.
fn summary(editor: &Editor) -> String {
    let cores = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
    let memory = crate::memory::process_bytes().unwrap_or(0);
    let value = serde_json::json!({
        "Nerthus Version": env!("CARGO_PKG_VERSION"),
        "OS": std::env::consts::OS,
        "Architecture": std::env::consts::ARCH,
        "Cores": cores,
        "Process Memory": memory,
        "Engine Cache": editor.cache_bytes,
        "Verbose Logging": crate::logging::verbose(),
        "CPU Only": nerthus_compute::is_cpu_only(),
        "Project": editor.document.path.as_ref().map(|path| path.display().to_string()),
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// `Devices.txt`: the compute devices and which are on.
fn devices() -> String {
    let mut text = String::new();
    let active: Vec<String> = nerthus_compute::active_devices().into_iter().map(|device| device.id).collect();
    let (devices, problems) = nerthus_compute::available_devices();
    for device in devices {
        let state = if active.contains(&device.id) { "active" } else { "inactive" };
        text.push_str(&format!(" DEVICE: {} ({}) [{state}]\n    {}\n", device.device_name, device.backend_name, device.id));
    }
    if !problems.is_empty() {
        text.push_str(&format!("\n{problems}\n"));
    }
    text
}

/// Settings back to their defaults, the old ones copied to
/// `Settings Backup <time>` first if asked. Returns the backup's folder.
fn factory_reset(editor: &mut Editor, backup: bool) -> std::io::Result<Option<PathBuf>> {
    let Some(folder) = crate::user::data_folder() else {
        return Ok(None);
    };
    let names = ["user.options", "preferences.options", "onboard.dat", crate::logging::VERBOSE_FILE, nerthus_compute::CPU_ONLY_FILE];
    let mut backup_folder = None;
    if backup {
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let target = folder.join(format!("Settings Backup {stamp}"));
        std::fs::create_dir_all(&target)?;
        for name in names {
            let source = folder.join(name);
            if source.exists() {
                std::fs::copy(&source, target.join(name))?;
            }
        }
        backup_folder = Some(target);
    }
    for name in names {
        let path = folder.join(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    editor.user.options = crate::user::UserOptions::new();
    editor.user.preferences = crate::user::Preferences::new();
    editor.user.options.mark_changed();
    editor.user.preferences.mark_changed();
    Ok(backup_folder)
}

/// The data folder's `Logs` (but the log being written), `Cache` and
/// `Autosaves` emptied, as Watson deletes them. Returns how many files went.
fn cleanup() -> usize {
    let mut removed = 0;
    let current_log = crate::logging::current_log_file();
    for name in ["Logs", "Cache", "Autosaves"] {
        if let Some(folder) = crate::files::data_subfolder(name) {
            removed += delete_files(&folder, current_log.as_deref());
        }
    }
    removed
}

fn delete_files(folder: &Path, keep: Option<&Path>) -> usize {
    let mut removed = 0;
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            removed += delete_files(&path, keep);
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        if Some(path.as_path()) == keep {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// A zip archive of files stored as they are (no compression).
fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in entries {
        let crc = crc32(contents);
        let offset = archive.len() as u32;
        let name_bytes = name.as_bytes();
        let size = contents.len() as u32;
        let header = |signature: u32, central: bool| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&signature.to_le_bytes());
            if central {
                bytes.extend_from_slice(&20u16.to_le_bytes());
            }
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0x0800u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0x21u16.to_le_bytes());
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes
        };
        archive.extend_from_slice(&header(0x0403_4b50, false));
        archive.extend_from_slice(name_bytes);
        archive.extend_from_slice(contents);
        directory.extend_from_slice(&header(0x0201_4b50, true));
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name_bytes);
    }
    let directory_offset = archive.len() as u32;
    let count = entries.len() as u16;
    archive.extend_from_slice(&directory);
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&count.to_le_bytes());
    archive.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

/// CRC-32 (IEEE), as zip files carry it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keeps_its_title_and_buttons_in_small_windows() {
        use crate::ui::widgets::fit_checks;
        let context = fit_checks::context();
        let mut editor = Editor::new(crate::document::Document::new());
        let mut window = DiagnosticsWindow::open();
        window.task = Task::FactoryReset;
        for line in 0..40 {
            window.say(format!("Line {line}"));
        }
        for screen in fit_checks::SMALL_SCREENS {
            let words = fit_checks::settle(&context, screen, |ctx| {
                show(ctx, &mut editor, &mut window, false);
            });
            let expected = ["Nerthus Diagnostics", "This program is designed", "Start", "Close"];
            fit_checks::assert_fits(&context, screen, Id::new("diagnostics"), &words, &expected);
        }
    }

    #[test]
    fn package_viewer_keeps_its_navigation_in_small_windows() {
        use crate::ui::widgets::fit_checks;
        let context = fit_checks::context();
        let mut editor = Editor::new(crate::document::Document::new());
        let mut window = DiagnosticsWindow::open();
        let package = DiagnosticsPackage {
            path: PathBuf::from("/Documents/Nerthus_Diagnostics_test.zip"),
            entries: vec![("summary.json".to_string(), b"unselected summary contents".to_vec()), ("Hardware.txt".to_string(), "Hardware report\n".repeat(100).into_bytes())],
            selected: 1,
            copy_status: String::new(),
        };
        window.package = Some(package);
        window.viewing_package = true;
        for screen in fit_checks::SMALL_SCREENS {
            let words = fit_checks::settle(&context, screen, |ctx| {
                show(ctx, &mut editor, &mut window, false);
            });
            let expected = ["Diagnostics Package", "Close", "Back"];
            fit_checks::assert_fits(&context, screen, window_id(), &words, &expected);
            assert!(words.iter().any(|(text, _, _)| text.contains("Hardware report")));
            assert!(!words.iter().any(|(text, _, _)| text.contains("unselected summary contents")));
        }
    }

    #[test]
    fn known_updates_warn_in_diagnostics() {
        use crate::ui::widgets::fit_checks;
        let context = fit_checks::context();
        let mut editor = Editor::new(crate::document::Document::new());
        let mut window = DiagnosticsWindow::open();
        let words = fit_checks::settle(&context, egui::Vec2::new(1000.0, 1000.0), |ctx| {
            show(ctx, &mut editor, &mut window, true);
        });
        assert!(words.iter().any(|(text, _, _)| text.contains("A newer version of Nerthus is available")));
        assert!(words.iter().any(|(text, _, _)| text.contains("Run Nerthus in Safe Mode")));
        assert!(words.iter().any(|(text, _, _)| text.contains("High Accuracy (20 passes)")));
        assert!(words.iter().any(|(text, _, _)| text.contains("Very High Accuracy (50 passes)")));
        let words = fit_checks::settle(&context, egui::Vec2::new(1000.0, 1000.0), |ctx| {
            show(ctx, &mut editor, &mut window, false);
        });
        assert!(!words.iter().any(|(text, _, _)| text.contains("A newer version of Nerthus is available")));
    }

    #[test]
    fn restarts_replace_mode_aliases_and_remove_diagnostics_only() {
        let current = [
            "project.terrain", "-Verbose:false", "--SafeMode", "--safe_mode", "--cpu_only", "--cpuonly=true", "--On-Board", "--diagnostics",
            "--show=console,diagnostics,build-settings:terrain", "--no-start", "--size", "1280x720",
        ];
        let restarted = restart_arguments(current.into_iter().map(str::to_string), &["--safe-mode"]);
        assert_eq!(restarted, ["project.terrain", "--show", "console,build-settings:terrain", "--no-start", "--size", "1280x720", "--safe-mode"]);
        let current = ["--show", "Diagnostics", "--safemode:false"];
        let restarted = restart_arguments(current.into_iter().map(str::to_string), &["--verbose"]);
        assert_eq!(restarted, ["--verbose"]);
    }

    #[test]
    fn completed_packages_open_in_the_viewer() {
        let mut window = DiagnosticsWindow::open();
        let package = DiagnosticsPackage {
            path: PathBuf::from("Nerthus_Diagnostics_test.zip"),
            entries: vec![("summary.json".to_string(), b"{}".to_vec())],
            selected: 0,
            copy_status: String::new(),
        };
        let (sender, receiver) = crossbeam_channel::bounded(1);
        window.package_job = Some(receiver);
        sender.send(Ok(package)).unwrap();
        poll_package(&mut window);
        assert!(window.package_job.is_none());
        assert!(window.viewing_package);
        assert_eq!(window.package.unwrap().entries[0].0, "summary.json");
        assert!(window.output.contains("Diagnostics package created"));
    }

    #[test]
    fn a_disconnected_benchmark_worker_releases_the_start_button() {
        let mut window = DiagnosticsWindow::open();
        let (sender, receiver) = crossbeam_channel::unbounded();
        window.benchmark = Some(receiver);
        sender.send(Some("Benchmarking device".to_string())).unwrap();
        drop(sender);
        poll_benchmark(&mut window);
        assert!(window.benchmark.is_none());
        assert!(window.output.contains("Benchmarking device"));
        assert!(window.output.contains("stopped before completing"));
        assert!(!window.output.contains("Benchmark Results"), "old scores must not be presented as a finished run");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn packages_include_the_hardware_installation_and_process_reports() {
        let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
        let folder = std::env::temp_dir().join(format!("nerthus-package-{}-{}", std::process::id(), elapsed.as_nanos()));
        let package = create_package("{}".to_string(), &folder).unwrap();
        let archive = std::fs::read(&package.path).unwrap();
        for name in ["summary.json", "Devices.txt", "Hardware.txt", "ProgramFiles.txt", "Processes.txt", "SessionLog.txt"] {
            let entry = package.entries.iter().find(|(entry_name, _)| entry_name == name).expect("required report");
            assert!(archive.windows(name.len()).any(|window| window == name.as_bytes()), "missing {name} in archive");
            if name != "SessionLog.txt" {
                assert!(!entry.1.is_empty(), "empty {name}");
            }
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn crc_matches_the_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// Archive tools read what Nerthus zips (`unzip -t`).
    #[test]
    #[ignore]
    fn write_sample_zip() {
        let path = std::env::var("NERTHUS_SAMPLE_OUT").expect("set NERTHUS_SAMPLE_OUT");
        let long_text = "Nerthus log line\n".repeat(500);
        let entries: [(&str, &[u8]); 3] = [("summary.json", b"{}"), ("Logs/one.txt", long_text.as_bytes()), ("Settings/user.options", b"")];
        std::fs::write(path, stored_zip(&entries)).unwrap();
    }

    #[test]
    fn zip_has_its_entries() {
        let archive = stored_zip(&[("a.txt", b"hello"), ("b/c.txt", b"world")]);
        assert_eq!(&archive[0..4], &0x0403_4b50u32.to_le_bytes());
        let end = archive.len() - 22;
        assert_eq!(&archive[end..end + 4], &0x0605_4b50u32.to_le_bytes());
        assert_eq!(u16::from_le_bytes([archive[end + 10], archive[end + 11]]), 2);
    }
}
