//! Expression evaluation framework for vectorized predicate pushdown.
//!
//! Inspired by DataFusion's `Expr` tree: represents filter predicates as an AST
//! that can be evaluated against columnar batches to produce selection bitmaps.
//! Late materialization is baked in -- only rows in the input `selection` are
//! evaluated, and the result is a bitmap of surviving rows.

use crate::bitmap::Bitmap;

use super::BATCH_WORDS;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Scalar constant that can appear in an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    I64(i64),
    F64(f64),
    Bool(bool),
    Null,
}

/// Binary operators supported in expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Add,
    Sub,
    Mul,
    Div,
}

/// Expression tree — the core IR for filter predicates.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Reference to a column by positional index into the columns slice.
    Column(usize),
    /// Constant value.
    Literal(ScalarValue),
    /// Binary operator applied to two sub-expressions.
    BinaryOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Logical NOT.
    Not(Box<Expr>),
    /// Conjunction (AND) of zero or more children.
    /// Empty vec is treated as all-ones (vacuously true).
    And(Vec<Expr>),
    /// Disjunction (OR) of zero or more children.
    /// Empty vec is treated as all-zeros (vacuously false).
    Or(Vec<Expr>),
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Evaluate `expr` against `columns` considering only the rows set in
/// `selection`. Returns a bitmap of rows that pass the filter.
///
/// `num_rows` bounds the valid row range; bits beyond `num_rows` are guaranteed
/// to be clear in the returned bitmap.
pub fn evaluate_expr(
    expr: &Expr,
    columns: &[Vec<i64>],
    selection: &Bitmap<BATCH_WORDS>,
    num_rows: usize,
) -> Bitmap<BATCH_WORDS> {
    match expr {
        Expr::Column(idx) => {
            let col = &columns[*idx];
            let mut result = Bitmap::zeroed();
            for row in selection.iter_set_bits() {
                if row >= num_rows {
                    break;
                }
                // A bare column reference is truthy if the value is non-zero.
                if col[row] != 0 {
                    result.set(row);
                }
            }
            result
        }

        Expr::Literal(sv) => {
            let truthy = match sv {
                ScalarValue::Bool(b) => *b,
                ScalarValue::I64(v) => *v != 0,
                ScalarValue::F64(v) => *v != 0.0,
                ScalarValue::Null => false,
            };
            if truthy {
                selection.clone()
            } else {
                Bitmap::zeroed()
            }
        }

        Expr::BinaryOp { op, left, right } => {
            // Fast path: detect the common pattern (col op literal) to avoid
            // materializing intermediate bitmaps for both sides.
            if let (Expr::Column(lidx), Expr::Literal(rv)) = (left.as_ref(), right.as_ref()) {
                return eval_col_bin_literal(*lidx, *op, rv, columns, selection, num_rows);
            }
            if let (Expr::Literal(lv), Expr::Column(ridx)) = (left.as_ref(), right.as_ref()) {
                // Flip comparison direction for commutative/ordered ops.
                let flipped = match op {
                    BinOp::Lt => BinOp::Gt,
                    BinOp::LtEq => BinOp::GtEq,
                    BinOp::Gt => BinOp::Lt,
                    BinOp::GtEq => BinOp::LtEq,
                    other => *other,
                };
                return eval_col_bin_literal(*ridx, flipped, lv, columns, selection, num_rows);
            }

            // General case: evaluate both sides to bitmaps, then combine.
            let lb = evaluate_expr(left, columns, selection, num_rows);
            let rb = evaluate_expr(right, columns, selection, num_rows);
            // For comparison ops, zip through both bitmaps.
            eval_binop_bitmaps(op, &lb, &rb)
        }

        Expr::Not(inner) => {
            let inner_result = evaluate_expr(inner, columns, selection, num_rows);
            // NOT only within the valid selection mask.
            selection.and(&inner_result.not())
        }

        Expr::And(children) => {
            if children.is_empty() {
                // Vacuously true: all selected rows pass.
                return selection.clone();
            }
            let mut result = evaluate_expr(&children[0], columns, selection, num_rows);
            for child in &children[1..] {
                let child_result = evaluate_expr(child, columns, selection, num_rows);
                result = result.and(&child_result);
            }
            result
        }

        Expr::Or(children) => {
            if children.is_empty() {
                return Bitmap::zeroed();
            }
            let mut result = evaluate_expr(&children[0], columns, selection, num_rows);
            for child in &children[1..] {
                let child_result = evaluate_expr(child, columns, selection, num_rows);
                result = result.or(&child_result);
            }
            result
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Evaluate `columns[col_idx] OP literal` for each selected row.
fn eval_col_bin_literal(
    col_idx: usize,
    op: BinOp,
    rv: &ScalarValue,
    columns: &[Vec<i64>],
    selection: &Bitmap<BATCH_WORDS>,
    num_rows: usize,
) -> Bitmap<BATCH_WORDS> {
    let col = &columns[col_idx];
    let mut result = Bitmap::zeroed();

    let rv_i64 = match rv {
        ScalarValue::I64(v) => Some(*v),
        _ => None,
    };

    match op {
        BinOp::Gt => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] > rv {
                        result.set(row);
                    }
                }
            }
        }
        BinOp::GtEq => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] >= rv {
                        result.set(row);
                    }
                }
            }
        }
        BinOp::Lt => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] < rv {
                        result.set(row);
                    }
                }
            }
        }
        BinOp::LtEq => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] <= rv {
                        result.set(row);
                    }
                }
            }
        }
        BinOp::Eq => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] == rv {
                        result.set(row);
                    }
                }
            }
        }
        BinOp::NotEq => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    if col[row] != rv {
                        result.set(row);
                    }
                }
            }
        }
        // Arithmetic ops on col vs literal produce values, not bitmaps.
        // For simplicity, treat result as truthy (non-zero).
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
            if let Some(rv) = rv_i64 {
                for row in selection.iter_set_bits() {
                    if row >= num_rows {
                        break;
                    }
                    let val = match op {
                        BinOp::Add => col[row].wrapping_add(rv),
                        BinOp::Sub => col[row].wrapping_sub(rv),
                        BinOp::Mul => col[row].wrapping_mul(rv),
                        BinOp::Div => {
                            if rv == 0 {
                                0
                            } else {
                                col[row] / rv
                            }
                        }
                        _ => unreachable!(),
                    };
                    if val != 0 {
                        result.set(row);
                    }
                }
            }
        }
    }
    result
}

