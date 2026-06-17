// WorkingSetSize is the number of physical RAM pages currently mapped into the
// process which is apparently the Windows equivalent of RSS on Linux/macOS. It is what Task
// Manager shows as "Working Set" and I am treating it as RSS equivalently.
//
// GetCurrentProcess() returns a pseudo-handle (-1) that is always valid for
// the calling process and does not need to be closed based on other code on github.

use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::GetCurrentProcess;

/// Returns the current process Working Set size in bytes, or 0 on error.
pub fn rss_bytes() -> u64 {
    unsafe {
        let handle = GetCurrentProcess();
        let mut pmc = PROCESS_MEMORY_COUNTERS::default();
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

        if GetProcessMemoryInfo(handle, &mut pmc, size).is_ok() {
            pmc.WorkingSetSize as u64
        } else {
            0
        }
    }
}
