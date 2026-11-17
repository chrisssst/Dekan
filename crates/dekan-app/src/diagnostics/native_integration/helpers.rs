pub fn utc_now_micros() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let micros = duration.subsec_micros();
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let hours = day_secs / 3600;
    let mins = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    let mut year = 1970;
    let mut day_count = days;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if day_count < days_in_year {
            break;
        }
        day_count -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let feb = if leap { 29 } else { 28 };
    let days_in_months = [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1;
    for &dim in &days_in_months {
        if day_count < dim {
            break;
        }
        day_count -= dim;
        month += 1;
    }
    let day = day_count + 1;

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{seconds:02}.{micros:06}Z")
}

#[cfg(windows)]
pub fn query_process_memory_usage() -> u64 {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    let mut pmc = std::mem::MaybeUninit::<ProcessMemoryCounters>::zeroed();
    let cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
    let ok = unsafe {
        let ptr = pmc.as_mut_ptr();
        (*ptr).cb = cb;
        K32GetProcessMemoryInfo(GetCurrentProcess(), ptr, cb)
    };
    if ok != 0 {
        let initialized = unsafe { pmc.assume_init() };
        initialized.working_set_size as u64
    } else {
        0
    }
}

#[cfg(not(windows))]
pub fn query_process_memory_usage() -> u64 {
    0
}
