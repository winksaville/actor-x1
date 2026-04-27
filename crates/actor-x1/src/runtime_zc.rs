//! Multi-threaded zerocopy runtime — message-moving
//! infrastructure.
//!
//! Pairs with [`crate::actor_manager`], which owns the actor
//! model surface ([`ActorZC`], [`ContextZC`], [`ActorManager`]).
//! This module is one consumer of that surface; future
//! transports (M:N scheduler, alternative channel types, …)
//! would be peers.
//!
//! Responsibilities held here:
//!
//! - Owns the [`Pool<S>`]; spawns actor threads; wires
//!   per-actor `mpsc` channels; runs the warmup / measure /
//!   shutdown lifecycle.
//! - Knows nothing about "seeds", named lookups, or what a
//!   benchmark is. Initial messages are passed to
//!   [`RuntimeZC::run`] directly by the app; the runtime
//!   delivers them after spawn / before warmup, then forgets.
//! - Drains the manager via [`ActorManager::take_actors`] +
//!   [`ActorManager::probe_name_prefix`] when `run` is called.
//!
//! Two run entry points on [`RuntimeZC`]:
//!
//! - [`RuntimeZC::run`] — probe-free; returns `count` per
//!   actor. The default measurement primitive (used by the
//!   `goalzc-crit` criterion bench so measurement is not
//!   contaminated by probe overhead).
//! - [`RuntimeZC::run_probed`] — `tprobe`-instrumented;
//!   returns `(count, TProbe)` per actor. Opt-in for
//!   diagnostic instrumentation (used by the `goalzc` binary).
//!
//! Lifecycle inside `run` / `run_probed`:
//!
//! - Drain actors from the Manager.
//! - Build per-actor channels.
//! - Spawn one thread per actor, optionally pinned.
//! - Deliver `initial_messages` over the senders.
//! - Sleep `warmup`.
//! - Send `ClearProbe` to every actor.
//! - Sleep `measurement`.
//! - Send `Shutdown` to every actor.
//! - Drop main-thread senders so the channels close.
//! - Join, return per-actor results.
//!
//! Drain invariant: by the time the join completes, every
//! [`PooledMsg`] that was ever in flight has been dropped, so
//! `pool.free_len() == pool.size()`.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::actor_manager::{ActorManager, ActorZC, ContextZC};
use crate::pool::{BufRefStore, MutexLifo, Pool, PoolError, PooledMsg};
use tprobe::TProbe;

/// Internal channel payload.
///
/// - `User(msg)` — caller-visible traffic.
/// - `ClearProbe` — runtime injects at warmup → measurement
///   boundary so the report only sees steady-state data.
/// - `Shutdown` — runtime injects after the measurement
///   window; actor exits its `recv` loop on receipt.
enum SignalZC<S: BufRefStore> {
    User(PooledMsg<S>),
    ClearProbe,
    Shutdown,
}

/// Multi-threaded zerocopy runtime — message-moving
/// infrastructure.
///
/// - Owns the [`Pool<S>`] (cloned cheaply to each actor
///   thread).
/// - Spawns threads, wires channels, runs the warmup /
///   measure / shutdown lifecycle.
/// - Knows nothing about "seeds", named lookups, or what's
///   being measured. Initial messages come in as a parameter
///   to `run` and are delivered after spawn / before warmup.
/// - Generic over the store backend `S` so concurrency
///   strategies can be swapped and benchmarked. Default
///   [`MutexLifo`].
pub struct RuntimeZC<S: BufRefStore = MutexLifo> {
    pool: Pool<S>,
}

