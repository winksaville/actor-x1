# Chores-02

Discussions and notes on various chores in github compatible markdown.
There is also a [todo.md](todo.md) file and it tracks tasks and in
general there should be a chore section for each task with the why
and how this task will be completed.

This file picks up at the `0.5.0` ladder (the zerocopy / pooled
`ActorZC` track). Earlier work — Stage 1 runtime, tprobe extraction,
band-table polish, criterion cross-check, and assorted future-work
notes — lives in [chores-01.md](chores-01.md).

See [Chores format](README.md#chores-format)

## goalzc + RuntimeZC: pooled zerocopy ping-pong — plan marker (0.5.0-0)

Adds a multi-threaded ping-pong workload whose messages are
pool-backed byte buffers, rather than the unit `Message` that
goal1 / goal2 exchange. Actors handle `&[u8]`; callers can
cast inside their handlers via the `zerocopy` crate if they
want typed views. A companion `goalzc-crit` criterion bench
measures exactly the same work, probe-free.

### Shape goalzz + RunteimZC

- `Pool` + `PooledMsg` — new `crates/actor-x1/src/pool.rs`.
  - Buffers are fixed-size (`MAX_SIZE`) `Vec<u8>` owned by `PooledMsg`.
  - `PooledMsg::Drop` returns the buffer to an `Arc<PoolInner>` shared across actor threads.
  - Inner storage: `Mutex<Vec<Vec<u8>>>` — simple, some contention, upgradeable to lock-free later.
  - `Deref<Target=[u8]>` so the handler's `&[u8]` is just `&*pooled`.
  - Lazy allocation: pool starts empty; grows to steady-state (2 buffers for ping-pong) on demand.
- `ActorZC` + `ContextZC` — new `crates/actor-x1/src/runtime_zc.rs`.
  - `trait ActorZC { fn handle_message(&mut self, ctx: &mut dyn ContextZC, msg: &[u8]); }`
  - `trait ContextZC { fn get_msg(&mut self, size: usize) -> PooledMsg; fn send(&mut self, dst_id: u32, msg: PooledMsg); }`
- `RuntimeZC` — same file.
  - 1:1 thread-per-actor, mirroring `MultiThreadRuntime`.
  - Channel payload is `Signal::User(PooledMsg)`.
  - Two entry points:
    - `run(warmup, measurement, pins) -> Vec<(u64, TProbe)>` — tprobe-instrumented; for `goalzc`.
    - `run_no_probe(warmup, measurement, pins) -> Vec<u64>` — probe-free; for `goalzc-crit`.
- `goalzc` binary — new `crates/actor-x1/src/bin/goalzc.rs`.
  - CLI mirrors `goal2`'s (`duration_s`, `--warmup`, `--ticks`, `--decimals`, `--pin`).
  - Adds `--size N` — message byte size; default 64; validated `1..=MAX_SIZE`.
  - Prints throughput + tprobe band-table per actor.
- `goalzc-crit` bench — new `crates/actor-x1/benches/goalzc-crit.rs`.
  - Criterion `iter_custom` over `RuntimeZC::run_no_probe`.
  - Reuses the `measurement * iters / total_count` scaling trick from `goal2-crit`.
  - `Throughput::Elements(1)`.

### Multi-step ladder goalzc + RuntimeZC

- `0.5.0-0` — Plan marker.
  - Bump `actor-x1` to `0.5.0-0`.
  - This chores section.
  - `notes/todo.md` entry under `## In Progress`.
  - `CLAUDE.md`: add "Notes writing style: prefer sub-bullets" section (captures in-session feedback).
  - No code change.
- `0.5.0-1` — `pool.rs`: `Pool` + `PooledMsg` with `Drop` return path.
  - Unit tests: round-trip, drop-returns, size enforcement.
- `0.5.0-2` — `actor_manager.rs` + `runtime_zc.rs`: `ActorZC` / `ContextZC` traits + `ActorManager` (catalog) in the first; `RuntimeZC` (transport) with `run` + `run_no_probe` in the second.
  - Unit tests: ping-pong handling, clean shutdown, pool drain on exit.
- `0.5.0-3` — `bin/goalzc.rs`.
  - Install + smoke-run.
- `0.5.0-4` — `benches/goalzc-crit.rs`.
  - `cargo bench` + spot-check throughput matches `goalzc`'s.
- `0.5.0` — Closing marker.
  - Drop `-N` suffix.
  - Update `crates/actor-x1/README.md` Binaries + Benches.
  - Move `notes/todo.md` entries to `## Done`.

### Design decisions recorded here

- Handler takes `&[u8]`; runtime owns the `PooledMsg`.
  - Runtime receives `Signal::User(PooledMsg)` off the channel.
  - Hands the handler `&*pooled` for the duration of `handle_message`.
  - Drops the `PooledMsg` after the handler returns; buffer goes back to the pool.
  - Pool traffic is one `get` + one `put` per dispatched message — exactly the cost goalzc exists to measure against goal2's unit-`Message` baseline.
  - Passing `PooledMsg` by value into the handler would let a clever actor reuse the incoming buffer (avoiding pool churn).
    - Rejected: `&[u8]` matches the shape of real zerocopy receivers and is the cleaner cross-validation target.
- Fixed-size buffers (`MAX_SIZE`) for the MVP.
  - Simpler pool (`Vec<Vec<u8>>`).
  - Matches a ping-pong workload where every message is the same size.
  - Size-classed sub-pools are future work; mentioned here explicitly so the extension isn't lost.
- Lazy pool allocation.
  - Pool starts empty; `get_msg` allocates a fresh `Vec<u8>` if the free list is empty.
  - Steady state settles to 2 buffers for ping-pong after the first couple of dispatches.
  - Pre-populating a fixed capacity is a micro-optimization that distorts first-dispatch latency (already dropped by warmup); deferred.
- `--size N` CLI flag, not a compile-time const.
  - goalzc is a benchmark; size sensitivity is exactly what a reader wants to sweep.
  - `N` is validated against `MAX_SIZE` at startup.
  - If `N > MAX_SIZE`, the binary errors cleanly rather than silently truncating.
- `RuntimeZC` is a parallel runtime, not a replacement.
  - `MultiThreadRuntime` stays untouched — existing `goal2` / `goal2-crit` keep working.
  - `RuntimeZC` lives in its own module; no shared code with `MultiThreadRuntime`.
  - Duplication is the price for a clean split that's easy to revisit once the shape stabilizes.
  - The goal2-crit probe-contamination bug stays un-patched in `MultiThreadRuntime` for now; `run_no_probe` exists here from the start instead.
- Multi-step `0.5.0` ladder.
  - Five natural checkpoints: pool, runtime, bin, bench, close.
  - Each reviewable on its own.
  - A single-step `0.5.0` would be a large diff to review in one pass.

## Pool + PooledMsg (0.5.0-1)

Implements the fixed-capacity buffer pool that `RuntimeZC`
will use to ship zerocopy messages between actors. The
shipped shape deviates from the `0.5.0-0` plan in several
ways — all decided during review of successive design cuts:

- Constructor takes explicit byte / count: `Pool::new(msg_size: u32, msg_count: u32)`. Pre-allocates `msg_count` buffers of `msg_size` bytes each; no lazy growth, no global `MAX_SIZE` const.
- `get_msg` returns `Result<PooledMsg<S>, PoolError>` rather than panicking. Two variants: `SizeTooLarge { requested, max }` and `NoMsgs`.
- Buffers are `Box<[u8]>` (fixed-size allocation) plus a separate `len: usize` on `PooledMsg` for the logical message length, instead of `Vec<u8>`'s redundant `cap` field.
- The free list lives behind a `BufRefStore` trait so different concurrency strategies (Mutex, lock-free queue, ring buffer, hand-rolled atomics, …) can be benchmarked against each other. The default is `MutexLifo`.
- `Pool` and `PooledMsg` are generic over `S: BufRefStore`, with `MutexLifo` as the default type — ordinary call sites write `Pool::new(64, 128)` unchanged.
- A `BufRef` type alias documents the `Option<Box<[u8]>>` niche-optimization idiom in one place.

### File-by-file

- `crates/actor-x1/Cargo.toml`: `0.5.0-0` → `0.5.0-1`.
- `crates/actor-x1/src/lib.rs`: `pub mod pool;` added before `pub mod runtime;`.
- `crates/actor-x1/src/pool.rs`: new module.
  - `pub type BufRef = Option<Box<[u8]>>;` — module-level alias documenting the niche-optimized "null or non-null pointer" pattern.
  - `pub enum PoolError { SizeTooLarge { requested, max }, NoMsgs }` — `Display` + `std::error::Error` impls.
  - `pub trait BufRefStore: Send + Sync` — `from_buffers`, `get`, `ret`, `len`, `size`, `is_empty` (default-impl over `len`).
  - `pub struct MutexLifo` — initial impl: `Mutex<Vec<Box<[u8]>>>` + cached `size: usize` for O(1) lock-free `size()`.
  - `pub struct Pool<S: BufRefStore = MutexLifo>` — `Arc<PoolInner<S>>` handle; manual `Clone` impl avoids spurious `S: Clone` bound.
    - `Pool::new(msg_size: u32, msg_count: u32)` — allocates buffers, calls `S::from_buffers`.
    - `Pool::msg_size() -> u32` — bytes per buffer.
    - `Pool::size() -> usize` — total buffer capacity (delegates to the store; will sum across sub-stores once multi-size pools land).
    - `Pool::get_msg(size: usize) -> Result<PooledMsg<S>, PoolError>` — validates `size`, pops via `S::get`, returns the wrapped buffer.
    - `Pool::free_len()` — `#[cfg(test)]` only.
  - `pub struct PooledMsg<S: BufRefStore = MutexLifo>` — `{ buf: BufRef, len: usize, pool: Arc<PoolInner<S>> }`.
    - `Deref<Target=[u8]>` / `DerefMut` — slice the underlying `Box<[u8]>` to `[..len]`.
    - `Drop` — `S::ret(self.buf.take().unwrap())` returns the buffer to the store.

### Tests added (all `#[cfg(test)]` in `pool.rs`, 15 total)

- `new_reports_msg_size_and_size` — fresh pool reports `msg_size`, `size`, and free-list `len`.
- `pool_size_is_constant_and_matches_store` — `size()` stays at `msg_count` while in-flight buffers reduce `len()`.
- `get_msg_returns_requested_size` — `len()` of the returned slice matches the requested size.
- `get_msg_at_msg_size_ok` — `size == msg_size` boundary works.
- `get_msg_zero_size_ok` — `size == 0` yields an empty slice.
- `get_msg_over_msg_size_returns_size_too_large` — error carries `requested` + `max` correctly.
- `get_msg_on_exhaustion_returns_no_msgs` — after all slots are in flight, `get_msg` yields `NoMsgs`.
- `zero_count_pool_always_no_msgs` — `Pool::new(_, 0)` always errors; sanity check for the zero edge.
- `drop_restores_slot_and_get_succeeds` — dropping an in-flight msg re-opens a slot.
- `buffer_is_reused_across_get_drop_get` — pointer equality survives a get / drop / get cycle (LIFO, no realloc).
- `clone_pool_shares_free_list` — drop on one `Pool` clone is visible from another.
- `writes_round_trip` — `DerefMut` writes are visible through `Deref`.
- `pool_works_across_threads` — `Pool` cloned to a worker thread; buffer dropped on worker returns to the shared free list.
- `pool_error_display` — both variants render readable one-liners.
- `mutex_lifo_trait_contract` — `BufRefStore` contract direct on `MutexLifo` without a `Pool` wrapper: `from_buffers` / `get` / `ret` / `len` / `size` round-trip end-to-end.

### Design decisions recorded here

- `BufRefStore` trait + generic `Pool<S>` rather than a hardcoded backend.
  - Primary motivation: the user wants to swap concurrency strategies and benchmark them — Mutex baseline today, lock-free `ArrayQueue`, ring buffers, hand-rolled atomics later.
  - Generic-with-default (`S: BufRefStore = MutexLifo`) keeps the common call site unchanged (`Pool::new(64, 128)`) while letting benchmarks opt into other backends with `Pool::<X>::new(64, 128)`.
  - Trait surface stays minimal: `from_buffers` / `get` / `ret` / `len` / `size`. No "ordering" or "is_lockfree" methods; ordering is implementation-defined and `Pool` treats buffers as fungible.
  - Dyn-dispatch via `Box<dyn BufRefStore>` was considered and rejected for benchmark fidelity — a `dyn` indirection in the hot path would contaminate the very thing we're trying to compare.
- `BufRef = Option<Box<[u8]>>` alias.
  - Documents the "safe null/non-null pointer" idiom in one place rather than re-explaining at each use site.
  - Used in `PooledMsg.buf` (nullable transiently during `Drop::drop`) and as `BufRefStore::get`'s return (signaling empty store).
- Fixed-capacity pre-allocation rather than lazy growth.
  - Supersedes the `0.5.0-0` plan's "starts empty, grows on demand" design.
  - Makes `NoMsgs` a real, testable condition — back-pressure is exposed instead of hidden behind silent allocation.
  - Steady-state behavior for ping-pong is unchanged (two buffers cycling).
- `Result<PooledMsg, PoolError>` rather than panic on misuse.
  - Supersedes the `0.5.0-0` plan's panic on `size > MAX_SIZE`.
  - `NoMsgs` is a runtime condition; a benchmark hitting exhaustion should not crash, it should report saturation.
  - `SizeTooLarge` follows the same shape for one error path.
- `Box<[u8]>` + separate `len` rather than `Vec<u8>`.
  - `Vec`'s `cap` field is dead weight when capacity is fixed at `msg_size`; `Box<[u8]>` is `(ptr, len)` only.
  - `Vec` also implies "growable", which is misleading here.
  - `len` lives on `PooledMsg` instead of in the store, so the store treats buffers as identical fixed-size allocations.
- `Option<Box<[u8]>>` inside `PooledMsg` so `Drop` can move the inner `Box` out.
  - Niche-optimized to a single pointer at runtime (no discriminant tag).
  - `None` only observed transiently inside `Drop::drop`; `Deref` / `DerefMut` can't see it.
  - `unsafe` alternatives (`ManuallyDrop` + a destructor) would shave the `unwrap`s but add unsafety; not worth it.
- `BufRefStore::size()` returns the constant store capacity.
  - Distinct from `len()` (current count), which fluctuates as buffers go in flight.
  - Cached at `from_buffers` time so `MutexLifo::size()` is O(1) lock-free.
  - Test `pool_size_is_constant_and_matches_store` validates the invariant.
  - Future-proofs the multi-size sub-pool extension: `Pool::size()` already sums "across sub-stores" (one for now).
- `msg_size` / `msg_count` typed as `u32`.
  - Matches the user's requested shape: explicit byte-range and count limits at the type level.
  - Internally converted to `usize` for `Vec` / slice APIs; widening on every platform actor-x1 targets.
  - `get_msg(size: usize)` stays `usize` for clean interop with slice/`Vec` lengths; if `size > u32::MAX` the `SizeTooLarge` guard catches it.
- `is_empty` as a default-impl trait method.
  - Required by clippy's `len_without_is_empty` lint when a type / trait exposes `len()`.
  - Default body `len() == 0` is correct for any conforming impl; concrete impls can override if they have a faster path.
- No zeroing on drop.
  - Slots `0..new_size` of a reused buffer carry residual bytes from the previous occupant.
  - For ping-pong workloads the next consumer either overwrites or ignores those bytes — benign.
  - Real applications crossing trust domains would need an explicit `zeroize`-on-drop step; documented in the module-level comment.
- No `Default for Pool`.
  - There's no sensible default for `msg_size` / `msg_count`; every caller has to choose.

## ActorManager + RuntimeZC: split catalog from transport (0.5.0-2)

Adds the multi-threaded zerocopy actor runtime, split across two
modules with disjoint responsibilities:

- `actor_manager.rs` — actor model surface. `ActorZC` / `ContextZC` traits + `ActorManager<S>` catalog. Defines *what* an actor is and how the catalog addresses them.
- `runtime_zc.rs` — message-moving infrastructure. `RuntimeZC<S>` with `run` and `run_no_probe` entry points. One consumer of the actor model surface above; future transports (M:N scheduler, alternative channels) would be peers.

The shipped shape deviates from the `0.5.0-0` plan in four ways,
all decided during review:

- Two types instead of one — `ActorManager<S>` (catalog) + `RuntimeZC<S>` (transport). The runtime moves messages; the manager holds instances and assigns ids; they only touch through `take_actors` + `probe_name_prefix` when `run` is called.
- Two files instead of one — actor model surface lives in `actor_manager.rs`, transport in `runtime_zc.rs`. Reinforces the type-level split at the file level so future growth on either side (control-plane manager-actors; alternative transports) lands cleanly.
- Direct instance registration rather than factory closures. `ActorZC: Send` is on the trait, so the factory pattern's "don't require Send" benefit is moot; `mgr.add(PingPong { peer: 1 })` reads cleaner than `mgr.add_actor(|| PingPong { peer: 1 })`.
- Initial messages live in the app, not the runtime. `run(&mut mgr, initial_messages, …)` takes them as a parameter; neither manager nor runtime stores them. The benchmark / app is what knows "ping-pong needs one seed message".

### Tests added (6 new total, 28 lib tests)

In `actor_manager::tests`:

- `take_actors_drains_and_resets` — after `take_actors`, the manager is empty and follow-up `add` restarts ids from 0.
- `manager_exposes_probe_name_prefix` — verifies the prefix accessor; full `"<prefix>.actor<id>"` composition is verified end-to-end via `goalzc`'s output later (TProbe's name is private).

