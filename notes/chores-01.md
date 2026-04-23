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

## Vendored perf probe + Goal1 instrumentation (0.1.0-2)

Copies `tprobe2` and its transitive dependencies into the crate and
wraps every `handle_message` dispatch with a probe scope so Goal1
reports per-dispatch latency alongside message count.

- `Cargo.toml`: version bump to 0.1.0-2; add `hdrhistogram = "7"` and
  `minstant = "0.1"` dependencies.
- `src/perf/mod.rs`: new module root; documents vendor source path
  (`../iiac-perf/src`) and divergences (import paths, `// OK: …`
  on `unwrap*`, one diagnostic string).
- `src/perf/tprobe2.rs`: vendored from `iiac-perf/src/tprobe2.rs`.
  Imports rewritten to `crate::perf::{band_table, ticks}`; two
  `unwrap` calls gained `// OK: …` justifications.
- `src/perf/band_table.rs`: vendored from `iiac-perf/src/band_table.rs`.
  Imports rewritten to `crate::perf::fmt` and `crate::perf::ticks`;
  `unwrap_or(0)` calls gained `// OK: …` justifications.
- `src/perf/ticks.rs`: vendored unchanged except for the
  `compile_error!` diagnostic now mentions `actor-x1 probes`.
- `src/perf/ticks/x86_64.rs`: vendored unchanged.
- `src/perf/fmt.rs`: extracted `fmt_commas` and `fmt_commas_f64`
  from `iiac-perf/src/harness.rs` (lines 160–188) — avoids vendoring
  the 393-line harness for two helpers. `unwrap_or(0)` gained a
  `// OK: …` justification.
- `src/lib.rs`: adds `pub mod perf;`.
- `src/runtime.rs`: `SingleThreadRuntime::new(probe_name)` now takes
  a name, embeds a `TProbe2`, and wraps each `handle_message` call
  with `probe.start(dst_id as u64)` / `probe.end(id)`. `Default` impl
  dropped (new takes an arg). `probe_mut()` exposes the probe for
  end-of-run reporting. Unit tests pass a probe name.
- `src/bin/goal1.rs`: new arg parser supports `-t` / `--ticks`
  (raw-ticks display); runtime constructed as `"goal1.dispatch"`;
  calls `rt.probe_mut().report(as_ticks)` after the dispatch loop.
  Usage errors now `exit(2)` with a usage line instead of panicking.

### Smoke measurements (release build)

- `goal1 0.5` — 10.56 M messages in 0.5s (~21.1 M msg/s).
  Per-dispatch: mean 9 ns, mean min-p99 9 ns, p99 max 67.5 µs
  (likely an OS interrupt in the tail).
- `goal1 0.5 -t` — 10.57 M messages in 0.5s (~21.1 M msg/s).
  Per-dispatch: mean 35 tk, mean min-p99 34 tk, p99 max 114,687 tk.

Adding the probe cuts throughput roughly in half vs. 0.1.0-1
(~38.7 → ~21.1 M msg/s). The bot thinks the delta is dominated by
two `rdtsc` reads plus `Vec::push` per dispatch (~20 ns of the
~47 ns per-dispatch cost); the 9 ns measured by the probe is the
`handle_message` body alone.

## Inner batching, warmup, clap (0.1.0-3)

Adds CLI polish and measurement-quality improvements to Goal1.
Scope:

- **Inner batching** — one probe scope now covers `--inner N`
  consecutive dispatches (default 1). At larger `N` the probe's
  two-`rdtsc`-plus-push apparatus cost is amortized, and stored
  tick values land in a range where the histogram's 0.1 %-relative
  buckets actually resolve jitter.
- **Warmup** — `--warmup SECS` (default **10.0**) runs the same
  dispatch loop with the probe active, then clears probe state;
  measurement phase inherits stable-state CPU freq / cache /
  branch-predictor conditions. Warmup message counts are never
  reported.
- **clap** — hand-rolled arg parsing replaced with a derive-based
  `Cli`. Validates non-negative finite `duration_s` and `warmup`;
  enforces `inner ≥ 1`. Auto-generated `--help` / `--version`.
- **TProbe2 additions** (vendored-code divergence from iiac-perf):
  - `Record` gains a `batch: u64` field.
  - `TProbe2::end_batch(id, n)` appends with `batch = n`; existing
    `end(id)` sets `batch = 1`.
  - `TProbe2::report` now divides stored deltas by `batch` at
    drain time so the histogram always holds per-event cost,
    regardless of scope granularity.
  - `TProbe2::clear()` resets records buffer and histogram;
    called between warmup and measurement.

File-level edits:

- `Cargo.toml`: bump to 0.1.0-3; add
  `clap = { version = "4", features = ["derive", "wrap_help"] }`.
- `src/perf/tprobe2.rs`: `Record.batch`, `end_batch`, `clear`,
  per-event division in `report`. Doc comment at file head lists
  divergences from upstream. Three new tests
  (`end_batch_stores_batch_size`, `report_divides_batched_delta`,
  `clear_empties_records_and_hist`).
- `src/runtime.rs`: `run_for(duration, inner)` wraps each batch
  of `inner` dispatches in a probe scope; site_id is the first
  `dst_id` in the batch. Partial trailing batches (queue empties
  mid-scope) are discarded — scope dropped, count not incremented
  — so throughput numbers reflect whole-batch work only. New test
  `partial_trailing_batch_is_dropped` covers the drop path.
- `src/bin/goal1.rs`: clap-derive `Cli` with `duration_s`,
  `--warmup`, `--inner`, `-t/--ticks`. Runs warmup, clears probe,
  runs measurement, prints count/throughput (now also reports
  `inner=N`), then calls `probe.report`.

### Smoke measurements (release, `--warmup 0`)

| Args | Throughput | Probe mean (min-p99) | Notes |
|---|---|---|---|
| `0.5` (inner=1) | 19.7 M msg/s | 9 ns | Apparatus overhead dominates, distribution stuck in two spike buckets |
| `0.5 --inner 1000` | 205 M msg/s | 5 ns | ~10× throughput; per-dispatch mean drops because apparatus is amortized |
| `0.5 --inner 1000 -t` | 204 M msg/s | 18 tk | Ticks view; tail p99-max 449 tk vs ~115 k tk at inner=1 (batch smooths OS interrupts) |

