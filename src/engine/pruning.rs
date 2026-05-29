//! Statistical batch pruning — skip batches whose min/max statistics prove
//! no row can possibly match a filter expression.

use super::expr::{BinOp, Expr, ScalarValue};

/// Per-column statistics for a batch of rows.
pub struct BatchStatistics {
    pub min: Vec<i64>,
    pub max: Vec<i64>,
    pub count: usize,
}

impl BatchStatistics {
    pub fn new(min: Vec<i64>, max: Vec<i64>, count: usize) -> Self {
        debug_assert_eq!(min.len(), max.len());
        Self { min, max, count }
    }

    /// Compute statistics from a columnar batch.
    ///
    /// `columns[col_idx]` holds the i64 values for column `col_idx`.
    /// `num_rows` bounds the valid row range.
    pub fn from_batch(columns: &[Vec<i64>], num_rows: usize) -> Self {
        let num_cols = columns.len();
        let mut min = vec![i64::MAX; num_cols];
        let mut max = vec![i64::MIN; num_cols];

        if num_rows == 0 {
            return Self {
                min: vec![0; num_cols],
                max: vec![0; num_cols],
                count: 0,
            };
        }

        for col_idx in 0..num_cols {
            let col = &columns[col_idx];
            let len = col.len().min(num_rows);
            for &v in col.iter().take(len) {
                if v < min[col_idx] {
                    min[col_idx] = v;
                }
                if v > max[col_idx] {
                    max[col_idx] = v;
                }
            }
        }

        Self {
            min,
            max,
            count: num_rows,
        }
    }

    /// Returns true if the expression can never match given these statistics.
    ///
    /// Conservative: returns `false` (cannot prune) when unsure.
    pub fn can_prune(expr: &Expr, stats: &Self) -> bool {
        if stats.count == 0 {
            return true;
        }

        match expr {
            Expr::Column(_) | Expr::Literal(_) => false,

            Expr::BinaryOp { op, left, right } => {
                // We only handle the pattern (Column(idx) op Literal(val)).
                if let Expr::Column(idx) = left.as_ref() {
                    if let Some(val) = literal_i64(right) {
                        return can_prune_col_op(*idx, *op, val, stats);
                    }
                }
                // Also handle (Literal(val) op Column(idx)) by flipping.
                if let Expr::Column(idx) = right.as_ref() {
                    if let Some(val) = literal_i64(left) {
                        let flipped = match op {
                            BinOp::Lt => BinOp::Gt,
                            BinOp::LtEq => BinOp::GtEq,
                            BinOp::Gt => BinOp::Lt,
                            BinOp::GtEq => BinOp::LtEq,
                            other => *other,
                        };
                        return can_prune_col_op(*idx, flipped, val, stats);
                    }
                }
                false
            }

            Expr::Not(_) => false,

            Expr::And(children) => children.iter().any(|c| Self::can_prune(c, stats)),

            Expr::Or(children) => {
                if children.is_empty() {
                    return true;
                }
                children.iter().all(|c| Self::can_prune(c, stats))
            }

            Expr::In(col_idx, values) => {
                if col_idx >= &stats.min.len() || values.is_empty() {
                    return col_idx >= &stats.min.len();
                }
                let col_min = stats.min[*col_idx];
                let col_max = stats.max[*col_idx];
                // If every value falls outside [col_min, col_max], no match possible.
                values.iter().all(|&v| v < col_min || v > col_max)
            }
        }
    }
}

/// Extract the i64 value from a Literal expression, if applicable.
fn literal_i64(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Literal(ScalarValue::I64(v)) => Some(*v),
        _ => None,
    }
}

