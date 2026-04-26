# actor-x1

An experiment in the actor model in Rust (Communicating
Sequential Processes, Hoare 1978). Part of the
[actor-x1 workspace](../..); see the workspace
[README](../../README.md) for overview, usage, and
contribution conventions.

## Binaries

- **`goal1`** — two actors on one thread, ping-ponging an
  empty message through a shared `VecDeque`.
- **`goal2`** — two actors on two threads, ping-ponging
  over `std::sync::mpsc` channels.
- **`goalzc`** — two zerocopy actors on two threads,
  ping-ponging pool-backed `PooledMsg` buffers (handler view:
  `&[u8]`) over `std::sync::mpsc`. Adds `--size N` (default
  64) to sweep payload size; the pool is built at startup so
  every `get_msg(size)` call inside the handler clears the
  bound by construction.

Install from the workspace root:

```
cargo install --path crates/actor-x1
```

## Benches

Criterion cross-validation / smoke benches for the goal1 /
goal2 / goalzc workloads. Run from this crate directory:

```
cargo bench --bench goal1-crit
cargo bench --bench goal2-crit
cargo bench --bench goalzc-crit
```

- `goal1-crit` sweeps `inner ∈ {1, 100, 1000}` to mirror
  goal1's `--inner` knob.
- `goal2-crit` runs a single two-thread mpsc ping-pong shape.
- `goalzc-crit` runs the zerocopy ping-pong shape via
  `RuntimeZC::run_no_probe` so the measurement is
  probe-clean. Smoke-bench scope: regression insurance for
  the pool-backed payload path, not fidelity-perfect
  agreement with the `goalzc` binary (the bench's
  fresh-runtime-per-sample shape never reaches `goalzc`'s
  long-lived steady state — collapses out when the lifecycle
  refactor lands in 0.6.0).

Compare criterion's per-message time and throughput against
the corresponding bin's tprobe `mean min-p99` / `adj mean
min-p99` / `M msg/s` at the same config.

## Notes

- [`notes/design.md`](notes/design.md) — staged actor-model
  design. Stage 1 shipped at `0.1.0`; Stage 2 is next.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR
[Apache-2.0](LICENSE-APACHE).
