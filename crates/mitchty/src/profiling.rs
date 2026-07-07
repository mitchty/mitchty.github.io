//! Memory profiling plugin.
//!
//! Debug cfg gated to only apply when added via cargo build --features
//! stats-alloc to help debug resource leaks. How I tracked down wgpu bind group
//! leaks in the old implementations of Materials. More work to add but this is
//! good enough to commit as-is even if it is jank af.
//!
//! Future me feature notes cause this stuffs not likely to be needed often.
//! Though if I'm honest I should create some kinda integration test to validate
//! that memory leaks don't seem to be present after running for N
//! minutes/hours. I need to noodle on that one though that could get messy and
//! implicitly will take a lot of time. Also need to make it possible to have
//! the binary run through different parts at runtime as well and that will be a
//! fun thing to add into this heap of complexity.
//!
//! ## `stats-alloc`
//! Logs live heap bytes and per-interval delta. Use this first to detect:
//! - delta near zero, RSS grows -> fragmentation or GPU/mmap RSS outside the heap;
//!   switch to `jemalloc-pprof` to confirm.
//! - live_allocs flat but bytes grow -> a small number of large/growing allocations
//!   (Vec/buffer capacity growth, cache without eviction); jeprof diff two snapshots.
//! - delta consistently positive -> heap leak; try `dhat-heap` or bisect plugins.
//!
//! ## `jemalloc-pprof`
//! Logs jemalloc's `allocated` / `resident` / `retained` every `STATS_DURATION` s.
//! Interpretation:
//! - `allocated` grows               -> true heap leak; enable prof dumps and jeprof it.
//! - `allocated` flat, `resident` grows   -> internal allocator fragmentation.
//! - `resident` flat, `retained` grows    -> jemalloc holding freed VA space; not a real leak.
//!
//! To capture heap-dump files for call-site attribution:
//!
//! **IMPORTANT**: this crate uses `unprefixed_malloc_on_supported_platforms`
//! so jemalloc reads `MALLOC_CONF` (not `_RJEM_MALLOC_CONF`). Also,
//! `prof:true` **must** be set at process start - it cannot be enabled at
//! runtime via ctl. The `prof_active` and `lg_prof_interval` tunables can
//! be changed at runtime. Programmatic dumps are triggered via
//! `jemalloc_ctl::raw::write("prof.dump", ...)` and are used below.
//!
//! **Env var**: `unprefixed_malloc_on_supported_platforms` is NOT enabled (it
//! routes all shared-lib malloc through jemalloc, contaminating profiles with
//! driver allocations). Without it, jemalloc reads `_RJEM_MALLOC_CONF`, not
//! `MALLOC_CONF`. Always use the prefixed form.
//!
//! Build separately then run directly so build-subprocess jemalloc noise
//! (from system jemalloc seeing the env var) doesn't appear in output:
//! ```text
//! cargo build --features jemalloc-pprof --profile profiling
//! _RJEM_MALLOC_CONF=prof:true,prof_active:true,prof_prefix:/tmp/heap \
//!   ./target/profiling/mitchty
//! ```
//! The plugin dumps automatically each `STATS_DURATION` tick when allocated is
//! growing, so no `lg_prof_interval` threshold is needed.
//! Diff two snapshots to isolate growth:
//! ```text
//! jeprof --base=/tmp/heap.<pid>.0001.heap --pdf \
//!   ./target/debug/mitchty /tmp/heap.<pid>.0003.heap > leak_diff.pdf
//! ```

use bevy::prelude::*;

pub struct ProfilingPlugin;

// This whole things is only gated to features stats-alloc or jemalloc-pprof so
// it shows up as unused for default compiles, so ignore it.
#[allow(dead_code)]
const STATS_DURATION: u64 = 60;

impl Plugin for ProfilingPlugin {
    fn build(&self, app: &mut App) {
        // suppress unused warning in non-profiling builds, its annoying af
        let _ = app;

        #[cfg(feature = "stats-alloc")]
        app.add_systems(
            Update,
            log_alloc_stats.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(STATS_DURATION),
            )),
        );

        #[cfg(all(
            feature = "jemalloc-pprof",
            not(target_arch = "wasm32"),
            not(target_env = "msvc")
        ))]
        app.add_systems(
            Update,
            log_jemalloc_stats.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(STATS_DURATION),
            )),
        );
    }
}

#[cfg(feature = "stats-alloc")]
fn log_alloc_stats(mut prev_net: Local<Option<i64>>) {
    let s = crate::ALLOC_STATS.stats();

    // Net live bytes = total ever allocated minus total ever freed.
    let net = s.bytes_allocated as i64 - s.bytes_deallocated as i64;
    let live_allocs = s.allocations as i64 - s.deallocations as i64;

    match *prev_net {
        Some(prev) => {
            let delta = net - prev;
            info!(
                "stats_alloc net={} live_allocs={} delta={}/{}s{}",
                fmt_bytes(net),
                live_allocs,
                fmt_bytes_signed(delta),
                STATS_DURATION,
                if delta > 0 { " growing" } else { "" },
            );
        }
        None => {
            info!(
                "stats_alloc net={} live_allocs={} prime sample",
                fmt_bytes(net),
                live_allocs,
            );
        }
    }

    *prev_net = Some(net);
}