impl<S: BufRefStore + 'static> RuntimeZC<S> {
    /// Build a runtime that draws buffers from `pool`.
    ///
    /// - The runtime stores a cheap `Arc` clone; the caller
    ///   keeps the original handle so it can build initial
    ///   messages or inspect (`free_len`, `size`).
    pub fn new(pool: Pool<S>) -> Self {
        Self { pool }
    }

    /// Borrow the runtime's pool handle.
    pub fn pool(&self) -> &Pool<S> {
        &self.pool
    }

    /// Drain the manager's actors and run with no probe
    /// instrumentation. The default measurement primitive.
    ///
    /// - `initial_messages` are delivered after thread spawn,
    ///   before warmup.
    /// - Returns per-actor `count` in id order.
    /// - Used by the `goalzc-crit` criterion bench so the
    ///   measurement is probe-clean. The lesson learned from
    ///   `goal2-crit`'s probe contamination: keeping a probe-
    ///   free path on the runtime from the start avoids the
    ///   trap of a bench that secretly measures `work + tprobe`.
    pub fn run(
        &mut self,
        mgr: &mut ActorManager<S>,
        initial_messages: Vec<(u32, PooledMsg<S>)>,
        warmup: Duration,
        measurement: Duration,
        pin_cores: &[usize],
    ) -> Vec<u64> {
        self.run_inner(
            mgr,
            initial_messages,
            warmup,
            measurement,
            pin_cores,
            actor_loop::<S>,
        )
    }

    /// Same orchestration as [`RuntimeZC::run`], plus a
    /// per-message [`TProbe`] in the hot loop.
    ///
    /// - Each actor thread owns a `TProbe` recording per-
    ///   `handle_message` latency at `inner=1`.
    /// - Returns `(count, probe)` per actor in id order.
    /// - Used by the `goalzc` binary's diagnostic report.
    ///   Avoid in benches: probe overhead contaminates the
    ///   measurement.
    pub fn run_probed(
        &mut self,
        mgr: &mut ActorManager<S>,
        initial_messages: Vec<(u32, PooledMsg<S>)>,
        warmup: Duration,
        measurement: Duration,
        pin_cores: &[usize],
    ) -> Vec<(u64, TProbe)> {
        self.run_inner(
            mgr,
            initial_messages,
            warmup,
            measurement,
            pin_cores,
            actor_loop_probed::<S>,
        )
    }

    /// Shared orchestration body.
    ///
    /// - Drains the manager, builds channels, spawns one
    ///   thread per actor, delivers initial messages, runs
    ///   the warmup / measure / shutdown lifecycle, joins,
    ///   and returns per-actor results.
    /// - Per-thread work is delegated to `actor_loop`, a
    ///   function pointer (`ActorLoopFn<S, R>`) so this method
    ///   monomorphizes per `R` without the closure / `Fn`-bound
    ///   overhead.
    fn run_inner<R: Send + 'static>(
        &mut self,
        mgr: &mut ActorManager<S>,
        initial_messages: Vec<(u32, PooledMsg<S>)>,
        warmup: Duration,
        measurement: Duration,
        pin_cores: &[usize],
        actor_loop: ActorLoopFn<S, R>,
    ) -> Vec<R> {
        let actors = mgr.take_actors();
        let n = actors.len();
        let pool = self.pool.clone();
        let probe_name_prefix = mgr.probe_name_prefix().to_string();

        let mut senders: Vec<Sender<SignalZC<S>>> = Vec::with_capacity(n);
        let mut receivers: Vec<Option<Receiver<SignalZC<S>>>> = Vec::with_capacity(n);
        for _ in 0..n {
            let (tx, rx) = mpsc::channel();
            senders.push(tx);
            receivers.push(Some(rx));
        }

        let mut handles: Vec<JoinHandle<R>> = Vec::with_capacity(n);
        for (id, actor) in actors.into_iter().enumerate() {
            #[allow(clippy::unwrap_used)]
            // OK: receivers[id] was set to `Some(...)` in the loop above.
            let rx = receivers[id].take().unwrap();
            let peers: Vec<Sender<SignalZC<S>>> = senders.to_vec();
            let probe_name = format!("{}.actor{}", probe_name_prefix, id);
            let pin_target = if pin_cores.is_empty() {
                None
            } else {
                Some(pin_cores[id % pin_cores.len()])
            };
            let actor_pool = pool.clone();
            let h = thread::Builder::new()
                .name(format!("actor-zc-{id}"))
                .spawn(move || {
                    tprobe::pin::pin_current(pin_target);
                    actor_loop(actor, rx, peers, actor_pool, probe_name)
                })
                .expect("failed to spawn actor thread");
            handles.push(h);
        }

        for (dst, msg) in initial_messages {
            #[allow(clippy::unwrap_used)]
            // OK: receivers are alive (threads just spawned); a send
            //   failure indicates an immediate thread panic at startup
            //   which the join below surfaces with a clearer message.
            senders[dst as usize].send(SignalZC::User(msg)).unwrap();
        }

        thread::sleep(warmup);
        for tx in &senders {
            let _ = tx.send(SignalZC::ClearProbe);
        }

        thread::sleep(measurement);
        for tx in &senders {
            let _ = tx.send(SignalZC::Shutdown);
        }
        drop(senders);

        let mut results = Vec::with_capacity(n);
        for h in handles {
            results.push(h.join().expect("actor thread panicked"));
        }
        results
    }
}

