//! Shared band-table renderer for tick-valued histograms.
//!
//! [`TProbe`] (scope API, records → drain) stores hardware tick
//! deltas and wants a consistent band-table output shape —
//! min/p1/…/p99/max rows with first/last/range/count/mean
//! columns, plus summary lines for mean, stdev, mean min-p99,
//! stdev min-p99. This module provides the implementation.
//!
//! (Upstream `iiac-perf` also has a `TProbe` fast-path/direct-
//! histogram variant that shares this renderer; `tprobe` here
//! only vendored the scope-API probe.)
//!
//! Display unit is chosen by `as_ticks`: `false` converts stored
//! tick values to nanoseconds via [`crate::ticks::ticks_per_ns`];
//! `true` shows raw ticks.

use hdrhistogram::Histogram;

use crate::fmt::{fmt_commas, fmt_commas_f64};
use crate::ticks;

const BOUNDARY_PCTS: &[f64] = &[
    0.0, 0.01, 0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 0.99, 1.0,
];
const BOUNDARY_NAMES: &[&str] = &[
    "min", "p1", "p10", "p20", "p30", "p40", "p50", "p60", "p70", "p80", "p90", "p99", "max",
];

/// Render a band-table report for `hist`, interpreting stored
/// values as hardware ticks (raw — no overhead subtraction is
/// applied to histogram-stored values). `kind` is the header
/// label (e.g. `"tprobe"`) and `name` is the probe's name.
/// `as_ticks=false` converts to ns; `true` keeps raw ticks.
///
/// `correction` (in ticks) is the per-event overhead to display
/// in an `adj mean` column alongside the raw mean. If `Some(c)`,
/// each band's / summary row's raw mean has `c` subtracted
/// (clamped at 0) for the `adj mean` column. `stdev` rows leave
/// that column blank — stdev is shift-invariant. If `None`, no
/// adj column is rendered and the output matches the pre-0.3.0
/// layout.
///
/// `decimals` controls the fractional precision of numeric
/// cells. `None` uses the smart default: `0` in ticks mode
/// (integers are already exact), `1` in ns mode (so sub-ns
/// detail is visible rather than rounded away). `Some(n)`
/// overrides both modes.
pub(crate) fn render(
    kind: &str,
    name: &str,
    hist: &Histogram<u64>,
    as_ticks: bool,
    correction: Option<u64>,
    decimals: Option<usize>,
) {
    let sample_count = hist.len();
    println!("  {kind}: {name} [count={}]", fmt_commas(sample_count));
    if sample_count == 0 {
        println!();
        return;
    }

    let unit = if as_ticks { "tk" } else { "ns" };
    let tpn = ticks::ticks_per_ns();
    let conv = |v: u64| -> f64 { if as_ticks { v as f64 } else { v as f64 / tpn } };
    let conv_f = |v: f64| -> f64 { if as_ticks { v } else { v / tpn } };
    // Smart default: integer ticks are already precise, ns loses
    // sub-ns resolution at 0 decimals (1 tk ≈ 0.26 ns renders as
    // "0 ns"), so ns defaults to 1 decimal. Explicit `--decimals N`
    // overrides.
    let decimals = decimals.unwrap_or(if as_ticks { 0 } else { 1 });
    let correction_ticks_f = correction.map(|c| c as f64).unwrap_or(0.0);
    let show_adj = correction.is_some();

    let n_bands = BOUNDARY_PCTS.len() - 1;
    let mut band_first = vec![u64::MAX; n_bands];
    let mut band_last = vec![0u64; n_bands];
    let mut band_count = vec![0u64; n_bands];
    let mut band_sum = vec![0u128; n_bands];

    let mut cumulative = 0u64;
    for iv in hist.iter_recorded() {
        let value = iv.value_iterated_to();
        let count = iv.count_at_value();
        let mid_rank = (cumulative as f64 + count as f64 / 2.0) / sample_count as f64;
        let idx = BOUNDARY_PCTS[1..]
            .iter()
            .position(|&b| mid_rank < b)
            .unwrap_or(n_bands - 1); // OK: mid_rank in [0,1]; last boundary is 1.0 so position() only misses at the exact upper edge, which maps to the last band
        band_first[idx] = band_first[idx].min(value);
        band_last[idx] = band_last[idx].max(value);
        band_count[idx] += count;
        band_sum[idx] += value as u128 * count as u128;
        cumulative += count;
    }

    struct BandRow {
        label: String,
        first: String,
        last: String,
        range: String,
        count: String,
        mean: String,
        adj_mean: String,
    }

    let mut rows: Vec<BandRow> = Vec::new();
    for i in 0..n_bands {
        let count = band_count[i];
        let (first_v, last_v, range_v, mean_v, adj_mean_v) = if count == 0 {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        } else {
            let mean_ticks = band_sum[i] as f64 / count as f64;
            let adj_ticks = (mean_ticks - correction_ticks_f).max(0.0);
            let range_raw = band_last[i] - band_first[i] + 1;
            (
                conv(band_first[i]),
                conv(band_last[i]),
                conv(range_raw),
                conv_f(mean_ticks),
                conv_f(adj_ticks),
            )
        };
        rows.push(BandRow {
            label: format!("{}-{}", BOUNDARY_NAMES[i], BOUNDARY_NAMES[i + 1]),
            first: fmt_commas_f64(first_v, decimals),
            last: fmt_commas_f64(last_v, decimals),
            range: fmt_commas_f64(range_v, decimals),
            count: fmt_commas(count),
            mean: fmt_commas_f64(mean_v, decimals),
            adj_mean: fmt_commas_f64(adj_mean_v, decimals),
        });
    }

    // Summary row values. Raw strings always; adj strings only
    // meaningful when `show_adj`.
    let hist_mean_ticks = hist.mean();
    let hist_stdev_ticks = hist.stdev();
    let hist_mean_str = fmt_commas_f64(conv_f(hist_mean_ticks), decimals);
    let hist_mean_adj_str = fmt_commas_f64(
        conv_f((hist_mean_ticks - correction_ticks_f).max(0.0)),
        decimals,
    );
    let hist_stdev_str = fmt_commas_f64(conv_f(hist_stdev_ticks), decimals);

    // Trimmed (min-p99) stats. `have_trim` is true when the
    // first 11 bands hold any samples; if all samples land in
    // `p99-max`, the trimmed summary is skipped.
    let trim_count: u64 = band_count[..n_bands - 1].iter().sum();
    let (trim_mean_str, trim_mean_adj_str, trim_stdev_str, have_trim) = if trim_count > 0 {
        let trim_sum: u128 = band_sum[..n_bands - 1].iter().sum();
        let trim_mean = trim_sum as f64 / trim_count as f64;

        let mut trim_var_sum = 0.0f64;
        let mut trim_var_count = 0u64;
        let mut cum = 0u64;
        for iv in hist.iter_recorded() {
            let value = iv.value_iterated_to();
            let count = iv.count_at_value();
            let mid_rank = (cum as f64 + count as f64 / 2.0) / sample_count as f64;
            let idx = BOUNDARY_PCTS[1..]
                .iter()
                .position(|&b| mid_rank < b)
                .unwrap_or(n_bands - 1); // OK: mid_rank in [0,1]; same reasoning as above
            if idx < n_bands - 1 {
                let diff = value as f64 - trim_mean;
                trim_var_sum += diff * diff * count as f64;
                trim_var_count += count;
            }
            cum += count;
        }
        let trim_stdev = if trim_var_count > 1 {
            (trim_var_sum / trim_var_count as f64).sqrt()
        } else {
            0.0
        };
        let trim_mean_adj = (trim_mean - correction_ticks_f).max(0.0);
        (
            fmt_commas_f64(conv_f(trim_mean), decimals),
            fmt_commas_f64(conv_f(trim_mean_adj), decimals),
            fmt_commas_f64(conv_f(trim_stdev), decimals),
            true,
        )
    } else {
        (String::new(), String::new(), String::new(), false)
    };

    let label_w = rows
        .iter()
        .map(|r| r.label.len())
        .max()
        .unwrap_or(0) // OK: rows always contains n_bands=12 entries after the loop above
        .max("stdev min-p99".len());
    let first_w = rows.iter().map(|r| r.first.len()).max().unwrap_or(0); // OK: rows always contains n_bands=12 entries
    let last_w = rows.iter().map(|r| r.last.len()).max().unwrap_or(0); // OK: rows always contains n_bands=12 entries
    let range_w = rows.iter().map(|r| r.range.len()).max().unwrap_or(0); // OK: rows always contains n_bands=12 entries
    let count_w = rows.iter().map(|r| r.count.len()).max().unwrap_or(0); // OK: rows always contains n_bands=12 entries
    let mean_w = rows
        .iter()
        .map(|r| r.mean.len())
        .max()
        .unwrap_or(0) // OK: rows always contains n_bands=12 entries
        .max(hist_mean_str.len())
        .max(hist_stdev_str.len())
        .max(trim_mean_str.len())
        .max(trim_stdev_str.len());
    let adj_w = if show_adj {
        rows.iter()
            .map(|r| r.adj_mean.len())
            .max()
            .unwrap_or(0) // OK: rows always contains n_bands=12 entries
            .max(hist_mean_adj_str.len())
            .max(trim_mean_adj_str.len())
    } else {
        0
    };

    const INDENT: &str = "    ";
    const GAP: &str = "    ";
    // Extra separation before the `adj mean` column so the
    // derived value is visually distinct from the raw columns.
    const ADJ_SEP: &str = "        ";

    let first_col = INDENT.len() + label_w + 1 + first_w;
    let unit_len = 1 + unit.len();
    let last_gap = unit_len + GAP.len() + last_w;
    let range_gap = unit_len + GAP.len() + range_w;
    let count_gap = unit_len + GAP.len() + count_w;
    let mean_gap = GAP.len() + mean_w;
    // Data-cell width in the adj column, including the trailing
    // space + unit (e.g. ` ns`). Used for blank-cell alignment on
    // stdev rows.
    let adj_cell_w = adj_w + unit_len;
    if show_adj {
        let adj_gap = unit_len + ADJ_SEP.len() + adj_w;
        println!(
            "{:>first_col$}{:>last_gap$}{:>range_gap$}{:>count_gap$}{:>mean_gap$}{:>adj_gap$}",
            "first", "last", "range", "count", "mean", "adj mean",
        );
    } else {
        println!(
            "{:>first_col$}{:>last_gap$}{:>range_gap$}{:>count_gap$}{:>mean_gap$}",
            "first", "last", "range", "count", "mean",
        );
    }

    for r in &rows {
        if show_adj {
            println!(
                "{INDENT}{:<label_w$} {:>first_w$} {unit}{GAP}{:>last_w$} {unit}{GAP}{:>range_w$} {unit}{GAP}{:>count_w$}{GAP}{:>mean_w$} {unit}{ADJ_SEP}{:>adj_w$} {unit}",
                r.label, r.first, r.last, r.range, r.count, r.mean, r.adj_mean,
            );
        } else {
            println!(
                "{INDENT}{:<label_w$} {:>first_w$} {unit}{GAP}{:>last_w$} {unit}{GAP}{:>range_w$} {unit}{GAP}{:>count_w$}{GAP}{:>mean_w$} {unit}",
                r.label, r.first, r.last, r.range, r.count, r.mean,
            );
        }
    }

    let skip = first_w
        + unit_len
        + GAP.len()
        + last_w
        + unit_len
        + GAP.len()
        + range_w
        + unit_len
        + GAP.len()
        + count_w;
    if show_adj {
        println!(
            "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}{ADJ_SEP}{:>adj_w$} {unit}",
            "mean", "", hist_mean_str, hist_mean_adj_str,
        );
        println!(
            "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}{ADJ_SEP}{:>adj_cell_w$}",
            "stdev", "", hist_stdev_str, "",
        );
    } else {
        println!(
            "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}",
            "mean", "", hist_mean_str,
        );
        println!(
            "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}",
            "stdev", "", hist_stdev_str,
        );
    }

    if have_trim {
        if show_adj {
            println!(
                "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}{ADJ_SEP}{:>adj_w$} {unit}",
                "mean min-p99", "", trim_mean_str, trim_mean_adj_str,
            );
            println!(
                "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}{ADJ_SEP}{:>adj_cell_w$}",
                "stdev min-p99", "", trim_stdev_str, "",
            );
        } else {
            println!(
                "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}",
                "mean min-p99", "", trim_mean_str,
            );
            println!(
                "{INDENT}{:<label_w$} {:>skip$}{GAP}{:>mean_w$} {unit}",
                "stdev min-p99", "", trim_stdev_str,
            );
        }
    }
    println!();
}
