# Overhead model

Definitions and policy for what `tprobe` calls "overhead"
and what gets subtracted from measured values.

## Measured quantities

Calibration runs a two-point fit on an empty `black_box(1)`
bench and produces two numbers, both recorded on the
`Overhead` struct.

### Framing — `framing_ticks` (per scope)

The hardware cost of the two `rdtsc` reads that bracket
every `TProbe` scope. Paid once per scope, independent of
how many events the scope covers. On modern x86 this is
~20 ticks (~5 ns at ~3.7 GHz).

Why two-point (and not single-point)? `rdtsc` is not a
serializing instruction, so two back-to-back reads with
nothing between them can execute out-of-order and measure
as little as 0–1 ticks — well below the true apparatus
cost. Bracketing with `_mm_lfence` drains the pipeline
harder than real work does and overcorrects. The two-point
fit avoids both biases by varying an `inner` size and
solving

```
raw_delta = framing + inner · loop_per_iter
```

in `f64` arithmetic (no integer truncation) across two
empty benches at `N_LOW = 100` and `N_HIGH = 10_000`. The
intercept is framing; the slope is `loop_per_iter` (below).

Noise amplification on the intercept is
`d(framing) / d(raw_low) = N_HIGH / (N_HIGH − N_LOW) ≈ 1.01`
with the current constants — any slop on `raw_low`
propagates roughly 1:1 to framing.

### Loop-per-iter — `loop_per_iter_ticks` (per inner iter)

The slope of the same fit — the per-iteration cost of the
calibration's empty inner loop, one `black_box(1)` per
iteration. On modern x86 this is small, on the order of one
tick per iter.

It models the loop scaffolding a real measurement loop also
pays: loop branch, counter increment, plus whatever cost a
"does-nothing" iteration has. It is not a perfect model of
a real dispatch loop — the real loop doesn't execute the
`black_box(1)` itself, so its per-iter scaffolding is
slightly cheaper than what calibration reports.

## Unmeasurable overhead

Things calibration does not separately measure and that are
not separately subtractable from stored scope deltas:

- Cache misses and branch mispredicts inside the scope.
- Context switches and OS interrupts during the scope.
- Microarchitectural noise (TLB misses, frequency changes,
  thermal throttling).

These are part of what the real system experiences and
remain in the measurement. The bot thinks they typically
manifest in the `p99-max` tail of the histogram — which is
why the band-table report also prints `mean min-p99`
(whole-distribution statistics excluding the top 1 % tail,
robust against rare outliers).

No future calibration can separate these per-event without
a fundamentally different measurement technique; they are
the irreducible noise floor of "whatever happened between
the two `rdtsc` reads". Keeping them in the measurement is
honest — they are part of the real cost the measured code
pays.

## Subtraction policy

`Overhead::per_event_ticks(batch)` returns the per-event
correction `TProbe::report` subtracts from every stored
value:

```
correction = framing_ticks / batch + loop_per_iter_ticks
                └── amortized       └── paid per inner
                    over the scope      iteration
```

At `batch = 1` the framing term is paid in full; at
`batch = N` it is spread across `N` events and rounds
toward zero as `N` grows. `loop_per_iter_ticks` is added
back because the real measurement loop pays per-iter
scaffolding (loop branch + counter increment) that the
empty-bench fit captures. The sum is rounded to the
nearest tick.

The bot thinks this matches "cost of the work alone" more
closely than framing-only subtraction: the real loop's
per-iter scaffolding is structural overhead relative to
the dispatch work, and `black_box(1)`'s iter cost is a
reasonable proxy. The tradeoff is a small downward bias —
the real loop doesn't execute the `black_box(1)` itself,
so the measured `loop_per_iter_ticks` is slightly larger
than the real loop's scaffolding. The bot thinks the bias
is on the order of a tick per event and is worth the
simpler story in the output.

## Per-event correction when batches mix

Today every record in a given probe drain carries the same
`batch` (the runtime commits to one `inner` per run), so
`correction` is a single constant across the drain. If a
future probe ever mixes `batch` values within one drain —
e.g. an actor whose scopes cover both fast-path singles
(`batch = 1`, framing term paid in full) and slow-path
batches (`batch = 1000`, framing term ≈ 0) — the correct
overhead-adjusted mean per band requires per-record
correction tracked per band, not a single global average.
The bot thinks the change at that point is:

- drain loop accumulates `band_correction_sum[i]` alongside
  `band_sum[i]`,
- `adj_mean[i] = (band_sum[i] - band_correction_sum[i]) /
  band_count[i]`.

Deferred until that situation arises.