Warmup wall-clock sanity: `goal1 0.3 --warmup 1 --inner 1000` ran
in 1.33s total. Argv error paths (`-1`, `--inner 0`, `--warmup nan`)
all exit 2 with clap-formatted messages.

The bot thinks the inner=1 vs inner=1000 gap (9 ns vs 5 ns
per-event) is ~4 ns of apparatus overhead; 0.1.0-5's overhead
calibration will confirm that estimate directly.

## Always render all 12 percentile bands (0.1.0-4)

Previously the renderer skipped any band where no hdrhistogram
bucket was assigned by mid-rank (`if band_count[i] == 0 { continue;
}`), so tight distributions produced reports with only 3–5 visible
rows and it was hard to tell at a glance whether a band was
genuinely empty or just not rendered. This step always emits all
12 rows (`min-p1` through `p99-max`); empty bands render as zeros
in every numeric column.

File-level edits:

- `Cargo.toml`: bump to 0.1.0-4.
- `src/perf/band_table.rs`: drop the skip-on-empty guard; the band
  loop now computes `(first_v, last_v, range_v, mean_v)` from
  either the band aggregates (for non-empty bands) or `(0, 0, 0, 0)`
  for empty ones. `unwrap_or(0)` `// OK: …` justifications updated
  ("rows always contains n_bands=12 entries").

### Display example (inner=10, 0.5 s)

All 12 rows always visible — the zero-valued bands make the
distribution's shape explicit:

```
    min-p1        4 ns        4 ns        0 ns        8,712     4 ns
    p1-p10        0 ns        0 ns        0 ns            0     0 ns
    ...
    p40-p50       5 ns        5 ns        0 ns    4,794,952     5 ns
    ...
    p99-max       7 ns    3,197 ns    3,190 ns       61,924    10 ns
```

### Caveat — this only fixes visibility, not the bucket-into-band
mapping

The renderer still walks hdrhistogram buckets and assigns each
bucket to exactly one band by cumulative mid-rank. In the inner=10
smoke above, 4,794,952 samples (~89 %) land in `p40-p50` while
`p1-p10` through `p30-p40` stay at zero — the single hot bucket at
"5 ns" got attributed to one band, not spread across the percentile
range its samples actually span. The bot thinks the principled
fix (switch to `hist.value_at_quantile(q)` per band boundary) is
worth considering if the zero-band display starts feeling
misleading; deferred for now.

### Divergence from upstream

Further modification of the vendored `src/perf/band_table.rs`.
Noted in the perf module's divergence list; already has other
actor-x1-specific edits (`// OK:` comments, import paths).

## CLI polish: help width + version banner (0.1.0-5)

Two small quality-of-life tweaks to the `goal1` CLI.

### Help width cap at 80 cols

clap's default help-width uses the terminal width, which on wide
terminals produces hard-to-scan long lines for the `--warmup` and
`--inner` descriptions. `max_term_width = 80` caps rendering at
80 columns while still using terminal width if smaller.

### Version banner

First line of `goal1` output — and of `goal1 -h` — is now
`{CARGO_PKG_NAME} {CARGO_PKG_VERSION}` (e.g. `actor-x1 0.1.0-5`),
matching what `--version` already prints. Helps identify which
build produced a given result sheet (or which help text a user is
reading) when lines are pasted out of context. The existing
`goal1: …` line below carries the binary identity, so the banner
doesn't repeat "goal1". Implemented in two places: a `println!`
at the top of `main` for normal runs, and clap's `before_help`
attribute for help output (`concat!(env!(CARGO_PKG_NAME), " ",
env!(CARGO_PKG_VERSION))` — compile-time concat of package env
vars).

### File-level edits

- `Cargo.toml`: bump to 0.1.0-5.
- `src/bin/goal1.rs`: extend the `Cli` struct's `#[command(...)]`
  attribute with `max_term_width = 80` and
  `before_help = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"))`;
  `println!` the same banner at the top of `main` for
  non-help runs.

## Apparatus overhead calibration (0.1.0-6)

Adds a calibration phase between warmup and measurement that fits
the probe's framing cost (the bias on every stored
`end_tsc − start_tsc` delta) and subtracts the appropriate per-
event correction at report time. `--raw` opts out.

### Why two-point (not single-point)

`rdtsc` isn't a serializing instruction, so two back-to-back reads
with nothing between can execute out-of-order and measure ~0 ticks
— below the true apparatus cost. `_mm_lfence` bracketing drains
the pipeline harder than real work does (measured 38 tk on this
box, 2× the real bias). The two-point fit gives the right answer
by varying `inner` on an empty bench and solving
`raw_delta = framing + inner · loop_per_iter` in `f64` (no
integer-division loss at small values).

On this box the fit lands on **framing = 21 tk (~5.5 ns)**,
reproducible across runs. With the correction applied,
`inner=1` probe mean drops from 9 ns (raw) to 4 ns, which matches
the `inner=1000` raw mean of 5 ns (where the correction is
negligible) — both phases converge on the same actual
`handle_message` cost.

### File-level edits

- `Cargo.toml`: bump to 0.1.0-6.
- `src/perf/overhead.rs`: new module. Constants `CAL_WARMUP = 10_000`,
  `CAL_SAMPLES = 1_000`, `N_LOW = 100`, `N_HIGH = 10_000`. `Overhead`
  struct carries `framing_ticks`, `loop_per_iter_ticks`,
  `cal_raw_low_ticks`, `cal_raw_high_ticks`, `cal_duration`.
  `calibrate()` runs the fit; `Overhead::per_event_ticks(batch)`
  returns `framing_ticks / batch.max(1)` for subtraction. Two unit
  tests (amortization math + calibration smoke).
- `src/perf/mod.rs`: `pub mod overhead` + re-export
  `Overhead`/`calibrate`.