In `runtime_zc::tests`:

- `ping_pong_runs_and_shuts_down` — two-actor ping-pong drives traffic for 40 ms each phase; both actors record nonzero counts.
- `pool_is_full_after_shutdown` — after `run` returns, every buffer that was ever in flight has come back; `pool.free_len() == pool.size()`.
- `run_no_probe_returns_counts` — probe-clean path runs the same workload, returns counts only, drains the pool the same way.
- `runtime_pool_accessor_returns_same_handle` — `RuntimeZC::pool()` exposes the caller's handle (verifies `msg_size` / `size`).

### Design decisions recorded here

- Generic `ActorZC<S>` / `ContextZC<S>` over the store backend.
  - Pool is generic over `S` since `0.5.0-1`; static dispatch all the way down preserves benchmark fidelity.
  - Type-erasure in `ContextZC::send` would add a vtable hop per send, contaminating the measurement.
  - Default `S = MutexLifo` keeps ergonomic call sites unannotated.
- Handler takes `msg: &[u8]`; runtime owns the inbound `PooledMsg<S>`.
  - The runtime's `actor_loop` calls `actor.handle_message(&mut ctx, &msg)` then `drop(msg)` — buffer returns to the pool one channel hop later, predictably and once per dispatch.
- Two run methods rather than one with a `bool probe_enabled` parameter.
  - A runtime branch on every dispatch would itself be a cost goalzc might want to measure or eliminate.
  - Function-pointer dispatch (`ActorLoopFn`) monomorphizes cleanly per `R`.
