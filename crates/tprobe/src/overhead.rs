//! Apparatus-overhead calibration for [`TProbe2`]. Measures the
//! hardware floor cost of two back-to-back `rdtsc` reads — the
//! bias contaminating every probe scope's stored `end_tsc -
//! start_tsc` delta — via a two-point fit matching the approach
//! in `iiac-perf/src/overhead.rs`.
//!
//! ### Why two-point, not single-point
//!
//! `rdtsc` isn't a serializing instruction. Two back-to-back
//! reads with nothing between can execute out-of-order and
//! measure a delta of 0–1 ticks — far below the true framing
//! cost. Bracketing with `_mm_lfence` overcorrects (lfence drains
//! the pipeline, which real work doesn't fully do), so single-
//! point measurement is biased in both directions.
//!
//! Two-point fits a line through empty benches at two `inner`
//! sizes (`N_LOW`, `N_HIGH`):
//! - raw delta at inner=N = framing + N · loop_per_iter
//! - slope between (N_LOW, raw_low) and (N_HIGH, raw_high) gives
//!   loop_per_iter; intercept gives framing.
//!
//! The `black_box(1)` inside the inner loop keeps OoO from
//! collapsing iterations and gives `loop_per_iter` a consistent
//! value. Noise amplification on framing:
//! `d(framing)/d(raw_low) = N_HIGH / (N_HIGH − N_LOW)`. At
//! 100/10_000 that's ~1.01 — any slop on `raw_low` propagates
//! roughly 1:1 to framing.
//!
//! We fit using raw tick deltas (not per-event values) in `f64`
//! arithmetic so integer truncation can't hide small framing
//! components.
//!
//! ### What gets subtracted
//!
//! Only framing. The two-point fit also yields
//! `loop_per_iter_ticks` (cost of one black_box iteration), which
//! we keep on the [`Overhead`] struct for diagnostics but do not
//! subtract from probe samples — the caller's inner-loop cost
//! (queue pop, deadline check, etc.) is part of what the user is
//! actually measuring, and the `black_box(1)` loop_per_iter
//! doesn't model it anyway.
//!
//! [`TProbe2`]: crate::TProbe2
//! [`TProbe2::report`]: crate::TProbe2::report

use std::hint::black_box;
use std::time::{Duration, Instant};

use crate::ticks;

/// Calibration warmup iterations — lets CPU frequency boost ramp
/// before the first real sample.
pub const CAL_WARMUP: u64 = 10_000;

/// Samples per minimum-pass. The reported floor approaches the
/// hardware lower bound as samples grow; 1,000 is enough for
/// convergence and keeps total calibration wall-clock short.
pub const CAL_SAMPLES: u64 = 1_000;

/// Inner-loop count for the low-N calibration point.
pub const N_LOW: u64 = 100;

/// Inner-loop count for the high-N calibration point. Wide spread
/// keeps noise amplification on fitted framing small
/// (`N_HIGH / (N_HIGH − N_LOW) ≈ 1.01`).
pub const N_HIGH: u64 = 10_000;

/// Apparatus-overhead model fitted by [`calibrate`]. Callers pass
/// a reference to [`TProbe2::report`] (as `Option<&Overhead>`) to
/// get framing subtracted from each stored per-event value.
///
/// [`TProbe2::report`]: crate::TProbe2::report
#[derive(Debug, Clone, Copy)]
pub struct Overhead {
    /// Fitted per-scope framing cost — the bias on every stored
    /// `end_tsc − start_tsc` delta regardless of scope batch size,
    /// in raw ticks. This is what gets subtracted via
    /// [`Self::per_event_ticks`].
    pub framing_ticks: u64,
    /// Fitted per-iteration cost of the `black_box(1)` empty
    /// bench loop body, in ticks. Kept for diagnostic visibility
    /// only — not used in the subtraction path (the real dispatch
    /// loop's per-iter cost is what we want to measure, not subtract).
    pub loop_per_iter_ticks: f64,
    /// Raw tick delta at `inner = N_LOW` (for `-v` style logging).
    pub cal_raw_low_ticks: u64,
    /// Raw tick delta at `inner = N_HIGH` (for `-v` style logging).
    pub cal_raw_high_ticks: u64,
    /// Wall-clock duration of the full calibration run.
    pub cal_duration: Duration,
}