- `src/perf/tprobe2.rs`: `TProbe2::report` signature gains
  `overhead: Option<&Overhead>`; drain loop subtracts
  `ovh.per_event_ticks(batch)` from each `per_event_raw` before
  histogram insertion (via `saturating_sub`, clamp at 1). New
  test `report_subtracts_overhead`. Old tests updated to pass `None`.
- `src/bin/goal1.rs`: new `--raw` flag; calibrates on the warmed
  system between `probe.clear()` and the measurement `run_for`;
  prints an `apparatus: …` diagnostic line before the band-table
  report.

### What's *not* subtracted

Only `framing_ticks`. `loop_per_iter_ticks` (from the black_box
empty bench) is kept on `Overhead` for diagnostic visibility but
deliberately not subtracted — the real dispatch loop's per-iter
cost (queue pop, deadline check, SingleCtx construction, dynamic
dispatch) is part of what the user wants to measure, and
`black_box(1)`'s loop_per_iter doesn't model it anyway. User
already signed off on this scope.

### Smoke

- `goal1 0.5 --warmup 0 --inner 1` — 19.9 M msg/s, apparatus
  21 tk / 5.54 ns, probe mean min-p99 **4 ns** (from 9 ns raw).
- `goal1 0.5 --warmup 0 --inner 1000` — 204 M msg/s, apparatus
  correction 0 tk (rounds down), probe mean min-p99 5 ns
  (unchanged — raw already clean at this batch).
- `goal1 0.5 --warmup 0 --inner 1 --raw` — apparatus: "raw (no
  overhead subtraction)"; no calibration run.
- 14/14 tests pass. Calibration adds ~10 ms to default runs.

## Goal2: per-actor threads + mpsc channels (0.1.0-7)

Implements Goal2 of Stage 1: two actors running on their own
threads, ping-ponging an empty [`Message`] over `std::sync::mpsc`
channels, with the same warmup/measurement/calibration lifecycle
as goal1.

### Design decisions

- **Factory closures instead of pre-boxed actors.** `add_actor`
  takes `FnOnce() -> A` where `A: Actor + Send + 'static`; the
  factory runs inside the actor's own thread. Keeps the [`Actor`]
  trait free of a `Send` bound and matches how real actor systems
  typically spawn state.
- **In-band shutdown via internal `Signal` enum.** Channel payload
  is `Signal { User(Message), ClearProbe, Shutdown }`, not
  `Message` directly. Main thread injects `ClearProbe` at the
  warmup → measurement boundary and `Shutdown` after measurement;
  actors match on the variant in their `recv()` loop. Avoids an
  out-of-band atomic flag and the associated polling.
- **`inner` fixed at 1.** Ping-pong keeps at most one message in
  flight per channel, so batched probe scopes (`inner > 1`) could
  never fill — every scope would hit the partial-batch drop path.
  The `--inner` flag from goal1 is not exposed; the hardcoded
  `inner=1` is reflected in the throughput line.
- **Per-actor probes, merged reporting.** Each actor thread owns
  a [`TProbe2`] named `"goal2.dispatch.actor{id}"`. Main collects
  `(count, probe)` on join and prints a per-actor band table (not
  merged — two independent distributions are more informative
  than one blended histogram in a ping-pong workload where the
  actors should be near-identical).
- **Calibration pre-spawn on main thread.** Matches goal1's
  calibration code path. Same CPU → same `Overhead` applies to
  all actor threads. Could be refined later to calibrate
  per-thread, but unnecessary for Stage 1.

### File-level edits

- `Cargo.toml`: bump to 0.1.0-7.
- `src/runtime.rs`: add `MultiThreadRuntime`, `Signal` enum,
  `ActorFactory` alias, `MultiCtx`, and `actor_loop` helper below
  the existing single-thread code. New `#[cfg(test)] mod mt_tests`
  with a ping-pong smoke test (50 ms warmup + 50 ms measurement).
- `src/bin/goal2.rs`: new binary mirroring goal1's CLI shape
  (version banner, help width cap, --warmup, -t/--ticks, --raw),
  no --inner. Constructs `MultiThreadRuntime`, adds two
  `PingPongActor` factories, seeds one message, calls
  `rt.run(warmup, measurement)`, prints summary + apparatus +
  per-actor reports.

### Smoke (release, `--warmup 0`)

- `goal2 0.5` — 116,495 messages in 0.5 s (~0.23 M msg/s).
  Mean per-dispatch ~1.17 µs; median ~1.34 µs; p99 tail to
  ~22 µs (mostly scheduling jitter). Actors near-symmetric
  (58,247 vs 58,248 messages).
- `goal2 -h` — wraps at 80 cols, shows the `actor-x1 0.1.0-7`
  banner, no `--inner` flag.
- `goal2 0.2 --raw` — apparatus: `raw (no overhead subtraction)`;
  calibration skipped.
- goal1 unchanged: still ~200 M msg/s at `--inner 1000`.
- Tests: 15/15 pass (adds `multi_thread_runs_measures_and_shuts_down_cleanly`).

### The ~250× single-thread gap

Goal1 at `inner=1000` hits ~205 M msg/s (~5 ns/dispatch);
goal2 at `inner=1` hits ~0.23 M msg/s (~4 µs/round-trip, ~1.2 µs
per-event inside the probe scope). The bot thinks the gap is
dominated by: (1) mpsc's `Mutex` + condvar wake for every
message, (2) two cross-thread context switches per ping-pong
round, (3) cache-line bouncing between the two actors' threads.
An unbounded channel or a lock-free mpsc implementation would
close some of this but the context-switch floor is fundamental.

## `--pin` for thread CPU affinity (0.1.0-8)

Adds `--pin <CORES>` to both binaries. Tightens the stdev of
probe reports by eliminating OS thread-migration noise, at the
cost of exposing placement sensitivity in goal2 (pinning to
cross-CCX cores raises the mean because every message forces a
cross-complex cache coherence round-trip).

Adapted from `../iiac-perf/src/pin.rs` with only the two
primitives we need (`parse_cores`, `pin_current`); the affinity
save/restore, `--no-pin-cal`, and mask-summary helpers are
elided — Stage 1 doesn't need them.

