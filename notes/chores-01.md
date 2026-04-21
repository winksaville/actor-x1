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
