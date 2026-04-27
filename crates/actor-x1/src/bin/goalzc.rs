//! Goalzc binary: two zerocopy actors, each on its own thread,
//! ping-ponging a pool-backed `PooledMsg` (handler view: `&[u8]`)
//! over `std::sync::mpsc` channels for a caller-supplied
//! duration in seconds, after an optional warmup window.
//! Smoke generator for the pool-backed payload path: prints
//! messages-handled / aggregate throughput plus the `pinning:`
//! line if `--pin` was set.
//!
//! No diagnostic instrumentation in this version. Probing
//! re-enters post-`0.6.0` via an actor-wrapper trait or
//! `TProbe::Counter` (out of scope here).
//!
//! Compared to goal2 (unit `Message`), goalzc exercises the
//! `Pool` + `PooledMsg` path: every dispatch costs one `pool.get`
//! plus one drop-back. `--size N` sweeps payload size; pool is
//! sized for ping-pong steady state (2 buffers in flight) with
//! headroom (4 buffers) so a wakeup race never starves.
//!
//! Usage: `goalzc --help`

use std::time::Duration;

use actor_x1::actor_manager::{ActorManager, ActorZC, ContextZC};
use actor_x1::pool::{BufRefStore, Pool};
use actor_x1::runtime_zc::RuntimeZC;
use clap::Parser;
use tprobe::{fmt_commas, pin};

/// Zerocopy ping-pong actor: on every inbound message, fabricate
/// a reply of the same length from the pool and forward it to
/// the configured peer. Drops the inbound `PooledMsg` after the
/// handler returns (runtime-owned), which returns its buffer to
/// the pool — so per dispatch we see exactly one pool `get` +
/// one pool `put`.
struct PingPongZC {
    peer_id: u32,
}

impl<S: BufRefStore> ActorZC<S> for PingPongZC {
    /// Reply with a same-sized buffer to `self.peer_id`. Never blocks.
    fn handle_message(&mut self, ctx: &mut dyn ContextZC<S>, msg: &[u8]) {
        let reply = ctx.get_msg(msg.len()).expect("get_msg should succeed"); // OK: pool sized for ping-pong steady state (2 in flight; 4 capacity)
        ctx.send(self.peer_id, reply);
    }
}

/// CLI for goalzc: two-thread two-actor zerocopy ping-pong
/// smoke generator. Reports aggregate throughput.
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

    /// Warmup duration in seconds before measurement. Runs
    /// the same dispatch loop; per-thread counts are zeroed
    /// at the warmup → measurement boundary via
    /// `Handle::reset_count` so reported throughput reflects
    /// only the measurement window.
    #[arg(short = 'w', long, default_value_t = 0.5, value_parser = parse_non_negative_secs)]
    warmup: f64,

    /// Message payload size in bytes. Pool is built with this
    /// `msg_size`, so `get_msg(size)` calls inside the handler
    /// always pass the bound check by construction. Must be
    /// `>= 1`.
    #[arg(short = 's', long, default_value_t = 64, value_parser = parse_positive_size)]
    size: u32,

    /// Pin each actor thread to a logical CPU. Accepts a
    /// comma-separated / range list; actor `i` pins to
    /// `pin[i % pin.len()]`. Examples: `--pin 0,1` pairs two
    /// actors to two CPUs; `--pin 0` oversubscribes both onto
    /// one core. Tightens stdev by eliminating OS thread
    /// migration noise.
    #[arg(short = 'p', long)]
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

/// `value_parser` helper: parse a positive (>=1) `u32`.
fn parse_positive_size(s: &str) -> Result<u32, String> {
    let v: u32 = s
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    if v == 0 {
        Err(format!("'{s}' must be >= 1"))
    } else {
        Ok(v)
    }
}

/// Parse CLI, run two zerocopy ping-pong actors on their own
/// threads via the lifecycle API, print aggregate throughput.
fn main() {
    let cli = Cli::parse();
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"),);

    // Pool sized for ping-pong steady state (2 in flight) with
    // headroom; `Pool::new(size, 4)` makes the `--size` runtime
    // bound auto-satisfied since `pool.msg_size() == size`.
    let pool: Pool = Pool::new(cli.size, 4);
    let mut rt = RuntimeZC::new(pool.clone());
    let mut mgr = ActorManager::new("goalzc.dispatch");
    let a_id = mgr.add(PingPongZC { peer_id: 1 });
    let b_id = mgr.add(PingPongZC { peer_id: 0 });
    assert_eq!((a_id, b_id), (0, 1));

    let initial = vec![(
        a_id,
        pool.get_msg(cli.size as usize).expect("seed get_msg"), // OK: fresh pool with capacity 4 satisfies a single get
    )];

    let pin_cores: Vec<usize> = match cli.pin.as_deref() {
        None => vec![],
        Some(spec) => match pin::parse_cores(spec) {
            Ok(cores) => cores,
            Err(e) => {
                eprintln!("error: --pin: {e}");
                std::process::exit(2);
            }
        },
    };

    let n_actors = 2;
    let handle = rt.startup(&mut mgr, initial, &pin_cores);
    handle.run(Duration::from_secs_f64(cli.warmup));
    handle.reset_count();
    handle.run(Duration::from_secs_f64(cli.duration_s));
    let total_count = handle.stop();

    let mmps = total_count as f64 / cli.duration_s / 1e6;
    println!(
        "goalzc: {} messages in {:.3}s ({mmps:.3} M msg/s, {n_actors} actors, size={size} B)",
        fmt_commas(total_count),
        cli.duration_s,
        size = cli.size,
    );
    if pin_cores.is_empty() {
        println!("  pinning: none (unpinned)");
    } else {
        let plan: Vec<String> = (0..n_actors)
            .map(|i| format!("actor{i}→core{}", pin_cores[i % pin_cores.len()]))
            .collect();
        println!("  pinning: {}", plan.join(", "));
    }
}