### File-level edits

- `Cargo.toml`: bump to 0.1.0-8; add `core_affinity = "0.8"`.
- `src/perf/pin.rs`: new module. `parse_cores(&str) -> Result<Vec<usize>, String>`
  (comma-separated / range list, duplicates preserved for
  oversubscription) and `pin_current(Option<usize>)` (pin the
  calling thread; no-op on `None`). 7 unit tests for the parser
  (plain list, range, mixed, duplicates, empty string, reverse
  range errors, garbage errors).
- `src/perf/mod.rs`: `pub mod pin`.
- `src/runtime.rs`: `MultiThreadRuntime::run` gains a
  `pin_cores: &[usize]` parameter; actor `i` pins to
  `pin_cores[i % pin_cores.len()]` inside its spawned thread
  (before the actor factory runs or the probe is created). Empty
  slice = unpinned (existing behavior). The lone mt_test updated
  to pass `&[]`.
- `src/bin/goal1.rs`: `--pin <CORES>` flag (`Option<String>`;
  parsed via `pin::parse_cores` in `main`). Uses the first core
  of the list (goal1 is single-threaded); extra cores silently
  ignored. Pins the main thread before calibration and dispatch.
  New `pinning: …` line in the output.
- `src/bin/goal2.rs`: `--pin <CORES>` flag, same parsing. Parsed
  list passed through to `rt.run(warmup, measurement, &pins)`.
  `pinning: actor0→coreN, actor1→coreM, …` line in the output.

### A/B on this box (3900X, 12-core / 24-thread)

`goal2 0.5 --warmup 0.5` — same workload, three placements:

| Pinning     | Throughput | Mean / event | Mean min-p99 | Stdev min-p99 |
|---          |---         |---           |---           |---            |
| unpinned    | 0.374 M/s  | 632 ns       | 622 ns       | **246 ns**    |
| `--pin 0,1` | 0.308 M/s  | 667 ns       | 654 ns       | 171 ns        |
| `--pin 0,4` | 0.212 M/s  | 1,496 ns     | 1,487 ns     | **80 ns**     |

**Stdev tightens dramatically** with pinning: min-p99 stdev
drops 246 → 80 ns (3×) at `--pin 0,4`. Migration noise was a
real component of the unpinned variance.