- Caller-supplied `Pool<S>`.
  - Lets the app build initial messages from the same pool the runtime uses.
  - Lets tests inspect `free_len` after shutdown to verify drain.
  - Lets `goalzc-crit` keep the pool across `iter_custom` calls (long-lived) while the runtime is rebuilt each call.
- `Send` bound on `ActorZC` (not `ContextZC`).
  - Actors get moved into spawned threads; must be `Send`.
  - `ContextZC` lives only inside one thread (constructed per-recv inside `actor_loop`); no `Send` needed.
- `let _ = sender.send(...)` in `MultiCtxZC::send` — closed channel drops the message silently.
  - The `Err(SendError(SignalZC::User(msg)))` case drops the `SignalZC`, which drops the inner `PooledMsg`, which returns the buffer to the pool — no leak.
- Drain assertion as part of the test contract.
  - `pool_is_full_after_shutdown` codifies "every PooledMsg that was ever in flight returns to the pool by the time `run` joins".
  - Non-obvious because it depends on `Drop for PooledMsg`, mpsc receivers dropping queued messages on close, and main-thread senders being dropped before join.
  - Codifying it now means a future change that breaks one of those steps gets caught immediately.

### Future direction (noted, not built today)

- `ActorManager` itself becomes an `ActorZC` for control-plane work — a sibling actor on the same Runtime that handles "register a new actor at runtime", "lookup by name", "drain probes", etc.
- Multiple manager-actors for separate concerns (GUI, CLI, log drain, probe collect) all running on one Runtime.
- Discovery beyond `u32` ids — name-based, type-indexed.
- Closure-based init for richer post-spawn / pre-warmup work (`init: FnOnce(&Mailboxes<S>)`) if a workload ever needs more than a static `initial_messages` list.
- `add_with_factory` escape hatch if a `!Send`-during-construction actor type ever appears.

