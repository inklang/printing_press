//! Constant folding optimization for Inklang.
//!
//! Evaluates constant expressions at compile time when all operands are literals.

use super::ast::{Expr, Stmt};
use super::token::{Token, TokenType};
use super::value::Value;

/// Constant folder - folds constant expressions at compile time.
#[derive(Debug, Clone, Default)]
pub struct ConstantFolder;

impl ConstantFolder {
    /// Create a new constant folder.
    pub fn new() -> Self {
        Self
    }

    /// Fold a list of statements.
    pub fn fold(&mut self, stmts: &[Stmt]) -> Vec<Stmt> {
        stmts.iter().map(|s| self.fold_stmt(s.clone())).collect()
    }

    /// Fold a single statement.
    pub fn fold_stmt(&mut self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Let {
                annotations,
                name,
                type_annot,
                value,
            } => Stmt::Let {
                annotations,
                name,
                type_annot,
                value: self.fold_expr(value),
            },
            Stmt::Const {
                name,
                type_annot,
                value,
            } => Stmt::Const {
                name,
                type_annot,
                value: self.fold_expr(value),
            },
            Stmt::Expr(expr) => Stmt::Expr(self.fold_expr(expr)),
            Stmt::Return(value) => Stmt::Return(value.map(|v| self.fold_expr(v))),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => Stmt::If {
                condition: self.fold_expr(condition),
                then_branch: Box::new(self.fold_stmt(*then_branch)),
                else_branch: else_branch.map(|eb| Box::new(self.fold_else_branch(*eb))),
            },
            Stmt::While {
                condition,
                body,
            } => Stmt::While {
                condition: self.fold_expr(condition),
                body: Box::new(self.fold_stmt(*body)),
            },
            Stmt::Block(stmts) => Stmt::Block(self.fold(&stmts)),
            Stmt::Fn {
                annotations,
                name,
                params,
                return_type,
                body,
                is_async,
            } => Stmt::Fn {
                annotations,
                name,
                params,
                return_type,
                body: Box::new(self.fold_stmt(*body)),
                is_async,
            },
            Stmt::For {
                variable,
                iterable,
                body,
            } => Stmt::For {
                variable,
                iterable: self.fold_expr(iterable),
                body: Box::new(self.fold_stmt(*body)),
            },
            other => other,
        }
    }

    /// Fold an else branch.
    fn fold_else_branch(&mut self, branch: Stmt) -> Stmt {
        match branch {
            Stmt::Block(stmts) => Stmt::Block(self.fold(&stmts)),
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => Stmt::If {
                condition: self.fold_expr(condition),
                then_branch: Box::new(self.fold_stmt(*then_branch)),
                else_branch: else_branch.map(|eb| Box::new(self.fold_else_branch(*eb))),
            },
            _ => self.fold_stmt(branch),
        }
    }

    /// Fold an expression.
    pub fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Binary { left, op, right } => {
                let folded_left = self.fold_expr(*left);
                let folded_right = self.fold_expr(*right);
                self.fold_binary_op(folded_left, &op, folded_right)
            }
            Expr::Group(inner) => {
                let folded = self.fold_expr(*inner);
                match folded {
                    Expr::Literal(_) => folded,
                    other => Expr::Group(Box::new(other)),
                }
            }
            Expr::Unary { op, right } => {
                let folded = self.fold_expr(*right);
                self.fold_unary_op(&op, folded)
            }
            other => other,
        }
    }

    /// Fold a binary operation if both operands are literals.
    fn fold_binary_op(&mut self, left: Expr, op: &Token, right: Expr) -> Expr {
        if let (Expr::Literal(l), Expr::Literal(r)) = (&left, &right) {
            if let Some(result) = self.fold_arith(l, r, &op.typ) {
                return result;
            }
        }
        Expr::Binary {
            left: Box::new(left),
            op: op.clone(),
            right: Box::new(right),
        }
    }

    /// Fold a unary operation if the operand is a literal.
    fn fold_unary_op(&mut self, op: &Token, operand: Expr) -> Expr {
        if let Expr::Literal(ref lit) = operand {
            match op.typ {
                TokenType::Minus => {
                    if let Value::Int(n) = lit {
                        return Expr::Literal(Value::Int(-n));
                    } else if let Value::Float(n) = lit {
                        return Expr::Literal(Value::Float(-n));
                    } else if let Value::Double(n) = lit {
                        return Expr::Literal(Value::Double(-n));
                    }
                }
                TokenType::Bang => {
                    if let Value::Boolean(b) = lit {
                        return Expr::Literal(Value::Boolean(!b));
                    }
                }
                TokenType::KwNot => {
                    if let Value::Boolean(b) = lit {
                        return Expr::Literal(Value::Boolean(!b));
                    }
                }
                _ => {}
            }
        }
        Expr::Unary {
            op: op.clone(),
            right: Box::new(operand),
        }
    }

    /// Perform arithmetic constant folding.
    fn fold_arith(&self, l: &Value, r: &Value, op: &TokenType) -> Option<Expr> {
        match (l, r, op) {
            // Addition
            (Value::Int(a), Value::Int(b), TokenType::Plus) => {
                return Some(Expr::Literal(Value::Int(a + b)));
            }
            // Subtraction
            (Value::Int(a), Value::Int(b), TokenType::Minus) => {
                return Some(Expr::Literal(Value::Int(a - b)));
            }
            // Multiplication
            (Value::Int(a), Value::Int(b), TokenType::Star) => {
                return Some(Expr::Literal(Value::Int(a * b)));
            }
            // Division
            (Value::Int(a), Value::Int(b), TokenType::Slash) if *b != 0 => {
                return Some(Expr::Literal(Value::Int(a / b)));
            }
            // Modulo
            (Value::Int(a), Value::Int(b), TokenType::Percent) if *b != 0 => {
                return Some(Expr::Literal(Value::Int(a % b)));
            }
            // For mixed types, convert to double
            _ => {
                // Try to convert to double for floating point operations
                if let (Some(a), Some(b)) = (self.to_double(l), self.to_double(r)) {
                    match op {
                        TokenType::Plus => return Some(Expr::Literal(Value::Double(a + b))),
                        TokenType::Minus => return Some(Expr::Literal(Value::Double(a - b))),
                        TokenType::Star => return Some(Expr::Literal(Value::Double(a * b))),
                        TokenType::Slash if b != 0.0 => {
                            return Some(Expr::Literal(Value::Double(a / b)))
                        }
                        _ => {}
                    }
                }
                None
            }
        }
    }

    /// Convert a Value to a double for arithmetic.
    fn to_double(&self, v: &Value) -> Option<f64> {
        match v {
            Value::Int(n) => Some(*n as f64),
            Value::Float(f) => Some(*f as f64),
            Value::Double(d) => Some(*d),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::token::TokenType;

    fn make_token(typ: TokenType, lexeme: &str) -> Token {
        Token {
            typ,
            lexeme: lexeme.to_string(),
            line: 1,
            column: 0,
        }
    }

    fn int_expr(n: i64) -> Expr {
        Expr::Literal(Value::Int(n))
    }

    fn plus_expr(l: i64, r: i64) -> Expr {
        Expr::Binary {
            left: Box::new(int_expr(l)),
            op: make_token(TokenType::Plus, "+"),
            right: Box::new(int_expr(r)),
        }
    }

    fn minus_expr(l: i64, r: i64) -> Expr {
        Expr::Binary {
            left: Box::new(int_expr(l)),
            op: make_token(TokenType::Minus, "-"),
            right: Box::new(int_expr(r)),
        }
    }

    fn mul_expr(l: i64, r: i64) -> Expr {
        Expr::Binary {
            left: Box::new(int_expr(l)),
            op: make_token(TokenType::Star, "*"),
            right: Box::new(int_expr(r)),
        }
    }

    fn div_expr(l: i64, r: i64) -> Expr {
        Expr::Binary {
            left: Box::new(int_expr(l)),
            op: make_token(TokenType::Slash, "/"),
            right: Box::new(int_expr(r)),
        }
    }

    #[test]
    fn test_fold_int_add() {
        let mut folder = ConstantFolder::new();
        let folded = folder.fold_expr(plus_expr(1, 2));
        assert!(matches!(folded, Expr::Literal(Value::Int(3))));
    }

    #[test]
    fn test_fold_int_subtract() {
        let mut folder = ConstantFolder::new();
        let folded = folder.fold_expr(minus_expr(5, 3));
        assert!(matches!(folded, Expr::Literal(Value::Int(2))));
    }

    #[test]
    fn test_fold_int_multiply() {
        let mut folder = ConstantFolder::new();
        let folded = folder.fold_expr(mul_expr(4, 7));
        assert!(matches!(folded, Expr::Literal(Value::Int(28))));
    }

    #[test]
    fn test_fold_int_divide() {
        let mut folder = ConstantFolder::new();
        let folded = folder.fold_expr(div_expr(20, 4));
        assert!(matches!(folded, Expr::Literal(Value::Int(5))));
    }

    #[test]
    fn test_no_fold_variable() {
        let mut folder = ConstantFolder::new();
        let var = Expr::Variable(make_token(TokenType::Identifier, "x"));
        let folded = folder.fold_expr(var);
        assert!(matches!(folded, Expr::Variable(_)));
    }

    #[test]
    fn test_no_fold_with_variable() {
        let mut folder = ConstantFolder::new();
        // x + 2 should NOT be folded since x is not a literal
        let expr = Expr::Binary {
            left: Box::new(Expr::Variable(make_token(TokenType::Identifier, "x"))),
            op: make_token(TokenType::Plus, "+"),
            right: Box::new(int_expr(2)),
        };
        let folded = folder.fold_expr(expr);
        assert!(matches!(folded, Expr::Binary { .. }));
    }

    #[test]
    fn test_fold_nested() {
        let mut folder = ConstantFolder::new();
        // (1 + 2) + 3 should fold to 6
        let expr = Expr::Binary {
            left: Box::new(plus_expr(1, 2)),
            op: make_token(TokenType::Plus, "+"),
            right: Box::new(int_expr(3)),
        };
        let folded = folder.fold_expr(expr);
        assert!(matches!(folded, Expr::Literal(Value::Int(6))));
    }

    #[test]
    fn test_fold_group_literal() {
        let mut folder = ConstantFolder::new();
        let expr = Expr::Group(Box::new(int_expr(42)));
        let folded = folder.fold_expr(expr);
        assert!(matches!(folded, Expr::Literal(Value::Int(42))));
    }

    #[test]
    fn test_fold_group_non_literal() {
        let mut folder = ConstantFolder::new();
        let expr = Expr::Group(Box::new(Expr::Variable(make_token(TokenType::Identifier, "x"))));
        let folded = folder.fold_expr(expr);
        // Group should be preserved for non-literals
        assert!(matches!(folded, Expr::Group(_)));
    }

    #[test]
    fn test_fold_unary_minus_literal() {
        let mut folder = ConstantFolder::new();
        let expr = Expr::Unary {
            op: make_token(TokenType::Minus, "-"),
            right: Box::new(int_expr(5)),
        };
        let folded = folder.fold_expr(expr);
        assert!(matches!(folded, Expr::Literal(Value::Int(-5))));
    }

    #[test]
    fn test_fold_unary_bang() {
        let mut folder = ConstantFolder::new();
        let expr = Expr::Unary {
            op: make_token(TokenType::Bang, "!"),
            right: Box::new(Expr::Literal(Value::Boolean(true))),
        };
        let folded = folder.fold_expr(expr);
        assert!(matches!(folded, Expr::Literal(Value::Boolean(false))));
    }

    #[test]
    fn test_fold_stmt_let() {
        let mut folder = ConstantFolder::new();
        let stmt = Stmt::Let {
            annotations: vec![],
            name: make_token(TokenType::Identifier, "x"),
            type_annot: None,
            value: plus_expr(10, 5),
        };
        let folded = folder.fold_stmt(stmt);
        if let Stmt::Let { value, .. } = folded {
            assert!(matches!(value, Expr::Literal(Value::Int(15))));
        } else {
            panic!("Expected Let statement");
        }
    }

    #[test]
    fn test_fold_stmt_return() {
        let mut folder = ConstantFolder::new();
        let stmt = Stmt::Return(Some(plus_expr(3, 4)));
        let folded = folder.fold_stmt(stmt);
        if let Stmt::Return(Some(Expr::Literal(Value::Int(7)))) = folded {
            // Success
        } else {
            panic!("Expected Return(7)");
        }
    }

    #[test]
    fn test_fold_stmt_block() {
        let mut folder = ConstantFolder::new();
        let stmt = Stmt::Block(vec![
            Stmt::Let {
                annotations: vec![],
                name: make_token(TokenType::Identifier, "x"),
                type_annot: None,
                value: plus_expr(1, 2),
            },
            Stmt::Return(Some(int_expr(3))),
        ]);
        let folded = folder.fold_stmt(stmt);
        if let Stmt::Block(stmts) = folded {
            assert_eq!(stmts.len(), 2);
            if let Stmt::Let { value, .. } = &stmts[0] {
                assert!(matches!(value, Expr::Literal(Value::Int(3))));
            }
        } else {
            panic!("Expected Block");
        }
    }

    #[test]
    fn test_fold_stmt_while() {
        let mut folder = ConstantFolder::new();
        let stmt = Stmt::While {
            condition: Expr::Literal(Value::Boolean(true)),
            body: Box::new(Stmt::Block(vec![])),
        };
        let folded = folder.fold_stmt(stmt);
        // While with true condition should be preserved (can't optimize away)
        assert!(matches!(folded, Stmt::While { .. }));
    }
}
