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
