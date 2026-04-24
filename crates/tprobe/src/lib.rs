//! `tprobe` — tick-counting performance probe for short code-path
//! measurement.
//!
//! See `notes/design.md` for architecture, data flow, and upstream
//! vendoring history. See `notes/overhead-model.md` for the formal
//! overhead model (framing, `loop_per_iter`, the unmeasurable
//! "everything else" category, and the current subtraction policy).

pub(crate) mod band_table;
pub mod fmt;
pub mod overhead;
pub mod pin;
pub mod ticks;
pub mod tprobe;

pub use fmt::{fmt_commas, fmt_commas_f64};
pub use overhead::{Overhead, calibrate};
pub use tprobe::{TProbe, TProbeRecId};
