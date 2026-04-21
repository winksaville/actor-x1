//! Runtimes that drive [`Actor`]s. The single-thread runtime lives
//! here today; the multi-thread runtime arrives in 0.1.0-3.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::{Actor, Context, Message};

/// Single-threaded runtime: all actors run on the calling thread,
/// sharing one FIFO message queue. Dispatch loops until either the
/// caller-supplied deadline expires or the queue drains.
pub struct SingleThreadRuntime {
    actors: Vec<Box<dyn Actor>>,
    queue: VecDeque<(u32, Message)>,
}

impl SingleThreadRuntime {
    /// Create an empty runtime with no actors or pending messages.
    pub fn new() -> Self {
        Self {
            actors: Vec::new(),
            queue: VecDeque::new(),
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
    /// empties. Returns the total number of messages handled.
    pub fn run_for(&mut self, duration: Duration) -> u64 {
        let deadline = Instant::now() + duration;
        let mut count = 0u64;
        while Instant::now() < deadline {
            let Some((dst, msg)) = self.queue.pop_front() else {
                break;
            };
            let Self { actors, queue } = self;
            let actor = &mut actors[dst as usize];
            let mut ctx = SingleCtx { queue };
            actor.handle_message(&mut ctx, msg);
            count += 1;
        }
        count
    }
}

impl Default for SingleThreadRuntime {
    /// Same as [`SingleThreadRuntime::new`].
    fn default() -> Self {
        Self::new()
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
        let mut rt = SingleThreadRuntime::new();
        let id = rt.add_actor(Box::new(Sink));
        rt.seed(id, Message);
        let n = rt.run_for(Duration::from_millis(1));
        assert_eq!(n, 1);
    }

    #[test]
    fn bounded_ping_pong_count() {
        let mut rt = SingleThreadRuntime::new();
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
        // seed (1) + 5 from a + 5 from b = 11
        let n = rt.run_for(Duration::from_secs(1));
        assert_eq!(n, 11);
    }

    #[test]
    fn add_actor_assigns_sequential_ids() {
        let mut rt = SingleThreadRuntime::new();
        assert_eq!(rt.add_actor(Box::new(Sink)), 0);
        assert_eq!(rt.add_actor(Box::new(Sink)), 1);
        assert_eq!(rt.add_actor(Box::new(Sink)), 2);
    }
}
