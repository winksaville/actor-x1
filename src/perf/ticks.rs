//! Hardware tick counter abstraction: thin wrapper over the
//! target architecture's fixed-rate monotonic counter.
//!
//! Probes call three functions; the per-arch impl lives in a
//! child module gated by `#[cfg(target_arch = ...)]`:
//!
//! - [`read_ticks`] — current counter value.
//! - [`ticks_per_ns`] — calibrated conversion ratio.
//! - [`require_ok`] — exit the process if the counter isn't
//!   usable for probe measurements.
//!
//! Only `x86_64` is implemented today. AArch64 (`CNTVCT_EL0`)
//! and RISC-V (`time` CSR) both have architecturally invariant
//! counters by ISA spec, so their `require_ok` will be a no-op
//! once those impls land.

#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
use x86_64 as imp;

#[cfg(not(target_arch = "x86_64"))]
compile_error!(
    "actor-x1 probes currently only support target_arch = \"x86_64\". \
     Add a per-arch impl module (AArch64: CNTVCT_EL0; RISC-V: time CSR) \
     and wire it into src/perf/ticks.rs."
);

/// Read the current tick counter. Monotonic and fixed-rate.
#[inline(always)]
pub fn read_ticks() -> u64 {
    imp::read_ticks()
}

/// Calibrated conversion ratio: counter ticks per nanosecond.
/// Cached — the first call does the work.
pub fn ticks_per_ns() -> f64 {
    imp::ticks_per_ns()
}

/// Verify the tick counter is usable for probe measurements;
/// exit the process (code 1) with a diagnostic if not. The
/// checks performed depend on the target architecture.
pub fn require_ok() {
    imp::require_ok();
}