## bin/goalzc (0.5.0-3)

Adds the `goalzc` binary — two zerocopy ping-pong actors on
their own threads, exchanging pool-backed `PooledMsg`s of
configurable size. Mirrors `goal2`'s CLI shape and output;
swaps in `RuntimeZC` + `ActorManager` + `Pool` for the
unit-`Message` runtime, and adds `--size N` for payload-byte
sweeping.

### File-by-file

- `crates/actor-x1/Cargo.toml`: `0.5.0-2` → `0.5.0-3`. No
  `[[bin]]` entry needed — the file lives under `src/bin/`
  and Cargo auto-discovers it.
- `crates/actor-x1/src/bin/goalzc.rs`: new binary.
  - `PingPongZC { peer_id }` actor — implements
    `ActorZC<S>` for any `S: BufRefStore`. On each inbound
    `&[u8]`, fabricates a same-sized reply via
    `ctx.get_msg(msg.len())` and sends it to `peer_id`. The
    inbound `PooledMsg` is dropped by the runtime after the
    handler returns, returning its buffer to the pool — so
    each dispatch costs one pool `get` + one pool `put`.
  - `Cli` mirrors `goal2`'s flags (`duration_s` positional,
    `--warmup`, `--ticks`, `--decimals`, `--pin`) and adds
    `--size N` (`u32`, default 64).
  - `parse_positive_size` value parser rejects `0`. The
    upper bound on `--size` is auto-satisfied:
    `Pool::new(size, 4)` builds a pool whose `msg_size`
    equals `--size`, so `get_msg(size)` always passes the
    `SizeTooLarge` check inside the handler by construction.
  - Pool sized at `Pool::new(cli.size, 4)` — 2 buffers in
    flight for ping-pong steady state, plus 2-buffer headroom
    so a wakeup race never starves.
  - Runs `RuntimeZC::run` (probe-instrumented). Output line
    adds `size=<N> B` next to the `2 actors, inner=1` summary.