/// Core pruning logic for `columns[col_idx] OP val` given batch min/max.
fn can_prune_col_op(col_idx: usize, op: BinOp, val: i64, stats: &BatchStatistics) -> bool {
    if col_idx >= stats.min.len() {
        return false;
    }
    let col_min = stats.min[col_idx];
    let col_max = stats.max[col_idx];

    match op {
        BinOp::Gt => col_max <= val,
        BinOp::Lt => col_min >= val,
        BinOp::Eq => val < col_min || val > col_max,
        BinOp::GtEq => col_max < val,
        BinOp::LtEq => col_min > val,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(min: Vec<i64>, max: Vec<i64>, count: usize) -> BatchStatistics {
        BatchStatistics::new(min, max, count)
    }

    #[test]
    fn prune_gt_all_below() {
        // col0 > 200, but max[col0] = 150 -> can prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(200))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn no_prune_gt_some_above() {
        // col0 > 50, max = 150 -> cannot prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(50))),
        };
        assert!(!BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_lt_all_above() {
        // col0 < 5, but min = 10 -> can prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Lt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(5))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_eq_out_of_range() {
        // col0 == 200, but max = 150 -> can prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(200))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));

        // col0 == 5, but min = 10 -> can prune
        let expr2 = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(5))),
        };
        assert!(BatchStatistics::can_prune(&expr2, &stats));
    }

    #[test]
    fn no_prune_eq_in_range() {
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(50))),
        };
        assert!(!BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_gte() {
        // col0 >= 200, max = 150 -> can prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::GtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(200))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));

        // col0 >= 150, max = 150 -> cannot prune (150 >= 150 is true)
        let expr2 = Expr::BinaryOp {
            op: BinOp::GtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(150))),
        };
        assert!(!BatchStatistics::can_prune(&expr2, &stats));
    }

    #[test]
    fn prune_lte() {
        // col0 <= 5, min = 10 -> can prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::LtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(5))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_and_any_child() {
        // (col0 > 200) AND (col1 < 5) -> col0 > 200 is prunable
        let stats = make_stats(vec![10, 20], vec![150, 100], 5);
        let expr = Expr::And(vec![
            Expr::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(200))),
            },
            Expr::BinaryOp {
                op: BinOp::Lt,
                left: Box::new(Expr::Column(1)),
                right: Box::new(Expr::Literal(ScalarValue::I64(5))),
            },
        ]);
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_or_all_children() {
        // (col0 > 200) OR (col0 < 5) -> both prunable
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::Or(vec![
            Expr::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(200))),
            },
            Expr::BinaryOp {
                op: BinOp::Lt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(5))),
            },
        ]);
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn no_prune_or_some_matchable() {
        // (col0 > 200) OR (col0 < 50) -> first is prunable, second is not
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::Or(vec![
            Expr::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(200))),
            },
            Expr::BinaryOp {
                op: BinOp::Lt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(50))),
            },
        ]);
        assert!(!BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn no_prune_not() {
        // NOT(col0 > 200) -> conservative: cannot prune
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::Not(Box::new(Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(200))),
        }));
        assert!(!BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_empty_batch() {
        let stats = make_stats(vec![0], vec![0], 0);
        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(0))),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn prune_empty_or() {
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::Or(vec![]);
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }

    #[test]
    fn from_batch_computes_min_max() {
        let columns: Vec<Vec<i64>> = vec![vec![10, 50, -5, 200, 30], vec![0, -100, 50, 25, 75]];
        let stats = BatchStatistics::from_batch(&columns, 5);
        assert_eq!(stats.count, 5);
        assert_eq!(stats.min[0], -5);
        assert_eq!(stats.max[0], 200);
        assert_eq!(stats.min[1], -100);
        assert_eq!(stats.max[1], 75);
    }

    #[test]
    fn from_batch_respects_num_rows() {
        let columns: Vec<Vec<i64>> = vec![vec![10, 50, 1000]];
        // Only consider first 2 rows
        let stats = BatchStatistics::from_batch(&columns, 2);
        assert_eq!(stats.min[0], 10);
        assert_eq!(stats.max[0], 50);
        assert_eq!(stats.count, 2);
    }

    #[test]
    fn from_batch_empty() {
        let columns: Vec<Vec<i64>> = vec![vec![]];
        let stats = BatchStatistics::from_batch(&columns, 0);
        assert_eq!(stats.count, 0);
    }

    #[test]
    fn prune_commuted_literal() {
        // 200 < col0 is equivalent to col0 > 200
        let stats = make_stats(vec![10], vec![150], 5);
        let expr = Expr::BinaryOp {
            op: BinOp::Lt,
            left: Box::new(Expr::Literal(ScalarValue::I64(200))),
            right: Box::new(Expr::Column(0)),
        };
        assert!(BatchStatistics::can_prune(&expr, &stats));
    }
}
