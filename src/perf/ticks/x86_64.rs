//! x86_64 impl of the tick-counter abstraction: `rdtsc` for
//! reads, CPUID-based invariant-TSC detection, and a 10 ms
//! spin-loop calibration for ticks-per-nanosecond.

use std::sync::OnceLock;
use std::time::Duration;

/// Read the raw time-stamp counter via the `rdtsc` instruction.
#[inline(always)]
pub fn read_ticks() -> u64 {
    // Safe on any x86_64 CPU: TSC has been present since the
    // original Pentium.
    unsafe { core::arch::x86_64::_rdtsc() }
}

static TICKS_PER_NS: OnceLock<f64> = OnceLock::new();

/// Cached ratio of TSC ticks per nanosecond (first call calibrates).
pub fn ticks_per_ns() -> f64 {
    *TICKS_PER_NS.get_or_init(calibrate)
}

/// Spin for ~10 ms while reading `minstant::Instant` elapsed ns
/// and raw `rdtsc` ticks at each end. `minstant` computes its
/// own nanos-per-cycle internally but doesn't expose it, so we
/// rederive the ratio from the two independent measurements.
fn calibrate() -> f64 {
    let start_instant = minstant::Instant::now();
    let start_tsc = read_ticks();
    let target = Duration::from_millis(10);
    loop {
        let elapsed = start_instant.elapsed();
        if elapsed >= target {
            let end_tsc = read_ticks();
            let dtk = end_tsc.wrapping_sub(start_tsc) as f64;
            let dns = elapsed.as_nanos() as f64;
            return dtk / dns;
        }
        core::hint::spin_loop();
    }
}

/// Bail out unless the CPU has invariant TSC and the kernel
/// selected TSC as its clocksource.
pub fn require_ok() {
    if !has_invariant_tsc() {
        eprintln!(
            "error: invariant TSC not supported by this CPU \
             (CPUID.80000007h:EDX[bit 8] = 0). actor-x1 probes \
             require a fixed-rate, non-stopping TSC; refusing to run."
        );
        std::process::exit(1);
    }
    if !minstant::is_tsc_available() {
        eprintln!(
            "error: TSC not selected as a kernel clocksource. \
             The CPU advertises invariant TSC, but the kernel has \
             rejected it — likely a sync or drift issue. actor-x1 \
             won't use a clock source the kernel considers unreliable; \
             refusing to run."
        );
        std::process::exit(1);
    }
}

/// `CPUID.80000007h:EDX[bit 8]` — invariant TSC. Set iff the
/// TSC runs at a constant rate regardless of P-state changes
/// and keeps ticking in deep C-states. Both Intel and AMD
/// expose the feature at this bit.
fn has_invariant_tsc() -> bool {
    use core::arch::x86_64::__cpuid;
    let max_ext = __cpuid(0x8000_0000).eax;
    if max_ext < 0x8000_0007 {
        return false;
    }
    let leaf = __cpuid(0x8000_0007);
    (leaf.edx >> 8) & 1 == 1
}