### Smoke results (1 s, --size 64)

- `goalzc 1 --size 64`: 0.278 M msg/s.
- `goal2 1 --pin 0,1`: 0.354 M msg/s.
- Ratio ~0.79 — goalzc pays one pool `get` + one drop-back
  per dispatch on top of goal2's unit-`Message` baseline.
- The bot thinks the ~21% slowdown lines up with the per-
  dispatch `MutexLifo` lock pair plus `Box<[u8]>` zero-init
  on `get`; `goalzc-crit` (0.5.0-4) will refine the
  comparison without probe contamination.

### Design decisions recorded here

- `--size` validated in the value parser, not after
  `Pool::new`.
  - Catches misuse (`--size 0`) before any runtime work
    happens, with a clean clap error message.
  - The hand-off note's "is `--size` ≤ `pool.msg_size()`?"
    bound is satisfied by construction (`Pool::new(size, _)`
    sets `msg_size == size`); no runtime check needed.
- No `--pool-count` flag.
  - Hard-coded `4` covers ping-pong (steady state 2) with
    headroom. Future workloads with deeper queues will need
    a knob; deferred.
- `PingPongZC` is duplicated from `runtime_zc::tests::PingPong`
  rather than promoted to a shared `pub` type.
  - Both copies are five lines; the test version is intentionally
    minimal so refactors of the test workload don't ripple into
    the bin (and vice versa).
  - Promotion to `crate::actors::PingPongZC` is a future tweak —
    the noted choice in the hand-off section.
