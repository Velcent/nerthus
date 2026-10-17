//! The process's memory for the status bar (its working set), and the
//! memory installed in the machine.

/// Bytes resident in the process's working set (macOS: the physical
/// footprint Activity Monitor shows); `None` where Nerthus cannot ask.
pub fn process_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
        let process = std::process::id() as libc::c_int;
        // SAFETY: `proc_pid_rusage` fills a `rusage_info_v2` for the
        // `RUSAGE_INFO_V2` flavour; the buffer is that struct's size.
        let status = unsafe { libc::proc_pid_rusage(process, libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
        if status != 0 {
            return None;
        }
        // SAFETY: the call succeeded, so the struct is initialised.
        let usage = unsafe { usage.assume_init() };
        Some(usage.ri_phys_footprint)
    }
    #[cfg(target_os = "linux")]
    {
        // `statm` reports virtual, resident, and shared memory in pages.
        let info = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident_pages: u64 = info.split_whitespace().nth(1)?.parse().ok()?;
        // SAFETY: querying the page size takes no additional arguments.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return None;
        }
        resident_pages.checked_mul(page_size as u64)
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let counters_size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        counters.cb = counters_size;
        // SAFETY: the pseudo-handle refers to this process and must not be
        // closed. The counters buffer and its byte size match the API.
        let succeeded = unsafe {
            let process = GetCurrentProcess();
            K32GetProcessMemoryInfo(process, &mut counters, counters_size) != 0
        };
        if !succeeded {
            return None;
        }
        Some(counters.WorkingSetSize as u64)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// The physical memory installed, in bytes (the memory modules'
/// capacities added up); `None` where Nerthus cannot ask.
pub fn installed_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut bytes: u64 = 0;
        let mut size = std::mem::size_of::<u64>();
        // SAFETY: `hw.memsize` is a 64-bit integer; the buffer and its size
        // say so, and no new value is set.
        let status = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &mut size, std::ptr::null_mut(), 0) };
        if status != 0 || bytes == 0 {
            return None;
        }
        Some(bytes)
    }
    #[cfg(target_os = "linux")]
    {
        // `MemTotal:  16314344 kB`.
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = info.lines().find(|line| line.starts_with("MemTotal:"))?;
        let kilobytes: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kilobytes * 1024)
    }
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            /// `sysinfoapi.h`: the installed memory in kilobytes, as the
            /// firmware reports the memory modules.
            fn GetPhysicallyInstalledSystemMemory(total_memory_in_kilobytes: *mut u64) -> i32;
        }
        let mut kilobytes: u64 = 0;
        // SAFETY: the function writes one 64-bit integer.
        let succeeded = unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) } != 0;
        if !succeeded || kilobytes == 0 {
            return None;
        }
        Some(kilobytes * 1024)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// Hands memory the allocator holds but no longer uses back to the system
/// (in place of a forced garbage collection with heap compaction, `ForcedGC`).
/// Returns the bytes released where the system says.
pub fn release_unused() -> u64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            /// `malloc/malloc.h`: frees what every zone can (a null zone
            /// means all of them; a goal of 0 means as much as possible).
            fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: libc::size_t) -> libc::size_t;
        }
        // SAFETY: a null zone and a goal of 0 are documented arguments.
        let released = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        released as u64
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            /// glibc's `malloc_trim`: returns free memory at the top of
            /// the heap and in its arenas to the system.
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: `malloc_trim` takes any padding and returns 0 or 1.
        unsafe { malloc_trim(0) };
        0
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_env = "gnu"))))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn process_memory_is_known() {
        let bytes = super::process_bytes().expect("the system reports the process's resident memory");
        assert!(bytes > 0, "{bytes} bytes");
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn installed_memory_is_known() {
        let bytes = super::installed_bytes().expect("the system reports its memory");
        assert!(bytes >= 512 << 20, "{bytes} bytes");
    }
}
