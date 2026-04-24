//! Scope-based measurement probe: a named, single-writer
//! histogram plus a record buffer, populated via `start` /
//! `end` (or `end_batch`) rather than `record(ticks)`.
//!
//! `start(site_id)` reads the hardware tick counter and returns
//! an opaque [`TProbeRecId`] carrying `(site_id, start_tsc)`;
//! `end(id)` reads the tick counter again and appends a complete
//! `(site_id, start_tsc, end_tsc, batch=1)` record to the probe's
//! internal buffer. [`TProbe::end_batch`] is the same but records
//! a caller-supplied batch size `n`, which [`TProbe::report`]
//! uses to divide the scope's tick delta back down to a per-event
//! cost before histogram ingestion. No delta math, histogram
//! ingestion, or tick→ns conversion happens on the hot path.
//!
//! This primitive preserves record-order information across
//! interleaved scopes and sites (non-stack nesting is supported
//! by construction) and gives future evolution space for per-site
//! grouping, bounded buffers, background drain threads, and
//! long-term trace retention.
//!
//! Divergences from the `iiac-perf` original that this file was
//! vendored from:
//! - [`Record`] gained a `batch: u64` field.
//! - [`TProbe::end_batch`] added — scope covers `n` events,
//!   histogram stores per-event cost.
//! - [`TProbe::clear`] added — resets records and histogram,
//!   used at the warmup→measurement boundary.
//! - [`TProbe::report`] takes an optional [`Overhead`]. The
//!   histogram stores **raw** per-event values; the overhead
//!   correction is passed through to `band_table::render`
//!   so it can display an `adj mean` column alongside the
//!   raw columns. See `notes/overhead-model.md` for the
//!   subtraction policy.

use hdrhistogram::Histogram;

use crate::band_table;
use crate::overhead::Overhead;
use crate::ticks;

/// Opaque handle returned by [`TProbe::start`], consumed by
/// [`TProbe::end`] or [`TProbe::end_batch`]. Carries the
/// caller-supplied `site_id` and the start-time tick reading;
/// no probe-internal allocation happens at `start` time.
///
/// `#[must_use]` — dropping the id without passing it to `end*`
/// leaks the scope (no record is appended).
#[must_use]
#[derive(Clone, Copy, Debug)]
pub struct TProbeRecId {
    site_id: u64,
    start_tsc: u64,
}

/// A complete scope record: `(site_id, start_tsc, end_tsc, batch)`.
/// Appended at `end*` time; the record buffer only ever holds
/// complete records. Drained into the histogram at
/// [`TProbe::report`] time, dividing `end_tsc − start_tsc` by
/// `batch` so histogram values are always per-event costs.
#[derive(Clone, Copy, Debug)]
struct Record {
    #[allow(dead_code)] // read once per-site grouping lands.
    site_id: u64,
    start_tsc: u64,
    end_tsc: u64,
    batch: u64,
}

/// A named, single-writer histogram of hardware tick-counter
/// deltas plus a scope-record buffer. Not `Sync`; cross-thread
/// *sharing* is out of scope. `Send` so probes can be moved
/// between threads (e.g. returned via a `JoinHandle<TProbe>`
/// on shutdown).
pub struct TProbe {
    name: String,
    hist: Histogram<u64>,
    records: Vec<Record>,
}

impl TProbe {
    /// Create an empty probe. Histogram upper bound is 1e12
    /// ticks (~250 s at 4 GHz, ~100 s at 10 GHz), 3 significant
    /// figures.
    ///
    /// Exits the process (code 1) if the hardware tick counter
    /// isn't usable — see [`crate::ticks::require_ok`].
    pub fn new(name: &str) -> Self {
        ticks::require_ok();
        let _ = ticks::ticks_per_ns();
        Self {
            name: name.to_string(),
            hist: Histogram::<u64>::new_with_bounds(1, 1_000_000_000_000, 3).unwrap(), // OK: bounds are constant and valid (1 ≤ low < high, sigfigs in [0,5])
            records: Vec::new(),
        }
    }

    /// Begin a scope. Reads the hardware tick counter and
    /// returns an opaque [`TProbeRecId`] carrying `(site_id,
    /// start_tsc)`. The id must eventually be passed to
    /// [`TProbe::end`] or [`TProbe::end_batch`]; a dropped id
    /// leaves no record.
    #[inline]
    pub fn start(&mut self, site_id: u64) -> TProbeRecId {
        TProbeRecId {
            site_id,
            start_tsc: ticks::read_ticks(),
        }
    }

    /// End a single-event scope (batch = 1). Reads the hardware
    /// tick counter and appends a complete record. Delta and
    /// histogram ingestion are deferred to [`TProbe::report`].
    #[inline]
    pub fn end(&mut self, tpri: TProbeRecId) {
        let end_tsc = ticks::read_ticks();
        self.records.push(Record {
            site_id: tpri.site_id,
            start_tsc: tpri.start_tsc,
            end_tsc,
            batch: 1,
        });
    }

    /// End a batched scope covering `n` events. The histogram
    /// stores `(end − start) / n` at [`Self::report`] time so
    /// per-event cost is what gets rendered, regardless of batch
    /// size. Larger batches amortize probe apparatus overhead
    /// and push stored tick values into a range where the
    /// histogram's 0.1 %-relative buckets actually resolve small
    /// variations.
    #[inline]
    pub fn end_batch(&mut self, tpri: TProbeRecId, n: u64) {
        let end_tsc = ticks::read_ticks();
        self.records.push(Record {
            site_id: tpri.site_id,
            start_tsc: tpri.start_tsc,
            end_tsc,
            batch: n,
        });
    }

