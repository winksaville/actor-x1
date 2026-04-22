//! Runtimes that drive [`Actor`]s. Two flavors live here:
//! [`SingleThreadRuntime`] (Goal1: one thread, FIFO queue) and
//! [`MultiThreadRuntime`] (Goal2: one thread per actor, `mpsc`
//! channels).

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::perf::TProbe2;
use crate::{Actor, Context, Message};

/// Single-threaded runtime: all actors run on the calling thread,
/// sharing one FIFO message queue. Dispatch loops until either the
/// caller-supplied deadline expires or the queue drains.
///
/// Dispatch is wrapped in a [`TProbe2`] batched scope covering
/// `inner` consecutive `handle_message` calls; per-event latency
/// is available via [`Self::probe_mut`] after [`Self::run_for`]
/// returns. The scope's `site_id` is the first `dst_id` in the
/// batch (arbitrary-but-stable when `inner > 1`).
pub struct SingleThreadRuntime {
    actors: Vec<Box<dyn Actor>>,
    queue: VecDeque<(u32, Message)>,
    probe: TProbe2,
}

impl SingleThreadRuntime {
    /// Create an empty runtime with no actors or pending messages.
    /// `probe_name` names the embedded probe in its band-table
    /// report (e.g. `"goal1.dispatch"`).
    pub fn new(probe_name: &str) -> Self {
        Self {
            actors: Vec::new(),
            queue: VecDeque::new(),
            probe: TProbe2::new(probe_name),
        }
    }

    /// Register `actor`; the returned `u32` is its stable id for
    /// the lifetime of the runtime and is usable as `dst_id` in
    /// [`Context::send`]. Ids are allocated sequentially from 0.
    pub fn add_actor(&mut self, actor: Box<dyn Actor>) -> u32 {
        let id = self.actors.len() as u32;
        self.actors.push(actor);
        id
    }

    /// Enqueue an initial message for `dst_id`. Used to prime the
    /// dispatch loop before [`Self::run_for`].
    pub fn seed(&mut self, dst_id: u32, msg: Message) {
        self.queue.push_back((dst_id, msg));
    }

    /// Dispatch messages until `duration` elapses or the queue
    /// empties. Each probe scope wraps `inner` consecutive
    /// dispatches. A partial trailing batch (queue emptied
    /// mid-scope) is discarded — neither recorded nor counted —
    /// so throughput numbers reflect whole-batch work only.
    ///
    /// Returns the total number of messages counted toward
    /// whole batches.
    pub fn run_for(&mut self, duration: Duration, inner: u64) -> u64 {
        let deadline = Instant::now() + duration;
        let mut count = 0u64;
        while Instant::now() < deadline {
            let first_dst = match self.queue.front() {
                Some((d, _)) => *d,
                None => break,
            };
            let Self {
                actors,
                queue,
                probe,
            } = self;
            let id = probe.start(first_dst as u64);
            let mut done: u64 = 0;
            while done < inner {
                let Some((dst, msg)) = queue.pop_front() else {
                    break;
                };
                let actor = &mut actors[dst as usize];
                let mut ctx = SingleCtx { queue };
                actor.handle_message(&mut ctx, msg);
                done += 1;
            }
            if done == inner {
                probe.end_batch(id, inner);
                count += done;
            } else {
                // Partial trailing batch: drop the scope (don't
                // record or count) and stop.
                break;
            }
        }
        count
    }

    /// Mutable access to the embedded probe — used by callers to
    /// invoke [`TProbe2::clear`] at the warmup → measurement
    /// boundary and [`TProbe2::report`] after [`Self::run_for`]
    /// returns.
    pub fn probe_mut(&mut self) -> &mut TProbe2 {
        &mut self.probe
    }
}

/// [`Context`] impl used by [`SingleThreadRuntime`]: pushes onto
/// the runtime's shared queue.
struct SingleCtx<'a> {
    queue: &'a mut VecDeque<(u32, Message)>,
}

