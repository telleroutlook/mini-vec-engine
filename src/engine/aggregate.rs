use std::collections::HashMap;

use super::SelectionBitmap;
use crate::bitmap::Bitmap;

/// Evaluate `val > threshold` predicate, producing a selection bitmap.
pub fn evaluate_predicate(vals: &[i64], threshold: i64) -> SelectionBitmap {
    let mut sel = Bitmap::<{ super::BATCH_WORDS }>::zeroed();
    for (i, &v) in vals.iter().enumerate() {
        if v > threshold {
            sel.set(i);
        }
    }
    sel
}

/// Aggregate only rows indicated by the selection bitmap (late materialization).
pub fn aggregate_selected(
    keys: &[u32],
    vals: &[i64],
    selection: &SelectionBitmap,
    num_rows: usize,
    agg: &mut HashMap<u32, i64>,
) {
    for i in selection.iter_set_bits() {
        if i >= num_rows {
            break;
        }
        *agg.entry(unsafe { *keys.get_unchecked(i) }).or_insert(0) +=
            unsafe { *vals.get_unchecked(i) };
    }
}

/// Profiled wrapper: time `aggregate_selected` and return elapsed cycles.
#[cfg(feature = "profile")]
pub fn aggregate_selected_timed(
    keys: &[u32],
    vals: &[i64],
    selection: &SelectionBitmap,
    num_rows: usize,
    agg: &mut HashMap<u32, i64>,
) -> u64 {
    let t = crate::timing::OperationTimer::new();
    aggregate_selected(keys, vals, selection, num_rows, agg);
    t.elapsed()
}

/// Two-phase merge: combine thread-local hash maps into a global result.
pub fn merge_maps(maps: &[HashMap<u32, i64>]) -> HashMap<u32, i64> {
    let mut global = HashMap::new();
    for map in maps {
        for (&k, &v) in map {
            *global.entry(k).or_insert(0) += v;
        }
    }
    global
}
