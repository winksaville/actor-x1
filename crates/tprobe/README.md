# tprobe

A named, single-writer performance probe that records
hardware tick-counter deltas across scoped code regions and
renders a percentile band-table report.

Part of the [actor-x1 workspace](../..). Originally vendored
from [`iiac-perf`](https://github.com/winksaville/iiac-perf);
extracted into its own crate at `actor-x1 0.2.0`.

## At a glance

```rust
use tprobe::{TProbe, calibrate};

let mut probe = TProbe::new("dispatch");
let ovh = calibrate();

for _ in 0..n {
    let id = probe.start(site_id);
    // ... work to measure ...
    probe.end(id);
}

probe.report(/* as_ticks = */ false, Some(&ovh));
```

## Public surface

- `TProbe` / `TProbeRecId` — scope-based probe
  (`start` / `end` / `end_batch` → record buffer →
  histogram on `report`).
- `Overhead` / `calibrate` — apparatus-overhead calibration
  via a two-point fit.
- Modules `pin` (CPU affinity) and `ticks` (hardware tick
  counter).

## Notes

- [`notes/design.md`](notes/design.md) — architecture and
  hot-path / cold-path split.
- [`notes/overhead-model.md`](notes/overhead-model.md) —
  formal overhead model: what framing is, what
  `loop_per_iter` is, which overhead is unmeasurable and
  stays in the measurement, and the current subtraction
  policy.

## Platform

x86_64 only; `ticks.rs` emits `compile_error!` on other
architectures.

## License

Dual-licensed under [MIT](LICENSE-MIT) OR
[Apache-2.0](LICENSE-APACHE).