impl Context for SingleCtx<'_> {
    /// Push `(dst_id, msg)` onto the shared dispatch queue.
    fn send(&mut self, dst_id: u32, msg: Message) {
        self.queue.push_back((dst_id, msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal actor that drops every message without replying.
    struct Sink;

    impl Actor for Sink {
        fn handle_message(&mut self, _ctx: &mut dyn Context, _msg: Message) {}
    }

    /// Bounded ping-pong actor: replies to its peer up to `max`
    /// times, then goes silent. Used to produce a deterministic
    /// message count in unit tests without wall-clock dependence.
    struct Bouncer {
        peer: u32,
        sent: u32,
        max: u32,
    }

    impl Actor for Bouncer {
        fn handle_message(&mut self, ctx: &mut dyn Context, _msg: Message) {
            if self.sent < self.max {
                ctx.send(self.peer, Message);
                self.sent += 1;
            }
        }
    }

    #[test]
    fn seed_is_handled_and_queue_drains() {
        let mut rt = SingleThreadRuntime::new("test.drain");
        let id = rt.add_actor(Box::new(Sink));
        rt.seed(id, Message);
        let n = rt.run_for(Duration::from_millis(1), 1);
        assert_eq!(n, 1);
    }

    #[test]
    fn bounded_ping_pong_count() {
        let mut rt = SingleThreadRuntime::new("test.bounded");
        let a = rt.add_actor(Box::new(Bouncer {
            peer: 1,
            sent: 0,
            max: 5,
        }));
        let _b = rt.add_actor(Box::new(Bouncer {
            peer: 0,
            sent: 0,
            max: 5,
        }));
        rt.seed(a, Message);
        // seed (1) + 5 from a + 5 from b = 11, all whole batches at inner=1.
        let n = rt.run_for(Duration::from_secs(1), 1);
        assert_eq!(n, 11);
    }

    #[test]
    fn add_actor_assigns_sequential_ids() {
        let mut rt = SingleThreadRuntime::new("test.ids");
        assert_eq!(rt.add_actor(Box::new(Sink)), 0);
        assert_eq!(rt.add_actor(Box::new(Sink)), 1);
        assert_eq!(rt.add_actor(Box::new(Sink)), 2);
    }

    #[test]
    fn partial_trailing_batch_is_dropped() {
        // 3 total dispatches available, inner=2 ⇒ one whole batch
        // counted (2) and one partial (1) discarded.
        let mut rt = SingleThreadRuntime::new("test.partial");
        let a = rt.add_actor(Box::new(Bouncer {
            peer: 1,
            sent: 0,
            max: 1,
        }));
        let _b = rt.add_actor(Box::new(Bouncer {
            peer: 0,
            sent: 0,
            max: 1,
        }));
        rt.seed(a, Message);
        // Events: seed→b, b→a (b.sent=1), a→b (a.sent=1), b stops.
        // 3 dispatches total. inner=2 ⇒ first batch of 2 recorded
        // (count=2), then partial batch of 1 discarded.
        let n = rt.run_for(Duration::from_secs(1), 2);
        assert_eq!(n, 2);
    }
}

// --- MultiThreadRuntime -------------------------------------------------

/// Multi-threaded runtime: one thread per actor, messages flow
/// over per-actor [`std::sync::mpsc`] channels. Each actor thread
/// owns a [`TProbe2`] recording per-`handle_message` latency at
/// `inner=1`; probes are returned to the caller when threads join.
///
/// Unlike [`SingleThreadRuntime`], actors are registered as
/// factory closures — construction runs inside the actor's own
/// thread, so users don't need `Send` on the trait itself, only on
/// the actor implementor and its factory.
///
/// Shutdown is in-band: an internal `Signal::Shutdown` value
/// travels the same channel that carries user messages, so the
/// actor's `recv()` loop exits cleanly after the measurement
/// window elapses. A `Signal::ClearProbe` is used at the warmup →
/// measurement boundary so only steady-state data reaches the
/// report.
///
/// Ping-pong style workloads hold one message in flight between
/// two actors; `inner > 1` batching would leave every probe scope
/// partial (queue empties mid-batch) so this runtime hardcodes
/// `inner = 1`. If we later add a Goal with higher queue depth
/// we'll revisit.
pub struct MultiThreadRuntime {
    factories: Vec<ActorFactory>,
    probe_name_prefix: String,
    pending_seeds: Vec<(u32, Message)>,
}

/// Boxed factory: produces a `Box<dyn Actor + Send>` once, called
/// inside the actor's dedicated thread. `Send` on the outer
/// closure is required so it can be moved into `thread::spawn`.
type ActorFactory = Box<dyn FnOnce() -> Box<dyn Actor + Send> + Send>;

/// Internal channel payload. User sends become [`Signal::User`];
/// [`Signal::ClearProbe`] and [`Signal::Shutdown`] are injected by
/// the runtime to drive phase transitions and exit.
enum Signal {
    User(Message),
    ClearProbe,
    Shutdown,
}

impl MultiThreadRuntime {
    /// Create an empty runtime. `probe_name_prefix` is combined
    /// with the actor id to form each per-thread probe's name
    /// (e.g. `"goal2.dispatch.actor0"`).
    pub fn new(probe_name_prefix: &str) -> Self {
        Self {
            factories: Vec::new(),
            probe_name_prefix: probe_name_prefix.to_string(),
            pending_seeds: Vec::new(),
        }
    }

    /// Register an actor factory; returns the assigned id
    /// (0-indexed). `factory` is called inside the spawned thread
    /// that will drive this actor, so the actor type itself does
    /// not need to be `Send` before the call — only the factory.
    pub fn add_actor<F, A>(&mut self, factory: F) -> u32
    where
        F: FnOnce() -> A + Send + 'static,
        A: Actor + Send + 'static,
    {
        let id = self.factories.len() as u32;
        self.factories.push(Box::new(move || {
            Box::new(factory()) as Box<dyn Actor + Send>
        }));
        id
    }

    /// Enqueue an initial message for `dst_id`. Delivered after
    /// all threads spawn and before warmup begins.
    pub fn seed(&mut self, dst_id: u32, msg: Message) {
        self.pending_seeds.push((dst_id, msg));
    }

    /// Spawn one thread per registered actor, run the warmup
    /// phase, clear all probes, run the measurement phase, shut
    /// down and join every thread. Returns each actor's
    /// `(messages_handled, probe)` pair in id order. Consumes the
    /// runtime's registered factories and pending seeds.
    pub fn run(&mut self, warmup: Duration, measurement: Duration) -> Vec<(u64, TProbe2)> {
        let factories = std::mem::take(&mut self.factories);
        let seeds = std::mem::take(&mut self.pending_seeds);
        let n = factories.len();

        // Per-actor channels.
        let mut senders: Vec<Sender<Signal>> = Vec::with_capacity(n);
        let mut receivers: Vec<Option<Receiver<Signal>>> = Vec::with_capacity(n);
        for _ in 0..n {
            let (tx, rx) = mpsc::channel();
            senders.push(tx);
            receivers.push(Some(rx));
        }

        // Spawn one thread per actor.
        let mut handles: Vec<JoinHandle<(u64, TProbe2)>> = Vec::with_capacity(n);
        for (id, factory) in factories.into_iter().enumerate() {
            #[allow(clippy::unwrap_used)]
            // OK: receivers[id] was Some(...) since we pushed one per actor above.
            let rx = receivers[id].take().unwrap();
            let peers: Vec<Sender<Signal>> = senders.to_vec();
            let probe_name = format!("{}.actor{}", self.probe_name_prefix, id);
            let h = thread::Builder::new()
                .name(format!("actor-{id}"))
                .spawn(move || {
                    let actor = factory();
                    actor_loop(actor, rx, peers, probe_name)
                })
                .expect("failed to spawn actor thread");
            handles.push(h);
        }

        // Seed initial messages.
        for (dst, msg) in seeds {
            #[allow(clippy::unwrap_used)]
            // OK: receivers are alive (threads just spawned); a send failure would
            //   indicate an immediate thread panic on startup which the subsequent
            //   join will surface with a clearer message.
            senders[dst as usize].send(Signal::User(msg)).unwrap();
        }

        // Phase 1: warmup.
        thread::sleep(warmup);

        // Clear every probe at the phase boundary.
        for tx in &senders {
            let _ = tx.send(Signal::ClearProbe);
        }

        // Phase 2: measurement.
        thread::sleep(measurement);

        // Phase 3: shutdown.
        for tx in &senders {
            let _ = tx.send(Signal::Shutdown);
        }
        // Drop the main-thread copies so no dangling senders keep
        // channels open past thread exit.
        drop(senders);

        // Join and collect (count, probe) in id order.
        let mut results = Vec::with_capacity(n);
        for h in handles {
            results.push(h.join().expect("actor thread panicked"));
        }
        results
    }
}

/// [`Context`] impl used inside each actor thread: dispatches
/// user sends over the shared peer senders.
struct MultiCtx<'a> {
    senders: &'a [Sender<Signal>],
}

impl Context for MultiCtx<'_> {
    /// Send `msg` to `dst_id`'s mpsc channel. A closed channel
    /// (peer has already exited) is silently ignored — matches
    /// the shutdown semantics.
    fn send(&mut self, dst_id: u32, msg: Message) {
        let _ = self.senders[dst_id as usize].send(Signal::User(msg));
    }
}

