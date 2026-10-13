//! The process's memory for the status bar (its working set).

/// Bytes the process holds as the system reports it (macOS: the physical
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
    #[cfg(not(target_os = "macos"))]
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