    /// Discard all pending records and zero the histogram. Used
    /// at the warmup → measurement boundary so only steady-state
    /// data reaches the report.
    pub fn clear(&mut self) {
        self.records.clear();
        self.hist.reset();
    }

    /// Render a band-table report for this probe. `as_ticks`
    /// controls the display unit: `false` converts stored tick
    /// deltas to nanoseconds (default for the CLI); `true` shows
    /// raw ticks (`-t`/`--ticks`). `decimals` controls the
    /// fractional precision of numeric cells — `None` uses a
    /// mode-aware default (`0` for ticks, `1` for ns); `Some(n)`
    /// overrides.
    ///
    /// Drains any pending `start` / `end*` records into the
    /// histogram as **raw** per-event values:
    /// `per_event_raw = (end_tsc − start_tsc) / batch`, clamped
    /// to `1` since the histogram lower bound is 1. If
    /// `overhead` is `Some`, the per-event correction is
    /// averaged across drained records and passed to
    /// `band_table::render`, which displays it in an `adj mean`
    /// column alongside the raw columns. See
    /// `notes/overhead-model.md`.
    pub fn report(&mut self, as_ticks: bool, overhead: Option<&Overhead>, decimals: Option<usize>) {
        let mut total_correction_ticks: u128 = 0;
        let mut drained_count: u64 = 0;
        for r in self.records.drain(..) {
            let delta = r.end_tsc.saturating_sub(r.start_tsc);
            let batch = r.batch.max(1);
            let per_event_raw = (delta + batch / 2) / batch;
            self.hist.record(per_event_raw.max(1)).unwrap(); // OK: histogram bounds are [1, 1e12]; per_event_raw.max(1) ≥ 1, and per-event tick deltas stay well under 1e12
            if let Some(ovh) = overhead {
                total_correction_ticks += ovh.per_event_ticks(batch) as u128;
                drained_count += 1;
            }
        }
        let correction = if drained_count > 0 {
            Some((total_correction_ticks / drained_count as u128) as u64)
        } else {
            None
        };
        band_table::render(
            "tprobe", &self.name, &self.hist, as_ticks, correction, decimals,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_end_appends_one_record() {
        let mut p = TProbe::new("t");
        let id = p.start(42);
        p.end(id);
        assert_eq!(p.records.len(), 1);
        let r = &p.records[0];
        assert_eq!(r.site_id, 42);
        assert_eq!(r.batch, 1);
        assert!(r.end_tsc >= r.start_tsc);
    }

    #[test]
    fn start_end_preserves_start_tsc() {
        let mut p = TProbe::new("t");
        let id = p.start(7);
        let saved_start = id.start_tsc;
        p.end(id);
        let r = &p.records[0];
        assert_eq!(r.site_id, 7);
        assert_eq!(r.start_tsc, saved_start);
    }

    #[test]
    fn start_end_interleaved_non_stack() {
        let mut p = TProbe::new("t");
        let a = p.start(1);
        let b = p.start(2);
        p.end(a);
        p.end(b);
        assert_eq!(p.records.len(), 2);
        assert_eq!(p.records[0].site_id, 1);
        assert_eq!(p.records[1].site_id, 2);
    }

    #[test]
    fn end_batch_stores_batch_size() {
        let mut p = TProbe::new("t");
        let id = p.start(3);
        p.end_batch(id, 100);
        assert_eq!(p.records.len(), 1);
        assert_eq!(p.records[0].site_id, 3);
        assert_eq!(p.records[0].batch, 100);
    }

    #[test]
    fn report_divides_batched_delta() {
        let mut p = TProbe::new("t");
        // Inject a synthetic record: 1000 tick delta, batch 10.
        p.records.push(Record {
            site_id: 0,
            start_tsc: 0,
            end_tsc: 1000,
            batch: 10,
        });
        p.report(true, None, None);
        assert_eq!(p.hist.len(), 1);
        let v = p.hist.iter_recorded().next().unwrap().value_iterated_to();
        // (1000 + 5) / 10 = 100.
        assert_eq!(v, 100);
    }

    #[test]
    fn report_stores_raw_even_with_overhead() {
        use std::time::Duration;
        let mut p = TProbe::new("t");
        p.records.push(Record {
            site_id: 0,
            start_tsc: 0,
            end_tsc: 1000,
            batch: 10,
        });
        let ovh = Overhead {
            framing_ticks: 100,
            loop_per_iter_ticks: 0.0,
            cal_raw_low_ticks: 0,
            cal_raw_high_ticks: 0,
            cal_duration: Duration::from_millis(0),
        };
        p.report(true, Some(&ovh), None);
        let v = p.hist.iter_recorded().next().unwrap().value_iterated_to();
        // per_event_raw = (1000 + 5) / 10 = 100. Histogram stores
        // raw regardless of overhead; correction is applied only
        // in the `adj mean` column of the band-table report.
        assert_eq!(v, 100);
    }

    #[test]
    fn report_drains_records_into_histogram() {
        let mut p = TProbe::new("t");
        let id1 = p.start(1);
        p.end(id1);
        let id2 = p.start(2);
        p.end(id2);
        assert_eq!(p.hist.len(), 0);
        assert_eq!(p.records.len(), 2);

        p.report(false, None, None);
        assert_eq!(p.records.len(), 0);
        assert_eq!(p.hist.len(), 2);

        // Idempotent: a second report drains nothing, hist unchanged.
        p.report(false, None, None);
        assert_eq!(p.hist.len(), 2);
    }

    #[test]
    fn clear_empties_records_and_hist() {
        let mut p = TProbe::new("t");
        let id = p.start(1);
        p.end(id);
        p.report(false, None, None); // moves record into hist
        assert_eq!(p.hist.len(), 1);
        p.clear();
        assert_eq!(p.records.len(), 0);
        assert_eq!(p.hist.len(), 0);
    }
}
