# Chores-01

Discussions and notes on various chores in github compatible markdown.
There is also a [todo.md](todo.md) file and it tracks tasks and in
general there should be a chore section for each task with the why
and how this task will be completed.

See [Chores format](README.md#chores-format)

## Stage 1 runtime — plan marker (0.1.0-0)

Begins the Stage 1 implementation described in [design.md](design.md).
Multi-step ladder (see [Versioning during development](../CLAUDE.md#versioning)):

- `0.1.0-0` — bootstrap Cargo crate + this plan marker (no behavior).
- `0.1.0-1` — Goal1: two actors, single thread, ping-pong an empty
  `Message` for a caller-supplied duration in seconds. Reports
  total messages handled.
- `0.1.0-2` — vendor `tprobe2` (and its transitive deps: `band_table`,
  `ticks`, `fmt_commas*`) from `../iiac-perf/src` into `src/perf/`.
  Wrap per-dispatch `handle_message` calls with `probe.start`/`probe.end`
  using `site_id = actor_id`; print a band-table report at shutdown
  (ns by default, `-t/--ticks` for raw ticks). The bot thinks promoting
  this to a shared crate is worthwhile once Stage 2 is underway, but
  copying in now avoids a cross-repo dependency for the PoC.
- `0.1.0-3` — Goal2: same workload as Goal1 but each actor on its own
  thread, communicating via `std::sync::mpsc` channels. Shutdown by
  dropping senders. Probe instrumented the same way (per-thread
  `TProbe2`, report on join).
- `0.1.0` — final: drop the `-N` suffix, update `notes/todo.md` and
  `notes/README.md`.

### Design decisions recorded here

- **Actor trait signature** deviates from the design sketch by adding
  `&mut self` and a `&mut dyn Context` parameter, where `Context::send(dst_id, msg)`
  is the only way for a handler to emit messages. Required because
  the empty `Message {}` carries no reply-to information and the
  sketched signature `fn handle_message(msg: Message)` has nowhere
  to thread outbound sends through.
- **Two `Context` implementations**: the single-threaded runtime's
  context writes to a `VecDeque`; the multi-threaded runtime's
  context holds `Sender<Message>` per destination. Actors see
  only `&mut dyn Context` and are agnostic to which runtime is
  driving them.
- **x86_64-only for now**: `tprobe2`'s tick counter uses `rdtsc`
  and `iiac-perf/src/ticks.rs` emits a `compile_error!` on other
  arches. Development target is x86_64 so this is acceptable; the
  restriction is documented at the vendored module.

## Goal1: single-thread ping-pong runtime (0.1.0-1)

Implements Goal1 of Stage 1: two actors, one thread, ping-pong an
empty [`Message`] for a caller-supplied duration in seconds, then
report the message count and throughput.

- `src/lib.rs`: `Message` unit struct, `Actor` trait
  (`handle_message(&mut self, &mut dyn Context, Message)`),
  `Context` trait (`send(dst_id: u32, msg: Message)`), and
  `pub mod runtime`.
- `src/runtime.rs`: `SingleThreadRuntime` owns a
  `Vec<Box<dyn Actor>>` and a `VecDeque<(u32, Message)>`; `run_for`
  pops a message, field-split-borrows `actors` and `queue`,
  constructs a private `SingleCtx` wrapping the queue, dispatches,
  counts. Terminates on deadline or queue drain. Unit tests cover
  seed drain, bounded ping-pong count (11 = seed + 5·2), and
  sequential id assignment.
- `src/bin/goal1.rs`: CLI takes one positional `<duration_secs>`
  f64; constructs two `PingPongActor`s (each replies once per
  received message to its peer), seeds one message into actor 0,
  runs, prints `goal1: <count> messages in <secs>s (<M msg/s>)`.

Smoke run on this box: 0.5 s → ~19.3 M messages → ~38.7 M msg/s.

### Deviation from the design sketch

`struct Message {}` in `design.md` is realised as a unit struct
`pub struct Message;` — equivalent externally, instantiates as
`Message` rather than `Message {}`. Matches clippy's preference for
empty types; trivial to convert back if literal match matters.
