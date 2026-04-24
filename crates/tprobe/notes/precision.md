# Measurement precision and repeatability

Captures the precision floors a `tprobe` measurement is
bound by, the technique for determining empirical
repeatability across runs, and expected-scale heuristics
observed on this workspace's development hardware.

## Floors — the smallest things we can discriminate

- **Hardware tick resolution.** 1 TSC tick ≈ 0.27 ns on a
  3.7 GHz box. Nothing below this exists; it's the physical
  lower bound on discriminable time.
- **Apparatus framing noise.** Calibration fits
  `framing_ticks` with run-to-run noise — observed 21 → 29
  tick swings on the same box (~2 ns). That noise
  propagates directly to `adj mean` since framing is the
  bulk of the per-event correction at small `inner`. See
  [`overhead-model.md`](overhead-model.md)'s "Calibration
  preconditions" and "Framing" subsections.
- **OoO `rdtsc` coalescing.** Back-to-back `rdtsc` pairs
  with nothing serializing between them can measure 0–1
  ticks even when real work happened; the p1-p10 cluster
  at `inner=1` is exactly this. Not a bug — the probe
  reports what the hardware reported.
- **hdrhistogram bucket width.** 3 sig figs = 0.1 %
  relative precision. At 20 tk, bucket width ≈ 1 tk; at
  2,000 tk, ≈ 2 tk. Below ~1,000 tk, quantization is
  essentially exact.
- **Unmeasurable systemic noise.** Cache misses, context
  switches, interrupts (see
  [`overhead-model.md`](overhead-model.md)'s "Unmeasurable
  overhead" section). Shows up in the `p99-max` tail;
  handled in practice by preferring `mean min-p99` for A/B
  work.

## Technique — measuring repeatability empirically

For a given configuration, the right number is the
*run-to-run* stdev of `mean min-p99` (and/or
`adj mean min-p99`). Shell approximation without a built-in
flag:

```sh
for _ in $(seq 1 30); do
    goal1 2 -i 1000 -p 0 >> runs.txt
done
# extract `mean min-p99` from each report, compute sample stdev
```

The single-run `stdev min-p99` tells us *within-run* noise;
the *cross-run* stdev tells us the measurement's
repeatability. They aren't the same number — pinning
typically tightens within-run more than cross-run. The bot
thinks a useful heuristic for A/B work is:
**a difference smaller than `2 × cross-run stdev` is
probably not real.**

## Expected-scale heuristics (3900X, ~3.7 GHz)

The bot thinks the typical ranges on this box are:

- goal1 `-i 1000`: cross-run stdev ~0.2–0.5 ns on
  `mean min-p99` (~10 % of the signal).
- goal1 `-i 1`: cross-run stdev ~1–2 ns on
  `adj mean min-p99` — picks up the calibration-framing
  swing directly.
- goal2 pinned same-CCX (e.g. `--pin 1,12`): cross-run
  stdev ~30–100 tk on `mean min-p99` (~2–5 %).
- goal2 unpinned: cross-run stdev much higher (20–30 %
  swings observed across 1 s vs 20 s runs before pinning
  was applied).

Hardware-dependent numbers; compare against your own
baseline, not these.

## Future `--repeat N` flag

The bot thinks a built-in repeatability measurement would
be worth having: a `--repeat N` flag on `goal1` and `goal2`
that runs the full warmup+measurement sequence `N` times,
aggregates per-run summary lines, and prints cross-run mean
and stdev on `mean min-p99` and `adj mean min-p99`. Single
command, right number for A/B — replaces the shell loop
above and its one-off parsing. The bot thinks ~30–50 lines
across the two binaries, no probe-API change (just loops
the existing `run(warmup, measurement)` / `run_for` calls
and accumulates their reports).

Tracked in the workspace `notes/todo.md`.
