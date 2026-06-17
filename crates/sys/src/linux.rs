/// Reads process RSS from /proc/self/status VmRSS field directly.
///
/// /proc/self/status has a line like:
///   VmRSS:      1234 kB
///
/// Parse that line and return the value in bytes.
/// Falls back to 0 on any parse or I/O error.
///
/// Callers can deal with errors, maybe make this a Result if it ever matters.
pub fn rss_bytes() -> u64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };

    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let trimmed = rest.trim();
            // strip the trailing " kB" unit and convert to KiB
            let numeric = trimmed.trim_end_matches(" kB").trim();
            if let Ok(kb) = numeric.parse::<u64>() {
                return kb * 1024;
            }
        }
    }

    0
}
