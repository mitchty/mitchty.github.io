// Translated from https://stackoverflow.com/a/5918976 (Julian Panetta et al.)
// License: CC BY-SA 3.0
//
// Use MACH_TASK_BASIC_INFO for this stuff. TASK_BASIC_INFO varies by arch (5 on
// x86_64, 18 on aarch64); MACH_TASK_BASIC_INFO is a single constant that works
// on all platforms and uses mach_vm_size_t as u64 for resident_size rather than
// the arch-dependent vm_size_t.
//
// resident_size is in bytes on macOS.

use mach2::kern_return::KERN_SUCCESS;
use mach2::task::task_info;
use mach2::task_info::{MACH_TASK_BASIC_INFO, MACH_TASK_BASIC_INFO_COUNT, mach_task_basic_info};
use mach2::traps::mach_task_self;

/// Returns the current process RSS in bytes, or 0 on error.
pub fn rss_bytes() -> u64 {
    unsafe {
        let mut info: mach_task_basic_info = std::mem::zeroed();
        let mut count = MACH_TASK_BASIC_INFO_COUNT;

        let kr = task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            &mut info as *mut mach_task_basic_info as *mut _,
            &mut count,
        );

        if kr != KERN_SUCCESS {
            return 0;
        }

        info.resident_size
    }
}