**p10–p40 rises** (user's prediction): `--pin 0,4` puts the two
actors on different CCX complexes (each CCX = 6 cores on Zen 2),
so every ping-pong round-trip incurs a cross-CCX cache coherence
transfer. Mean climbs from 632 ns → 1,496 ns — the placement cost.
`--pin 0,1` (same CCX) stays close to unpinned throughput
(~0.31 M/s vs 0.37) but still narrows stdev somewhat.

The bot thinks these numbers are the intended story: pinning
trades a potentially higher mean for much more reproducible
measurements, which is the right tradeoff for code-change A/B
work. The placement itself then becomes a tunable — users pick
cores that match what they want to measure (best-case
same-complex, worst-case cross-socket, etc.).

### `goal1 --pin`

Single-thread goal1 sees little throughput change with
`--pin 5` (~206 M msg/s, identical to unpinned) — all the work
is on one thread anyway, and migration cost is already amortized
over the 200 M msg/s inner loop. Value of pinning is purely
reproducibility.

## Stage 1 complete (0.1.0)

Final release of the Stage 1 PoC. Drops the `-N` pre-release
suffix and marks the design.md "Stage 1 runtime" goal as shipped.
No behavior changes vs 0.1.0-8; this is the "done" marker.

- `Cargo.toml`: `0.1.0-8` → `0.1.0`.
- `notes/todo.md`: "Stage 1 runtime PoC" moved from
  `## In Progress` to `## Done`; reference `[2]` updated to point
  at this chores section.
- `README.md`: new `## Usage`, `## Reading the band table`, and
  `## The bot's understanding` sections inserted after
  `## Cloning` and before `## jj Tips for Git Users`. Existing
  dual-repo / jj / cross-repo-linking / contributing / license
  sections unchanged.
- `notes/chores-01.md`: this section.

### The Stage 1 ladder at a glance

| Version | Landmark |
|---|---|
| 0.1.0-0 | Cargo bootstrap + plan marker |
| 0.1.0-1 | Goal1: single-thread ping-pong runtime |
| 0.1.0-3 | `tprobe2` vendored from iiac-perf + inner batching + warmup + clap (folded in 0.1.0-2 retroactively) |
| 0.1.0-4 | Always render all 12 band rows |
| 0.1.0-5 | CLI polish: `max_term_width = 80`, version banner |
| 0.1.0-6 | Apparatus-overhead calibration (two-point fit) |
| 0.1.0-7 | Goal2: per-actor threads + `std::sync::mpsc` |
| 0.1.0-8 | `--pin` for thread CPU affinity |
| **0.1.0** | **Release marker** |

### What ships in 0.1.0

- Two runtimes: `SingleThreadRuntime` (Goal1) and
  `MultiThreadRuntime` (Goal2).
- Minimal `Actor` / `Context` / `Message` surface in `src/lib.rs`.
- Vendored perf probe stack under `src/perf/` (`tprobe2`,
  `band_table`, `ticks`, `fmt`, `overhead`, `pin`), diverged
  from `../iiac-perf/src` per actor-x1 conventions but otherwise
  tracking upstream.
- Two binaries: `goal1` and `goal2`, sharing a common CLI shape.
- 22 unit tests; no integration tests yet.

### What's next

Stage 2 (see [design.md](design.md)): three actors, a richer
`Message` with `src_id` / `dst_id` / `send_count`, per-actor
constructors and names. Promoting the vendored perf stack to a
shared library crate is also on the radar once Stage 2 exposes
what's stable.

## Extract perf into tprobe crate — plan marker (0.2.0-0)

Converts the single-crate repo into a Cargo workspace with
`actor-x1` and a new `tprobe` crate carved out of `src/perf/`.
The 0.1.0-2 chores note flagged "promoting this to a shared
crate is worthwhile once Stage 2 is underway" — Stage 1 has
shipped and divergence from upstream `iiac-perf` has settled,
so this is the natural next step before Stage 2 adds new
actors and messages.

Multi-step ladder:

- `0.2.0-0` — Plan marker. Bump root `Cargo.toml` to `0.2.0-0`;
  this chore section; `notes/todo.md` entry under `## In Progress`.
  No structural change.
- `0.2.0-1` — Workspace layout. Move root `Cargo.toml` + `src/`
  into `crates/actor-x1/`. Root `Cargo.toml` becomes a virtual
  workspace (`[workspace]` only, `resolver = "2"`,
  `members = ["crates/actor-x1", "crates/tprobe"]`). Create
  `crates/tprobe/` skeleton (`Cargo.toml` + `src/lib.rs`
  docstring stub). `src/perf/*` still lives inside `actor-x1`;
  no probe file moves yet. `cargo test` passes; both binaries
  behave identically.
- `0.2.0-2` — Move perf into tprobe. Move
  `crates/actor-x1/src/perf/*` → `crates/tprobe/src/*`;
  rewrite internal imports (`crate::perf::x` → `crate::x`).
  Migrate `hdrhistogram`, `minstant`, `core_affinity` deps
  from actor-x1 to tprobe's `Cargo.toml`. Add
  `tprobe = { path = "../tprobe" }` to actor-x1's deps. Drop
  `pub mod perf;` from actor-x1's `src/lib.rs`. Rewire call
  sites in `runtime.rs`, `goal1.rs`, `goal2.rs`
  (`crate::perf::x` → `tprobe::x`). Behavior unchanged; all
  tests pass.
- `0.2.0-3` — Rename `TProbe2` → `TProbe` and
  `tprobe2.rs` → `tprobe.rs`. Pure sweep across the new crate
  and its callers. The "2" only communicated "second version
  of the probe" inside `iiac-perf`; outside that repo it's
  noise. Upstream divergence is already substantial, so the
  rename does not meaningfully raise any future re-sync cost.
- `0.2.0` — Done marker. Drop `-N` suffix. `notes/todo.md`:
  move this task to `## Done`. `notes/chores-01.md`: this
  section. `README.md`: update any source-tree or contributor
  notes that reference `src/perf/`.

### Design decisions recorded here

- **Virtual workspace, `crates/` subdir** (option (b) in the
  plan discussion): root is `[workspace]`-only, both packages
  live under `crates/`. Cleaner symmetric layout than mixing
  `[package]` and `[workspace]` at the root, at the cost of a
  one-time move of every actor-x1 source file.
- **Crate name `tprobe` drops the "2"**. The type rename is
  deferred to its own step (`0.2.0-3`) to keep the move diff
  (`0.2.0-2`) separate from the rename diff — easier review
  for each.
- **Perf deps move with the perf code.** `hdrhistogram`,
  `minstant`, and `core_affinity` migrate out of actor-x1's
  `Cargo.toml` in `0.2.0-2`; actor-x1 picks them up transitively
  via `tprobe` without redeclaring.

## Workspace layout + tprobe skeleton (0.2.0-1)

Converts the repo from a single crate at the root into a virtual
Cargo workspace with `actor-x1` and a new `tprobe` crate under
`crates/`. No probe code moves yet — `src/perf/*` still lives
inside `actor-x1`, and the binaries / tests are unchanged
behaviorally. This step only reshuffles the file layout so that
`0.2.0-2` can move perf into `tprobe` cleanly.

- `Cargo.toml` (new, root): virtual workspace.
  `[workspace]` with `resolver = "2"`,
  `members = ["crates/actor-x1", "crates/tprobe"]`. No
  `[package]` at the root anymore.
- `crates/actor-x1/Cargo.toml`: former root `Cargo.toml`, moved
  verbatim; version bumped `0.2.0-0` → `0.2.0-1`. Deps
  unchanged (`clap`, `core_affinity`, `hdrhistogram`,
  `minstant`) — they migrate to `tprobe` in `0.2.0-2`.
- `crates/actor-x1/src/`: former root `src/` moved verbatim.
  `lib.rs`, `runtime.rs`, `bin/{goal1,goal2}.rs`, and
  `perf/**` all in place; no content edits.
- `crates/tprobe/Cargo.toml`: new. `name = "tprobe"`,
  `version = "0.1.0-0"` (dev-ladder suffix matching the
  `actor-x1` convention — drops `-N` when perf moves in at
  `0.2.0-2`), same license, empty `[dependencies]`.
- `crates/tprobe/src/lib.rs`: new. `//!` docstring only;
  no public items yet.

### Side effects

- `Cargo.lock` stays at root (workspace-shared); cargo
  regenerated it to recognize the two members. Version lines
  updated automatically.
- `cargo install --path crates/actor-x1` is the new invocation
  for the `goal1` / `goal2` bins (root is virtual so `--path .`
  no longer works for install).
- `target/` stays at the root (workspace-shared); `.gitignore`
  already excludes it, no change needed.
- `notes/`, `README.md`, `LICENSE-*`, `CLAUDE.md`,
  `.vc-config.toml`, `.gitignore` remain at the workspace root.

### Verification

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -- -D warnings` clean across both
  members.
- `cargo test` — 22 pass (actor-x1) + 0 (tprobe stub) = 22/22;
  same count as `0.2.0-0`, confirming no test was lost in the
  move.
- `cargo install --path crates/actor-x1` replaces the prior
  `actor-x1 v0.2.0-0` install; `goal1 --version` and
  `goal2 --version` both report `actor-x1 0.2.0-1`.

## Move perf into tprobe crate (0.2.0-2)

Carries out the body of the `0.2.0` ladder: `src/perf/*` moves
from `crates/actor-x1/src/` into `crates/tprobe/src/`; perf
dependencies migrate with the code; actor-x1 picks `tprobe` up
as a path dependency. Behavior unchanged — tests and binaries
run the same workloads with identical output modulo the version
banner. The `TProbe2 → TProbe` rename is deliberately deferred
to `0.2.0-3`.

- `Cargo.lock`: updated by cargo for the new dep graph.
- `crates/actor-x1/Cargo.toml`: version `0.2.0-1` → `0.2.0-2`;
  drops `core_affinity`, `hdrhistogram`, `minstant`; adds
  `tprobe = { path = "../tprobe" }`.
- `crates/actor-x1/src/lib.rs`: drops `pub mod perf;`.
- `crates/actor-x1/src/runtime.rs`: `use crate::perf::TProbe2`
  → `use tprobe::TProbe2`; `crate::perf::pin::pin_current(...)`
  → `tprobe::pin::pin_current(...)`. `cargo fmt` reordered the
  import block (local `crate::` before external `tprobe`) per
  rustfmt's group-ordering.
- `crates/actor-x1/src/bin/goal1.rs`: `use actor_x1::perf::{self, pin, ticks}`
  → `use tprobe::{self as perf, pin, ticks}`. Call sites (e.g.
  `perf::calibrate()`) unchanged — the `self as perf` alias
  preserves them.
- `crates/actor-x1/src/bin/goal2.rs`: same import rewrite as
  goal1.
- `crates/tprobe/Cargo.toml`: version bumped `0.1.0-0` → `0.1.0`
  (skeleton → first functional release); adds `core_affinity`,
  `hdrhistogram`, `minstant` to `[dependencies]`.
- `crates/tprobe/src/lib.rs`: rewritten from docstring-only stub
  to the full module root — merges the vendored-from-iiac-perf
  notes previously in `src/perf/mod.rs`, declares the six
  submodules (`band_table`, `fmt`, `overhead`, `pin`, `ticks`,
  `tprobe2`), and re-exports `Overhead` / `calibrate` /
  `TProbe2` / `TProbe2RecId` at the crate root.
- `crates/tprobe/src/band_table.rs`, `fmt.rs`, `overhead.rs`,
  `pin.rs`, `ticks.rs`, `ticks/x86_64.rs`, `tprobe2.rs`: moved
  verbatim from `crates/actor-x1/src/perf/`. Internal imports
  rewritten — every `crate::perf::x` (both `use` lines and
  doc-link references) becomes `crate::x` since within `tprobe`
  the crate root *is* what `perf/mod.rs` used to be.

### Design decisions recorded here

- **`tprobe` version bumps `0.1.0-0` → `0.1.0`** at this step
  rather than mirroring actor-x1's `-N` ladder. tprobe's own
  history is "skeleton → populated", which is a single one-step
  release in its own right. Drawing its version clock separately
  from actor-x1's matches the "each crate versions itself"
  expectation callers have of any Cargo dep.
- **`use tprobe::{self as perf, pin, ticks}`** in the binaries
  preserves `perf::calibrate()` at the call site. Could have
  been rewritten to `tprobe::calibrate()` — chose the alias to
  keep the move diff (this step) minimal so `0.2.0-3`'s rename
  sweep is the only place the identifier changes. The alias
  disappears when the bot thinks the call site should read
  `tprobe::…` directly, which is a follow-up worth considering
  after the rename lands.
- **Imports inside tprobe drop the `perf::` segment** rather
  than keeping a nominal `perf` submodule at `crate::perf`. A
  standalone crate's root *is* the top of the namespace; adding
  an intermediate `perf` module inside `tprobe` would be
  redundant and would force every call site to say
  `tprobe::perf::x` — worse in every direction.

### Verification

- `cargo fmt` clean after one small auto-reorg in
  `runtime.rs` (see bullet above).
- `cargo clippy --all-targets -- -D warnings` clean across both
  members.
- `cargo test` — 22 pass (5 in actor-x1 runtime tests + 17 in
  tprobe's moved unit tests) = 22/22. Same count as before the
  move, confirming no test was lost.
- `cargo install --path crates/actor-x1` replaces the prior
  `actor-x1 v0.2.0-1` install; `goal1 --version` and
  `goal2 --version` both report `actor-x1 0.2.0-2`.

## Rename TProbe2 → TProbe (0.2.0-3)

Mechanical rename sweep. `TProbe2` → `TProbe`, `TProbe2RecId`
→ `TProbeRecId`, `tprobe2.rs` → `tprobe.rs`, and the band-table
header string `"tprobe2"` → `"tprobe"`. The "2" only carried
meaning inside upstream `iiac-perf`, where it distinguished
this scope-API probe from a sibling direct-histogram `TProbe`.
Inside this standalone crate there's only one probe, so the
suffix is noise.

- `Cargo.lock`: updated by cargo for the tprobe version bump.
- `crates/actor-x1/Cargo.toml`: `0.2.0-2` → `0.2.0-3`.
- `crates/actor-x1/src/runtime.rs`: six textual occurrences of
  `TProbe2` → `TProbe` (field type, `use`, constructor call,
  `probe_mut` return type, and two doc-links).
- `crates/actor-x1/src/bin/goal1.rs`: docstring phrase
  `` `tprobe2` band-table report `` → `` `tprobe` band-table
  report ``.
- `crates/actor-x1/src/bin/goal2.rs`: two copies of the same
  docstring phrase updated.
- `crates/tprobe/Cargo.toml`: `0.1.0` → `0.1.1` (patch bump —
  public API rename).
- `crates/tprobe/src/lib.rs`: module declaration
  `pub mod tprobe2` → `pub mod tprobe`; re-export
  `pub use tprobe2::{TProbe2, TProbe2RecId}` → `pub use
  tprobe::{TProbe, TProbeRecId}`; two docstring references
  (`[`tprobe2`]` → `[`tprobe`]`, `[`TProbe2`]` → `[`TProbe`]`).
- `crates/tprobe/src/tprobe.rs` (was `tprobe2.rs`): file
  renamed; all `TProbe2` / `TProbe2RecId` identifier
  occurrences → `TProbe` / `TProbeRecId` (struct defs, impls,
  tests, doc-links); band-table header string `"tprobe2"` →
  `"tprobe"`.
- `crates/tprobe/src/overhead.rs`: four doc-link references
  `TProbe2` / `TProbe2::report` → `TProbe` / `TProbe::report`.
- `crates/tprobe/src/band_table.rs`: module-level docstring
  reworded — previously referenced both `TProbe` (fast path)
  and `TProbe2` (scope API) from upstream; now notes that
  only the scope-API probe was vendored and still shares the
  renderer. Function docstring example list trimmed from
  `(`"tprobe"`, `"tprobe2"`, …)` to `(e.g. `"tprobe"`)`.
- `notes/chores-01.md`: this section. Earlier sections that
  record the arrival of `TProbe2` during Stage 1 (`0.1.0-2`,
  `0.1.0-3`, `0.1.0-6`) are left as-is — they describe events
  at the time, when the identifier was literally `TProbe2`.
- `notes/todo.md`: `Rename TProbe2 → TProbe (0.2.0-3) [[6]]`
  added to `## Done` + reference.

### Design decisions recorded here

- **Single `replace_all` on `TProbe2` handles both identifiers**
  because `TProbe2RecId` contains `TProbe2` as a prefix; the
  substring replacement produces `TProbeRecId` without a
  separate pass.
- **The submodule keeps the name `tprobe`**, matching the crate
  name. External callers reach types via the `pub use` at the
  crate root (`tprobe::TProbe`); the fully-qualified internal
  path `tprobe::tprobe::TProbe` is slightly awkward but never
  written by consumers. The alternative — moving the struct
  directly into `lib.rs` and eliminating the submodule — would
  inflate `lib.rs` with ~290 lines of probe implementation
  and make the crate root less navigable. Keeping the
  submodule is cheaper for readability.
- **tprobe version bumps `0.1.0` → `0.1.1`** rather than
  `0.2.0`. Pre-1.0 semver calls for a minor bump on API break,
  but `actor-x1` is the only consumer, is updated atomically
  in the same commit, and doesn't pin a version requirement
  (path deps skip that). The bump is informational; a patch
  level is enough to signal "content changed" in cargo's
  accounting.

### Verification

- `cargo fmt` clean (no reorgs required this time).
- `cargo clippy --all-targets -- -D warnings` clean across both
  members.
- `cargo test` — 22 pass (5 in actor-x1 + 17 in tprobe). Same
  count as `0.2.0-2`; rename is mechanical so no test delta
  expected.
- `cargo install --path crates/actor-x1` replaces the prior
  `actor-x1 v0.2.0-2` install; both binaries report
  `actor-x1 0.2.0-3`.
- No remaining `TProbe2` or `tprobe2` (case-sensitive) in the
  `crates/` tree per a final grep — confirms the sweep is
  complete.

## Workspace split complete (0.2.0)

Closes the `0.2.0` ladder. The `-N` suffix drops; the repo is
now a two-crate virtual workspace with `tprobe` extracted from
what used to live at `crates/actor-x1/src/perf/`. No behavior
change vs. `0.2.0-3`; this is the closing marker.

- `crates/actor-x1/Cargo.toml`: `0.2.0-3` → `0.2.0`.
- `README.md`: three updates to reflect the post-split state
  (`cargo install --path crates/actor-x1`; sample banner
  `0.1.0` → `0.2.0`; three `tprobe2` → `tprobe` references in
  sample output and the "Reading the band table" section).
- `notes/todo.md`: overall `Extract perf into tprobe workspace
  crate` entry moves from `## In Progress` to `## Done` with a
  new reference `[7]` pointing at this section (Stage 1's
  entry uses the same pattern — pointing at the closing
  section rather than the plan marker).
- `notes/chores-01.md`: this section.

### The 0.2.0 ladder at a glance

| Version | Landmark |
|---|---|
| `0.2.0-0` | Plan marker + CLAUDE.md push-flow clarifications |
| `0.2.0-1` | Workspace layout — `crates/actor-x1` + empty `crates/tprobe` |
| `0.2.0-2` | Move perf from `actor-x1` into `tprobe` |
| `0.2.0-3` | Rename `TProbe2` → `TProbe`, `tprobe2.rs` → `tprobe.rs` |
| **`0.2.0`** | **Closing marker** |

### What ships in 0.2.0

- Two workspace members under `crates/`:
  `actor-x1` (the binaries and runtimes) and `tprobe`
  (the vendored-from-iiac-perf probe stack).
- `tprobe` at version `0.1.1` — standalone crate providing
  `TProbe`, `TProbeRecId`, `Overhead` / `calibrate`, and the
  `band_table`, `fmt`, `pin`, `ticks` submodules.
- `actor-x1` picks up `tprobe` via a path dependency; perf
  crates (`core_affinity`, `hdrhistogram`, `minstant`) no
  longer declared directly.
- Session-flow conventions codified in CLAUDE.md:
  `vc-x1 push` invocation, prerequisite `.gitignore` entry
  for `/.vc-x1`, prerequisite `jj bookmark track` on first
  push, `other-repo` terminology block, `## <other-repo>
  session notes:` body tail, clarified `-N`-is-complete
  Versioning wording.
- 22 unit tests (5 in actor-x1's runtime module, 17 in
  `tprobe`). Same count as `0.1.0` — the split was purely
  structural.

### What's next

Stage 2 (see [design.md](design.md)): three actors with a
richer `Message { src_id, dst_id, send_count }`, per-actor
constructors, and names. `tprobe`'s API settled enough in
this ladder that promoting it to a sibling repo (if ever
wanted) would now be a straightforward copy rather than
another extraction round.

## Band-table format + overhead docs — plan marker (0.3.0-0)

Reworks the band-table report so the existing columns show
raw (unadjusted) values and a new rightmost `adj mean` column
shows the overhead-subtracted mean. Along the way, formalizes
what "overhead" means in `tprobe` — framing, loop-per-iter,
and an unmeasurable "everything else" category — and extends
the subtraction to include `loop_per_iter_ticks` on top of
framing. CLI is simplified: `--raw` goes away (calibration is
~10 ms, always run), and the `--warmup` default drops from 10 s
to 0.5 s so quick iteration doesn't require typing
`--warmup 0` every time. Notes are split per crate in
preparation for eventually separating the two crates into
their own repos.

Multi-step ladder:

- `0.3.0-0` — Plan marker. Bump `actor-x1` to `0.3.0-0`; this
  chore section; `notes/todo.md` entry under `## In Progress`.
  No structural change.
- `0.3.0-1` — Notes reorg + per-crate `README.md` +
  `overhead-model.md`. Move `notes/design.md` →
  `crates/actor-x1/notes/design.md`. Create
  `crates/actor-x1/notes/`, `crates/tprobe/notes/`,
  `crates/actor-x1/README.md`, `crates/tprobe/README.md`.
  Write `crates/tprobe/notes/overhead-model.md` defining
  framing, loop-per-iter, and the unmeasurable-overhead
  category (cache misses, branch mispredicts, context
  switches, interrupts — they stay in the measurement because
  we can't attribute them per-event). Replace long prose in
  `overhead.rs` / `tprobe.rs` doc comments with pointers to
  `overhead-model.md`. Docs-only; no behavior change.
- `0.3.0-2` — Extend subtraction to framing + loop-per-iter.
  `Overhead::per_event_ticks(batch)` returns
  `framing_ticks / batch + loop_per_iter_ticks` instead of
  just `framing_ticks / batch`. `apparatus:` line shows both
  components so readers see the split. Tests updated.
- `0.3.0-3` — Band-table `adj mean` column. `TProbe::report`
  stores raw per-event values in the histogram (no correction
  applied). `band_table::render` gains an optional per-event
  correction; existing `first` / `last` / `range` / `count` /
  `mean` columns show raw; new rightmost `adj mean` column
  shows `raw − correction`. Summary rows `mean` and
  `mean min-p99` get adj values in the new column; `stdev`
  rows leave it blank (shift-invariant). Uses global-average
  correction (exact for today's constant-batch PoC; a note in
  `overhead-model.md` flags where per-band tracking would be
  needed if batches ever mix within one probe).
- `0.3.0-4` — Remove `--raw` flag. Drop the flag from `goal1`
  and `goal2`; calibration runs unconditionally. Drop the
  "raw (no overhead subtraction)" apparatus-line branch.
- `0.3.0-5` — Warmup default `10.0 → 0.5`. Both binaries.
- `0.3.0` — Closing marker. Drop `-N` suffix on `actor-x1`;
  top-level `README.md` updated for new output format, no
  `--raw`, new warmup default, per-crate READMEs. Move
  `notes/todo.md` entries to `## Done`.

### Design decisions recorded here

- **Extend subtraction to `loop_per_iter_ticks`.** Earlier
  (`0.1.0-6`) the subtraction was deliberately scoped to
  framing only, on the grounds that "the real dispatch loop's
  per-iter cost is part of what the user wants to measure".
  Revisited: the real loop's scaffolding (loop branch,
  counter increment) is structural overhead relative to the
  dispatch work, and `black_box(1)`'s `loop_per_iter_ticks`
  is a close-enough proxy for it. Biased slightly low
  because the real loop doesn't execute the `black_box`
  itself; the bias is small and worth the simpler story in
  the output. `overhead-model.md` records this tradeoff
  explicitly.
- **Global average correction, not per-band.** Correction
  depends on `batch`; in the current PoC every record in a
  given probe has the same `batch`, so a single constant
  correction applies uniformly. If later probes ever mix
  `batch` sizes within one drain, a per-band correction sum
  would be more faithful. Deferred until that situation
  actually arises; `overhead-model.md` flags the extension
  point.
- **Per-crate `notes/` and `README.md`.** Anticipates
  splitting the crates into separate repos later. Workspace
  `/notes/` keeps format conventions, VCS tips, release
  chronology, workspace-level todo/done; each crate's own
  design, architecture, and model notes travel with it.

## Future work: linkme/inventory benchmark harness (0.3.0-0)

Idea captured during `0.3.0-0`; not scheduled for
implementation yet. Explores using compile-time static
registration (`linkme` distributed slices or `inventory` via
`ctor`) to build a `tprobe`-driven benchmark runner where
individual benchmarks declare themselves in separate files
and the runner picks them up with no `main()` edits per
benchmark — the ergonomic pattern criterion-rs offers.

### Fit for benchmarks

The bot thinks this is a strong fit. Each benchmark file
drops in a `bench!(...)` invocation; the runner iterates
the registered entries, sets up warmup/calibration/pin
config once, and prints a `tprobe` band table per
benchmark. The measurement primitive already exists
(`tprobe`); the missing piece is the registration and
iteration layer. Result: adding a benchmark is a
one-file change with no central table to edit.

### Fit for actor applications

The bot thinks this is a partial fit. Registration gives
discovery ("enumerate every `Actor` type the binary
knows"), but actor construction still needs runtime config
— id, name, channel endpoints, thread placement — which
lives in `main` / runtime setup rather than at declaration
sites. Useful if we find ourselves hand-editing a registry
table per new actor; not a silver bullet for the full
wiring problem.

### Tradeoffs to validate before committing

- `linkme` uses linker-section magic. Works on
  `x86_64-unknown-linux-gnu` (our target) out of the box;
  the bot thinks aggressive `strip` / LTO / unusual link
  modes could in principle drop registrations, so a
  release-mode smoke test with our actual flags is worth
  doing before relying on it.
- `inventory` uses static constructors (via `ctor`).
  Broader platform support, pays a small init-time cost.
  The bot thinks the main historical failure mode is
  reachability elision — a registration in a
  conditionally-compiled or test-only module silently not
  registering — which is usually fine in a binary crate
  but worth a sanity check.
- Both approaches lose the "grep `main.rs` to see every
  registered item" property. Taste call; the bot thinks
  the file-per-benchmark ergonomic outweighs it for
  benchmarks, and the call is less clear for actors.

### Recommendation

The bot's suggested order: start with the benchmark
harness (clear win, bounded scope), learn the mechanism,
then decide whether the actor-registration case justifies
a second use. Tracked as a todo entry (`[9]`) against this
section; promote to a versioned plan marker when scheduled.
