# Todo

This file contains near term tasks with a short description
and reference links to more details.

## In Progress

- Extend subtraction to framing + loop_per_iter (0.3.0-2) [[11]]

## Todo

See [Foramt details](README.md#todo-format)

- Initial design [[1]]
- PoC implementation of a manager
- Explore `linkme` / `inventory` for a benchmark harness on top of
  `tprobe` — self-registering benchmarks (no `main()` edits per
  benchmark); possibly reusable for actor registration too [[9]]
- Band-table raw + `adj mean` column (0.3.0-3)
- Remove `--raw` flag, always calibrate (0.3.0-4)
- Lower warmup default 10 s → 0.5 s (0.3.0-5)
- After 0.3.0: add a `criterion` benchmark mirroring the goal1 /
  goal2 workloads, so `tprobe`'s numbers can be cross-validated
  against an established statistical-sampling harness
- Remove unnecessary `let Self { actors, queue, probe } = self;`
  destructure in `SingleThreadRuntime::run_for`. Disjoint `&mut`
  through named field paths (`self.actors[i]`, `self.queue`,
  `self.probe`) compiles without the split. Also update the
  struct doc comment that references "field-split-borrows
  actors and queue"

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
