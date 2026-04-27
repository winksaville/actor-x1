# Chores-03.md

Chores for the `0.6.0` ladder onward.

## RuntimeZC lifecycle: startup / run / stop — plan marker (0.6.0-0)

Plan marker for the `0.6.0` multi-step ladder. Splits the
runtime's all-in-one `run` / `run_probed` into a caller-
orchestrated lifecycle (`startup` / `Handle::run` /
`Handle::stop`). Probe support leaves the runtime entirely;
re-enters later via `TProbe::Counter` or an actor-wrapper
trait (out of scope for this ladder). `stop` is the
explicit graceful primitive, matching the post-`0.6.0`
typed-message-to-runtime story.

### Shape

```
let handle = rt.startup(&mut mgr, initial, pin_cores);
handle.run(window);          // optional, repeatable
let total = handle.stop();   // u64 aggregate; sends Shutdown
```

### Multi-step ladder

- `0.6.0-0` — plan marker (this section + version bump).
- `0.6.0-1` — `Handle` + `RuntimeZC::startup` /
  `Handle::run` / `Handle::stop`; tests updated. Existing
  `run` / `run_probed` stay alongside (additive step).
- `0.6.0-2` — Migrate `goalzc` bin and `goalzc-crit` bench
  to the lifecycle API. `goalzc` loses its band-table
  (no probing); keeps the msg/s line as a smoke generator.
  `goalzc-crit` keeps the current cycle-per-sample shape,
  just through the new API.
- `0.6.0-3` — Remove `RuntimeZC::run` / `run_probed` and
  `actor_loop_probed`. Runtime is probe-free +
  lifecycle-only.
- `0.6.0` — close.

### Decisions locked in

- **Counter**: per-thread local `u64`, no atomics. Returned
  via `JoinHandle` at `stop`. `stop -> u64` aggregate. KIS;
  per-actor breakdown is a future `TProbe::Counter`
  concern.
- **`Handle::run(window) -> ()`** — just sleeps the window.
  Counts not readable mid-flight by design.
- **`Handle::Drop`** calls `stop` if forgotten; result is
  discarded. Join errors are swallowed (drop must not
  panic-during-drop).
- **Probing leaves the runtime.** `goalzc`'s band-table
  goes with it. Probe re-introduction is post-`0.6.0`
  work, via `TProbe::Counter` or an actor-wrapper trait.
- **Methods on `RuntimeZC`**, not a trait. Trait when a
  second runtime impl appears.

### Defaults that aren't yet locked

- `Handle::pool()` accessor so the caller can inject ad-hoc
  messages post-startup. Default yes; revisit during
  `0.6.0-1` if the API gets ugly.

## Handle + RuntimeZC::startup / Handle::run / Handle::stop (0.6.0-1)

Adds the lifecycle API alongside the existing all-in-one
`run` / `run_probed`. Caller drives the actor mesh through
[`startup`] → [`Handle::run`] (any number of windows) →
[`Handle::stop`] (returns the aggregate `u64` count and
joins). Probe-free; counts come from per-thread local
`u64`s summed via `JoinHandle` at `stop`. The all-in-one
paths still work (refactored to share spawn / seed logic);
they go away in `0.6.0-3`.

- `crates/actor-x1/Cargo.toml`: `0.6.0-0` → `0.6.0-1`.
- `crates/actor-x1/src/runtime_zc.rs`:
  - New private `RuntimeZC::spawn_and_seed<R>` extracts
    the spawn + channel-wire + initial-message-deliver
    logic shared by the all-in-one `run_inner` and the
    new `startup`. `run_inner` is now a thin wrapper that
    composes `spawn_and_seed` with the warmup / clear /
    measurement / shutdown / join sequence.
  - New `pub fn RuntimeZC::startup(mgr, initial, pin) ->
    Handle<S>` returns a live handle to the actor mesh.
  - New `pub struct Handle<S = MutexLifo>` with
    `pub fn run(&self, window: Duration)` (just sleeps),
    `pub fn pool(&self) -> &Pool<S>` (caller injection /
    inspection), `pub fn stop(self) -> u64` (sends
    `Shutdown`, joins, sums per-thread counts — panics
    on actor panic).
  - `impl Drop for Handle` performs silent graceful
    shutdown if `stop` was not called: sends `Shutdown`,
    joins, swallows panics and counts. Drop must not
    panic-during-drop.
  - Module-level docstring: rewritten around the two API
    styles (Lifecycle preferred / All-in-one legacy).
  - Tests: `lifecycle_round_trip_returns_total`,
    `lifecycle_multiple_run_windows_accumulate`,
    `lifecycle_handle_pool_accessor`,
    `lifecycle_handle_drop_shuts_down_cleanly`. All four
    validate the drain invariant (`pool.free_len() ==
    pool.size()` after the handle goes away).
- `notes/todo.md`: mark `0.6.0-1` done.
- `notes/chores-03.md`: this section.

### Design notes

- **`Handle`'s state**: `senders: Option<Vec<Sender<...>>>`
  and `handles: Option<Vec<JoinHandle<u64>>>`. `stop`
  consumes via `take()`; `Drop` checks the same `Option`s
  and runs the silent path only if `stop` didn't.
  Simpler than typestate; KIS.
- **Counter aggregation**: each actor thread keeps a
  local `u64`, returned via `JoinHandle::Output`. `stop`
  sums them with `saturating_add`. No atomics in the hot
  path.
- **No mid-flight count read**: `Handle::run(window)`
  returns `()`. Per-window deltas would require atomics
  or signal-and-reply; both rejected as not worth the
  complexity at the user's KIS preference.
- **`spawn_and_seed` lives on `RuntimeZC`**, not as a
  free function, because it uses `self.pool` and is the
  natural place for the shared logic. Generic over `R`
  so both probe-free and probed actor loops share it.
- **Pool sizing in tests**: the lifecycle tests use the
  `Pool::new(., 4)` ping-pong-steady-state recommendation
  from the `0.5.0` chores (4 buffers covers 2 in flight
  + 2 headroom). One initial test draft used the
  capacity-2 pool from a non-running test and hit
  `NoMsgs` under traffic; standardized on 4.

### What's not in 0.6.0-1

- `goalzc` and `goalzc-crit` still call the legacy
  `run_probed` / `run` paths. `0.6.0-2` migrates them.
- Legacy `RuntimeZC::run` / `run_probed` /
  `actor_loop_probed` / the `ClearProbe` signal path stay
  untouched for now. `0.6.0-3` removes them.
