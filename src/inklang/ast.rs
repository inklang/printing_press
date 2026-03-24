//! Abstract Syntax Tree types for Inklang.

use std::collections::HashMap;

use super::token::Token;
use super::value::Value;

/// Parameter in a function or event declaration.
#[derive(Debug, Clone)]
pub struct Param {
    pub annotations: Vec<Expr>,
    pub name: Token,
    pub type_annot: Option<Token>,
    pub default: Option<Expr>,
}

/// Enum variant definition.
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: Token,
    pub fields: Vec<Param>,
}

/// Field in a config declaration.
#[derive(Debug, Clone)]
pub struct ConfigField {
    pub name: Token,
    pub type_: Token,
    pub default_value: Option<Expr>,
}

/// Field in a table declaration.
#[derive(Debug, Clone)]
pub struct TableField {
    pub name: Token,
    pub type_: Option<String>,
    pub is_key: bool,
    pub default_value: Option<Expr>,
}

/// Field in an annotation declaration.
#[derive(Debug, Clone)]
pub struct AnnotationField {
    pub name: Token,
    pub type_: Token,
    pub default_value: Option<Expr>,
}

/// Parameter in an event declaration.
#[derive(Debug, Clone)]
pub struct EventParam {
    pub name: Token,
    pub type_: Token,
}

/// Else branch in an if statement.
#[derive(Debug, Clone)]
pub enum ElseBranch {
    Else(Box<Stmt>),
    ElseIf(Box<Stmt>),
}

