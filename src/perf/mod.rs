//! Performance measurement primitives, vendored from
//! [`iiac-perf`](https://github.com/winksaville/iiac-perf) —
//! source path at the time of vendoring: `../iiac-perf/src`.
//!
//! Files adopted:
//! - [`tprobe2`] — scope-based probe (`start` / `end` → record buffer
//!   → histogram at `report` time).
//! - [`band_table`] — shared percentile-band renderer.
//! - [`ticks`] — hardware tick counter (x86_64 `rdtsc`).
//! - [`fmt`] — `fmt_commas` / `fmt_commas_f64` extracted from
//!   `iiac-perf/src/harness.rs` so we don't vendor the whole 393-line
//!   benchmark harness for two helpers.
//!
//! The bot thinks these should be promoted to a shared crate once
//! Stage 2 work is underway; copying them in here for now keeps the
//! PoC self-contained and avoids a cross-repo dependency while the
//! API is still settling.
//!
//! Divergences from upstream while vendoring:
//! - Import paths rewritten for actor-x1 module layout.
//! - `// OK: …` justifications added to `unwrap*` calls per this
//!   crate's code conventions (see `CLAUDE.md`).
//! - One diagnostic string in `ticks.rs` mentions `actor-x1 probes`
//!   instead of `iiac-perf probes`.

pub(crate) mod band_table;
pub(crate) mod fmt;
pub mod overhead;
pub mod ticks;
pub mod tprobe2;

pub use overhead::{Overhead, calibrate};
pub use tprobe2::{TProbe2, TProbe2RecId};
