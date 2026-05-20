//! Platform-specific startup quirks. Mostly only wine/windows for now.
//! Everything else runs sane it seems. The quirks here are just for jiff
//! timezone calls to work sanely.

#[cfg(target_os = "windows")]
use std::ffi::c_void;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleA(lpModuleName: *const u8) -> *const c_void;
    fn GetProcAddress(hModule: *const c_void, lpProcName: *const u8) -> *const c_void;
}

/// Returns `true` when the binary is seemingly running under Wine.
///
/// Probes `kernel32.dll` for `wine_get_unix_file_name`, which Wine exports
/// but real Windows never does. "allegedly"
#[cfg(target_os = "windows")]
fn is_wine() -> bool {
    unsafe {
        let kernel32 = GetModuleHandleA("kernel32.dll\0".as_ptr());
        if kernel32.is_null() {
            return false;
        }
        !GetProcAddress(kernel32, "wine_get_unix_file_name\0".as_ptr()).is_null()
    }
}

/// Apply platform-specific startup fixups.
///
/// On Windows and Wine only: strips `TZDIR` from the environment so that `jiff`
/// falls back to the bundled timezone database instead of looking for a
/// zoneinfo directory that may not exist in a Wine prefix. Like my test nixos
/// install *cough*.
pub fn platform_startup() {
    #[cfg(target_os = "windows")]
    {
        if is_wine() {
            log::info!("running under Wine - applying startup quirks");
            // Yes this is unsafe cause in a posix env itself is thread unsafe
            // for removing an env var boo on posix/unix design from before I
            // was born.
            unsafe {
                std::env::remove_var("TZDIR");
            }
            log::info!("wine quirk: removed TZDIR so jiff uses bundled tzdb");
        } else {
            log::debug!("Windows native - no Wine quirks needed");
        }
    }
}
