//! Machine and installation reports included in a diagnostics package.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn hardware() -> String {
    let cores = std::thread::available_parallelism().map(usize::from).unwrap_or(1);
    let cpu = nerthus_compute::fallback_device_info();
    let mut report = format!(
        "Nerthus {}\nOperating system: {}\nArchitecture: {}\nProcessor: {}\nLogical processors: {cores}\n",
        env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, cpu.device_name,
    );
    if let Some(bytes) = crate::memory::installed_bytes() {
        let _ = writeln!(report, "Installed memory: {bytes} bytes");
    }
    #[cfg(target_os = "macos")]
    append_command(&mut report, "Hardware, memory and displays", "system_profiler", &["SPHardwareDataType", "SPMemoryDataType", "SPDisplaysDataType", "-detailLevel", "mini"]);
    #[cfg(target_os = "linux")]
    {
        append_command(&mut report, "System", "uname", &["-a"]);
        append_command(&mut report, "Processor", "lscpu", &[]);
        for path in ["/etc/os-release", "/proc/meminfo"] {
            if let Ok(text) = std::fs::read_to_string(path) {
                let _ = write!(report, "\n{path}\n{text}");
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = concat!(
            "Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber,OSArchitecture | Format-List; ",
            "Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | Format-List; ",
            "Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model,TotalPhysicalMemory | Format-List; ",
            "Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,AdapterRAM | Format-List",
        );
        append_command(&mut report, "Hardware and operating system", "powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", script]);
    }
    report
}

pub fn processes() -> String {
    let mut report = "Running processes at package creation\n".to_string();
    #[cfg(target_os = "windows")]
    append_command(&mut report, "Processes", "tasklist", &["/FO", "CSV"]);
    #[cfg(unix)]
    append_command(&mut report, "PID, parent PID, CPU %, memory %, resident memory (KiB), executable", "ps", &["-axo", "pid,ppid,%cpu,%mem,rss,comm"]);
    #[cfg(not(any(unix, target_os = "windows")))]
    report.push_str("Process enumeration is unavailable on this platform.\n");
    report
}

fn append_command(report: &mut String, title: &str, program: &str, arguments: &[&str]) {
    let _ = writeln!(report, "\n{title}\n");
    let mut command = Command::new(program);
    command.args(arguments);
    command.stdin(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Background reports do not need a console window.
        command.creation_flags(0x0800_0000);
    }
    match command.output() {
        Ok(output) => {
            report.push_str(&String::from_utf8_lossy(&output.stdout));
            if !output.status.success() {
                let _ = writeln!(report, "\n{program} exited with {}", output.status);
            }
            if !output.stderr.is_empty() {
                report.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            report.push('\n');
        }
        Err(error) => {
            let _ = writeln!(report, "Cannot run {program}: {error}");
        }
    }
}

pub fn program_files() -> String {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => return format!("Cannot locate the program: {error}\n"),
    };
    let app_bundle = program.ancestors().find(|path| path.extension().is_some_and(|extension| extension == "app"));
    let Some(folder) = app_bundle.or_else(|| program.parent()) else {
        return format!("Cannot locate the program folder: {}\n", program.display());
    };
    if folder.parent().is_none() {
        return format!("Executable: {}\nThe program is in the filesystem root; a recursive inventory was skipped.\n", program.display());
    }
    let mut report = format!("Executable: {}\n", program.display());
    report.push_str(&files_in(folder));
    report
}

fn files_in(folder: &Path) -> String {
    let mut report = format!("Program folder: {}\n\nSize (bytes)\tFile\n", folder.display());
    list_files(folder, folder, &mut report);
    report
}

fn list_files(root: &Path, folder: &Path, report: &mut String) {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) => {
            let _ = writeln!(report, "Cannot read {}: {error}", folder.display());
            return;
        }
    };
    let mut entries: Vec<_> = entries.collect();
    entries.sort_by_key(|entry| entry.as_ref().map(|entry| entry.path()).unwrap_or_default());
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let _ = writeln!(report, "Cannot read a directory entry: {error}");
                continue;
            }
        };
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                let _ = writeln!(report, "Cannot inspect {}: {error}", relative.display());
                continue;
            }
        };
        if metadata.is_dir() {
            list_files(root, &path, report);
        } else if metadata.is_symlink() {
            let _ = writeln!(report, "symlink\t{}", relative.display());
        } else {
            let _ = writeln!(report, "{}\t{}", metadata.len(), relative.display());
        }
    }
}

/// Copies the package without replacing an existing desktop file.
pub fn copy_to_desktop(source: &Path) -> std::io::Result<PathBuf> {
    let directories = directories::UserDirs::new();
    let desktop = directories.as_ref().and_then(|directories| directories.desktop_dir());
    let folder = desktop.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "The desktop folder could not be found."))?;
    copy_to_folder(source, folder)
}

fn copy_to_folder(source: &Path, folder: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let name = source.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "The package has no file name."))?;
    let stem = source.file_stem().unwrap_or(name);
    let mut source_file = std::fs::File::open(source)?;
    for number in 0.. {
        let mut candidate = name.to_os_string();
        if number > 0 {
            candidate = stem.to_os_string();
            candidate.push(format!(" ({number})"));
            if let Some(extension) = source.extension() {
                candidate.push(".");
                candidate.push(extension);
            }
        }
        let destination = folder.join(candidate);
        let mut options = std::fs::OpenOptions::new();
        options.write(true);
        options.create_new(true);
        let mut destination_file = match options.open(&destination) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = std::io::copy(&mut source_file, &mut destination_file) {
            drop(destination_file);
            let _ = std::fs::remove_file(&destination);
            return Err(error);
        }
        return Ok(destination);
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_folder() -> PathBuf {
        let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
        std::env::temp_dir().join(format!("nerthus-diagnostics-{}-{}", std::process::id(), elapsed.as_nanos()))
    }

    #[test]
    fn installation_inventory_includes_nested_files_without_following_symlinks() {
        let folder = temporary_folder();
        std::fs::create_dir_all(folder.join("resources")).unwrap();
        std::fs::write(folder.join("nerthus"), b"binary").unwrap();
        std::fs::write(folder.join("resources/shader.wgsl"), b"shader data").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&folder, folder.join("loop")).unwrap();
        let report = files_in(&folder);
        assert!(report.contains("6\tnerthus"));
        let shader_path = Path::new("resources").join("shader.wgsl");
        assert!(report.contains(&format!("11\t{}", shader_path.display())));
        assert!(!report.contains("shader data"));
        #[cfg(unix)]
        assert!(report.contains("symlink\tloop"));
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn desktop_copy_preserves_existing_files_and_the_original_package() {
        let folder = temporary_folder();
        let desktop = folder.join("Desktop");
        std::fs::create_dir_all(&desktop).unwrap();
        let source = folder.join("Nerthus_Diagnostics_test.zip");
        let existing = desktop.join("Nerthus_Diagnostics_test.zip");
        std::fs::write(&source, b"package").unwrap();
        std::fs::write(&existing, b"previous copy").unwrap();
        let copied = copy_to_folder(&source, &desktop).unwrap();
        assert_eq!(copied.file_name().unwrap(), "Nerthus_Diagnostics_test (1).zip");
        assert_eq!(std::fs::read(copied).unwrap(), b"package");
        assert_eq!(std::fs::read(source).unwrap(), b"package");
        assert_eq!(std::fs::read(existing).unwrap(), b"previous copy");
        std::fs::remove_dir_all(folder).unwrap();
    }
}
