# Todo

This file contains near term tasks with a short description
and reference links to more details.

## In Progress

- Band-table raw columns + `adj mean` column (0.3.0-3) [[12]]

## Todo

See [Foramt details](README.md#todo-format)

- Initial design [[1]]
- PoC implementation of a manager
- Explore `linkme` / `inventory` for a benchmark harness on top of
  `tprobe` — self-registering benchmarks (no `main()` edits per
  benchmark); possibly reusable for actor registration too [[9]]
- Remove `--raw` flag, always calibrate (0.3.0-4)
- Lower warmup default 10 s → 0.5 s (0.3.0-5)
- After 0.3.0: add a `criterion` benchmark mirroring the goal1 /
  goal2 workloads, so `tprobe`'s numbers can be cross-validated
  against an established statistical-sampling harness
- Revisit band-table `range` semantics — drop the `+1` so
  `first == last → range = 0` (true "no spread" instead of
  today's "1 tk / rounds to 0 ns"). `last - first` reads
  more naturally and matches what a reader expects "range"
  to mean. Pre-existing behavior, not introduced in 0.3.0-3
- **Bound `TProbe`'s records-buffer memory.** Each scope
  appends a 32-byte `Record` to an unbounded `Vec`; at
  `goal1 --inner 10` (~13M scopes/s), 20 s of measurement
  buffers 260M records ≈ 8 GB minimum, ~16 GB after
  `Vec` power-of-2 over-allocation, which swaps/OOMs on
  typical dev boxes. Pre-existing from 0.1.0-2 when the
  probe was vendored; becomes hard to miss at long
  measurement windows + small `inner`. Options (pick one
  once we instrument drain cost):
  - (a) **Hard cap + truncation counter.** Fixed max
    records (e.g. 64M ≈ 2 GB); past the cap, increment a
    dropped counter and skip the `push`. Report shows
    `truncated: N scopes dropped`. Preserves hot-path
    cost. Partial data on very long runs.
  - (b) **Periodic inline drain.** When records hit a
    threshold, drain into the histogram and continue.
    Memory bounded. Adds occasional drain stalls (ms-ish)
    that show up in the `p99-max` tail.
  - (c) **Inline drain, no buffer.** `end` / `end_batch`
    compute `per_event` and call `hist.record` directly.
    Fully bounded memory, no stalls, but adds ~15–20 ns
    to the hot path — non-trivial when the probe is
    measuring ~5 ns work per event.
  - (d) **Skinnier record (just `u64` per-event ticks).**
    4× memory reduction. Buys time but doesn't fix the
    unbounded-growth shape.
- Clean up `SingleThreadRuntime::run_for` inner loop:
  (a) remove the unnecessary `let Self { actors, queue, probe } = self;`
      destructure — disjoint `&mut` through named field paths
      (`self.actors[i]`, `self.queue`, `self.probe`) compiles
      without the split;
  (b) replace `while done < inner { …; done += 1; }` with
      `for _ in 0..inner { …; done += 1; }` (break on empty
      queue stays) — more idiomatic for a known upper bound,
      same generated code under `--release`;
  (c) update the struct doc comment that references
      "field-split-borrows actors and queue".

## Done

Completed tasks are moved from `## Todo` to here, `## Done`, as they are completed
and older `## Done` sections are moved to [done.md](done.md) to keep this file small.

- Stage 1 runtime PoC [[2]]
- Extract perf into tprobe crate — plan marker (0.2.0-0) [[3]]
- Workspace layout + tprobe skeleton (0.2.0-1) [[4]]
- Move perf into tprobe crate (0.2.0-2) [[5]]
- Rename TProbe2 → TProbe (0.2.0-3) [[6]]
- Extract perf into tprobe workspace crate (0.2.0) [[7]]
- Band-table format + overhead docs — plan marker (0.3.0-0) [[8]]
- Notes reorg + per-crate READMEs + overhead-model.md (0.3.0-1) [[10]]
- Extend subtraction to framing + loop_per_iter (0.3.0-2) [[11]]


# References

[1]: ../crates/actor-x1/notes/design.md
[2]: chores-01.md#stage-1-complete-010
[3]: chores-01.md#extract-perf-into-tprobe-crate--plan-marker-020-0
[4]: chores-01.md#workspace-layout--tprobe-skeleton-020-1
[5]: chores-01.md#move-perf-into-tprobe-crate-020-2
[6]: chores-01.md#rename-tprobe2--tprobe-020-3
[7]: chores-01.md#workspace-split-complete-020
[8]: chores-01.md#band-table-format--overhead-docs--plan-marker-030-0
[9]: chores-01.md#future-work-linkmeinventory-benchmark-harness
[10]: chores-01.md#notes-reorg--per-crate-readmes--overhead-modelmd-030-1
[11]: chores-01.md#extend-subtraction-to-framing--loop_per_iter-030-2
[12]: chores-01.md#band-table-raw-columns--adj-mean-column-030-3
