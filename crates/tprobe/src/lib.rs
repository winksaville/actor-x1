//! `tprobe` — tick-counting performance probe for short code-path
//! measurement.
//!
//! Vendored from
//! [`iiac-perf`](https://github.com/winksaville/iiac-perf) — source
//! path at the time of vendoring: `../iiac-perf/src`. Graduated from
//! `actor-x1`'s in-tree `src/perf/` module at `actor-x1 0.2.0-2`.
//!
//! Files:
//! - [`tprobe2`] — scope-based probe (`start` / `end` → record buffer
//!   → histogram at `report` time).
//! - [`band_table`] — shared percentile-band renderer.
//! - [`ticks`] — hardware tick counter (x86_64 `rdtsc`).
//! - [`fmt`] — `fmt_commas` / `fmt_commas_f64` extracted from
//!   `iiac-perf/src/harness.rs` so we don't carry the whole 393-line
//!   benchmark harness for two helpers.
//! - [`overhead`] — two-point apparatus-overhead calibration for
//!   [`TProbe2`].
//! - [`pin`] — CPU-affinity helpers (thread pinning).
//!
//! Divergences from upstream at the time of graduation:
//! - Import paths reflect the standalone-crate layout (`crate::x`
//!   rather than `crate::perf::x`).
//! - `// OK: …` justifications on every `unwrap*` call per the
//!   host workspace's code conventions.
//! - `ticks.rs` diagnostic string mentions "tprobe" rather than
//!   "iiac-perf".

pub(crate) mod band_table;
pub(crate) mod fmt;
pub mod overhead;
pub mod pin;
pub mod ticks;
pub mod tprobe2;

pub use overhead::{Overhead, calibrate};
pub use tprobe2::{TProbe2, TProbe2RecId};
