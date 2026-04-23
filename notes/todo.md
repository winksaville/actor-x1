# Todo

This file contains near term tasks with a short description
and reference links to more details.

## In Progress

- Band-table format + overhead docs — plan marker (0.3.0-0) [[8]]

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

1. Stage 1 runtime PoC [[2]]
1. Extract perf into tprobe crate — plan marker (0.2.0-0) [[3]]
1. Workspace layout + tprobe skeleton (0.2.0-1) [[4]]
1. Move perf into tprobe crate (0.2.0-2) [[5]]
1. Rename TProbe2 → TProbe (0.2.0-3) [[6]]
1. Extract perf into tprobe workspace crate (0.2.0) [[7]]


# References

[1]: design.md
[2]: chores-01.md#stage-1-complete-010
[3]: chores-01.md#extract-perf-into-tprobe-crate--plan-marker-020-0
[4]: chores-01.md#workspace-layout--tprobe-skeleton-020-1
[5]: chores-01.md#move-perf-into-tprobe-crate-020-2
[6]: chores-01.md#rename-tprobe2--tprobe-020-3
[7]: chores-01.md#workspace-split-complete-020
[8]: chores-01.md#band-table-format--overhead-docs--plan-marker-030-0
[9]: chores-01.md#future-work-linkmeinventory-benchmark-harness-030-0