/// Function-pointer type for the per-thread actor loop, used
/// by [`RuntimeZC::run_inner`]. `fn(...) -> R` is `Copy +
/// Send + Sync`, which keeps the orchestration scaffold
/// generic without closure / `Fn`-bound complications.
type ActorLoopFn<S, R> = fn(
    Box<dyn ActorZC<S> + Send>,
    Receiver<SignalZC<S>>,
    Vec<Sender<SignalZC<S>>>,
    Pool<S>,
    String,
) -> R;

/// `ContextZC` impl used inside each actor thread.
///
/// - Holds borrowed references to the actor's peer senders
///   and the shared pool.
/// - `get_msg` delegates to [`Pool::get_msg`].
/// - `send` ships a `PooledMsg` over the appropriate channel;
///   on closed channel the message drops back to the pool.
struct MultiCtxZC<'a, S: BufRefStore> {
    senders: &'a [Sender<SignalZC<S>>],
    pool: &'a Pool<S>,
}

impl<S: BufRefStore> ContextZC<S> for MultiCtxZC<'_, S> {
    fn get_msg(&mut self, size: usize) -> Result<PooledMsg<S>, PoolError> {
        self.pool.get_msg(size)
    }

    fn send(&mut self, dst_id: u32, msg: PooledMsg<S>) {
        // Ignored Err contains the un-sent SignalZC::User(msg);
        // dropping it returns the buffer to the pool.
        let _ = self.senders[dst_id as usize].send(SignalZC::User(msg));
    }
}

/// Per-thread loop with `tprobe` instrumentation.
///
/// - One scope per `handle_message` (`inner=1`).
/// - `Signal::User` records.
/// - `Signal::ClearProbe` resets probe + count.
/// - `Signal::Shutdown` (or closed channel) exits.
fn actor_loop_probed<S: BufRefStore + 'static>(
    mut actor: Box<dyn ActorZC<S> + Send>,
    rx: Receiver<SignalZC<S>>,
    peers: Vec<Sender<SignalZC<S>>>,
    pool: Pool<S>,
    probe_name: String,
) -> (u64, TProbe) {
    let mut probe = TProbe::new(&probe_name);
    let mut count: u64 = 0;
    loop {
        match rx.recv() {
            Ok(SignalZC::User(msg)) => {
                let id = probe.start(0);
                let mut ctx = MultiCtxZC {
                    senders: &peers,
                    pool: &pool,
                };
                actor.handle_message(&mut ctx, &msg);
                probe.end(id);
                drop(msg);
                count += 1;
            }
            Ok(SignalZC::ClearProbe) => {
                probe.clear();
                count = 0;
            }
            Ok(SignalZC::Shutdown) | Err(_) => break,
        }
    }
    (count, probe)
}

