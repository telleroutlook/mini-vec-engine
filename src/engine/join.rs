//! Sort-merge join for sorted columnar batches.
//!
//! Both inputs must be pre-sorted by join key. Runs in O(N+M) time with no
//! hash table required. Inspired by DataFusion's `SortMergeJoinExec`.

/// Inner sort-merge join: emits rows only when keys match.
///
/// Returns `(left_key, left_payload, right_payload)` for every matching pair.
/// Handles duplicates on both sides via nested scans over equal-key runs.
pub fn sort_merge_join(
    left_key: &[i64],
    left_payload: &[i64],
    right_key: &[i64],
    right_payload: &[i64],
) -> Vec<(i64, i64, i64)> {
    debug_assert_eq!(left_key.len(), left_payload.len());
    debug_assert_eq!(right_key.len(), right_payload.len());

    let mut result = Vec::new();
    let mut li = 0usize;
    let mut ri = 0usize;

    while li < left_key.len() && ri < right_key.len() {
        let lk = left_key[li];
        let rk = right_key[ri];

        match lk.cmp(&rk) {
            std::cmp::Ordering::Less => {
                li += 1;
            }
            std::cmp::Ordering::Greater => {
                ri += 1;
            }
            std::cmp::Ordering::Equal => {
                let dup_ri = ri;

                while li < left_key.len() && left_key[li] == lk {
                    let rj_start = dup_ri;
                    let mut rj = rj_start;
                    while rj < right_key.len() && right_key[rj] == rk {
                        result.push((lk, left_payload[li], right_payload[rj]));
                        rj += 1;
                    }
                    li += 1;
                }
                ri = dup_ri;
                while ri < right_key.len() && right_key[ri] == rk {
                    ri += 1;
                }
            }
        }
    }

    result
}

