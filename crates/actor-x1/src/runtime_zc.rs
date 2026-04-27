//! Multi-threaded zerocopy runtime — message-moving
//! infrastructure.
//!
//! Pairs with [`crate::actor_manager`], which owns the actor
//! model surface ([`ActorZC`], [`ContextZC`], [`ActorManager`]).
//! This module is one consumer of that surface; future
//! transports (M:N scheduler, alternative channel types, …)
//! would be peers.
//!
//! Two API styles in this version:
//!
//! - **Lifecycle (preferred, new in `0.6.0-1`)**: caller
//!   composes [`RuntimeZC::startup`] →
//!   [`Handle::run`] (any number of run windows) →
//!   [`Handle::stop`]. `stop` returns the aggregate `u64`
//!   message count across all actor threads. Probe-free;
//!   diagnostic instrumentation will re-enter post-`0.6.0`
//!   via an actor-wrapper trait or `TProbe::Counter`.
//! - **All-in-one (legacy, removed in `0.6.0-3`)**:
//!   [`RuntimeZC::run`] (probe-free, returns per-actor
//!   counts) and [`RuntimeZC::run_probed`] (probe-
//!   instrumented, returns per-actor `(count, TProbe)`).
//!   These manage their own warmup → measurement → shutdown
//!   internally; the warmup boundary issues `ClearProbe` so
//!   counts and the histogram only see steady-state data.
//!
//! Drain invariant: by the time the join completes, every
//! [`PooledMsg`] that was ever in flight has been dropped, so
//! `pool.free_len() == pool.size()`. Holds for both API
//! styles.

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

    /// Spawn actor threads and deliver initial messages.
    /// Shared startup logic for [`RuntimeZC::run`] /
    /// [`RuntimeZC::run_probed`] (legacy all-in-one) and
    /// [`RuntimeZC::startup`] (lifecycle).
    ///
    /// - Drains the manager's actors via
    ///   [`ActorManager::take_actors`].
    /// - Builds one `mpsc` channel per actor; spawns one
    ///   thread per actor running `actor_loop`, optionally
    ///   pinned via `pin_cores[id % pin_cores.len()]`.
    /// - Sends `initial_messages` over the per-actor
    ///   senders before returning.
    /// - Returns the senders (for `ClearProbe` / `Shutdown`
    ///   signaling) and join handles (for the eventual join).
    fn spawn_and_seed<R: Send + 'static>(
        &self,
        mgr: &mut ActorManager<S>,
        initial_messages: Vec<(u32, PooledMsg<S>)>,
        pin_cores: &[usize],
        actor_loop: ActorLoopFn<S, R>,
    ) -> (Vec<Sender<SignalZC<S>>>, Vec<JoinHandle<R>>) {
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

        (senders, handles)
    }

    /// Shared orchestration body for the legacy all-in-one
    /// [`RuntimeZC::run`] / [`RuntimeZC::run_probed`].
    ///
    /// - Calls [`Self::spawn_and_seed`] to spawn threads and
    ///   deliver initial messages.
    /// - Sleeps `warmup`, sends `ClearProbe` to every actor,
    ///   sleeps `measurement`, sends `Shutdown`, drops senders,
    ///   joins, returns per-actor results.
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
        let (senders, handles) = self.spawn_and_seed(mgr, initial_messages, pin_cores, actor_loop);

        thread::sleep(warmup);
        for tx in &senders {
            let _ = tx.send(SignalZC::ClearProbe);
        }

        thread::sleep(measurement);
        for tx in &senders {
            let _ = tx.send(SignalZC::Shutdown);
        }
        drop(senders);

        let mut results = Vec::with_capacity(handles.len());
        for h in handles {
            results.push(h.join().expect("actor thread panicked"));
        }
        results
    }

    /// Spawn actor threads and deliver initial messages,
    /// returning a [`Handle`] that owns the actor mesh.
    ///
    /// - The caller drives the lifecycle from there:
    ///   [`Handle::run`] sleeps a run window (any number
    ///   of times); [`Handle::stop`] sends `Shutdown`,
    ///   joins, and returns the aggregate `u64` count.
    /// - No probe support — counts only. Probing re-enters
    ///   post-`0.6.0` via an actor-wrapper trait or
    ///   `TProbe::Counter`.
    /// - On `Drop` without `stop`, the handle does a silent
    ///   shutdown (panics on actor threads are swallowed).
    pub fn startup(
        &mut self,
        mgr: &mut ActorManager<S>,
        initial_messages: Vec<(u32, PooledMsg<S>)>,
        pin_cores: &[usize],
    ) -> Handle<S> {
        let (senders, handles) =
            self.spawn_and_seed(mgr, initial_messages, pin_cores, actor_loop::<S>);
        Handle {
            senders: Some(senders),
            handles: Some(handles),
            pool: self.pool.clone(),
        }
    }
}

/// Live handle to actor threads spawned by
/// [`RuntimeZC::startup`].
///
/// - Holds the per-actor senders and join handles plus a
///   clone of the runtime's pool.
/// - [`Handle::run`] sleeps a run window; actor threads continue
///   processing messages in the meantime. May be called any
///   number of times before `stop`.
/// - [`Handle::stop`] sends `Shutdown` to every actor, joins
///   all threads, and returns the aggregate `u64` count
///   (sum of per-thread local counters, no atomics in the
///   hot path).
/// - On `Drop` without `stop`, sends `Shutdown` and joins
///   silently (panics on actor threads are swallowed; drop
///   must not panic-during-drop).
pub struct Handle<S: BufRefStore = MutexLifo> {
    senders: Option<Vec<Sender<SignalZC<S>>>>,
    handles: Option<Vec<JoinHandle<u64>>>,
    pool: Pool<S>,
}

