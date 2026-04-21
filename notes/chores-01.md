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
