//! Goal1 binary: two actors on a single thread ping-pong an empty
//! [`Message`] for a caller-supplied duration in seconds (f64),
//! after an optional warmup phase. Prints messages-handled /
//! throughput and a `tprobe2` band-table report of per-dispatch
//! latency.
//!
//! Usage: `goal1 --help`

use std::time::Duration;

use actor_x1::runtime::SingleThreadRuntime;
use actor_x1::{Actor, Context, Message};
use clap::Parser;
use tprobe::{self as perf, pin, ticks};

/// Actor that, on every message received, sends exactly one message
/// back to its configured peer.
struct PingPongActor {
    peer_id: u32,
}

impl Actor for PingPongActor {
    /// Forward a single `Message` to `self.peer_id`. Never blocks.
    fn handle_message(&mut self, ctx: &mut dyn Context, _msg: Message) {
        ctx.send(self.peer_id, Message);
    }
}

/// CLI for goal1: single-thread two-actor ping-pong with warmup,
/// per-dispatch probe batching, and ns-or-ticks band-table report.
#[derive(Parser, Debug)]
#[command(
    version,
    about,
    long_about = None,
    max_term_width = 80,
    before_help = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")),
)]
struct Cli {
    /// Measurement window in seconds (runs after warmup).
    #[arg(value_parser = parse_non_negative_secs)]
    duration_s: f64,

    /// Warmup duration in seconds before measurement. Runs the
    /// same dispatch loop as the measurement phase (probe active)
    /// so cache/branch-predictor/frequency land in the same state
    /// by the time the probe is cleared and measurement begins.
    #[arg(long, default_value_t = 10.0, value_parser = parse_non_negative_secs)]
    warmup: f64,

    /// Number of dispatches per probe scope. Larger values
    /// amortize probe apparatus overhead and push stored tick
    /// values into a range where the histogram's 0.1 %-relative
    /// buckets resolve small variations.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    inner: u64,

    /// Display probe values as raw ticks instead of nanoseconds.
    #[arg(short = 't', long)]
    ticks: bool,

    /// Skip apparatus-overhead calibration and report uncorrected
    /// per-event values. Use to see the raw cost the probe sees,
    /// including its own two-rdtsc framing (~4 ns on modern x86).
    #[arg(long)]
    raw: bool,

    /// Pin the workload thread to a logical CPU. Accepts a single
    /// id or a comma-separated / range list; only the first core
    /// is used (goal1 is single-threaded). Tightens stdev by
    /// eliminating OS thread migration noise.
    #[arg(long)]
    pin: Option<String>,
}

/// `value_parser` helper: reject negative, NaN, and infinite values.
fn parse_non_negative_secs(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .parse()
        .map_err(|e: std::num::ParseFloatError| e.to_string())?;
    if !v.is_finite() || v < 0.0 {
        Err(format!("'{s}' is not a non-negative finite number"))
    } else {
        Ok(v)
    }
}

/// Parse CLI, run warmup + measurement on two single-thread
/// ping-pong actors, print message count + throughput, then
/// render the per-dispatch band-table report.
fn main() {
    let cli = Cli::parse();

    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"),);

    // Pin the main thread before any work — calibration and the
    // dispatch loop both run here, so they share the pinned core.
    let pin_core: Option<usize> = match cli.pin.as_deref() {
        None => None,
        Some(spec) => match pin::parse_cores(spec) {
            Ok(cores) => cores.first().copied(),
            Err(e) => {
                eprintln!("error: --pin: {e}");
                std::process::exit(2);
            }
        },
    };
    pin::pin_current(pin_core);

    let mut rt = SingleThreadRuntime::new("goal1.dispatch");
    let a_id = rt.add_actor(Box::new(PingPongActor { peer_id: 1 }));
    let b_id = rt.add_actor(Box::new(PingPongActor { peer_id: 0 }));
    assert_eq!((a_id, b_id), (0, 1));
    rt.seed(a_id, Message);

    // Warmup: same loop, probe active, records discarded at the boundary.
    rt.run_for(Duration::from_secs_f64(cli.warmup), cli.inner);
    rt.probe_mut().clear();

    // Calibration (skipped with --raw). Runs on the warmed system,
    // before measurement begins, so freq/cache state matches what
    // the probe will see during measurement.
    let overhead = if cli.raw {
        None
    } else {
        Some(perf::calibrate())
    };

    // Measurement.
    let count = rt.run_for(Duration::from_secs_f64(cli.duration_s), cli.inner);
    let mmps = count as f64 / cli.duration_s / 1e6;
    println!(
        "goal1: {count} messages in {:.3}s ({mmps:.3} M msg/s, inner={})",
        cli.duration_s, cli.inner,
    );
    match pin_core {
        Some(c) => println!("  pinning: main → core {c}"),
        None => println!("  pinning: none (unpinned)"),
    }
    if let Some(ovh) = &overhead {
        let framing_ns = ovh.framing_ticks as f64 / ticks::ticks_per_ns();
        let per_event_tk = ovh.per_event_ticks(cli.inner);
        let per_event_ns = per_event_tk as f64 / ticks::ticks_per_ns();
        println!(
            "  apparatus: framing={} tk ({:.2} ns); per-event at inner={} = {} tk ({:.2} ns)",
            ovh.framing_ticks, framing_ns, cli.inner, per_event_tk, per_event_ns,
        );
    } else {
        println!("  apparatus: raw (no overhead subtraction)");
    }
    println!();
    rt.probe_mut().report(cli.ticks, overhead.as_ref());
}
