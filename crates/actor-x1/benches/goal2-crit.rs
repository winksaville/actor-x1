//! goal2-crit: criterion cross-validation benchmark for goal2.
//!
//! Drives the same two-thread two-actor mpsc ping-pong workload
//! the goal2 binary runs, via [`MultiThreadRuntime`]. Invoked as
//! `cargo bench --bench goal2-crit` from `crates/actor-x1`.
//!
//! Shape note. `goal2`'s runtime is wall-clock driven — actor
//! threads stay alive and ping-pong freely for a caller-supplied
//! duration; there is no "do N messages" primitive. To fit
//! criterion's iteration-count model, each `iter_custom` call
//! runs a short fixed measurement window, counts the messages
//! handled, then scales the reported duration to
//! `measurement * iters / total_count` so criterion's per-iter
//! estimate reflects per-message time. Each iteration is "one
//! `handle_message` call at either actor". `Throughput::Elements(1)`
//! makes criterion's throughput summary read as msgs / s.
//!
//! Compare the reported per-message time and throughput to
//! goal2's `mean min-p99` / `adj mean min-p99` and `M msg/s` at
//! the same pinning configuration (default: unpinned).

use std::time::Duration;

use actor_x1::runtime::MultiThreadRuntime;
use actor_x1::{Actor, Context, Message};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

/// Same actor goal2's binary uses: reply to every message. Must
/// be `Send` to cross thread boundaries.
struct PingPongActor {
    peer_id: u32,
}

impl Actor for PingPongActor {
    /// Forward a single `Message` to `self.peer_id`.
    fn handle_message(&mut self, ctx: &mut dyn Context, _msg: Message) {
        ctx.send(self.peer_id, Message);
    }
}

/// Per `iter_custom` call: spin up a fresh runtime, run a short
/// steady-state window, scale to iters. See module docs for why.
fn bench_goal2(c: &mut Criterion) {
    let mut group = c.benchmark_group("goal2-crit");
    group.throughput(Throughput::Elements(1));
    // Keep total bench time bounded. Per sample: 50 ms warmup +
    // 100 ms measurement + thread spawn/join (~a few ms) ≈ 160 ms.
    // 50 samples * 160 ms ≈ 8 s of the sampling phase, plus
    // criterion's own internal warmup (a handful of calls).
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(50);
    group.bench_function("pingpong", |b| {
        b.iter_custom(|iters| {
            let mut rt = MultiThreadRuntime::new("goal2-crit");
            rt.add_actor(|| PingPongActor { peer_id: 1 });
            rt.add_actor(|| PingPongActor { peer_id: 0 });
            rt.seed(0, Message);

            let warmup = Duration::from_millis(50);
            let measurement = Duration::from_millis(100);
            let results = rt.run(warmup, measurement, &[]);

            let total_count: u64 = results.iter().map(|(c, _)| *c).sum();
            // Scale: `measurement` of wall time produced
            // `total_count` messages. Criterion wants total time
            // for `iters` iterations, so project linearly.
            let per_msg_ns = measurement.as_nanos() as u64 / total_count.max(1);
            Duration::from_nanos(per_msg_ns.saturating_mul(iters))
        });
    });
    group.finish();
}

criterion_group!(benches, bench_goal2);
criterion_main!(benches);