- Output format mirrors `goal2`'s exactly, plus `size=<N> B`.
  - Keeps cross-binary comparison readable; same band-table
    report shape for `tprobe` per actor.

## benches/goalzc-crit (0.5.0-4)

Adds `goalzc-crit`, a criterion smoke bench for `goalzc`.
Drives the same two-thread two-actor zerocopy ping-pong
workload via `RuntimeZC::run_no_probe` so the measurement
is probe-clean. Mirrors `goal2-crit`'s shape (`iter_custom`,
fresh runtime per sample, scale to iters).

Scope. `RuntimeZC` is structurally `goal2` with a swapped
payload — same threads, same `mpsc`, same warmup / measure /
shutdown flow — so the runtime style is already validated
by `goal2-crit`. `goalzc-crit`'s job is regression insurance
for the pool-backed payload path ("does it still spin up,
send, and tear down?"), not fidelity-perfect agreement with
the `goalzc` binary. The bench-vs-bin gap question gets
answered for free when the lifecycle refactor lands (planned
0.6.0): warmed long-lived threads + per-sample `run` window
collapses the gap structurally.

### File-by-file

- `crates/actor-x1/Cargo.toml`: `0.5.0-3` → `0.5.0-4`; new
  `[[bench]] name = "goalzc-crit", harness = false` entry.
- `crates/actor-x1/benches/goalzc-crit.rs`: new bench.
  - Bench-local `PingPongZC { peer_id }` actor — a copy of the
    `goalzc` bin's actor. Same five-line `handle_message` body
    (`get_msg(msg.len())` then `ctx.send`).
  - `bench_goalzc` builds a fresh `Pool::new(64, 4)` /
    `RuntimeZC` / `ActorManager` per `iter_custom` call,
    seeds `actor 0`, runs 50 ms warmup + 100 ms measurement
    via `run_no_probe`, sums per-actor counts, and scales the
    reported duration to `measurement * iters / total_count`.
  - `Throughput::Elements(1)` — criterion's throughput row
    reads as msgs / s. Single fixed `SIZE = 64`; size-sweeping
    is a future tweak.
  - 10 s `measurement_time`, 50 samples — same dial as
    `goal2-crit`.

### Smoke results (`--size 64`, unpinned)

- `goalzc-crit/pingpong`: median ~232 K msg/s, criterion CI
  [222, 246] K msg/s. Spins up, ping-pongs, tears down across
  50 samples without panic — that's the bar this bench is
  meeting.

### Design decisions recorded here

- `RuntimeZC::run_no_probe`, not `run`.
  - Direct lesson from `goal2-crit`'s probe contamination —
    the probe-free path exists for exactly this caller.
  - `run` would secretly measure `work + tprobe` instead of
    `work` alone; numbers would be inflated and not
    comparable to `goalzc`'s `adj mean min-p99`.
- Single fixed `--size` (64) rather than a sweep.
  - Matches `goal2-crit`'s "one workload, one number" shape.
  - Sweeping sizes via `BenchmarkId` + `Throughput::Bytes(size)`
    is a future tweak — useful once we have a reason to
    compare per-byte throughput across sizes.
- Bench-local `PingPongZC` instead of importing the bin's.
  - The bin's actor is in `src/bin/goalzc.rs`; binaries
    aren't importable as a library by other crate targets,
    so a copy is the cleanest path.
  - Five lines of duplication; the alternative
    (`pub mod actors`) is the future tweak the hand-off
    section flagged.

## Hand-off — closing the 0.5.0 ladder (post 0.5.0-2)

Picks up where 0.5.0-2 leaves off; captures the as-built
API surface and the remaining steps. The 0.5.0-0 "Shape"
subsection above is the *original plan* — `0.5.0-1` and
`0.5.0-2` deviated from it (see those sections), so this
hand-off is the canonical starting point for the next
session rather than the plan-marker shape.

### Status

- `0.5.0-0` — plan marker. Done & committed.
- `0.5.0-1` — `Pool` + `PooledMsg` + `BufRefStore` + `MutexLifo`. Done & committed.
- `0.5.0-2` — `actor_manager.rs` (catalog + traits) + `runtime_zc.rs` (transport). Done; awaiting commit at hand-off time.
- `0.5.0-3` — `bin/goalzc.rs`. Done & committed.
- `0.5.0-4` — `benches/goalzc-crit.rs`. Done & committed.
- `0.5.0` — closing marker. See `## goalzc + RuntimeZC: pooled zerocopy ping-pong (0.5.0)` below.

### As-built API surface (deviations from `0.5.0-0` baked in)

- `Pool::new(msg_size: u32, msg_count: u32) -> Pool<MutexLifo>` — pre-allocates `msg_count` `Box<[u8]>` of `msg_size` bytes; no lazy growth, no `MAX_SIZE` const.
- `pool.get_msg(size: usize) -> Result<PooledMsg<S>, PoolError>` where `PoolError ∈ { SizeTooLarge { requested, max }, NoMsgs }`.
- `pool.msg_size() -> u32`, `pool.size() -> usize` — use `msg_size()` for `--size` validation; `NoMsgs` is the saturation signal at runtime.
- `crates/actor-x1/src/actor_manager.rs`:
  - `trait ActorZC<S = MutexLifo>: Send` — `fn handle_message(&mut self, ctx: &mut dyn ContextZC<S>, msg: &[u8])`.
  - `trait ContextZC<S = MutexLifo>` — `get_msg(size) -> Result<PooledMsg<S>, PoolError>`, `send(dst_id: u32, msg: PooledMsg<S>)`.
  - `ActorManager::new(prefix: &str)` / `add<A: ActorZC<S> + 'static>(actor: A) -> u32` / `take_actors() -> Vec<…>` / `probe_name_prefix() -> &str`.
- `crates/actor-x1/src/runtime_zc.rs`:
  - `RuntimeZC::new(pool: Pool<S>) -> Self` / `pool() -> &Pool<S>`.
  - `run(&mut self, mgr, initial_messages: Vec<(u32, PooledMsg<S>)>, warmup, measurement, pin_cores: &[usize]) -> Vec<(u64, TProbe)>`.
  - `run_no_probe(...) -> Vec<u64>` — same orchestration, probe-free hot loop.
- App pattern (drives both `goalzc` and `goalzc-crit`):
  ```rust
  let pool = Pool::new(size, 4);                    // 4 buffers ≫ ping-pong steady state of 2
  let mut rt  = RuntimeZC::new(pool.clone());
  let mut mgr = ActorManager::new("goalzc.dispatch");
  let a = mgr.add(PingPong { peer: 1 });
  let _ = mgr.add(PingPong { peer: 0 });
  let initial = vec![(a, pool.get_msg(size).expect("seed"))];
  let results = rt.run(&mut mgr, initial, warmup, measurement, &pins);
  ```

### 0.5.0-3 — `bin/goalzc.rs`

- CLI mirrors `goal2`'s positional / flags: `duration_s` (positional), `--warmup`, `--ticks`, `--decimals`, `--pin`.
- New flag: `--size N` (`u32`, default 64). Validate `1 ≤ N ≤ pool.msg_size()` at startup; error cleanly if violated. The pool is constructed at runtime, so this is a runtime check, not a const.
- Pool sizing: 4 buffers covers ping-pong (steady state is 2). A `--pool-count` flag is a future tweak, not needed for 0.5.0.
- Use `RuntimeZC::run` (probe-instrumented).
- Output: throughput summary (use `tprobe::fmt::commafmt` or similar) + per-actor band tables. Crib pattern from `crates/actor-x1/src/bin/goal2.rs`.
- Doc comments on `#[arg]` fields with bullet lists need `#[arg(verbatim_doc_comment, ...)]` per the CLAUDE.md clap caveat.
- Smoke-test post-install: `goalzc 1 --size 64`, `goalzc 1 --size 64 --pin 0,1`.

### 0.5.0-4 — `benches/goalzc-crit.rs`

- Pattern: `iter_custom`, fresh runtime per sample, scaled by `measurement * iters / total_count`. Lift directly from `crates/actor-x1/benches/goal2-crit.rs`.
- **Use `RuntimeZC::run_no_probe`, not `run`.** The lesson from `goal2-crit`'s probe contamination is the entire reason `run_no_probe` exists; calling `run` here would re-introduce the same bug.
- Add `[[bench]]` entry to `crates/actor-x1/Cargo.toml`:
  ```
  [[bench]]
  name = "goalzc-crit"
  harness = false
  ```
- Default to `--size 64`. Sweeping sizes is optional — `Throughput::Bytes(size as u64)` if size-sweeping; `Throughput::Elements(1)` for fixed-size.
- Cross-check: run `goalzc` and `goalzc-crit` at the same size; throughput should agree within the goal2-vs-goal2-crit window (~5–10%; `goal2`/`goal2-crit` reported 228 K vs 240 K msg/s during 0.4.0 development).

### 0.5.0 — close

- Bump `crates/actor-x1/Cargo.toml` from `0.5.0-N` → `0.5.0`.
- Update `crates/actor-x1/README.md`:
  - Binaries section: add `goalzc` (with `--size` description).
  - Benches section: add `goalzc-crit`.
- `notes/todo.md`: move the In-Progress `[20]` entry to `## Done` with reference `[20]`.
- Final `vc-x1 push main` to land the closing commit.

### Gotchas / non-obvious things

- **Drain invariant**: `pool.free_len() == pool.size()` must hold after every `run` / `run_no_probe` returns. Tests `pool_is_full_after_shutdown` and `run_no_probe_returns_counts` codify this; don't break them.
- **Probe contamination**: never call `RuntimeZC::run` from a criterion bench. Use `run_no_probe`.
- **`--size` validation** hits `pool.msg_size()`, not a const. The bound is set when the pool is built; the CLI pool gets built from the `--size` flag, so the validation is "is `--size` ≤ `pool.msg_size()` after pool construction" — easiest is `Pool::new(size, 4)` then nothing further to validate, since by construction `pool.msg_size() == size`. The validation matters more when sizes are runtime-variable (future multi-size sub-pools).
- **Bullet doc comments on `#[arg]` fields → reflowed to prose by clap unless `verbatim_doc_comment` is set.** See `vc-x1/src/init.rs` for worked examples; the CLAUDE.md "Writing style: prefer sub-bullets" section calls this out.
- **Initial messages are the app's responsibility** — neither manager nor runtime stores them. `goalzc` / `goalzc-crit` build the seed `PooledMsg` from the same `pool` they pass to `RuntimeZC::new`.
- **`PingPong` test struct**: lives in `runtime_zc::tests`. `goalzc` / `goalzc-crit` will need their own copy (it's small) or a shared `pub` version under `crate::actors` — leaving as a duplication choice for the next session.

## goalzc + RuntimeZC: pooled zerocopy ping-pong (0.5.0)

Closes the `0.5.0` ladder. The `-N` suffix drops; the workspace
ships `Pool` / `ActorManager` / `RuntimeZC`, the `goalzc` bin,
and the `goalzc-crit` smoke bench. No behavior change vs
`0.5.0-4` — closing marker plus README refresh.

- `crates/actor-x1/Cargo.toml`: `0.5.0-4` → `0.5.0`.
- `crates/actor-x1/README.md`:
  - Binaries: add `goalzc` (zerocopy, `--size N`).
  - Benches: add `goalzc-crit`. Smoke scope; gap collapses
    with the `0.6.0` lifecycle refactor.
- `notes/todo.md`: collapse the In-Progress `0.5.0` ladder
  entry into one `## Done` line with refs
  `[[20]],[[21]],[[22]]`; `In Progress` becomes `(none)`.
- `notes/chores-02.md`: this section.

### The 0.5.0 ladder at a glance

See the `(0.5.0-N)` sections earlier in this file —
`-0` plan marker, `-1` `Pool`, `-2` `ActorManager` +
`RuntimeZC`, `-3` `goalzc` bin, `-4` `goalzc-crit` bench.

### What ships in 0.5.0

- `Pool<S: BufRefStore>` — fixed-capacity `Box<[u8]>` pool,
  default `S = MutexLifo`. `get_msg` returns
  `Result<PooledMsg, PoolError>`; `Drop` returns the buffer.
- `ActorManager<S>` — catalog of zerocopy actors. `add()`
  returns the id; `take_actors()` drains for the runtime.
- `RuntimeZC<S>` — multi-threaded transport. One thread per
  actor, per-actor `mpsc`, warmup / measure / shutdown
  lifecycle. Entry points: `run` (probe) and `run_no_probe`
  (clean).
- `goalzc` bin — two-thread zerocopy ping-pong, `--size N`
  (default 64), mirrors `goal2`'s flags.
- `goalzc-crit` bench — criterion smoke harness via
  `run_no_probe`.

### What's next

- `0.6.0-0` — lifecycle refactor on `RuntimeZC`: split
  `run` into `startup` / `run` / `teardown` so spawn /
  warmup amortize across multiple `run` windows.
- The bot thinks the split also closes the `goalzc` /
  `goalzc-crit` ~17.5% gap as a side effect.