/// Combine two bitmaps according to a binary operator.
/// Used for the general (non-fast-path) case.
fn eval_binop_bitmaps(
    op: &BinOp,
    left: &Bitmap<BATCH_WORDS>,
    right: &Bitmap<BATCH_WORDS>,
) -> Bitmap<BATCH_WORDS> {
    match op {
        BinOp::Eq => left.and(right),
        BinOp::NotEq => {
            // XOR: true where exactly one side is true.
            let both = left.and(right);
            left.or(right).and(&both.not())
        }
        BinOp::Gt => left.and(&right.not()),
        BinOp::GtEq => left.clone(),
        BinOp::Lt => right.and(&left.not()),
        BinOp::LtEq => right.clone(),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => left.or(right),
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

/// Convert an expression tree to a human-readable string for debugging.
pub fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Column(idx) => format!("col{}", idx),
        Expr::Literal(sv) => match sv {
            ScalarValue::I64(v) => v.to_string(),
            ScalarValue::F64(v) => format!("{:.6}", v),
            ScalarValue::Bool(b) => b.to_string(),
            ScalarValue::Null => "NULL".to_string(),
        },
        Expr::BinaryOp { op, left, right } => {
            let op_str = match op {
                BinOp::Eq => "=",
                BinOp::NotEq => "!=",
                BinOp::Lt => "<",
                BinOp::LtEq => "<=",
                BinOp::Gt => ">",
                BinOp::GtEq => ">=",
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
            };
            format!("({} {} {})", expr_to_string(left), op_str, expr_to_string(right))
        }
        Expr::Not(inner) => format!("NOT ({})", expr_to_string(inner)),
        Expr::And(children) => {
            if children.is_empty() {
                return "true".to_string();
            }
            let parts: Vec<_> = children.iter().map(expr_to_string).collect();
            format!("({})", parts.join(" AND "))
        }
        Expr::Or(children) => {
            if children.is_empty() {
                return "false".to_string();
            }
            let parts: Vec<_> = children.iter().map(expr_to_string).collect();
            format!("({})", parts.join(" OR "))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a selection bitmap with all rows 0..n selected.
    fn all_selected(n: usize) -> Bitmap<BATCH_WORDS> {
        let mut sel = Bitmap::zeroed();
        for i in 0..n {
            sel.set(i);
        }
        sel
    }

    /// Helper: collect result bitmap into a Vec<bool> for easy assertion.
    fn bitmap_to_bools(bm: &Bitmap<BATCH_WORDS>, n: usize) -> Vec<bool> {
        (0..n).map(|i| bm.get(i)).collect()
    }

    /// Naive evaluation: evaluate expr row-by-row, returning Vec<bool>.
    fn naive_eval(expr: &Expr, columns: &[Vec<i64>], n: usize) -> Vec<bool> {
        let mut result = vec![false; n];
        for row in 0..n {
            result[row] = eval_row(expr, columns, row);
        }
        result
    }

    fn eval_row(expr: &Expr, columns: &[Vec<i64>], row: usize) -> bool {
        match expr {
            Expr::Column(idx) => columns[*idx][row] != 0,
            Expr::Literal(sv) => match sv {
                ScalarValue::Bool(b) => *b,
                ScalarValue::I64(v) => *v != 0,
                ScalarValue::F64(v) => *v != 0.0,
                ScalarValue::Null => false,
            },
            Expr::BinaryOp { op, left, right } => {
                let lv = eval_value(left, columns, row);
                let rv = eval_value(right, columns, row);
                match op {
                    BinOp::Eq => lv == rv,
                    BinOp::NotEq => lv != rv,
                    BinOp::Lt => lv < rv,
                    BinOp::LtEq => lv <= rv,
                    BinOp::Gt => lv > rv,
                    BinOp::GtEq => lv >= rv,
                    BinOp::Add => lv.wrapping_add(rv) != 0,
                    BinOp::Sub => lv.wrapping_sub(rv) != 0,
                    BinOp::Mul => lv.wrapping_mul(rv) != 0,
                    BinOp::Div => {
                        if rv == 0 { false } else { lv / rv != 0 }
                    }
                }
            }
            Expr::Not(inner) => !eval_row(inner, columns, row),
            Expr::And(children) => children.iter().all(|c| eval_row(c, columns, row)),
            Expr::Or(children) => children.iter().any(|c| eval_row(c, columns, row)),
        }
    }

    fn eval_value(expr: &Expr, columns: &[Vec<i64>], row: usize) -> i64 {
        match expr {
            Expr::Column(idx) => columns[*idx][row],
            Expr::Literal(sv) => match sv {
                ScalarValue::I64(v) => *v,
                _ => 0,
            },
            Expr::BinaryOp { op, left, right } => {
                let lv = eval_value(left, columns, row);
                let rv = eval_value(right, columns, row);
                match op {
                    BinOp::Add => lv.wrapping_add(rv),
                    BinOp::Sub => lv.wrapping_sub(rv),
                    BinOp::Mul => lv.wrapping_mul(rv),
                    BinOp::Div => {
                        if rv == 0 { 0 } else { lv / rv }
                    }
                    // Comparison ops: return 1 for true, 0 for false.
                    BinOp::Eq => (lv == rv) as i64,
                    BinOp::NotEq => (lv != rv) as i64,
                    BinOp::Lt => (lv < rv) as i64,
                    BinOp::LtEq => (lv <= rv) as i64,
                    BinOp::Gt => (lv > rv) as i64,
                    BinOp::GtEq => (lv >= rv) as i64,
                }
            }
            Expr::Not(_) | Expr::And(_) | Expr::Or(_) => 0,
        }
    }

    #[test]
    fn simple_col_gt_literal() {
        let col0 = vec![10, 50, 100, 150, 200];
        let columns = vec![col0];
        let n = 5;
        let sel = all_selected(n);

        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(100))),
        };

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        assert!(result.get(3)); // 150 > 100
        assert!(result.get(4)); // 200 > 100
        assert!(!result.get(2)); // 100 == 100
        assert!(!result.get(0)); // 10 < 100
    }

    #[test]
    fn and_two_comparisons() {
        let col0 = vec![10, 50, 100, 150, 200];
        let col1 = vec![60, 40, 30, 55, 10];
        let columns = vec![col0, col1];
        let n = 5;
        let sel = all_selected(n);

        // col0 > 40 AND col1 < 50
        let expr = Expr::And(vec![
            Expr::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(40))),
            },
            Expr::BinaryOp {
                op: BinOp::Lt,
                left: Box::new(Expr::Column(1)),
                right: Box::new(Expr::Literal(ScalarValue::I64(50))),
            },
        ]);

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        // row0: 10>40=F -> F
        // row1: 50>40=T, 40<50=T -> T
        // row2: 100>40=T, 30<50=T -> T
        // row3: 150>40=T, 55<50=F -> F
        // row4: 200>40=T, 10<50=T -> T
        assert!(result.get(1));
        assert!(result.get(2));
        assert!(result.get(4));
        assert!(!result.get(0));
        assert!(!result.get(3));
    }

    #[test]
    fn or_two_comparisons() {
        let col0 = vec![10, 50, 100, 150, 200];
        let columns = vec![col0];
        let n = 5;
        let sel = all_selected(n);

        // col0 < 20 OR col0 > 180
        let expr = Expr::Or(vec![
            Expr::BinaryOp {
                op: BinOp::Lt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(20))),
            },
            Expr::BinaryOp {
                op: BinOp::Gt,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(180))),
            },
        ]);

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        assert!(result.get(0)); // 10 < 20
        assert!(result.get(4)); // 200 > 180
        assert!(!result.get(2)); // 100 neither
    }

    #[test]
    fn not_comparison() {
        let col0 = vec![10, 50, 100, 150];
        let columns = vec![col0];
        let n = 4;
        let sel = all_selected(n);

        // NOT (col0 >= 100)
        let expr = Expr::Not(Box::new(Expr::BinaryOp {
            op: BinOp::GtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(100))),
        }));

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        assert!(result.get(0)); // 10 < 100
        assert!(result.get(1)); // 50 < 100
        assert!(!result.get(2)); // 100 >= 100
        assert!(!result.get(3)); // 150 >= 100
    }

    #[test]
    fn nested_complex() {
        // (col0 > 100) AND (col1 < 50) OR (col0 == 0)
        let col0 = vec![0, 150, 200, 50, 120];
        let col1 = vec![80, 30, 60, 10, 40];
        let columns = vec![col0, col1];
        let n = 5;
        let sel = all_selected(n);

        let expr = Expr::Or(vec![
            Expr::And(vec![
                Expr::BinaryOp {
                    op: BinOp::Gt,
                    left: Box::new(Expr::Column(0)),
                    right: Box::new(Expr::Literal(ScalarValue::I64(100))),
                },
                Expr::BinaryOp {
                    op: BinOp::Lt,
                    left: Box::new(Expr::Column(1)),
                    right: Box::new(Expr::Literal(ScalarValue::I64(50))),
                },
            ]),
            Expr::BinaryOp {
                op: BinOp::Eq,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(0))),
            },
        ]);

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        // row0: (0>100=F AND 80<50=F) OR (0==0=T) -> T
        // row1: (150>100=T AND 30<50=T) OR (150==0=F) -> T
        // row2: (200>100=T AND 60<50=F) OR (200==0=F) -> F
        // row3: (50>100=F AND 10<50=T) OR (50==0=F) -> F
        // row4: (120>100=T AND 40<50=T) OR (120==0=F) -> T
        assert!(result.get(0));
        assert!(result.get(1));
        assert!(!result.get(2));
        assert!(!result.get(3));
        assert!(result.get(4));
    }

    #[test]
    fn selection_mask_respected() {
        // Only rows 1 and 3 are in the selection; rows 0, 2, 4 are not evaluated.
        let col0 = vec![10, 50, 100, 150, 200];
        let columns = vec![col0];
        let n = 5;
        let mut sel = Bitmap::zeroed();
        sel.set(1);
        sel.set(3);

        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(100))),
        };

        let result = evaluate_expr(&expr, &columns, &sel, n);
        assert!(!result.get(0));
        assert!(!result.get(1)); // 50 is not > 100
        assert!(!result.get(2));
        assert!(result.get(3));  // 150 > 100, and row 3 was selected
        assert!(!result.get(4));
    }

    #[test]
    fn equality_and_inequality() {
        let col0 = vec![42, 42, 0, -1, 42];
        let columns = vec![col0];
        let n = 5;
        let sel = all_selected(n);

        // col0 == 42
        let eq_expr = Expr::BinaryOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(42))),
        };
        let result = evaluate_expr(&eq_expr, &columns, &sel, n);
        let expected = naive_eval(&eq_expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        assert!(result.get(0));
        assert!(result.get(1));
        assert!(!result.get(2));
        assert!(!result.get(3));
        assert!(result.get(4));

        // col0 != 42
        let ne_expr = Expr::BinaryOp {
            op: BinOp::NotEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(42))),
        };
        let result = evaluate_expr(&ne_expr, &columns, &sel, n);
        let expected = naive_eval(&ne_expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        assert!(!result.get(0));
        assert!(!result.get(1));
        assert!(result.get(2));
        assert!(result.get(3));
        assert!(!result.get(4));
    }

    #[test]
    fn lteq_and_gteq() {
        let col0 = vec![10, 50, 100, 150, 200];
        let columns = vec![col0];
        let n = 5;
        let sel = all_selected(n);

        // col0 <= 100
        let lte_expr = Expr::BinaryOp {
            op: BinOp::LtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(100))),
        };
        let result = evaluate_expr(&lte_expr, &columns, &sel, n);
        let expected = naive_eval(&lte_expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);

        // col0 >= 100
        let gte_expr = Expr::BinaryOp {
            op: BinOp::GtEq,
            left: Box::new(Expr::Column(0)),
            right: Box::new(Expr::Literal(ScalarValue::I64(100))),
        };
        let result = evaluate_expr(&gte_expr, &columns, &sel, n);
        let expected = naive_eval(&gte_expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
    }

    #[test]
    fn empty_and_or() {
        let sel = all_selected(3);
        let columns: Vec<Vec<i64>> = vec![vec![1, 2, 3]];
        let n = 3;

        // Empty AND = all true
        let empty_and = Expr::And(vec![]);
        let result = evaluate_expr(&empty_and, &columns, &sel, n);
        assert!(result.get(0));
        assert!(result.get(1));
        assert!(result.get(2));

        // Empty OR = all false
        let empty_or = Expr::Or(vec![]);
        let result = evaluate_expr(&empty_or, &columns, &sel, n);
        assert!(!result.get(0));
        assert!(!result.get(1));
        assert!(!result.get(2));
    }

    #[test]
    fn literal_commuted_comparison() {
        // 100 < col0 (literal on the left, column on the right)
        let col0 = vec![10, 50, 100, 150, 200];
        let columns = vec![col0];
        let n = 5;
        let sel = all_selected(n);

        let expr = Expr::BinaryOp {
            op: BinOp::Lt,
            left: Box::new(Expr::Literal(ScalarValue::I64(100))),
            right: Box::new(Expr::Column(0)),
        };

        let result = evaluate_expr(&expr, &columns, &sel, n);
        let expected = naive_eval(&expr, &columns, n);
        assert_eq!(bitmap_to_bools(&result, n), expected);
        // 100 < 150 and 100 < 200
        assert!(result.get(3));
        assert!(result.get(4));
        assert!(!result.get(0));
        assert!(!result.get(1));
        assert!(!result.get(2));
    }

    #[test]
    fn expr_to_string_formatting() {
        let expr = Expr::Or(vec![
            Expr::And(vec![
                Expr::BinaryOp {
                    op: BinOp::Gt,
                    left: Box::new(Expr::Column(0)),
                    right: Box::new(Expr::Literal(ScalarValue::I64(100))),
                },
                Expr::BinaryOp {
                    op: BinOp::Lt,
                    left: Box::new(Expr::Column(1)),
                    right: Box::new(Expr::Literal(ScalarValue::I64(50))),
                },
            ]),
            Expr::BinaryOp {
                op: BinOp::Eq,
                left: Box::new(Expr::Column(0)),
                right: Box::new(Expr::Literal(ScalarValue::I64(0))),
            },
        ]);

        let s = expr_to_string(&expr);
        assert_eq!(s, "(((col0 > 100) AND (col1 < 50)) OR (col0 = 0))");
    }

    #[test]
    fn expr_to_string_not_and_literal_types() {
        let not_expr = Expr::Not(Box::new(Expr::BinaryOp {
            op: BinOp::GtEq,
            left: Box::new(Expr::Column(2)),
            right: Box::new(Expr::Literal(ScalarValue::F64(3.14))),
        }));
        assert_eq!(expr_to_string(&not_expr), "NOT ((col2 >= 3.140000))");

        let bool_lit = Expr::Literal(ScalarValue::Bool(true));
        assert_eq!(expr_to_string(&bool_lit), "true");

        let null_lit = Expr::Literal(ScalarValue::Null);
        assert_eq!(expr_to_string(&null_lit), "NULL");
    }

    #[test]
    fn against_naive_randomized() {
        // Fuzz-style: generate random data and compare evaluate_expr vs naive_eval.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let seed = 0xDEADBEEFu64;
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        let mut rng_state = hasher.finish();

        let next_u64 = |state: &mut u64| -> u64 {
            // xorshift64
            let mut x = *state;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *state = x;
            x
        };

        let next_i64 = |state: &mut u64| -> i64 {
            next_u64(state) as i64
        };

        for trial in 0..20 {
            let n = 16 + (next_u64(&mut rng_state) as usize % 64);
            let col0: Vec<i64> = (0..n).map(|_| next_i64(&mut rng_state) % 300 - 150).collect();
            let col1: Vec<i64> = (0..n).map(|_| next_i64(&mut rng_state) % 300 - 150).collect();
            let columns = vec![col0, col1];

            let threshold0 = next_i64(&mut rng_state) % 300 - 150;
            let threshold1 = next_i64(&mut rng_state) % 300 - 150;

            // Build expression: (col0 > T0) AND (col1 < T1) OR NOT (col0 == T0)
            let expr = Expr::Or(vec![
                Expr::And(vec![
                    Expr::BinaryOp {
                        op: BinOp::Gt,
                        left: Box::new(Expr::Column(0)),
                        right: Box::new(Expr::Literal(ScalarValue::I64(threshold0))),
                    },
                    Expr::BinaryOp {
                        op: BinOp::Lt,
                        left: Box::new(Expr::Column(1)),
                        right: Box::new(Expr::Literal(ScalarValue::I64(threshold1))),
                    },
                ]),
                Expr::Not(Box::new(Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column(0)),
                    right: Box::new(Expr::Literal(ScalarValue::I64(threshold0))),
                })),
            ]);

            let sel = all_selected(n);
            let result = evaluate_expr(&expr, &columns, &sel, n);
            let expected = naive_eval(&expr, &columns, n);
            let actual_bools = bitmap_to_bools(&result, n);
            if actual_bools != expected {
                panic!(
                    "Trial {}: mismatch at n={}, t0={}, t1={}\n  actual:   {:?}\n  expected: {:?}",
                    trial, n, threshold0, threshold1, actual_bools, expected
                );
            }
        }
    }
}
