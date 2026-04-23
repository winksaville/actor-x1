# Todo

This file contains near term tasks with a short description
and reference links to more details.

## In Progress

- Notes reorg + per-crate READMEs + overhead-model.md (0.3.0-1) [[10]]

## Todo

See [Foramt details](README.md#todo-format)

- Initial design [[1]]
- PoC implementation of a manager
- Explore `linkme` / `inventory` for a benchmark harness on top of
  `tprobe` — self-registering benchmarks (no `main()` edits per
  benchmark); possibly reusable for actor registration too [[9]]

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