/// Expression variants in the AST.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Literal value: 42, "hello", true, null
    Literal(Value),
    /// List literal: [1, 2, 3]
    List(Vec<Expr>),
    /// Set literal: #{1, 2, 3}
    Set(Vec<Expr>),
    /// Tuple literal: (1, 2, 3)
    Tuple(Vec<Expr>),
    /// Map literal: {"key": value}
    Map(Vec<(Box<Expr>, Box<Expr>)>),
    /// Variable reference: foo
    Variable(Token),
    /// Assignment: x = 5, x += 1
    Assign {
        target: Box<Expr>,
        op: Token,
        value: Box<Expr>,
    },
    /// Binary operation: a + b, a and b
    Binary {
        left: Box<Expr>,
        op: Token,
        right: Box<Expr>,
    },
    /// Unary operation: -x, !y, not z
    Unary {
        op: Token,
        right: Box<Expr>,
    },
    /// Ternary: condition ? then : else
    Ternary {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// Grouped expression: (x + y)
    Group(Box<Expr>),
    /// Function call: foo(arg1, arg2)
    Call {
        callee: Box<Expr>,
        paren: Token,
        arguments: Vec<Expr>,
    },
    /// Lambda: fn(x) { ... }
    Lambda {
        params: Vec<Param>,
        body: Box<Stmt>,
        is_async: bool,
    },
    /// Property access: obj.field
    Get {
        obj: Box<Expr>,
        name: Token,
    },
    /// Index access: arr[0]
    Index {
        obj: Box<Expr>,
        index: Box<Expr>,
    },
    /// Type check: x is Int
    Is {
        expr: Box<Expr>,
        type_: Token,
    },
    /// Field existence check: obj has field
    Has {
        target: Box<Expr>,
        field: Box<Expr>,
    },
    /// Safe call: obj?.field
    SafeCall {
        obj: Box<Expr>,
        name: Token,
    },
    /// Elvis operator: left ?? right
    Elvis {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Named argument: foo(x: 5)
    NamedArg {
        name: Token,
        value: Box<Expr>,
    },
    /// Await expression
    Await(Box<Expr>),
    /// Spawn expression: spawn virtual { ... }
    Spawn {
        expr: Box<Expr>,
        virtual_: bool,
    },
    /// Throw expression
    Throw(Box<Expr>),
    /// Annotation: @annotationName(arg1=value1)
    Annotation {
        name: String,
        args: HashMap<String, Expr>,
    },
}

/// Statement variants in the AST.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// Expression statement: just expr;
    Expr(Expr),
    /// Let binding: let x = 5
    Let {
        name: Token,
        type_annot: Option<Token>,
        value: Expr,
    },
    /// Const binding: const x = 5
    Const {
        name: Token,
        type_annot: Option<Token>,
        value: Expr,
    },
    /// Block of statements: { ... }
    Block(Vec<Stmt>),
    /// If statement
    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    /// While loop
    While {
        condition: Expr,
        body: Box<Stmt>,
    },
    /// For range loop: for i in 0..10 { ... }
    For {
        variable: Token,
        iterable: Expr,
        body: Box<Stmt>,
    },
    /// Return statement: return value
    Return(Option<Expr>),
    /// Break statement
    Break,
    /// Next statement
    Next,
    /// Function declaration
    Fn {
        name: Token,
        params: Vec<Param>,
        return_type: Option<Token>,
        body: Box<Stmt>,
        is_async: bool,
    },
    /// Class declaration
    Class {
        name: Token,
        superclass: Option<Token>,
        body: Box<Stmt>,
    },
    /// Enum declaration
    Enum {
        name: Token,
        variants: Vec<EnumVariant>,
    },
    /// Config declaration
    Config {
        name: Token,
        fields: Vec<ConfigField>,
    },
    /// Table declaration
    Table {
        name: Token,
        fields: Vec<TableField>,
    },
    /// Try-catch-finally
    Try {
        body: Box<Stmt>,
        catch_var: Option<Token>,
        catch_body: Option<Box<Stmt>>,
        finally_body: Option<Box<Stmt>>,
    },
    /// Throw statement
    Throw(Expr),
    /// Event declaration: event player_join(player: Player)
    EventDecl {
        name: Token,
        params: Vec<EventParam>,
    },
    /// Event handler: on player_join(event, player) { ... }
    On {
        event: Token,
        handler: Box<Stmt>,
    },
    /// Enable block: enable { ... }
    Enable(Box<Stmt>),
    /// Disable block: disable { ... }
    Disable(Box<Stmt>),
    /// Import statement: import foo
    Import(Vec<String>),
    /// Import from statement: from foo import bar
    ImportFrom {
        path: Vec<String>,
        items: Vec<String>,
    },
    /// Annotation definition
    AnnotationDef {
        name: Token,
        args: Vec<Param>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inklang::token::TokenType;

    #[test]
    fn test_expr_clone() {
        let expr = Expr::Literal(Value::Int(42));
        let cloned = expr.clone();
        assert!(matches!(cloned, Expr::Literal(Value::Int(42))));
    }

    #[test]
    fn test_expr_literal() {
        let lit = Expr::Literal(Value::Int(42));
        assert!(matches!(lit, Expr::Literal(Value::Int(42))));
    }

    #[test]
    fn test_expr_variable() {
        let var = Expr::Variable(Token {
            typ: TokenType::Identifier,
            lexeme: "x".into(),
            line: 1,
            column: 0,
        });
        assert!(matches!(var, Expr::Variable(_)));
    }

    #[test]
    fn test_expr_binary() {
        let expr = Expr::Binary {
            left: Box::new(Expr::Literal(Value::Int(1))),
            op: Token {
                typ: TokenType::Plus,
                lexeme: "+".into(),
                line: 1,
                column: 2,
            },
            right: Box::new(Expr::Literal(Value::Int(2))),
        };
        assert!(matches!(expr, Expr::Binary { .. }));
    }

    #[test]
    fn test_expr_list() {
        let list = Expr::List(vec![
            Expr::Literal(Value::Int(1)),
            Expr::Literal(Value::Int(2)),
        ]);
        assert!(matches!(list, Expr::List(_)));
        if let Expr::List(elems) = list {
            assert_eq!(elems.len(), 2);
        }
    }

    #[test]
    fn test_expr_map() {
        let map = Expr::Map(vec![
            (
                Box::new(Expr::Literal(Value::String("key".to_string()))),
                Box::new(Expr::Literal(Value::Int(42))),
            ),
        ]);
        assert!(matches!(map, Expr::Map(_)));
    }

    #[test]
    fn test_expr_has() {
        let expr = Expr::Has {
            target: Box::new(Expr::Variable(Token {
                typ: TokenType::Identifier,
                lexeme: "obj".into(),
                line: 1,
                column: 0,
            })),
            field: Box::new(Expr::Literal(Value::String("field".to_string()))),
        };
        assert!(matches!(expr, Expr::Has { .. }));
    }

    #[test]
    fn test_expr_safe_call() {
        let expr = Expr::SafeCall {
            obj: Box::new(Expr::Variable(Token {
                typ: TokenType::Identifier,
                lexeme: "obj".into(),
                line: 1,
                column: 0,
            })),
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "method".into(),
                line: 1,
                column: 3,
            },
        };
        assert!(matches!(expr, Expr::SafeCall { .. }));
    }

    #[test]
    fn test_expr_elvis() {
        let expr = Expr::Elvis {
            left: Box::new(Expr::Variable(Token {
                typ: TokenType::Identifier,
                lexeme: "x".into(),
                line: 1,
                column: 0,
            })),
            right: Box::new(Expr::Literal(Value::Int(0))),
        };
        assert!(matches!(expr, Expr::Elvis { .. }));
    }

    #[test]
    fn test_expr_annotation() {
        let mut args = HashMap::new();
        args.insert("value".to_string(), Expr::Literal(Value::Int(5)));
        let expr = Expr::Annotation {
            name: "annotationName".to_string(),
            args,
        };
        assert!(matches!(expr, Expr::Annotation { .. }));
    }

    #[test]
    fn test_stmt_let() {
        let stmt = Stmt::Let {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "x".into(),
                line: 1,
                column: 0,
            },
            type_annot: None,
            value: Expr::Literal(Value::Int(5)),
        };
        assert!(matches!(stmt, Stmt::Let { .. }));
    }

    #[test]
    fn test_stmt_const() {
        let stmt = Stmt::Const {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "PI".into(),
                line: 1,
                column: 0,
            },
            type_annot: None,
            value: Expr::Literal(Value::Double(3.14)),
        };
        assert!(matches!(stmt, Stmt::Const { .. }));
    }

    #[test]
    fn test_stmt_block() {
        let stmt = Stmt::Block(vec![Stmt::Expr(Expr::Literal(Value::Int(1)))]);
        assert!(matches!(stmt, Stmt::Block(_)));
    }

    #[test]
    fn test_stmt_if() {
        let stmt = Stmt::If {
            condition: Expr::Literal(Value::Boolean(true)),
            then_branch: Box::new(Stmt::Block(vec![])),
            else_branch: None,
        };
        assert!(matches!(stmt, Stmt::If { .. }));
    }

    #[test]
    fn test_stmt_while() {
        let stmt = Stmt::While {
            condition: Expr::Literal(Value::Boolean(true)),
            body: Box::new(Stmt::Block(vec![])),
        };
        assert!(matches!(stmt, Stmt::While { .. }));
    }

    #[test]
    fn test_stmt_for() {
        let stmt = Stmt::For {
            variable: Token {
                typ: TokenType::Identifier,
                lexeme: "i".into(),
                line: 1,
                column: 4,
            },
            iterable: Expr::Variable(Token {
                typ: TokenType::Identifier,
                lexeme: "items".into(),
                line: 1,
                column: 10,
            }),
            body: Box::new(Stmt::Block(vec![])),
        };
        assert!(matches!(stmt, Stmt::For { .. }));
    }

    #[test]
    fn test_stmt_return() {
        let stmt = Stmt::Return(Some(Expr::Literal(Value::Int(42))));
        assert!(matches!(stmt, Stmt::Return(_)));
    }

    #[test]
    fn test_stmt_break() {
        let stmt = Stmt::Break;
        assert!(matches!(stmt, Stmt::Break));
    }

    #[test]
    fn test_stmt_next() {
        let stmt = Stmt::Next;
        assert!(matches!(stmt, Stmt::Next));
    }

    #[test]
    fn test_stmt_fn() {
        let stmt = Stmt::Fn {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "add".into(),
                line: 1,
                column: 0,
            },
            params: vec![],
            return_type: None,
            body: Box::new(Stmt::Block(vec![])),
            is_async: false,
        };
        assert!(matches!(stmt, Stmt::Fn { .. }));
    }

    #[test]
    fn test_stmt_class() {
        let stmt = Stmt::Class {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "MyClass".into(),
                line: 1,
                column: 0,
            },
            superclass: None,
            body: Box::new(Stmt::Block(vec![])),
        };
        assert!(matches!(stmt, Stmt::Class { .. }));
    }

    #[test]
    fn test_stmt_enum() {
        let stmt = Stmt::Enum {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "Color".into(),
                line: 1,
                column: 0,
            },
            variants: vec![
                EnumVariant {
                    name: Token {
                        typ: TokenType::Identifier,
                        lexeme: "Red".into(),
                        line: 1,
                        column: 7,
                    },
                    fields: vec![],
                },
                EnumVariant {
                    name: Token {
                        typ: TokenType::Identifier,
                        lexeme: "Green".into(),
                        line: 1,
                        column: 12,
                    },
                    fields: vec![],
                },
            ],
        };
        assert!(matches!(stmt, Stmt::Enum { .. }));
    }

    #[test]
    fn test_stmt_config() {
        let stmt = Stmt::Config {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "Settings".into(),
                line: 1,
                column: 0,
            },
            fields: vec![ConfigField {
                name: Token {
                    typ: TokenType::Identifier,
                    lexeme: "port".into(),
                    line: 1,
                    column: 9,
                },
                type_: Token {
                    typ: TokenType::KwInt,
                    lexeme: "Int".into(),
                    line: 1,
                    column: 14,
                },
                default_value: Some(Expr::Literal(Value::Int(8080))),
            }],
        };
        assert!(matches!(stmt, Stmt::Config { .. }));
    }

    #[test]
    fn test_stmt_try() {
        let stmt = Stmt::Try {
            body: Box::new(Stmt::Block(vec![])),
            catch_var: Some(Token {
                typ: TokenType::Identifier,
                lexeme: "e".into(),
                line: 2,
                column: 9,
            }),
            catch_body: Some(Box::new(Stmt::Block(vec![]))),
            finally_body: None,
        };
        assert!(matches!(stmt, Stmt::Try { .. }));
    }

    #[test]
    fn test_stmt_event_decl() {
        let stmt = Stmt::EventDecl {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "player_join".into(),
                line: 1,
                column: 0,
            },
            params: vec![EventParam {
                name: Token {
                    typ: TokenType::Identifier,
                    lexeme: "player".into(),
                    line: 1,
                    column: 13,
                },
                type_: Token {
                    typ: TokenType::Identifier,
                    lexeme: "Player".into(),
                    line: 1,
                    column: 21,
                },
            }],
        };
        assert!(matches!(stmt, Stmt::EventDecl { .. }));
    }

    #[test]
    fn test_stmt_on() {
        let stmt = Stmt::On {
            event: Token {
                typ: TokenType::Identifier,
                lexeme: "player_join".into(),
                line: 1,
                column: 3,
            },
            handler: Box::new(Stmt::Block(vec![])),
        };
        assert!(matches!(stmt, Stmt::On { .. }));
    }

    #[test]
    fn test_stmt_enable() {
        let stmt = Stmt::Enable(Box::new(Stmt::Block(vec![])));
        assert!(matches!(stmt, Stmt::Enable(_)));
    }

    #[test]
    fn test_stmt_disable() {
        let stmt = Stmt::Disable(Box::new(Stmt::Block(vec![])));
        assert!(matches!(stmt, Stmt::Disable(_)));
    }

    #[test]
    fn test_stmt_import() {
        let stmt = Stmt::Import(vec!["foo".to_string(), "bar".to_string()]);
        assert!(matches!(stmt, Stmt::Import(_)));
    }

    #[test]
    fn test_stmt_import_from() {
        let stmt = Stmt::ImportFrom {
            path: vec!["foo".to_string()],
            items: vec!["bar".to_string(), "baz".to_string()],
        };
        assert!(matches!(stmt, Stmt::ImportFrom { .. }));
    }

    #[test]
    fn test_param() {
        let param = Param {
            annotations: vec![],
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "x".into(),
                line: 1,
                column: 0,
            },
            type_annot: Some(Token {
                typ: TokenType::KwInt,
                lexeme: "Int".into(),
                line: 1,
                column: 2,
            }),
            default: Some(Expr::Literal(Value::Int(0))),
        };
        assert!(matches!(param, Param { .. }));
    }

    #[test]
    fn test_stmt_annotation_def() {
        let stmt = Stmt::AnnotationDef {
            name: Token {
                typ: TokenType::Identifier,
                lexeme: "MyAnnotation".into(),
                line: 1,
                column: 0,
            },
            args: vec![],
        };
        assert!(matches!(stmt, Stmt::AnnotationDef { .. }));
    }
}