/// Per-thread loop with no probe involvement.
///
/// - Identical to [`actor_loop_probed`] minus `probe.start` /
///   `probe.end` / `probe.clear`.
/// - `_probe_name` is unused but kept in the signature so the
///   function pointer matches [`ActorLoopFn`] without
///   special-casing.
fn actor_loop<S: BufRefStore + 'static>(
    mut actor: Box<dyn ActorZC<S> + Send>,
    rx: Receiver<SignalZC<S>>,
    peers: Vec<Sender<SignalZC<S>>>,
    pool: Pool<S>,
    _probe_name: String,
) -> u64 {
    let mut count: u64 = 0;
    loop {
        match rx.recv() {
            Ok(SignalZC::User(msg)) => {
                let mut ctx = MultiCtxZC {
                    senders: &peers,
                    pool: &pool,
                };
                actor.handle_message(&mut ctx, &msg);
                drop(msg);
                count += 1;
            }
            Ok(SignalZC::ClearProbe) => {
                count = 0;
            }
            Ok(SignalZC::Shutdown) | Err(_) => break,
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::Pool;

    /// Generic ping-pong actor reusable across `S` choices.
    /// On each message: get a same-sized reply buffer, send
    /// to the configured peer, drop the inbound `PooledMsg`
    /// (returns its buffer to the pool).
    struct PingPong {
        peer: u32,
    }

    impl<S: BufRefStore> ActorZC<S> for PingPong {
        fn handle_message(&mut self, ctx: &mut dyn ContextZC<S>, msg: &[u8]) {
            let reply = ctx.get_msg(msg.len()).expect("get_msg should succeed");
            ctx.send(self.peer, reply);
        }
    }

    /// Two-actor ping-pong drives traffic and shuts down
    /// cleanly via the probe path; both actors record nonzero
    /// counts.
    #[test]
    fn ping_pong_runs_and_shuts_down() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.zc");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let results = rt.run_probed(
            &mut mgr,
            initial,
            Duration::from_millis(40),
            Duration::from_millis(40),
            &[],
        );
        assert_eq!(results.len(), 2);
        let total: u64 = results.iter().map(|(c, _)| *c).sum();
        assert!(total > 0, "expected nonzero throughput, got count={total}");
    }

    /// After `run_probed` returns, every buffer the pool ever
    /// handed out has come back. Tests the drain story
    /// across the channel-close path.
    #[test]
    fn pool_is_full_after_shutdown() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.drain");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let _ = rt.run_probed(
            &mut mgr,
            initial,
            Duration::from_millis(20),
            Duration::from_millis(20),
            &[],
        );
        assert_eq!(
            pool.free_len(),
            pool.size(),
            "all buffers should return to pool after shutdown"
        );
    }

    /// `run` (probe-free default) orchestrates identically
    /// and returns per-actor counts. Validates that the
    /// probe-clean path runs the same workload.
    #[test]
    fn run_returns_counts() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.np");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let results = rt.run(
            &mut mgr,
            initial,
            Duration::from_millis(20),
            Duration::from_millis(20),
            &[],
        );
        assert_eq!(results.len(), 2);
        let total: u64 = results.iter().sum();
        assert!(total > 0, "expected nonzero throughput, got {total}");
        assert_eq!(
            pool.free_len(),
            pool.size(),
            "all buffers should return to pool after probe-free shutdown"
        );
    }

    /// `Runtime::pool` exposes the caller's handle; the
    /// caller can build initial messages from it without
    /// holding a separate clone.
    #[test]
    fn runtime_pool_accessor_returns_same_handle() {
        let pool: Pool = Pool::new(32, 2);
        let rt = RuntimeZC::new(pool.clone());
        assert_eq!(rt.pool().msg_size(), 32);
        assert_eq!(rt.pool().size(), 2);
    }
}
