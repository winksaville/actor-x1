//! actor-x1 — an experiment in the actor model in Rust.
//!
//! See `notes/design.md` for the staged design. Stage 1 provides
//! a minimal runtime with two actors that ping-pong an empty
//! message for a caller-specified duration; Goal1 runs both
//! actors on one thread, Goal2 runs each actor on its own thread.
