//! Runtimes that drive [`Actor`]s. The single-thread runtime lives
//! here today; the multi-thread runtime arrives in a later step.

use std::collections::VecDeque;
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