impl Overhead {
    /// Per-event framing correction at a given `batch`, in ticks:
    /// the framing is paid once per scope and amortized across
    /// `batch` events. Integer division truncates toward zero, so
    /// at `batch > framing_ticks` the correction rounds to zero.
    pub fn per_event_ticks(&self, batch: u64) -> u64 {
        self.framing_ticks / batch.max(1)
    }
}

/// Measure the minimum raw tick delta over `samples` iterations
/// of an empty bench at inner size `inner`. Each iteration reads
/// `rdtsc`, runs `inner × black_box(1)` as filler (keeps OoO from
/// collapsing the scope to zero), reads `rdtsc` again.
#[inline(never)]
fn measure_empty_raw(samples: u64, inner: u64) -> u64 {
    let mut min_ticks = u64::MAX;
    for _ in 0..samples {
        let start = ticks::read_ticks();
        for _ in 0..inner {
            black_box(1u64);
        }
        let end = ticks::read_ticks();
        let delta = end.saturating_sub(start);
        if delta < min_ticks {
            min_ticks = delta;
        }
    }
    min_ticks
}

/// Two-point calibration. Runs [`CAL_WARMUP`] warmup iterations,
/// then two minimum-passes at `inner = N_LOW` and `inner = N_HIGH`,
/// and fits
/// `raw_delta = framing + inner · loop_per_iter`. Blocks for
/// ~10 ms on a modern x86.
pub fn calibrate() -> Overhead {
    let cal_start = Instant::now();

    for _ in 0..CAL_WARMUP {
        let a = ticks::read_ticks();
        black_box(a);
    }

    let raw_low = measure_empty_raw(CAL_SAMPLES, N_LOW);
    let raw_high = measure_empty_raw(CAL_SAMPLES, N_HIGH);

    let loop_per_iter_ticks = if raw_high > raw_low {
        (raw_high - raw_low) as f64 / (N_HIGH - N_LOW) as f64
    } else {
        0.0
    };
    let framing_f = (raw_low as f64 - N_LOW as f64 * loop_per_iter_ticks).max(0.0);
    let framing_ticks = framing_f.round() as u64;

    Overhead {
        framing_ticks,
        loop_per_iter_ticks,
        cal_raw_low_ticks: raw_low,
        cal_raw_high_ticks: raw_high,
        cal_duration: cal_start.elapsed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_event_ticks_amortizes_framing() {
        let ovh = Overhead {
            framing_ticks: 20,
            loop_per_iter_ticks: 0.0,
            cal_raw_low_ticks: 0,
            cal_raw_high_ticks: 0,
            cal_duration: Duration::from_millis(1),
        };
        assert_eq!(ovh.per_event_ticks(1), 20);
        assert_eq!(ovh.per_event_ticks(10), 2);
        assert_eq!(ovh.per_event_ticks(100), 0); // truncates
        assert_eq!(ovh.per_event_ticks(0), 20); // batch=0 treated as 1
    }

    #[test]
    fn calibrate_returns_positive_ticks_quickly() {
        let ovh = calibrate();
        // Modern x86 rdtsc framing is ~15–50 ticks; allow slack.
        assert!(ovh.framing_ticks > 0);
        assert!(ovh.framing_ticks < 10_000);
        // loop_per_iter should be small and non-negative.
        assert!(ovh.loop_per_iter_ticks >= 0.0);
        assert!(ovh.loop_per_iter_ticks < 100.0);
        // Calibration should complete well under a second.
        assert!(ovh.cal_duration < Duration::from_secs(1));
    }
}