#[cfg(all(
    feature = "jemalloc-pprof",
    not(target_arch = "wasm32"),
    not(target_env = "msvc")
))]
fn log_jemalloc_stats(mut prev_allocated: Local<Option<i64>>) {
    use tikv_jemalloc_ctl::{epoch, stats};

    // Advance the epoch so jemalloc refreshes cached statistics.
    let Ok(e) = epoch::mib() else {
        warn!("jemalloc failed to get epoch mib");
        return;
    };
    if e.advance().is_err() {
        warn!("jemalloc failed to advance epoch");
        return;
    }

    let (allocated, resident, retained) = match (
        stats::allocated::mib(),
        stats::resident::mib(),
        stats::retained::mib(),
    ) {
        (Ok(a), Ok(r), Ok(ret)) => match (a.read(), r.read(), ret.read()) {
            (Ok(av), Ok(rv), Ok(retv)) => (av as i64, rv as i64, retv as i64),
            _ => {
                warn!("jemalloc failed to read stats");
                return;
            }
        },
        _ => {
            warn!("jemalloc failed to get stats mib");
            return;
        }
    };

    // fragmentation = RSS that jemalloc holds but hasn't given to live objects
    // retained = virtual address space returned to jemalloc but not yet munmap'd
    let fragmentation = resident - allocated;

    // Compute delta outside the match so it's available for the dump call below.
    let delta = prev_allocated.map(|prev| allocated - prev);

    match delta {
        Some(d) => {
            info!(
                "jemalloc allocated={} resident={} frag={} retained={} delta={}/{}s{}",
                fmt_bytes(allocated),
                fmt_bytes(resident),
                fmt_bytes(fragmentation),
                fmt_bytes(retained),
                fmt_bytes_signed(d),
                STATS_DURATION,
                if d > 0 { " growing" } else { "" },
            );
        }
        None => {
            info!(
                "jemalloc allocated={} resident={} frag={} retained={} prime sample",
                fmt_bytes(allocated),
                fmt_bytes(resident),
                fmt_bytes(fragmentation),
                fmt_bytes(retained),
            );
        }
    }

    *prev_allocated = Some(allocated);

    // When allocated is growing, fire a programmatic heap dump so we get
    // call-site data without relying on lg_prof_interval byte thresholds.
    // Silently skips if prof:true was not set at startup.
    if delta.is_some_and(|d| d > 0) {
        dump_heap_profile();
    }
}

/// Attempt a programmatic jemalloc heap dump.
///
/// Requires `MALLOC_CONF=prof:true,prof_prefix:/tmp/heap` (or equivalent) at
/// process start. The dump path is controlled by `prof_prefix`; jemalloc
/// appends `.<pid>.<seq>.heap` automatically.
///
/// Silently no-ops when profiling was not enabled at startup - jemalloc
/// returns EFAULT/ENOENT for `prof.dump` when `prof:true` was absent, which
/// we swallow quietly to avoid noise in normal non-debug runs.
#[cfg(all(
    feature = "jemalloc-pprof",
    not(target_arch = "wasm32"),
    not(target_env = "msvc")
))]
fn dump_heap_profile() {
    // tikv_jemalloc_ctl doesn't expose a typed `prof.dump` mib, so use the
    // raw write interface. Passing a null ptr tells jemalloc to use the
    // prefix configured via `prof_prefix` in MALLOC_CONF.
    // Returns an error (EFAULT or ENOENT) when prof:true was not set at
    // startup - treat that as a silent no-op, not a warning.
    let result = unsafe {
        tikv_jemalloc_ctl::raw::write::<*const std::ffi::c_char>(b"prof.dump\0", std::ptr::null())
    };

    match result {
        Ok(_) => info!("jemalloc heap dump written (prof_prefix path)"),
        Err(_) => {
            // Profiling not enabled at startup (prof:true absent from
            // MALLOC_CONF) - silently skip rather than warn every interval.
        }
    }
}

/// Format an absolute byte count as a human-readable string.
// TODO: use humansize lazy dumdum
#[allow(dead_code)]
fn fmt_bytes(b: i64) -> String {
    let abs = b.unsigned_abs();
    if abs < 1024 {
        format!("{abs}B")
    } else if abs < 1024 * 1024 {
        format!("{:.1}KiB", abs as f64 / 1024.0)
    } else {
        format!("{:.2}MiB", abs as f64 / (1024.0 * 1024.0))
    }
}

/// Format a signed byte delta with an explicit +/- prefix.
#[allow(dead_code)]
fn fmt_bytes_signed(b: i64) -> String {
    let sign = if b >= 0 { "+" } else { "-" };
    let abs = b.unsigned_abs();
    if abs < 1024 {
        format!("{sign}{abs}B")
    } else if abs < 1024 * 1024 {
        format!("{sign}{:.1}KiB", abs as f64 / 1024.0)
    } else {
        format!("{sign}{:.2}MiB", abs as f64 / (1024.0 * 1024.0))
    }
}