/// Actor thread's main loop. Creates a [`TProbe2`] named
/// `probe_name`, blocks on `rx.recv()`, and for each
/// `Signal::User` message probes a scope, dispatches
/// `handle_message`, and increments the count.
/// `Signal::ClearProbe` resets the probe and count;
/// `Signal::Shutdown` (or a closed channel) exits the loop and
/// returns `(count, probe)`.
fn actor_loop(
    mut actor: Box<dyn Actor + Send>,
    rx: Receiver<Signal>,
    peers: Vec<Sender<Signal>>,
    probe_name: String,
) -> (u64, TProbe2) {
    let mut probe = TProbe2::new(&probe_name);
    let mut count: u64 = 0;
    loop {
        match rx.recv() {
            Ok(Signal::User(msg)) => {
                let id = probe.start(0);
                let mut ctx = MultiCtx { senders: &peers };
                actor.handle_message(&mut ctx, msg);
                probe.end(id);
                count += 1;
            }
            Ok(Signal::ClearProbe) => {
                probe.clear();
                count = 0;
            }
            Ok(Signal::Shutdown) | Err(_) => break,
        }
    }
    (count, probe)
}

#[cfg(test)]
mod mt_tests {
    use super::*;

    /// Unbounded ping-pong actor used for multi-thread tests.
    struct ThreadPingPong {
        peer: u32,
    }

    impl Actor for ThreadPingPong {
        fn handle_message(&mut self, ctx: &mut dyn Context, _: Message) {
            ctx.send(self.peer, Message);
        }
    }

    #[test]
    fn multi_thread_runs_measures_and_shuts_down_cleanly() {
        let mut rt = MultiThreadRuntime::new("test");
        rt.add_actor(|| ThreadPingPong { peer: 1 });
        rt.add_actor(|| ThreadPingPong { peer: 0 });
        rt.seed(0, Message);
        // 50ms each phase — leaves plenty of slack even on
        // heavily loaded CI, while still under 0.2s total.
        let results = rt.run(Duration::from_millis(50), Duration::from_millis(50));
        assert_eq!(results.len(), 2);
        // Ping-pong is symmetric: both actors should have handled
        // messages during the measurement window.
        let total: u64 = results.iter().map(|(c, _)| *c).sum();
        assert!(
            total > 0,
            "expected some messages processed during measurement, got {total}"
        );
    }
}
