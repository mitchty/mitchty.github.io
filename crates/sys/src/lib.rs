//! Thin platform-specific wrappers for process-level OS stats, plus a Bevy
//! plugin that logs RSS and Bevy diagnostics every second.
//!
//! Currently exposes:
//! - `rss_bytes()` - current process RSS in bytes
//! - `SysPlugin`   - Bevy plugin (always available, no feature gate required)
//!
//! Platform support for `rss_bytes()`:
//! - macOS   : mach `task_info(MACH_TASK_BASIC_INFO)` -> `resident_size`
//! - Linux   : `/proc/self/status` VmRSS field
//! - Windows : `GetProcessMemoryInfo` -> `WorkingSetSize`
//! - other   : stub returning 0
// TODO: wasm, do I care? can I even abuse this kinda stuff?

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

/// Returns the current process RSS in bytes.
///
/// - macOS/Linux/Windows: live reading from the OS.
/// - All other platforms: always returns 0.
/// - Returns 0 on any OS call failure.
pub fn rss_bytes() -> u64 {
    #[cfg(target_os = "macos")]
    return macos::rss_bytes();

    #[cfg(target_os = "linux")]
    return linux::rss_bytes();

    #[cfg(target_os = "windows")]
    return windows::rss_bytes();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return 0;
}

use bevy::diagnostic::{
    DiagnosticsStore, EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin,
};
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;

/// Emit a warning when process RSS exceeds this many GiB.
// TODO: This is all just simple for debugging.
const WARN_RSS_GIB: u64 = 2;

/// Bevy plugin that tracks per-process memory RSS and logs Bevy diagnostics
/// once per second.
///
/// - Logs current RSS + delta from baseline each second.
/// - Logs entity count, frame time, and FPS each second.
/// - Emits a `warn!` if RSS reaches or exceeds `WARN_RSS_GIB`.
///
/// No feature gate for it right now. The bevy sysinfo plugin doesn't give me
/// good memory data and I'm a hack so this'll do.
pub struct SysPlugin;

impl Plugin for SysPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EntityCountDiagnosticsPlugin::default())
            .add_systems(Startup, startup)
            .add_systems(
                Update,
                (update, warn_on_growth, log_diagnostics)
                    .run_if(on_timer(std::time::Duration::from_secs(1))),
            );
    }
}

/// Marker for the entity carrying process stats.
#[derive(Debug, Default, Component)]
pub struct PidStats;

/// Wall-clock instant the plugin started up.
#[derive(Debug, Component, Deref)]
struct Start(std::time::Instant);

/// Uptime in whole seconds since `Start`.
#[derive(Default, Debug, Component, Deref)]
pub struct Uptime(pub u64);

/// Current process RSS in bytes.
#[derive(Default, Debug, Component, Deref)]
pub struct Mem(pub u64);

/// RSS at plugin startup used to show growth relative to a known baseline from
/// startup.
#[derive(Default, Debug, Component, Deref)]
struct Baseline(u64);

fn startup(mut commands: Commands) {
    let rss = rss_bytes();
    debug!(
        "initial rss={}MiB ({:.2}GiB)",
        rss / (1024 * 1024),
        rss as f64 / (1024.0 * 1024.0 * 1024.0),
    );
    commands.spawn((
        PidStats,
        Start(std::time::Instant::now()),
        Uptime(0),
        Mem(rss),
        Baseline(rss),
    ));
}

fn update(mut stats: Query<(&Start, &mut Uptime, &mut Mem, &Baseline), With<PidStats>>) {
    let Ok((start, mut uptime, mut mem, baseline)) = stats.single_mut() else {
        return;
    };

    *mem = Mem(rss_bytes());
    *uptime = Uptime(std::time::Instant::now().duration_since(**start).as_secs());

    let rss_mib = **mem / (1024 * 1024);
    // delta is signed so we can see if RSS somehow drops aka after gc/free().
    let delta_mib = (**mem as i64 - **baseline as i64) / (1024 * 1024);
    debug!(
        "uptime={}s rss={}MiB ({:.2}GiB) delta={:+}MiB",
        **uptime,
        rss_mib,
        **mem as f64 / (1024.0 * 1024.0 * 1024.0),
        delta_mib,
    );
}

/// Logs Bevy's built-in diagnostics entity count and frame time once per second.
fn log_diagnostics(diag: Res<DiagnosticsStore>) {
    let entity_count = diag
        .get(&EntityCountDiagnosticsPlugin::ENTITY_COUNT)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0) as u64;

    let frame_ms = diag
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let fps = diag
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    // This doesn't need to be dumped out in a normal run, only if someone wants
    // it like me.
    debug!(
        "entities:{} | frame:{:.2}ms fps:{:.1}",
        entity_count, frame_ms, fps,
    );
}

/// Emits a warning every second while RSS is at or above the threshold.
fn warn_on_growth(stats: Query<&Mem, With<PidStats>>) {
    let Ok(mem) = stats.single() else {
        return;
    };

    if **mem / (1024 * 1024 * 1024) >= WARN_RSS_GIB {
        warn!(
            "RSS {:.2}GiB is above the default {}GiB threshold",
            **mem as f64 / (1024.0 * 1024.0 * 1024.0),
            WARN_RSS_GIB,
        );
    }
}
