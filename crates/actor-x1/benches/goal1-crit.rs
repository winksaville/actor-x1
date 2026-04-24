//! goal1-crit: criterion cross-validation benchmark for goal1.
//!
//! Drives the same single-thread two-actor ping-pong workload the
//! goal1 binary runs, but under criterion's statistical sampling
//! instead of tprobe's tick histogram. Invoked via
//! `cargo bench --bench goal1-crit` from `crates/actor-x1`.
//!
//! Each iteration is one call to
//! [`SingleThreadRuntime::dispatch_batch`] of size `inner`; the
//! benchmark sweeps `inner` ∈ {1, 100, 1000} to mirror goal1's
//! `--inner` knob. `Throughput::Elements(inner)` has criterion
//! report messages / second directly so the number can be
//! compared against goal1's `M msg/s` line and the per-message
//! time against goal1's `mean min-p99` / `adj mean min-p99` at
//! the same `inner`.

use std::time::Duration;

use actor_x1::runtime::SingleThreadRuntime;
use actor_x1::{Actor, Context, Message};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

/// Same actor goal1's binary uses: reply to every message.
struct PingPongActor {
    peer_id: u32,
}

impl Actor for PingPongActor {
    /// Forward a single `Message` to `self.peer_id`.
    fn handle_message(&mut self, ctx: &mut dyn Context, _msg: Message) {
        ctx.send(self.peer_id, Message);
    }
}

/// Build a freshly-seeded two-actor ping-pong runtime.
fn fresh_runtime() -> SingleThreadRuntime {
    let mut rt = SingleThreadRuntime::new("goal1-crit");
    rt.add_actor(Box::new(PingPongActor { peer_id: 1 }));
    rt.add_actor(Box::new(PingPongActor { peer_id: 0 }));
    rt.seed(0, Message);
    rt
}

/// Parameterize over `inner` to mirror goal1's `--inner` knob.
fn bench_goal1(c: &mut Criterion) {
    let mut group = c.benchmark_group("goal1-crit");
    // Longer sampling reduces run-to-run variation; total bench
    // time for the sweep stays under ~20 s because each batch
    // is cheap.
    group.measurement_time(Duration::from_secs(5));
    for &inner in &[1u64, 100, 1000] {
        group.throughput(Throughput::Elements(inner));
        group.bench_function(BenchmarkId::from_parameter(inner), |b| {
            let mut rt = fresh_runtime();
            b.iter(|| rt.dispatch_batch(inner));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_goal1);
criterion_main!(benches);