impl<S: BufRefStore> Handle<S> {
    /// Sleep for `duration` (a run window). Actor threads
    /// continue processing messages while the caller sleeps;
    /// counts accumulate in their per-thread local `u64`s.
    pub fn run(&self, duration: Duration) {
        thread::sleep(duration);
    }

    /// Borrow the runtime's pool handle (for ad-hoc post-
    /// startup `get_msg` / inspection).
    pub fn pool(&self) -> &Pool<S> {
        &self.pool
    }

    /// Send `ClearProbe` to every actor thread; on receipt
    /// each thread zeros its per-thread message count. Used
    /// to mark the warmup → measurement boundary so
    /// `stop`'s aggregate excludes warmup messages.
    ///
    /// - Caller-driven: not implicit. The lifecycle API has
    ///   no built-in warmup phase.
    /// - Best-effort: if a thread already exited, the send
    ///   silently fails. The count of an exited thread is
    ///   already captured in its `JoinHandle::Output`.
    pub fn reset_count(&self) {
        if let Some(senders) = self.senders.as_ref() {
            for tx in senders {
                let _ = tx.send(SignalZC::ClearProbe);
            }
        }
    }

    /// Send `Shutdown` to every actor, join all threads, and
    /// return the aggregate per-actor message count.
    ///
    /// - Panics if any actor thread panicked. To swallow
    ///   panics (and skip the count), drop the handle
    ///   instead of calling `stop`.
    pub fn stop(mut self) -> u64 {
        let mut total: u64 = 0;
        if let Some(senders) = self.senders.take() {
            for tx in &senders {
                let _ = tx.send(SignalZC::Shutdown);
            }
            drop(senders);
        }
        if let Some(handles) = self.handles.take() {
            for h in handles {
                total = total.saturating_add(h.join().expect("actor thread panicked"));
            }
        }
        total
    }
}

impl<S: BufRefStore> Drop for Handle<S> {
    /// Silent graceful shutdown when [`Handle::stop`] was
    /// not called: send `Shutdown`, join all threads,
    /// discard counts and any panics.
    fn drop(&mut self) {
        if let Some(senders) = self.senders.take() {
            for tx in &senders {
                let _ = tx.send(SignalZC::Shutdown);
            }
            drop(senders);
        }
        if let Some(handles) = self.handles.take() {
            for h in handles {
                let _ = h.join();
            }
        }
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

    /// Lifecycle round trip: `startup` → `run` (one run
    /// window) → `stop` returns the aggregate count and
    /// drains the pool.
    #[test]
    fn lifecycle_round_trip_returns_total() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.lifecycle");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let handle = rt.startup(&mut mgr, initial, &[]);
        handle.run(Duration::from_millis(40));
        let total = handle.stop();
        assert!(total > 0, "expected nonzero total, got {total}");
        assert_eq!(
            pool.free_len(),
            pool.size(),
            "all buffers should return to pool after stop"
        );
    }

    /// `Handle::run` may be called multiple times before
    /// `stop`; the aggregate count covers all run windows.
    #[test]
    fn lifecycle_multiple_run_windows_accumulate() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.multi");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let handle = rt.startup(&mut mgr, initial, &[]);
        handle.run(Duration::from_millis(20));
        handle.run(Duration::from_millis(20));
        let total = handle.stop();
        assert!(total > 0, "expected nonzero total, got {total}");
    }

    /// `Handle::pool` exposes the runtime's pool handle so
    /// callers can build ad-hoc messages post-startup
    /// without keeping a separate clone.
    #[test]
    fn lifecycle_handle_pool_accessor() {
        let pool: Pool = Pool::new(32, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.handle.pool");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let handle = rt.startup(&mut mgr, initial, &[]);
        assert_eq!(handle.pool().msg_size(), 32);
        assert_eq!(handle.pool().size(), 4);
        handle.run(Duration::from_millis(20));
        let _ = handle.stop();
    }

    /// `reset_count` zeros each actor thread's per-thread
    /// count at the warmup → measurement boundary. Smoke
    /// test: runtime stays healthy (drains, returns nonzero
    /// total for the post-reset run window).
    #[test]
    fn lifecycle_reset_count_runs_clean() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.reset");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        let handle = rt.startup(&mut mgr, initial, &[]);
        handle.run(Duration::from_millis(20));
        handle.reset_count();
        handle.run(Duration::from_millis(20));
        let total = handle.stop();
        assert!(total > 0, "expected nonzero total post-reset, got {total}");
        assert_eq!(
            pool.free_len(),
            pool.size(),
            "all buffers should return to pool after stop"
        );
    }

    /// Dropping `Handle` without calling `stop` performs a
    /// silent graceful shutdown: actors finish, threads
    /// join, pool drains.
    #[test]
    fn lifecycle_handle_drop_shuts_down_cleanly() {
        let pool: Pool = Pool::new(64, 4);
        let mut rt = RuntimeZC::new(pool.clone());
        let mut mgr = ActorManager::new("test.drop");
        let a = mgr.add(PingPong { peer: 1 });
        let _b = mgr.add(PingPong { peer: 0 });
        let initial = vec![(a, pool.get_msg(8).expect("seed"))];
        {
            let handle = rt.startup(&mut mgr, initial, &[]);
            handle.run(Duration::from_millis(20));
            // drop without stop — shutdown happens in Drop.
        }
        assert_eq!(
            pool.free_len(),
            pool.size(),
            "all buffers should return to pool after drop"
        );
    }
}
