# tprobe design

A named, single-writer histogram of hardware tick-counter
deltas, plus a record buffer, plus a percentile band-table
renderer. Tuned for the hot path to be as short as possible
— two `rdtsc` reads and a `Vec::push` — with delta math,
histogram ingestion, and tick→ns conversion deferred to
report time.

## Data flow

```
start(site_id)  ─┐
                 │
end(id)          ├─► Record { site_id, start_tsc, end_tsc, batch }
                 │             └─ appended to records: Vec<Record>
end_batch(id, n) ┘

                            report(as_ticks, overhead)
                                   │
                                   ▼
                        records.drain(..) →
                          per_event = (delta - correction) / batch
                                   │
                                   ▼
                             Histogram<u64>
                                   │
                                   ▼
                          band_table::render
```

- **Hot path** is `start` + `end*`: two `rdtsc` reads and a
  `Vec::push`. No division, no histogram updates, no unit
  conversion.
- **Cold path** is `report`: drains the record buffer,
  applies per-record division and overhead correction,
  ingests into the histogram, then renders the band table.
- **`clear`** zeroes both the record buffer and the
  histogram. Used at the warmup → measurement boundary so
  warmup samples don't contaminate the reported
  distribution.

## Modules

- `tprobe` — the probe type (`TProbe`, `TProbeRecId`) and
  the `start` / `end` / `end_batch` / `clear` / `report`
  surface.
- `overhead` — calibration and the `Overhead` struct. See
  [`overhead-model.md`](overhead-model.md) for the formal
  model.
- `band_table` — percentile-band renderer (12 bands,
  `first` / `last` / `range` / `count` / `mean` columns,
  summary rows). Shared by the `TProbe` report path.
- `ticks` — hardware tick-counter abstraction. x86_64 only;
  uses `rdtsc` via the `minstant` crate. Other architectures
  emit `compile_error!`.
- `fmt` — `fmt_commas` / `fmt_commas_f64`, extracted from
  `iiac-perf`'s harness so we don't carry the whole 393-line
  benchmark harness for two formatting helpers.
- `pin` — CPU-affinity helpers (`parse_cores`,
  `pin_current`). Adapted from `iiac-perf`'s `pin.rs`.

## Upstream

Vendored from
[`iiac-perf`](https://github.com/winksaville/iiac-perf) —
source path at the time of vendoring: `../iiac-perf/src`.
Graduated from `actor-x1`'s in-tree `src/perf/` module at
`actor-x1 0.2.0-2`.

Divergences from upstream at the time of graduation:

- Import paths reflect the standalone-crate layout
  (`crate::x` rather than `crate::perf::x`).
- `// OK: …` justifications on every `unwrap*` call per the
  host workspace's code conventions.
- `ticks.rs` diagnostic string mentions "tprobe" rather than
  "iiac-perf".
- `TProbe` here corresponds to upstream's scope-API probe
  (`TProbe2` in `iiac-perf`); the upstream fast-path
  direct-histogram `TProbe` was not vendored. The "2" suffix
  was dropped on extraction at `0.2.0-3`.
