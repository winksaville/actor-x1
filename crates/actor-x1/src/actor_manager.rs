//! Actor model surface — traits actors implement, plus the
//! catalog that registers and addresses them.
//!
//! This module defines the **what** of the zerocopy actor model:
//!
//! - [`ActorZC`] — the actor trait. One method, one message at
//!   a time.
//! - [`ContextZC`] — the send-side handle the runtime supplies.
//! - [`ActorManager`] — actor catalog. Holds instances and the
//!   probe-name prefix; assigns ids; nothing else.
//!
//! It deliberately knows nothing about the **how** — threads,
//! channels, the pool's lifecycle, or what's being measured —
//! that lives in [`crate::runtime_zc`]. The runtime is one
//! consumer of this surface; future control-plane / discovery
//! actors will be others.

use crate::pool::{BufRefStore, MutexLifo, PoolError, PooledMsg};

/// Actor behavior in the zerocopy runtime.
///
/// - Single method: handle one inbound message at a time;
///   never block.
/// - `msg: &[u8]` is the runtime-owned [`PooledMsg`]'s slice
///   view. The buffer returns to the pool after the handler
///   returns.
/// - Replies are fabricated via [`ContextZC::get_msg`] and
///   forwarded with [`ContextZC::send`].
/// - The trait requires `Send` so actors can ship into
///   spawned threads.
pub trait ActorZC<S: BufRefStore = MutexLifo>: Send {
    /// Handle one inbound message. Use `ctx` to fabricate /
    /// dispatch any outbound messages.
    fn handle_message(&mut self, ctx: &mut dyn ContextZC<S>, msg: &[u8]);
}

/// Send-side handle the runtime supplies to each
/// [`ActorZC::handle_message`] call.
///
/// - Carries access to the shared pool via
///   [`ContextZC::get_msg`].
/// - Dispatches outbound messages to other actors via
///   [`ContextZC::send`].
/// - Drops the `PooledMsg` if the destination channel is
///   closed (peer already shut down) so the buffer never
///   leaks.
pub trait ContextZC<S: BufRefStore = MutexLifo> {
    /// Acquire a buffer of length `size` from the pool.
    /// Forwards [`PoolError`] on failure (`SizeTooLarge` /
    /// `NoMsgs`); the actor handler decides how to react
    /// (typically `expect` for ping-pong workloads).
    fn get_msg(&mut self, size: usize) -> Result<PooledMsg<S>, PoolError>;

    /// Send `msg` to the actor identified by `dst_id`. A
    /// closed channel silently drops the message; the dropped
    /// `PooledMsg`'s buffer returns to the pool.
    fn send(&mut self, dst_id: u32, msg: PooledMsg<S>);
}

/// Actor catalog — registration and lifecycle, nothing else.
///
/// - Holds actor instances and the probe-name prefix.
/// - Hands out `u32` ids on `add`; that's the discovery
///   primitive other actors use as `dst_id` in
///   [`ContextZC::send`].
/// - Drained by a runtime at run time via
///   [`ActorManager::take_actors`]; single-shot for now.
/// - Knows nothing about threads, channels, the pool, or
///   what's being measured.
pub struct ActorManager<S: BufRefStore = MutexLifo> {
    actors: Vec<Box<dyn ActorZC<S> + Send>>,
    probe_name_prefix: String,
}

impl<S: BufRefStore + 'static> ActorManager<S> {
    /// Build an empty manager.
    ///
    /// `probe_name_prefix` is combined with each actor's id by
    /// the runtime to form per-thread probe names (e.g.
    /// `"goalzc.dispatch.actor0"`).
    pub fn new(probe_name_prefix: &str) -> Self {
        Self {
            actors: Vec::new(),
            probe_name_prefix: probe_name_prefix.to_string(),
        }
    }

    /// Register an actor instance.
    ///
    /// - Returns the assigned id (0-indexed; usable as
    ///   `dst_id` in [`ContextZC::send`]).
    /// - The actor must be `Send + 'static`. The trait already
    ///   requires `Send`; `'static` is the usual no-borrow
    ///   bound for moving across threads.
    pub fn add<A: ActorZC<S> + 'static>(&mut self, actor: A) -> u32 {
        let id = self.actors.len() as u32;
        self.actors.push(Box::new(actor));
        id
    }

    /// Drain the registered actors, leaving the manager empty.
    /// Called by the runtime at the start of a run.
    pub fn take_actors(&mut self) -> Vec<Box<dyn ActorZC<S> + Send>> {
        std::mem::take(&mut self.actors)
    }

    /// The probe-name prefix the runtime combines with each
    /// actor's id to name its `TProbe`.
    pub fn probe_name_prefix(&self) -> &str {
        &self.probe_name_prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trivial actor stub — never invoked by these tests; only
    /// needs to satisfy `ActorZC<S> + Send + 'static` for
    /// `ActorManager::add`.
    struct Noop;

    impl<S: BufRefStore> ActorZC<S> for Noop {
        fn handle_message(&mut self, _ctx: &mut dyn ContextZC<S>, _msg: &[u8]) {}
    }

    /// `take_actors` drains the manager and resets it to
    /// empty so a follow-up registration starts from id 0.
    #[test]
    fn take_actors_drains_and_resets() {
        let mut mgr: ActorManager = ActorManager::new("test.drain-mgr");
        assert_eq!(mgr.add(Noop), 0);
        assert_eq!(mgr.add(Noop), 1);
        let drained = mgr.take_actors();
        assert_eq!(drained.len(), 2);
        // Manager is now empty; new registrations restart from 0.
        assert_eq!(mgr.add(Noop), 0);
    }

    /// `ActorManager::probe_name_prefix` exposes the
    /// caller-supplied prefix verbatim; the runtime composes
    /// `"<prefix>.actor<id>"` for each thread's probe name
    /// (verified end-to-end via `goalzc` output rather than
    /// here, since `TProbe::name` is not on the public
    /// surface).
    #[test]
    fn manager_exposes_probe_name_prefix() {
        let mgr: ActorManager = ActorManager::new("custom.prefix");
        assert_eq!(mgr.probe_name_prefix(), "custom.prefix");
    }
}
