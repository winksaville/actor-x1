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
handle.run(duration);        // optional, repeatable
let total = handle.stop();   // u64 aggregate; sends Shutdown
```

### Multi-step ladder

(Revised mid-flight after `0.6.0-1`. Original ladder had
three sub-steps; expanded to five to make the trait
introduction and amortized-bench work explicit.)

- `0.6.0-0` — plan marker (this section + version bump).
- `0.6.0-1` — `Handle` + `RuntimeZC::startup` /
  `Handle::run` / `Handle::stop`; tests updated. Existing
  `run` / `run_probed` stay alongside (additive step).
- `0.6.0-2` — Migrate `goalzc` bin and `goalzc-crit` bench
  to the lifecycle API. New `Handle::reset_count` (sends
  `ClearProbe`; each actor thread zeros its per-thread
  count on receipt). `--warmup` restored on `goalzc` and
  in `goalzc-crit`'s warmup → measurement split. `goalzc`
  loses its band-table (no probing); keeps the msg/s line
  as a smoke generator.
- `0.6.0-3` — `Handle::query_count` via signal-and-reply
  (oneshot per thread). `goalzc-crit` goes amortized:
  `startup` once / `reset_count → run(duration) →
  query_count` per criterion sample. Reports the new
  amortized number; bench-vs-bin gap collapses (or
  doesn't, and we learn something).
- `0.6.0-4` — Introduce `Runtime` trait. `RuntimeZC` impls
  it. `goalzc` / `goalzc-crit` / tests reach through the
  trait, not the concrete type. `SignalZC::ClearProbe`
  renamed to `ClearCount` (probe is gone).
- `0.6.0-5` — Remove legacy `run` / `run_probed` /
  `actor_loop_probed`. Runtime is probe-free +
  lifecycle-only.
- `0.6.0` — close.

### Decisions locked in

- **Counter**: per-thread local `u64`, no atomics. Returned
  via `JoinHandle` at `stop`. `stop -> u64` aggregate. KIS;
  per-actor breakdown is a future `TProbe::Counter`
  concern.
- **`Handle::run(duration) -> ()`** — just sleeps for the
  run window. Counts not readable mid-flight by design.
- **`Handle::Drop`** calls `stop` if forgotten; result is
  discarded. Join errors are swallowed (drop must not
  panic-during-drop).
- **Probing leaves the runtime.** `goalzc`'s band-table
  goes with it. Probe re-introduction is post-`0.6.0`
  work, via `TProbe::Counter` or an actor-wrapper trait.
- **`Runtime` trait** introduced in `0.6.0-4`. Consumers
  reach through the trait so the post-`0.6.0`
  ActorManager-driven graceful shutdown work has a clean
  seam to drop into.

### Defaults that aren't yet locked

- `Handle::pool()` accessor so the caller can inject ad-hoc
  messages post-startup. Default yes; revisit during
  `0.6.0-1` if the API gets ugly.

### Naming: teardown → stop

The `0.5.0` close section's "What's next" pointer used
the name `teardown` for the lifecycle's shutdown
primitive. During the `0.6.0-0` planning round the name
settled on `stop` — matches the post-`0.6.0` typed-
message-to-runtime story. The chores-02 mention of
`teardown` is historical (accurate to the moment it was
written, pre-rename) and is left in place per the same
precedent `0.5.1` set with `run_no_probe → run`.

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

## Migrate goalzc bin and goalzc-crit bench to lifecycle (0.6.0-2)

`goalzc` and `goalzc-crit` move off the legacy all-in-one
`RuntimeZC::run_probed` / `run` to the lifecycle API
(`startup` / `Handle::run` / `Handle::stop`). Adds
`Handle::reset_count` so the warmup → measurement
boundary can zero per-thread counts mid-flight. `goalzc`
loses its band-table report (no probing in the runtime;
re-enters post-`0.6.0`) and becomes a smoke generator
that prints msg/s. `goalzc-crit` keeps its
cycle-per-sample shape, just through the new API.

- `crates/actor-x1/Cargo.toml`: `0.6.0-1` → `0.6.0-2`.
- `crates/actor-x1/src/runtime_zc.rs`:
  - New `pub fn Handle::reset_count(&self)` sends
    `SignalZC::ClearProbe` to every actor thread; on
    receipt each thread zeros its per-thread count.
    Best-effort: if a thread already exited, the send
    silently fails (its count is already captured in
    its `JoinHandle::Output`).
  - New test `lifecycle_reset_count_runs_clean` exercises
    `startup → run → reset_count → run → stop`; asserts
    nonzero post-reset total + drain invariant.
- `crates/actor-x1/src/bin/goalzc.rs`:
  - CLI: drop `--ticks` (`-t`), `--decimals` (`-d`).
    Keep `duration_s` (positional), `--warmup` (`-w`,
    default 0.5), `--size` (`-s`), `--pin` (`-p`).
  - Drop the apparatus calibration call and the
    per-thread `tprobe` band-table report.
  - Use `rt.startup` → `handle.run(warmup)` →
    `handle.reset_count()` → `handle.run(measurement)`
    → `handle.stop` to drive the actors.
  - Output: version banner, throughput line
    (`goalzc: <count> messages in <Ds> (<R> M msg/s,
    <N> actors, size=<S> B)`), and `pinning:` line if
    `--pin` was set. No `apparatus:` line, no band
    table.
  - Imports trimmed: `tprobe::{fmt_commas, pin}` only
    (was `tprobe::{self as perf, fmt_commas, pin,
    ticks}` plus probe-side report calls).
  - `n_actors = 2` hardcoded since the bin always
    creates two actors (assertion already pins
    `(a_id, b_id) == (0, 1)`).
- `crates/actor-x1/benches/goalzc-crit.rs`:
  - Switch `rt.run` → `rt.startup` /
    `handle.run(warmup)` / `handle.reset_count()` /
    `handle.run(measurement)` / `handle.stop`.
  - Keep the prior 50 ms warmup + 100 ms measurement
    split. `per_msg_ns = measurement / measurement_count`
    (post-reset only).
  - Module docstring rewritten around the lifecycle
    API and the smoke-bench scope.
- `notes/todo.md`: mark `0.6.0-2` done.
- `notes/chores-03.md`: this section.

### Smoke results (`--size 64`, unpinned)

- `goalzc-crit/pingpong`: ~260 K msg/s (criterion CI
  [250, 269] K elem/s; "Performance has improved",
  p=0.04 vs the pre-restoration single-window baseline
  at ~233 K). Restoring `--warmup` (50 ms) before the
  100 ms measurement window — and using `reset_count`
  to drop the warmup count — exposes the steady-state
  rate.
- `goalzc 10 -w 3`: ~293 K msg/s. Modest improvement vs
  the pre-`0.6.0` probed path (~281 K); the bot thinks
  this is the probe overhead leaving the hot loop —
  ~5.8 ns / dispatch ≈ 0.17% of the per-message cost —
  near the edge of run-to-run noise but pointing the
  right way.
- Bench-vs-bin ratio: 260 / 293 ≈ 0.89, narrower than
  the pre-restoration 232 / 281 ≈ 0.83. The remaining
  11% is the per-sample fresh-`startup` cost in the
  bench; `0.6.0-3`'s `startup`-once + `query_count`
  amortization aims at that.

### CLI breaking change in 0.6.0-2

`goalzc`'s `--ticks` / `--decimals` flags are gone (band-
table is gone). `--warmup` is preserved with the same
semantics it had pre-`0.6.0`: warmup window precedes
measurement, and reported throughput reflects only the
measurement window.

### Naming: window → duration

`Handle::run`'s parameter `window: Duration` renamed to
`duration: Duration` — matches `std::thread::sleep(dur)`
and `tokio::time::sleep(duration)` conventions, removes
the overloaded "window" reading from the public API. In
prose, bare "window" becomes "run window"; "warmup
window" / "measurement window" stay as already-qualified
compound terms. The `0.6.0-1` chores section above
describes the parameter as `window: Duration`, accurate
at the time, left in place per the same precedent
`0.5.1` set with `run_no_probe → run`.
