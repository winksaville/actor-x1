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