/// Left outer sort-merge join: emits all left rows, with `None` for the right
/// payload when no matching right key exists.
///
/// Returns `(left_key, left_payload, Option<right_payload>)`.
pub fn left_outer_join(
    left_key: &[i64],
    left_payload: &[i64],
    right_key: &[i64],
    right_payload: &[i64],
) -> Vec<(i64, i64, Option<i64>)> {
    debug_assert_eq!(left_key.len(), left_payload.len());
    debug_assert_eq!(right_key.len(), right_payload.len());

    let mut result = Vec::new();
    let mut li = 0usize;
    let mut ri = 0usize;

    while li < left_key.len() {
        let lk = left_key[li];

        if ri < right_key.len() && right_key[ri] < lk {
            ri += 1;
            continue;
        }

        if ri < right_key.len() && lk == right_key[ri] {
            let dup_ri = ri;
            while li < left_key.len() && left_key[li] == lk {
                let mut rj = dup_ri;
                while rj < right_key.len() && right_key[rj] == lk {
                    result.push((lk, left_payload[li], Some(right_payload[rj])));
                    rj += 1;
                }
                li += 1;
            }
            ri = dup_ri;
            while ri < right_key.len() && right_key[ri] == lk {
                ri += 1;
            }
        } else {
            result.push((lk, left_payload[li], None));
            li += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_basic_match() {
        let lk = [1, 2, 3];
        let lp = [10, 20, 30];
        let rk = [2, 3, 4];
        let rp = [200, 300, 400];
        let got = sort_merge_join(&lk, &lp, &rk, &rp);
        assert_eq!(got, vec![(2, 20, 200), (3, 30, 300)]);
    }

    #[test]
    fn inner_no_match() {
        let lk = [1, 3, 5];
        let lp = [10, 30, 50];
        let rk = [2, 4, 6];
        let rp = [20, 40, 60];
        let got = sort_merge_join(&lk, &lp, &rk, &rp);
        assert!(got.is_empty());
    }

    #[test]
    fn inner_duplicates_both_sides() {
        let lk = [1, 1, 2, 2, 2];
        let lp = [10, 11, 20, 21, 22];
        let rk = [1, 1, 2, 2];
        let rp = [100, 101, 200, 201];
        let got = sort_merge_join(&lk, &lp, &rk, &rp);
        // key 1: 2 left x 2 right = 4 rows
        // key 2: 3 left x 2 right = 6 rows
        assert_eq!(got.len(), 10);
        let key1_count = got.iter().filter(|(k, _, _)| *k == 1).count();
        let key2_count = got.iter().filter(|(k, _, _)| *k == 2).count();
        assert_eq!(key1_count, 4);
        assert_eq!(key2_count, 6);
    }

    #[test]
    fn inner_empty_left() {
        let lk: &[i64] = &[];
        let lp: &[i64] = &[];
        let rk = [1, 2, 3];
        let rp = [10, 20, 30];
        let got = sort_merge_join(lk, lp, &rk, &rp);
        assert!(got.is_empty());
    }

    #[test]
    fn inner_empty_right() {
        let lk = [1, 2, 3];
        let lp = [10, 20, 30];
        let rk: &[i64] = &[];
        let rp: &[i64] = &[];
        let got = sort_merge_join(&lk, &lp, rk, rp);
        assert!(got.is_empty());
    }

    #[test]
    fn inner_both_empty() {
        let got = sort_merge_join(&[], &[], &[], &[]);
        assert!(got.is_empty());
    }

    #[test]
    fn inner_single_row_match() {
        let lk = [5];
        let lp = [50];
        let rk = [5];
        let rp = [500];
        let got = sort_merge_join(&lk, &lp, &rk, &rp);
        assert_eq!(got, vec![(5, 50, 500)]);
    }

    #[test]
    fn inner_single_row_no_match() {
        let lk = [1];
        let lp = [10];
        let rk = [2];
        let rp = [20];
        let got = sort_merge_join(&lk, &lp, &rk, &rp);
        assert!(got.is_empty());
    }

    #[test]
    fn left_outer_basic_match_and_unmatched() {
        let lk = [1, 2, 3, 5];
        let lp = [10, 20, 30, 50];
        let rk = [2, 4, 5];
        let rp = [200, 400, 500];
        let got = left_outer_join(&lk, &lp, &rk, &rp);
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], (1, 10, None));
        assert_eq!(got[1], (2, 20, Some(200)));
        assert_eq!(got[2], (3, 30, None));
        assert_eq!(got[3], (5, 50, Some(500)));
    }

    #[test]
    fn left_outer_all_unmatched() {
        let lk = [1, 3, 5];
        let lp = [10, 30, 50];
        let rk = [2, 4, 6];
        let rp = [20, 40, 60];
        let got = left_outer_join(&lk, &lp, &rk, &rp);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|(_, _, rp)| rp.is_none()));
    }

    #[test]
    fn left_outer_empty_right() {
        let lk = [1, 2, 3];
        let lp = [10, 20, 30];
        let got = left_outer_join(&lk, &lp, &[], &[]);
        assert_eq!(got.len(), 3);
        assert!(got.iter().all(|(_, _, rp)| rp.is_none()));
    }

    #[test]
    fn left_outer_empty_left() {
        let got = left_outer_join(&[], &[], &[1, 2, 3], &[10, 20, 30]);
        assert!(got.is_empty());
    }

    #[test]
    fn left_outer_duplicates_both_sides() {
        let lk = [1, 1, 2];
        let lp = [10, 11, 20];
        let rk = [1, 1];
        let rp = [100, 101];
        let got = left_outer_join(&lk, &lp, &rk, &rp);
        // key 1: 2 left x 2 right = 4 matched rows
        // key 2: 1 left x 0 right = 1 unmatched row
        assert_eq!(got.len(), 5);
        let matched = got.iter().filter(|(_, _, rp)| rp.is_some()).count();
        let unmatched = got.iter().filter(|(_, _, rp)| rp.is_none()).count();
        assert_eq!(matched, 4);
        assert_eq!(unmatched, 1);
    }
}
