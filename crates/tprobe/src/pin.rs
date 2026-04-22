//! Thread CPU-pinning helpers: `--pin` parsing and per-thread
//! affinity setting.
//!
//! Adapted from `../iiac-perf/src/pin.rs`. The affinity
//! save/restore, diagnostics, and human-readable summary helpers
//! are intentionally elided — Stage 1 only needs parse + pin.
//! Revisit if/when the perf probes get a `--no-pin-cal` equivalent.

/// Parse a `--pin` value (comma-separated list with optional
/// ranges) into an ordered `Vec` of logical CPU ids. Accepts
/// forms like `"0,1"`, `"0-11"`, `"0,3-5,7"`. Duplicates are
/// preserved in order so oversubscription (`"0,0,0"`) works.
pub fn parse_cores(spec: &str) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match part.split_once('-') {
            None => out.push(
                part.parse::<usize>()
                    .map_err(|e| format!("invalid core id {part:?}: {e}"))?,
            ),
            Some((lo, hi)) => {
                let lo = lo
                    .trim()
                    .parse::<usize>()
                    .map_err(|e| format!("invalid range start {lo:?}: {e}"))?;
                let hi = hi
                    .trim()
                    .parse::<usize>()
                    .map_err(|e| format!("invalid range end {hi:?}: {e}"))?;
                if hi < lo {
                    return Err(format!("range {lo}-{hi} is empty"));
                }
                out.extend(lo..=hi);
            }
        }
    }
    Ok(out)
}

/// Pin the current thread to `logical_cpu`. No-op if `None`.
///
/// Constructs a `CoreId` directly rather than querying
/// `core_affinity::get_core_ids()`, which only returns cores in
/// the caller's current affinity mask — after the first pin that
/// mask is narrowed and subsequent lookups for other cores would
/// fail.
pub fn pin_current(logical_cpu: Option<usize>) {
    let Some(target) = logical_cpu else { return };
    core_affinity::set_for_current(core_affinity::CoreId { id: target });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_list() {
        assert_eq!(parse_cores("0,1,2").unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn parse_range() {
        assert_eq!(parse_cores("0-3").unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_mixed() {
        assert_eq!(parse_cores("0,3-5,7").unwrap(), vec![0, 3, 4, 5, 7]);
    }

    #[test]
    fn parse_duplicates_preserved() {
        assert_eq!(parse_cores("0,0,0").unwrap(), vec![0, 0, 0]);
    }

    #[test]
    fn parse_empty_string_ok() {
        assert_eq!(parse_cores("").unwrap(), Vec::<usize>::new());
    }

    #[test]
    fn parse_reverse_range_errs() {
        assert!(parse_cores("5-3").is_err());
    }

    #[test]
    fn parse_garbage_errs() {
        assert!(parse_cores("abc").is_err());
        assert!(parse_cores("1-x").is_err());
    }
}
