//! goalzc-crit: criterion smoke benchmark for the lifecycle
//! API on the pool-backed payload path.
//!
//! Drives the same two-thread two-actor zerocopy ping-pong
//! workload `goalzc` runs, via the lifecycle API
//! ([`RuntimeZC::startup`] → [`Handle::run`] → [`Handle::stop`]).
//! Invoked as `cargo bench --bench goalzc-crit` from
//! `crates/actor-x1`.
//!
//! Probe-free by construction (the runtime no longer ships a
//! probed path; that re-enters post-`0.6.0` via an actor-
//! wrapper trait or `TProbe::Counter`).
//!
//! Shape note. `Handle::run` is wall-clock driven — actor
//! threads ping-pong freely for a caller-supplied run window;
//! there is no "do N messages" primitive. To fit criterion's
//! iteration-count model, each `iter_custom` call runs a
//! short warmup window, calls `Handle::reset_count` to zero
//! each thread's per-thread count, runs a fixed measurement
//! window, reads the count via `Handle::stop`, then scales
//! the reported duration to
//! `measurement * iters / measurement_count` so criterion's
//! per-iter estimate reflects per-message time during the
//! measurement window only. Each iteration is "one
//! `handle_message` call at either actor".
//! `Throughput::Elements(1)` makes criterion's throughput
//! summary read as msgs / s.
//!
//! Compare the reported per-message time / throughput against
//! `goalzc`'s `M msg/s` line at the same `--size` and pinning
//! (default: unpinned, `--size 64`).

use std::time::Duration;

use actor_x1::actor_manager::{ActorManager, ActorZC, ContextZC};
use actor_x1::pool::{BufRefStore, Pool};
use actor_x1::runtime_zc::RuntimeZC;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Bench-local zerocopy ping-pong actor: forwards a same-sized
/// reply on each message. Mirrors `goalzc`'s `PingPongZC`.
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

/// Per `iter_custom` call: spin up a fresh pool / runtime /
/// manager, warm up briefly, reset counts, measure a fixed
/// run window via the lifecycle API, scale to iters.
fn bench_goalzc(c: &mut Criterion) {
    const SIZE: u32 = 64;
    let mut group = c.benchmark_group("goalzc-crit");
    group.throughput(Throughput::Elements(1));
    // Per sample: 50 ms warmup + 100 ms measurement + thread
    // spawn / join (~a few ms) ≈ 160 ms. 50 samples * 160 ms ≈
    // 8 s of the sampling phase, plus criterion's own internal
    // warmup (a handful of calls).
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(50);
    group.bench_function("pingpong", |b| {
        b.iter_custom(|iters| {
            let pool: Pool = Pool::new(SIZE, 4);
            let mut rt = RuntimeZC::new(pool.clone());
            let mut mgr = ActorManager::new("goalzc-crit");
            let a_id = mgr.add(PingPongZC { peer_id: 1 });
            let _b_id = mgr.add(PingPongZC { peer_id: 0 });
            let initial = vec![(
                a_id,
                pool.get_msg(SIZE as usize).expect("seed get_msg"), // OK: fresh pool with capacity 4 satisfies a single get
            )];

            let warmup = Duration::from_millis(50);
            let measurement = Duration::from_millis(100);
            let handle = rt.startup(&mut mgr, initial, &[]);
            handle.run(warmup);
            handle.reset_count();
            handle.run(measurement);
            let measurement_count = handle.stop();

            // Scale: `measurement` of wall time produced
            // `measurement_count` messages (post-reset). Criterion
            // wants total time for `iters` iterations, so project
            // linearly.
            let per_msg_ns = measurement.as_nanos() as u64 / measurement_count.max(1);
            Duration::from_nanos(per_msg_ns.saturating_mul(iters))
        });
    });
    group.finish();
}

criterion_group!(benches, bench_goalzc);
criterion_main!(benches);
