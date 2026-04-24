//! actor-x1 — an experiment in the actor model in Rust.
//!
//! See `crates/actor-x1/notes/design.md` for the staged design. Stage 1 provides
//! a minimal runtime with two actors that ping-pong an empty
//! [`Message`] for a caller-supplied duration; Goal1 runs both
//! actors on one thread, Goal2 runs each actor on its own thread.
//!
//! Public surface today:
//! - [`Message`]: the empty unit message Stage 1 actors exchange.
//! - [`Actor`]: the single-method actor trait.
//! - [`Context`]: the send-side handle passed to each dispatch.
//! - [`runtime`]: concrete runtimes (single-thread for Goal1; the
//!   multi-thread runtime will land in 0.1.0-3).

pub mod pool;
pub mod runtime;

/// Empty unit-type message exchanged between Stage 1 actors.
/// Carries no payload by design — Stage 1's contract is only
/// "dispatch happens"; Stage 2 replaces this with a richer type.
pub struct Message;

/// Actor behavior: handle one [`Message`] at a time, never blocking.
///
/// The only method is [`Actor::handle_message`]; all outbound
/// communication goes through the [`Context`] the runtime supplies.
pub trait Actor {
    /// Handle a single incoming message. Use `ctx` to emit any
    /// resulting sends. Must not block.
    fn handle_message(&mut self, ctx: &mut dyn Context, msg: Message);
}

/// Runtime-supplied send handle passed to each
/// [`Actor::handle_message`] call. Implementations differ between
/// runtimes: the single-thread runtime writes to a shared queue;
/// the (future) multi-thread runtime writes to per-actor channels.
pub trait Context {
    /// Enqueue `msg` for delivery to the actor identified by
    /// `dst_id`. Returns immediately; delivery is the runtime's job.
    fn send(&mut self, dst_id: u32, msg: Message);
}
