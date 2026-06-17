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
//! Logs live heap bytes and per 5 sec delta. Try using this feature first to track when:
//! - delta is near zero, but RSS grows -> fragmentation in the allocator,
//!   thread stacks, GPU/mmap RSS, or time switch to `jemalloc-pprof` to debug heap issues.
//! - delta consistently positive we have a leak on the heap, try `dhat-heap` to
//!   find out where or turn plugins on/off to bisect where the issue is.
//!
//! ## `jemalloc-pprof`
//! Logs jemalloc's `allocated` vs `resident` bytes every 5 s.
//! - allocated is flat but resident grows = allocator internal fragmentation; jemalloc
//!   itself shoul improve this, not seen this YET but who knows what polars abusage might bring.
//! - allocated grows = heap growth like above and then its time to enable prof dumps.
//!
//! For heap dump files, run crap with:
//! ```text
//! _RJEM_MALLOC_CONF=prof:true,prof_prefix:heap cargo run --features jemalloc-pprof
//! ```
//! Then: `jeprof --pdf ./target/debug/mitchty heap.*.heap > out.pdf`

use bevy::prelude::*;

pub struct ProfilingPlugin;

impl Plugin for ProfilingPlugin {
    fn build(&self, app: &mut App) {
        // suppress unused warning in non-profiling builds, its annoying af
        let _ = app;

        #[cfg(feature = "stats-alloc")]
        app.add_systems(
            Update,
            log_alloc_stats.run_if(bevy::time::common_conditions::on_timer(
                std::time::Duration::from_secs(5),
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
                std::time::Duration::from_secs(5),
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
                "stats_alloc net={} live_allocs={} delta={}/5s{}",
                fmt_bytes(net),
                live_allocs,
                fmt_bytes_signed(delta),
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

    let (allocated, resident) = match (stats::allocated::mib(), stats::resident::mib()) {
        (Ok(a), Ok(r)) => match (a.read(), r.read()) {
            (Ok(av), Ok(rv)) => (av as i64, rv as i64),
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

    let fragmentation = resident - allocated;

    match *prev_allocated {
        Some(prev) => {
            let delta = allocated - prev;
            info!(
                "jemalloc allocated={} resident={} frag={} delta={}/5s{}",
                fmt_bytes(allocated),
                fmt_bytes(resident),
                fmt_bytes(fragmentation),
                fmt_bytes_signed(delta),
                if delta > 0 { " growing" } else { "" },
            );
        }
        None => {
            info!(
                "jemalloc allocated={} resident={} frag={} prime sample",
                fmt_bytes(allocated),
                fmt_bytes(resident),
                fmt_bytes(fragmentation),
            );
        }
    }

    *prev_allocated = Some(allocated);
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
