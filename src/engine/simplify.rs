//! Expression simplification — algebraic rewrite rules on the Expr tree.
//!
//! Applies constant folding, identity removal, short-circuit evaluation,
//! double-negation elimination, De Morgan's law, and nested-flattening.

use super::expr::{BinOp, Expr, ScalarValue};

/// Recursively simplify an expression tree. The result is semantically
/// equivalent to the input but may have fewer nodes.
pub fn simplify(expr: &Expr) -> Expr {
    match expr {
        Expr::Column(_) | Expr::Literal(_) => expr.clone(),

        Expr::BinaryOp { op, left, right } => {
            let l = simplify(left);
            let r = simplify(right);

            // Constant folding: both sides are literals.
            if let (Expr::Literal(lv), Expr::Literal(rv)) = (&l, &r) {
                return fold_binary(*op, lv, rv);
            }

            Expr::BinaryOp {
                op: *op,
                left: Box::new(l),
                right: Box::new(r),
            }
        }

        Expr::Not(inner) => {
            let s = simplify(inner);

            // Double negation: NOT(NOT(x)) -> x
            if let Expr::Not(x) = &s {
                return *x.clone();
            }

            // De Morgan: NOT(And([a,b,...])) -> Or([NOT(a), NOT(b), ...])
            if let Expr::And(children) = &s {
                return Expr::Or(children.iter().map(|c| Expr::Not(Box::new(c.clone()))).collect());
            }

            // De Morgan: NOT(Or([a,b,...])) -> And([NOT(a), NOT(b), ...])
            if let Expr::Or(children) = &s {
                return Expr::And(children.iter().map(|c| Expr::Not(Box::new(c.clone()))).collect());
            }

            Expr::Not(Box::new(s))
        }

        Expr::And(children) => {
            let simplified: Vec<Expr> = children
                .iter()
                .flat_map(|c| {
                    let s = simplify(c);
                    // Flatten nested And: And([And(a,b), c]) -> [a, b, c]
                    if let Expr::And(inner) = s {
                        inner
                    } else {
                        vec![s]
                    }
                })
                .collect();

            // Short-circuit: if any child is false, whole thing is false.
            if simplified.iter().any(is_literal_false) {
                return Expr::Literal(ScalarValue::Bool(false));
            }

            // Identity removal: drop true literals.
            let filtered: Vec<Expr> = simplified
                .into_iter()
                .filter(|e| !is_literal_true(e))
                .collect();

            match filtered.len() {
                0 => Expr::Literal(ScalarValue::Bool(true)),
                1 => filtered.into_iter().next().unwrap(),
                _ => Expr::And(filtered),
            }
        }

        Expr::Or(children) => {
            let simplified: Vec<Expr> = children
                .iter()
                .flat_map(|c| {
                    let s = simplify(c);
                    // Flatten nested Or.
                    if let Expr::Or(inner) = s {
                        inner
                    } else {
                        vec![s]
                    }
                })
                .collect();

            // Short-circuit: if any child is true, whole thing is true.
            if simplified.iter().any(is_literal_true) {
                return Expr::Literal(ScalarValue::Bool(true));
            }

            // Identity removal: drop false literals.
            let filtered: Vec<Expr> = simplified
                .into_iter()
                .filter(|e| !is_literal_false(e))
                .collect();

            match filtered.len() {
                0 => Expr::Literal(ScalarValue::Bool(false)),
                1 => filtered.into_iter().next().unwrap(),
                _ => Expr::Or(filtered),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_literal_true(e: &Expr) -> bool {
    matches!(e, Expr::Literal(ScalarValue::Bool(true)))
}

fn is_literal_false(e: &Expr) -> bool {
    matches!(e, Expr::Literal(ScalarValue::Bool(false)))
}

/// Fold a binary operation on two scalar literals into a single literal.
fn fold_binary(op: BinOp, lv: &ScalarValue, rv: &ScalarValue) -> Expr {
    match (lv, rv) {
        (ScalarValue::I64(a), ScalarValue::I64(b)) => {
            let result = match op {
                BinOp::Add => Some(a.wrapping_add(*b)),
                BinOp::Sub => Some(a.wrapping_sub(*b)),
                BinOp::Mul => Some(a.wrapping_mul(*b)),
                BinOp::Div if *b != 0 => Some(a / b),
                BinOp::Eq => return Expr::Literal(ScalarValue::Bool(a == b)),
                BinOp::NotEq => return Expr::Literal(ScalarValue::Bool(a != b)),
                BinOp::Lt => return Expr::Literal(ScalarValue::Bool(a < b)),
                BinOp::LtEq => return Expr::Literal(ScalarValue::Bool(a <= b)),
                BinOp::Gt => return Expr::Literal(ScalarValue::Bool(a > b)),
                BinOp::GtEq => return Expr::Literal(ScalarValue::Bool(a >= b)),
                _ => None,
            };
            match result {
                Some(v) => Expr::Literal(ScalarValue::I64(v)),
                None => Expr::BinaryOp {
                    op,
                    left: Box::new(Expr::Literal(lv.clone())),
                    right: Box::new(Expr::Literal(rv.clone())),
                },
            }
        }
        (ScalarValue::F64(a), ScalarValue::F64(b)) => {
            let result = match op {
                BinOp::Add => Some(a + b),
                BinOp::Sub => Some(a - b),
                BinOp::Mul => Some(a * b),
                BinOp::Div if *b != 0.0 => Some(a / b),
                BinOp::Eq => return Expr::Literal(ScalarValue::Bool(a == b)),
                BinOp::NotEq => return Expr::Literal(ScalarValue::Bool(a != b)),
                BinOp::Lt => return Expr::Literal(ScalarValue::Bool(a < b)),
                BinOp::LtEq => return Expr::Literal(ScalarValue::Bool(a <= b)),
                BinOp::Gt => return Expr::Literal(ScalarValue::Bool(a > b)),
                BinOp::GtEq => return Expr::Literal(ScalarValue::Bool(a >= b)),
                _ => None,
            };
            match result {
                Some(v) => Expr::Literal(ScalarValue::F64(v)),
                None => Expr::BinaryOp {
                    op,
                    left: Box::new(Expr::Literal(lv.clone())),
                    right: Box::new(Expr::Literal(rv.clone())),
                },
            }
        }
        (ScalarValue::Bool(a), ScalarValue::Bool(b)) => match op {
            BinOp::Eq => Expr::Literal(ScalarValue::Bool(a == b)),
            BinOp::NotEq => Expr::Literal(ScalarValue::Bool(a != b)),
            _ => Expr::BinaryOp {
                op,
                left: Box::new(Expr::Literal(lv.clone())),
                right: Box::new(Expr::Literal(rv.clone())),
            },
        },
        _ => Expr::BinaryOp {
            op,
            left: Box::new(Expr::Literal(lv.clone())),
            right: Box::new(Expr::Literal(rv.clone())),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_removal_and() {
        // AND(true, x) -> x
        let expr = Expr::And(vec![
            Expr::Literal(ScalarValue::Bool(true)),
            Expr::Column(0),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Column(0));
    }

    #[test]
    fn identity_removal_or() {
        // OR(false, x) -> x
        let expr = Expr::Or(vec![
            Expr::Literal(ScalarValue::Bool(false)),
            Expr::Column(0),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Column(0));
    }

    #[test]
    fn short_circuit_and() {
        // AND(false, x) -> false
        let expr = Expr::And(vec![
            Expr::Literal(ScalarValue::Bool(false)),
            Expr::Column(0),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::Bool(false)));
    }

    #[test]
    fn short_circuit_or() {
        // OR(true, x) -> true
        let expr = Expr::Or(vec![
            Expr::Literal(ScalarValue::Bool(true)),
            Expr::Column(0),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::Bool(true)));
    }

    #[test]
    fn double_negation() {
        // NOT(NOT(x)) -> x
        let expr = Expr::Not(Box::new(Expr::Not(Box::new(Expr::Column(0)))));
        let result = simplify(&expr);
        assert_eq!(result, Expr::Column(0));
    }

    #[test]
    fn constant_fold_arithmetic() {
        // 3 + 5 -> 8
        let expr = Expr::BinaryOp {
            op: BinOp::Add,
            left: Box::new(Expr::Literal(ScalarValue::I64(3))),
            right: Box::new(Expr::Literal(ScalarValue::I64(5))),
        };
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::I64(8)));
    }

    #[test]
    fn constant_fold_comparison() {
        // 10 > 5 -> true
        let expr = Expr::BinaryOp {
            op: BinOp::Gt,
            left: Box::new(Expr::Literal(ScalarValue::I64(10))),
            right: Box::new(Expr::Literal(ScalarValue::I64(5))),
        };
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::Bool(true)));
    }

    #[test]
    fn de_morgan_and() {
        // NOT(AND([a, b])) -> OR([NOT(a), NOT(b)])
        let expr = Expr::Not(Box::new(Expr::And(vec![
            Expr::Column(0),
            Expr::Column(1),
        ])));
        let result = simplify(&expr);
        assert_eq!(
            result,
            Expr::Or(vec![
                Expr::Not(Box::new(Expr::Column(0))),
                Expr::Not(Box::new(Expr::Column(1))),
            ])
        );
    }

    #[test]
    fn de_morgan_or() {
        // NOT(OR([a, b])) -> AND([NOT(a), NOT(b)])
        let expr = Expr::Not(Box::new(Expr::Or(vec![
            Expr::Column(0),
            Expr::Column(1),
        ])));
        let result = simplify(&expr);
        assert_eq!(
            result,
            Expr::And(vec![
                Expr::Not(Box::new(Expr::Column(0))),
                Expr::Not(Box::new(Expr::Column(1))),
            ])
        );
    }

    #[test]
    fn flatten_nested_and() {
        // AND([AND([a, b]), c]) -> AND([a, b, c])
        let expr = Expr::And(vec![
            Expr::And(vec![Expr::Column(0), Expr::Column(1)]),
            Expr::Column(2),
        ]);
        let result = simplify(&expr);
        assert_eq!(
            result,
            Expr::And(vec![Expr::Column(0), Expr::Column(1), Expr::Column(2)])
        );
    }

    #[test]
    fn flatten_nested_or() {
        // OR([OR([a, b]), c]) -> OR([a, b, c])
        let expr = Expr::Or(vec![
            Expr::Or(vec![Expr::Column(0), Expr::Column(1)]),
            Expr::Column(2),
        ]);
        let result = simplify(&expr);
        assert_eq!(
            result,
            Expr::Or(vec![Expr::Column(0), Expr::Column(1), Expr::Column(2)])
        );
    }

    #[test]
    fn simplify_preserves_column() {
        assert_eq!(simplify(&Expr::Column(5)), Expr::Column(5));
    }

    #[test]
    fn simplify_preserves_literal() {
        assert_eq!(
            simplify(&Expr::Literal(ScalarValue::I64(42))),
            Expr::Literal(ScalarValue::I64(42))
        );
    }

    #[test]
    fn constant_fold_div() {
        // 10 / 3 -> 3
        let expr = Expr::BinaryOp {
            op: BinOp::Div,
            left: Box::new(Expr::Literal(ScalarValue::I64(10))),
            right: Box::new(Expr::Literal(ScalarValue::I64(3))),
        };
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::I64(3)));
    }

    #[test]
    fn constant_fold_div_by_zero() {
        // 10 / 0 -> stays as BinaryOp (can't fold)
        let expr = Expr::BinaryOp {
            op: BinOp::Div,
            left: Box::new(Expr::Literal(ScalarValue::I64(10))),
            right: Box::new(Expr::Literal(ScalarValue::I64(0))),
        };
        let result = simplify(&expr);
        assert_eq!(result, expr);
    }

    #[test]
    fn and_single_child_unwrap() {
        let expr = Expr::And(vec![Expr::Column(0)]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Column(0));
    }

    #[test]
    fn or_single_child_unwrap() {
        let expr = Expr::Or(vec![Expr::Column(0)]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Column(0));
    }

    #[test]
    fn and_all_true() {
        let expr = Expr::And(vec![
            Expr::Literal(ScalarValue::Bool(true)),
            Expr::Literal(ScalarValue::Bool(true)),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::Bool(true)));
    }

    #[test]
    fn or_all_false() {
        let expr = Expr::Or(vec![
            Expr::Literal(ScalarValue::Bool(false)),
            Expr::Literal(ScalarValue::Bool(false)),
        ]);
        let result = simplify(&expr);
        assert_eq!(result, Expr::Literal(ScalarValue::Bool(false)));
    }
}
