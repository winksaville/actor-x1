//! Goal1 binary: two actors on a single thread ping-pong an empty
//! [`Message`] for a caller-supplied duration in seconds (f64).
//!
//! Usage: `goal1 <duration_secs>`

use std::time::Duration;

use actor_x1::runtime::SingleThreadRuntime;
use actor_x1::{Actor, Context, Message};

/// Actor that, on every message received, sends exactly one message
/// back to its configured peer.
struct PingPongActor {
    peer_id: u32,
}

impl Actor for PingPongActor {
    /// Forward a single `Message` to `self.peer_id`. Never blocks.
    fn handle_message(&mut self, ctx: &mut dyn Context, _msg: Message) {
        ctx.send(self.peer_id, Message);
    }
}

/// Parse the duration from argv, run two ping-pong actors for that
/// long on one thread, and print messages handled plus throughput.
fn main() {
    let mut args = std::env::args().skip(1);
    let duration_s: f64 = args
        .next()
        .expect("usage: goal1 <duration_secs>")
        .parse()
        .expect("duration must be a non-negative number of seconds");

    let duration = Duration::from_secs_f64(duration_s);

    let mut rt = SingleThreadRuntime::new();
    let a_id = rt.add_actor(Box::new(PingPongActor { peer_id: 1 }));
    let b_id = rt.add_actor(Box::new(PingPongActor { peer_id: 0 }));
    assert_eq!((a_id, b_id), (0, 1));
    rt.seed(a_id, Message);

    let count = rt.run_for(duration);
    let mmps = count as f64 / duration_s / 1e6;
    println!("goal1: {count} messages in {duration_s:.3}s ({mmps:.3} M msg/s)");
}
