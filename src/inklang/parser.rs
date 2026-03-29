//! Pratt parser for Inklang source code.
//!
//! Takes tokens from the lexer and produces an AST through recursive descent
//! with Pratt precedence parsing for expressions.

use std::collections::HashMap;

use super::ast::{
    ConfigField, EnumVariant, EventParam, Expr, GrammarRuleBody, Param, Stmt,
    TableField,
};
use super::grammar::Rule;
use super::error::{Error, Result, Span};
use super::grammar::MergedGrammar;
use super::token::{Token, TokenType};
use super::value::Value;

/// Operator precedence levels (higher = tighter binding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Precedence {
    /// No precedence (for error recovery)
    None = 0,
    /// Assignment: =, +=, -=, *=, /=, %=
    Assignment = 10,
    /// Ternary: ?:
    Ternary = 14,
    /// Elvis: ??
    Elvis = 15,
    /// Logical Or: or
    Or = 20,
    /// Logical And: and
    And = 30,
    /// Type check: is
    Is = 35,
    /// Comparison: ==, !=, <, >, <=, >=
    Comparison = 45,
    /// Field existence: has
    Has = 40,
    /// Range: ..
    Range = 55,
    /// Term: +, -
    Term = 60,
    /// Factor: *, /, %
    Factor = 70,
    /// Unary: -, !, not, ++, --
    Unary = 90,
    /// Call: ., ?., []
    Call = 100,
    /// Primary: literals, identifiers, grouping
    Primary = 110,
}

/// Assignment operator types.
const ASSIGN_OPS: &[TokenType] = &[
    TokenType::Assign,
    TokenType::AddEquals,
    TokenType::SubEquals,
    TokenType::MulEquals,
    TokenType::DivEquals,
    TokenType::ModEquals,
];

/// Pratt parser for Inklang source code.
pub struct Parser<'a> {
    tokens: Vec<Token>,
    current: usize,
    grammar: Option<&'a MergedGrammar>,
}

impl<'a> Parser<'a> {
    /// Create a new parser from a vector of tokens.
    /// If grammar is provided, Ref variants in rules can be resolved.
    /// If grammar is None, Ref variants should be handled gracefully (they are
    /// for plugin-defined grammars, not core language parsing).
    pub fn new(tokens: Vec<Token>, grammar: Option<&'a MergedGrammar>) -> Self {
        Parser { tokens, current: 0, grammar }
    }

    /// Parse the token stream into a list of statements.
    pub fn parse(&mut self) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            match self.parse_statement() {
                Ok(stmt) => statements.push(stmt),
                Err(e) => {
                    self.synchronize();
                    return Err(e);
                }
            }
        }
        Ok(statements)
    }

    /// Parse a single statement.
    fn parse_statement(&mut self) -> Result<Stmt> {
        // Check for annotations first
        let annotations = self.parse_annotations()?;

        match self.peek().typ {
            TokenType::KwImport => self.parse_import(),
            TokenType::KwLet | TokenType::KwConst => self.parse_var(annotations),
            TokenType::KwIf => self.parse_if(),
            TokenType::KwClass => self.parse_class(annotations),
            TokenType::KwAsync => {
                // Check if this is async fn or async lambda
                if self.check(&TokenType::KwFn) {
                    self.advance(); // consume async
                    self.parse_func(annotations, false)
                } else if self.check_ahead(1, &TokenType::LParen) {
                    // async lambda expression at statement level: async (x) -> { x }
                    // Don't consume async here - parse_prefix will see it and set is_async
                    let expr = self.parse_prefix()?;
                    // Consume any trailing semicolon
                    if self.check(&TokenType::Semicolon) {
                        self.advance();
                    }
                    Ok(Stmt::Expr(expr))
                } else {
                    Err(Error::Parse {
                        message: format!(
                            "Expected 'fn' or '(' after 'async', found {:?}",
                            self.peek().typ
                        ),
                        span: Span { line: self.peek().line, column: self.peek().column },
                    })
                }
            }
            TokenType::KwFn => self.parse_func(annotations, false),
            TokenType::KwReturn => self.parse_return(),
            TokenType::KwBreak => {
                self.advance();
                Ok(Stmt::Break)
            }
            TokenType::KwNext => {
                self.advance();
                Ok(Stmt::Next)
            }
            TokenType::KwWhile => self.parse_while(),
            TokenType::KwFor => self.parse_for(),
            TokenType::KwEnum => self.parse_enum(),
            TokenType::KwTable => self.parse_table(),
            TokenType::KwConfig => self.parse_config(),
            TokenType::KwAnnotation => self.parse_annotation_decl(),
            TokenType::KwEvent => self.parse_event_decl(),
            TokenType::KwOn => self.parse_on_handler(),
            TokenType::KwEnable => self.parse_enable(),
            TokenType::KwDisable => self.parse_disable(),
            TokenType::Identifier => {
                // Check if this identifier matches a grammar declaration keyword
                let lexeme = self.peek().lexeme.clone();
                let decl_def = self.grammar.and_then(|g| {
                    g.declarations.iter().find(|d| d.keyword == lexeme).cloned()
                });
                if let Some(decl) = decl_def {
                    return self.parse_grammar_decl(&decl);
                }
                // Not a grammar keyword — fall through to expression parsing
                let expr = self.parse_expression(Precedence::None)?;
                if self.check(&TokenType::Assign) || self.check(&TokenType::AddEquals)
                    || self.check(&TokenType::SubEquals) || self.check(&TokenType::MulEquals)
                    || self.check(&TokenType::DivEquals) || self.check(&TokenType::ModEquals)
                {
                    // handled below
                }
                if self.check(&TokenType::Semicolon) {
                    self.advance();
                }
                return Ok(Stmt::Expr(expr));
            }
            TokenType::LBrace => {
                // Could be block or map/set literal
                // Use lookahead to distinguish:
                // - If next token is a statement keyword (let, const, if, etc.), it's a block
                // - If next token is `}` (empty block), it's a block
                // - If next token is an expression (Int, String, etc.) followed by semicolon, it's a block (statements)
                // - If next token is String/Identifier followed by ':', it's a map (expression parsing handles)
                // - Otherwise, try expression parsing first (could be set literal)
                let is_block = self.check_ahead(1, &TokenType::KwLet)
                    || self.check_ahead(1, &TokenType::KwConst)
                    || self.check_ahead(1, &TokenType::KwIf)
                    || self.check_ahead(1, &TokenType::KwWhile)
                    || self.check_ahead(1, &TokenType::KwFor)
                    || self.check_ahead(1, &TokenType::KwReturn)
                    || self.check_ahead(1, &TokenType::RBrace) // empty block
                    || (self.is_expression_starter(self.tokens[self.current + 1].typ)
                        && self.check_ahead(2, &TokenType::Semicolon)); // expr; indicates block with statements
                if is_block {
                    Ok(self.parse_block()?)
                } else {
                    let expr = self.parse_expression(Precedence::None)?;
                    if self.check(&TokenType::Semicolon) {
                        self.advance();
                    }
                    Ok(Stmt::Expr(expr))
                }
            }
            _ => {
                let expr = self.parse_expression(Precedence::None)?;
                if self.check(&TokenType::Assign) || self.check(&TokenType::AddEquals)
                    || self.check(&TokenType::SubEquals) || self.check(&TokenType::MulEquals)
                    || self.check(&TokenType::DivEquals) || self.check(&TokenType::ModEquals)
                {
                    // Handle assignment expressions as statements
                }
                if self.check(&TokenType::Semicolon) {
                    self.advance();
                }
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// Parse a let/const statement.
    fn parse_var(&mut self, annotations: Vec<Expr>) -> Result<Stmt> {
        let keyword = self.advance(); // consume let or const
        let name = self.consume(&TokenType::Identifier, "Expected variable name")?;
        let type_annot = if self.match_token(&[TokenType::Colon]) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let value = if self.match_token(&[TokenType::Assign]) {
            Some(self.parse_expression(Precedence::None)?)
        } else {
            None
        };
        if self.check(&TokenType::Semicolon) {
            self.advance();
        }
        if keyword.typ == TokenType::KwConst {
            Ok(Stmt::Const {
                name,
                type_annot,
                value: value.unwrap_or(Expr::Literal(Value::Null)),
            })
        } else {
            Ok(Stmt::Let {
                annotations,
                name,
                type_annot,
                value: value.unwrap_or(Expr::Literal(Value::Null)),
            })
        }
    }

    /// Parse an if statement.
    fn parse_if(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'if'
        let condition = self.parse_expression(Precedence::None)?;
        let then_branch = self.parse_block()?;
        let else_branch = if self.match_token(&[TokenType::KwElse]) {
            if self.check(&TokenType::KwIf) {
                Some(Box::new(self.parse_if()?))
            } else {
                Some(Box::new(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(Stmt::If {
            condition,
            then_branch: Box::new(then_branch),
            else_branch,
        })
    }

    /// Parse a class declaration.
    fn parse_class(&mut self, annotations: Vec<Expr>) -> Result<Stmt> {
        self.advance(); // consume 'class'
        let name = self.consume(&TokenType::Identifier, "Expected class name")?;
        let superclass = if self.match_token(&[TokenType::KwExtends]) {
            Some(self.consume(&TokenType::Identifier, "Expected superclass name")?)
        } else {
            None
        };
        let body = self.parse_block()?;
        Ok(Stmt::Class {
            annotations,
            name,
            superclass,
            body: Box::new(body),
        })
    }

    /// Parse a function declaration.
    fn parse_func(&mut self, annotations: Vec<Expr>, is_async: bool) -> Result<Stmt> {
        self.advance(); // consume 'fn'
        let name = self.consume(&TokenType::Identifier, "Expected function name")?;
        self.consume(&TokenType::LParen, "Expected '(' after function name")?;

        let mut params = Vec::new();
        let mut has_seen_default_param = false;

        if !self.check(&TokenType::RParen) {
            loop {
                let param_annotations = self.parse_annotations()?;
                let param_name = self.consume(&TokenType::Identifier, "Expected parameter name")?;
                let param_type = if self.match_token(&[TokenType::Colon]) {
                    Some(self.parse_type()?)
                } else {
                    None
                };

                let default = if self.match_token(&[TokenType::Assign]) {
                    has_seen_default_param = true;
                    Some(self.parse_expression(Precedence::None)?)
                } else {
                    if has_seen_default_param {
                        return Err(Error::Parse {
                            message: format!(
                                "Non-default parameter '{}' cannot follow default parameter",
                                param_name.lexeme
                            ),
                            span: Span { line: param_name.line, column: param_name.column },
                        });
                    }
                    None
                };

                params.push(Param {
                    annotations: param_annotations,
                    name: param_name,
                    type_annot: param_type,
                    default,
                });

                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }

        self.consume(&TokenType::RParen, "Expected ')' after parameters")?;

        let return_type = if self.match_token(&[TokenType::Arrow]) {
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Ok(Stmt::Fn {
            annotations,
            name,
            params,
            return_type,
            body: Box::new(body),
            is_async,
        })
    }

    /// Parse a type annotation token.
    fn parse_type(&mut self) -> Result<Token> {
        match self.peek().typ {
            TokenType::KwBool
            | TokenType::KwInt
            | TokenType::KwFloat
            | TokenType::KwDouble
            | TokenType::KwString
            | TokenType::Identifier => Ok(self.advance()),
            _ => Err(self.parse_error(format!("Expected type, found {:?}", self.peek().typ))),
        }
    }

    /// Parse a return statement.
    fn parse_return(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'return'
        let value = if !self.check(&TokenType::Semicolon) && !self.check(&TokenType::RBrace) && !self.is_at_end() {
            Some(self.parse_expression(Precedence::None)?)
        } else {
            None
        };
        if self.check(&TokenType::Semicolon) {
            self.advance();
        }
        Ok(Stmt::Return(value))
    }

    /// Parse a while loop.
    fn parse_while(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'while'
        let condition = self.parse_expression(Precedence::None)?;
        let body = self.parse_block()?;
        Ok(Stmt::While {
            condition,
            body: Box::new(body),
        })
    }

    /// Parse a for loop.
    fn parse_for(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'for'
        let variable = self.consume(&TokenType::Identifier, "Expected loop variable")?;
        self.consume(&TokenType::KwIn, "Expected 'in' after loop variable")?;
        let iterable = self.parse_expression(Precedence::None)?;
        let body = self.parse_block()?;
        Ok(Stmt::For {
            variable,
            iterable,
            body: Box::new(body),
        })
    }

    /// Parse an enum declaration.
    fn parse_enum(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'enum'
        let name = self.consume(&TokenType::Identifier, "Expected enum name")?;
        self.consume(&TokenType::LBrace, "Expected '{' after enum name")?;

        let mut variants = Vec::new();
        if !self.check(&TokenType::RBrace) {
            loop {
                let variant_name = self.consume(&TokenType::Identifier, "Expected variant name")?;
                let fields = if self.match_token(&[TokenType::LParen]) {
                    let mut params = Vec::new();
                    if !self.check(&TokenType::RParen) {
                        loop {
                            let param_name =
                                self.consume(&TokenType::Identifier, "Expected parameter name")?;
                            self.consume(&TokenType::Colon, "Expected ':' after parameter name")?;
                            let param_type = self.parse_type()?;
                            params.push(Param {
                                annotations: vec![],
                                name: param_name,
                                type_annot: Some(param_type),
                                default: None,
                            });
                            if !self.match_token(&[TokenType::Comma]) {
                                break;
                            }
                        }
                    }
                    self.consume(&TokenType::RParen, "Expected ')' after enum variant parameters")?;
                    params
                } else {
                    Vec::new()
                };
                variants.push(EnumVariant {
                    name: variant_name,
                    fields,
                });
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }

        self.consume(&TokenType::RBrace, "Expected '}' after enum body")?;
        Ok(Stmt::Enum { name, variants })
    }

    /// Parse a table declaration.
    fn parse_table(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'table'
        let name = self.consume(&TokenType::Identifier, "Expected table name")?;
        self.consume(&TokenType::LBrace, "Expected '{' after table name")?;

        let mut fields = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            let is_key = self.match_token(&[TokenType::KwKey]);
            let field_name = self.consume(&TokenType::Identifier, "Expected field name")?;
            let field_type = if self.match_token(&[TokenType::Colon]) {
                Some(self.parse_type()?.lexeme)
            } else {
                None
            };
            let default_value = if self.match_token(&[TokenType::Assign]) {
                Some(self.parse_expression(Precedence::None)?)
            } else {
                None
            };
            if self.check(&TokenType::Semicolon) {
                self.advance();
            }
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            fields.push(TableField {
                name: field_name,
                type_: field_type,
                is_key,
                default_value,
            });
        }

        self.consume(&TokenType::RBrace, "Expected '}' after table body")?;
        Ok(Stmt::Table { name, fields })
    }

    /// Parse a config declaration.
    fn parse_config(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'config'
        let name = self.consume(&TokenType::Identifier, "Expected config name")?;
        self.consume(&TokenType::LBrace, "Expected '{' after config name")?;

        let mut fields = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            let field_name = self.consume(&TokenType::Identifier, "Expected field name")?;
            self.consume(&TokenType::Colon, "Expected ':' after field name")?;
            let field_type = self.parse_type()?;
            let default_value = if self.match_token(&[TokenType::Assign]) {
                Some(self.parse_expression(Precedence::None)?)
            } else {
                None
            };
            if self.check(&TokenType::Semicolon) {
                self.advance();
            }
            fields.push(ConfigField {
                name: field_name,
                type_: field_type,
                default_value,
            });
        }

        self.consume(&TokenType::RBrace, "Expected '}' after config body")?;
        Ok(Stmt::Config { name, fields })
    }

    /// Parse an annotation declaration.
    fn parse_annotation_decl(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'annotation'
        let name = self.consume(&TokenType::Identifier, "Expected annotation name")?;
        self.consume(&TokenType::LBrace, "Expected '{' after annotation name")?;

        let mut args = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            let field_name = self.consume(&TokenType::Identifier, "Expected field name")?;
            self.consume(&TokenType::Colon, "Expected ':' after field name")?;
            let field_type = self.parse_type()?;
            let default_value = if self.match_token(&[TokenType::Assign]) {
                Some(self.parse_expression(Precedence::None)?)
            } else {
                None
            };
            if self.check(&TokenType::Semicolon) {
                self.advance();
            }
            if self.check(&TokenType::Comma) {
                self.advance();
            }
            args.push(Param {
                annotations: vec![],
                name: field_name,
                type_annot: Some(field_type),
                default: default_value,
            });
        }

        self.consume(&TokenType::RBrace, "Expected '}' after annotation body")?;
        Ok(Stmt::AnnotationDef { name, args })
    }

    /// Parse a grammar declaration: `keyword Name { scope_rules... }`
    ///
    /// Each scope rule that starts with a keyword is matched by checking
    /// the leading token. Block bodies are parsed as lists of statements.
    fn parse_grammar_decl(&mut self, decl: &super::grammar::DeclarationDef) -> Result<Stmt> {
        let keyword = self.advance().lexeme.clone(); // consume the grammar keyword
        let name = self.consume(&TokenType::Identifier, "Expected declaration name after grammar keyword")?.lexeme.clone();
        self.consume(&TokenType::LBrace, "Expected '{' after declaration name")?;

        let mut rules: Vec<GrammarRuleBody> = Vec::new();

        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            let mut matched = false;
            for rule_name in &decl.scope_rules {
                if let Some(grammar) = self.grammar {
                    if let Some(rule_entry) = grammar.rules.get(rule_name.as_str()) {
                        if let Some((leading_kw, has_block)) = Self::rule_leading_keyword_and_block(&rule_entry.rule) {
                            // Check if current token matches the leading keyword
                            if self.peek().typ == TokenType::Identifier && self.peek().lexeme == leading_kw {
                                self.advance(); // consume the leading keyword
                                if has_block {
                                    // Parse the block body: { stmts... }
                                    self.consume(&TokenType::LBrace, "Expected '{' after keyword in grammar rule")?;
                                    let mut body = Vec::new();
                                    while !self.check(&TokenType::RBrace) && !self.is_at_end() {
                                        body.push(self.parse_statement()?);
                                    }
                                    self.consume(&TokenType::RBrace, "Expected '}' to close grammar rule block")?;
                                    rules.push(GrammarRuleBody {
                                        rule_name: rule_name.clone(),
                                        leading_keyword: Some(leading_kw),
                                        body,
                                    });
                                    matched = true;
                                    break;
                                }
                            }
                        } else if Self::rule_is_bare_block(&rule_entry.rule) {
                            // Rule is just a block(), no leading keyword
                            if self.check(&TokenType::LBrace) {
                                self.consume(&TokenType::LBrace, "Expected '{' for grammar rule block")?;
                                let mut body = Vec::new();
                                while !self.check(&TokenType::RBrace) && !self.is_at_end() {
                                    body.push(self.parse_statement()?);
                                }
                                self.consume(&TokenType::RBrace, "Expected '}' to close grammar rule block")?;
                                rules.push(GrammarRuleBody {
                                    rule_name: rule_name.clone(),
                                    leading_keyword: None,
                                    body,
                                });
                                matched = true;
                                break;
                            }
                        }
                    }
                }
            }
            if !matched {
                // Skip unrecognized tokens to avoid infinite loop
                self.advance();
            }
        }

        self.consume(&TokenType::RBrace, "Expected '}' to close grammar declaration")?;
        Ok(Stmt::GrammarDecl { keyword, name, rules })
    }

    /// If the rule is `seq(keyword(k), block())`, return `Some((k, true))`.
    /// If the rule is `seq(keyword(k), ...)` without a block, return `Some((k, false))`.
    fn rule_leading_keyword_and_block(rule: &Rule) -> Option<(String, bool)> {
        if let Rule::Seq { items } = rule {
            if let Some(Rule::Keyword { value }) = items.first() {
                let has_block = items.iter().any(|it| matches!(it, Rule::Block { .. }));
                return Some((value.clone(), has_block));
            }
        }
        None
    }

    /// Returns true if the rule is just `block()` with no leading keyword.
    fn rule_is_bare_block(rule: &Rule) -> bool {
        matches!(rule, Rule::Block { .. })
    }

    /// Parse an event declaration.
    fn parse_event_decl(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'event'
        let name = self.consume(&TokenType::Identifier, "Expected event name")?;
        self.consume(&TokenType::LParen, "Expected '(' after event name")?;

        let mut params = Vec::new();
        if !self.check(&TokenType::RParen) {
            loop {
                let param_name = self.consume(&TokenType::Identifier, "Expected parameter name")?;
                self.consume(&TokenType::Colon, "Expected ':' after parameter name")?;
                let param_type = self.parse_type()?;
                params.push(EventParam {
                    name: param_name,
                    type_: param_type,
                });
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }

        self.consume(&TokenType::RParen, "Expected ')' after event parameters")?;
        Ok(Stmt::EventDecl { name, params })
    }

    /// Parse an on handler.
    fn parse_on_handler(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'on'
        let event_name = self.consume(&TokenType::Identifier, "Expected event name")?;
        self.consume(&TokenType::LParen, "Expected '(' after event name")?;

        // First param is always the event object (consume but don't use for now)
        let _event_param = self.consume(&TokenType::Identifier, "Expected event parameter name")?;

        if !self.check(&TokenType::RParen) {
            self.consume(&TokenType::Comma, "Expected ',' after event param")?;
            // Skip over data params for now
            while !self.check(&TokenType::RParen) && !self.is_at_end() {
                self.consume(&TokenType::Identifier, "Expected parameter name")?;
                self.consume(&TokenType::Colon, "Expected ':' after parameter name")?;
                self.parse_type()?;
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }

        self.consume(&TokenType::RParen, "Expected ')' after on handler parameters")?;

        let body = self.parse_block()?;

        Ok(Stmt::On {
            event: event_name,
            handler: Box::new(body),
        })
    }

    /// Parse an enable block.
    fn parse_enable(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'enable'
        self.consume(&TokenType::LBrace, "Expected '{' after 'enable'")?;

        let mut statements = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }
        self.consume(&TokenType::RBrace, "Expected '}' after enable block")?;

        Ok(Stmt::Enable(Box::new(Stmt::Block(statements))))
    }

    /// Parse a disable block.
    fn parse_disable(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'disable'
        self.consume(&TokenType::LBrace, "Expected '{' after 'disable'")?;

        let mut statements = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }
        self.consume(&TokenType::RBrace, "Expected '}' after disable block")?;

        Ok(Stmt::Disable(Box::new(Stmt::Block(statements))))
    }

    /// Parse an import statement.
    fn parse_import(&mut self) -> Result<Stmt> {
        self.advance(); // consume 'import'

        if self.check(&TokenType::Identifier)
            && (self.check_ahead(1, &TokenType::KwFrom)
                || self.check_ahead(1, &TokenType::Comma))
        {
            let mut tokens = Vec::new();
            tokens.push(self.consume(&TokenType::Identifier, "Expected identifier")?);
            while self.match_token(&[TokenType::Comma]) {
                tokens.push(self.consume(&TokenType::Identifier, "Expected identifier")?);
            }
            self.consume(&TokenType::KwFrom, "Expected 'from'")?;
            let namespace = self.consume(&TokenType::Identifier, "Expected namespace")?;
            if self.check(&TokenType::Semicolon) {
                self.advance();
            }
            Ok(Stmt::ImportFrom {
                path: vec![namespace.lexeme],
                items: tokens.into_iter().map(|t| t.lexeme).collect(),
            })
        } else {
            let namespace = self.consume(&TokenType::Identifier, "Expected namespace")?;
            if self.check(&TokenType::Semicolon) {
                self.advance();
            }
            Ok(Stmt::Import(vec![namespace.lexeme]))
        }
    }

    /// Parse a block statement.
    fn parse_block(&mut self) -> Result<Stmt> {
        self.consume(&TokenType::LBrace, "Expected '{'")?;

        let mut statements = Vec::new();
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }

        self.consume(&TokenType::RBrace, "Expected '}'")?;

        Ok(Stmt::Block(statements))
    }

    /// Parse annotations (zero or more).
    fn parse_annotations(&mut self) -> Result<Vec<Expr>> {
        let mut annotations = Vec::new();

        loop {
            // Consume any ASI-inserted semicolons before checking for next annotation
            self.match_token(&[TokenType::Semicolon]);

            if !self.match_token(&[TokenType::At]) {
                break;
            }

            let name_token = self.consume(&TokenType::Identifier, "Expected annotation name after @")?;
            let name = name_token.lexeme.clone();
            let mut args = HashMap::new();

            // Parse optional (arg1, arg2, ...) - named arguments only
            if self.match_token(&[TokenType::LParen]) {
                if !self.check(&TokenType::RParen) {
                    loop {
                        let arg_name_token =
                            self.consume(&TokenType::Identifier, "Expected named argument name")?;
                        let arg_name = arg_name_token.lexeme.clone();
                        self.consume(&TokenType::Colon, "Expected ':' after argument name")?;
                        let arg_value = self.parse_expression(Precedence::None)?;
                        args.insert(arg_name, arg_value);
                        if !self.match_token(&[TokenType::Comma]) {
                            break;
                        }
                    }
                }
                self.consume(&TokenType::RParen, "Expected ')' after annotation arguments")?;
            }

            annotations.push(Expr::Annotation { name, args });
        }

        Ok(annotations)
    }

    /// Parse an expression using Pratt parsing.
    fn parse_expression(&mut self, precedence: Precedence) -> Result<Expr> {
        let mut left = self.parse_prefix()?;

        loop {
            left = self.parse_postfix(left)?;

            let token = self.peek().clone();
            let token_precedence = self.get_precedence(&token.typ);

            if token_precedence < precedence || token_precedence == Precedence::None {
                break;
            }

            // Assignment operators - right associative
            if token.typ == TokenType::Assign || token.typ == TokenType::AddEquals
                || token.typ == TokenType::SubEquals || token.typ == TokenType::MulEquals
                || token.typ == TokenType::DivEquals || token.typ == TokenType::ModEquals
            {
                let target = left.clone();
                // Validate assignment target
                match &target {
                    Expr::Variable(_) | Expr::Get { .. } | Expr::Index { .. } => {}
                    _ => {
                        return Err(self.error_at(&token, format!(
                            "Invalid assignment target"
                        )));
                    }
                }
                self.advance(); // consume assignment op
                let value = self.parse_expression(precedence)?;
                left = Expr::Assign {
                    target: Box::new(target),
                    op: token,
                    value: Box::new(value),
                };
                continue;
            }

            // 'is' takes a type name (identifier), not an expression
            if token.typ == TokenType::KwIs {
                self.advance();
                let type_name = self.parse_type()?;
                left = Expr::Is {
                    expr: Box::new(left),
                    type_: type_name,
                };
                continue;
            }

            // 'has' takes a field name expression
            if token.typ == TokenType::KwHas {
                self.advance();
                let field = self.parse_expression(Precedence::Has)?;
                left = Expr::Has {
                    target: Box::new(left),
                    field: Box::new(field),
                };
                continue;
            }

            // Ternary: condition ? then : else
            if token.typ == TokenType::Question {
                self.advance(); // consume ?
                let then_branch = self.parse_expression(Precedence::None)?;
                self.consume(&TokenType::Colon, "Expected ':' in ternary expression")?;
                let else_branch = self.parse_expression(Precedence::None)?;
                left = Expr::Ternary {
                    condition: Box::new(left),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                };
                continue;
            }

            // Elvis: left ?? right
            if token.typ == TokenType::QuestionQuestion {
                self.advance(); // consume ??
                let right = self.parse_expression(Precedence::Elvis)?;
                left = Expr::Elvis {
                    left: Box::new(left),
                    right: Box::new(right),
                };
                continue;
            }

            // If token has no precedence (not a valid infix operator), break
            let token_precedence = self.get_precedence(&token.typ);
            if token_precedence == Precedence::None {
                break;
            }

            self.advance();
            let right = self.parse_expression(self.get_precedence_for_binary_op(&token.typ))?;
            left = Expr::Binary {
                left: Box::new(left),
                op: token,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Get precedence for binary operators.
    fn get_precedence(&self, typ: &TokenType) -> Precedence {
        match typ {
            TokenType::Assign
            | TokenType::AddEquals
            | TokenType::SubEquals
            | TokenType::MulEquals
            | TokenType::DivEquals
            | TokenType::ModEquals => Precedence::Assignment,
            TokenType::Question => Precedence::Ternary,
            TokenType::QuestionQuestion => Precedence::Elvis,
            TokenType::KwOr => Precedence::Or,
            TokenType::KwAnd => Precedence::And,
            TokenType::KwIs => Precedence::Is,
            TokenType::EqEq | TokenType::BangEq => Precedence::Comparison,
            TokenType::Lt | TokenType::Gt | TokenType::Lte | TokenType::Gte => Precedence::Comparison,
            TokenType::KwHas => Precedence::Has,
            TokenType::DotDot => Precedence::Range,
            TokenType::Plus | TokenType::Minus => Precedence::Term,
            TokenType::Star | TokenType::Slash | TokenType::Percent => Precedence::Factor,
            _ => Precedence::None,
        }
    }

    /// Get precedence for the right-hand side of a binary operator.
    fn get_precedence_for_binary_op(&self, typ: &TokenType) -> Precedence {
        match typ {
            // Assignment is right-associative but handled specially
            TokenType::Assign
            | TokenType::AddEquals
            | TokenType::SubEquals
            | TokenType::MulEquals
            | TokenType::DivEquals
            | TokenType::ModEquals => Precedence::Assignment,
            // Other binary operators are left-associative
            _ => {
                let p = self.get_precedence(typ);
                if p == Precedence::None {
                    Precedence::None
                } else {
                    // Increment precedence for left-associative operators
                    Precedence::from(p.to_u8() + 1)
                }
            }
        }
    }

    /// Parse postfix operators (calls, indexing, safe calls).
    fn parse_postfix(&mut self, mut expr: Expr) -> Result<Expr> {
        loop {
            expr = match self.peek().typ {
                TokenType::LParen => {
                    self.advance(); // consume (
                    let mut args = Vec::new();
                    let mut seen_named = false;

                    if !self.check(&TokenType::RParen) {
                        loop {
                            // Check if this is a named argument: name = expr
                            if self.check(&TokenType::Identifier)
                                && self.check_ahead(1, &TokenType::Assign)
                            {
                                let name = self.advance(); // consume identifier
                                self.advance(); // consume =
                                let value = self.parse_expression(Precedence::None)?;
                                args.push(Expr::NamedArg {
                                    name,
                                    value: Box::new(value),
                                });
                                seen_named = true;
                            } else {
                                if seen_named {
                                    return Err(self.parse_error(
                                        "Positional argument cannot follow named argument"
                                    ));
                                }
                                args.push(self.parse_expression(Precedence::None)?);
                            }
                            if !self.match_token(&[TokenType::Comma]) {
                                break;
                            }
                        }
                    }

                    let paren = self.consume(&TokenType::RParen, "Expected ')' after arguments")?;
                    Expr::Call {
                        callee: Box::new(expr),
                        paren,
                        arguments: args,
                    }
                }
                TokenType::QuestionDot => {
                    self.advance(); // consume ?.
                    let name = match self.peek().typ {
                        TokenType::Identifier => self.advance(),
                        TokenType::KwHas => self.advance(), // has can be used as method name
                        TokenType::KwIs => self.advance(),  // is can be used as method name
                        _ => {
                            return Err(self.parse_error(
                                "Expected field name after '?.'"
                            ));
                        }
                    };
                    Expr::SafeCall {
                        obj: Box::new(expr),
                        name,
                    }
                }
                TokenType::Dot => {
                    self.advance(); // consume .
                    let name = match self.peek().typ {
                        TokenType::Identifier => self.advance(),
                        TokenType::KwHas => self.advance(), // has can be used as method name
                        TokenType::KwIs => self.advance(),  // is can be used as method name
                        _ => {
                            return Err(self.parse_error(
                                "Expected field name after '.'"
                            ));
                        }
                    };
                    Expr::Get {
                        obj: Box::new(expr),
                        name,
                    }
                }
                TokenType::LSquare => {
                    self.advance(); // consume [
                    let index = self.parse_expression(Precedence::None)?;
                    self.consume(&TokenType::RSquare, "Expected ']'")?;
                    Expr::Index {
                        obj: Box::new(expr),
                        index: Box::new(index),
                    }
                }
                _ => break,
            };
        }
        Ok(expr)
    }

    /// Parse a prefix expression (literals, identifiers, grouping, lambda, etc.).
    fn parse_prefix(&mut self) -> Result<Expr> {
        let token = self.advance();

        match token.typ {
            TokenType::Identifier => Ok(Expr::Variable(token)),
            TokenType::KwTrue => Ok(Expr::Literal(Value::Boolean(true))),
            TokenType::KwFalse => Ok(Expr::Literal(Value::Boolean(false))),
            TokenType::KwNull => Ok(Expr::Literal(Value::Null)),
            TokenType::KwInt => {
                let value = token
                    .lexeme
                    .parse::<i64>()
                    .map_err(|_| self.error_at(&token, format!("Invalid integer literal: {}", token.lexeme)))?;
                Ok(Expr::Literal(Value::Int(value)))
            }
            TokenType::KwDouble => {
                let value = token.lexeme.parse::<f64>().map_err(|_| {
                    self.error_at(&token, format!("Invalid double literal: {}", token.lexeme))
                })?;
                Ok(Expr::Literal(Value::Double(value)))
            }
            TokenType::KwString => {
                let value = self.unescape_string(&token.lexeme);
                // Check for interpolation
                if self.check(&TokenType::InterpolationStart) {
                    self.advance(); // consume INTERPOLATION_START
                    Ok(self.parse_interpolated_string(Expr::Literal(Value::String(
                        value,
                    )))?)
                } else {
                    Ok(Expr::Literal(Value::String(value)))
                }
            }
            TokenType::InterpolationStart => {
                // Interpolation starting at beginning of string (empty string prefix)
                Ok(self.parse_interpolated_string(Expr::Literal(Value::String(
                    String::new(),
                )))?)
            }
            TokenType::Minus => {
                let right = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Unary {
                    op: token,
                    right: Box::new(right),
                })
            }
            TokenType::Bang => {
                let right = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Unary {
                    op: token,
                    right: Box::new(right),
                })
            }
            TokenType::KwNot => {
                let right = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Unary {
                    op: token,
                    right: Box::new(right),
                })
            }
            TokenType::Increment | TokenType::Decrement => {
                let right = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Unary {
                    op: token,
                    right: Box::new(right),
                })
            }
            TokenType::KwAwait => {
                let right = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Await(Box::new(right)))
            }
            TokenType::KwSpawn => {
                let virtual_ = self.match_token(&[TokenType::KwVirtual]);
                let expr = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::Spawn {
                    expr: Box::new(expr),
                    virtual_,
                })
            }
            TokenType::KwAsync => {
                // async (params) -> { body } - async lambda
                // Check: previous token is 'async' (already consumed), current is '('
                if self.check(&TokenType::LParen) && self.is_async_lambda_ahead() {
                    let params = self.parse_lambda_params()?;
                    self.consume(&TokenType::Arrow, "Expected '->' after lambda params")?;
                    let body = self.parse_block()?;
                    Ok(Expr::Lambda {
                        params,
                        body: Box::new(body),
                        is_async: true,
                    })
                } else {
                    return Err(self.parse_error("Expected async lambda expression"));
                }
            }
            TokenType::LBrace => {
                // Could be map literal or set literal
                if self.check(&TokenType::RBrace) {
                    return Err(self.parse_error(
                        "Empty '{}' is not valid in expression position. Use '()' for empty tuple or '{key: value}' for single-element map."
                    ));
                }
                let first_expr = self.parse_expression(Precedence::None)?;
                if self.match_token(&[TokenType::Colon]) {
                    // Map literal: { key: value, ... }
                    let mut entries = Vec::new();
                    entries.push((Box::new(first_expr), Box::new(self.parse_expression(Precedence::None)?)));
                    while self.match_token(&[TokenType::Comma]) {
                        let key = self.parse_expression(Precedence::None)?;
                        self.consume(&TokenType::Colon, "Expected ':' after map key")?;
                        let value = self.parse_expression(Precedence::None)?;
                        entries.push((Box::new(key), Box::new(value)));
                    }
                    self.consume(&TokenType::RBrace, "Expected '}' after map literal")?;
                    Ok(Expr::Map(entries))
                } else {
                    // Set literal: { expr, expr, ... } or { expr; expr; ... }
                    let mut elements = Vec::new();
                    elements.push(first_expr);
                    loop {
                        if self.match_token(&[TokenType::Comma]) || self.match_token(&[TokenType::Semicolon]) {
                            elements.push(self.parse_expression(Precedence::None)?);
                        } else {
                            break;
                        }
                    }
                    self.consume(&TokenType::RBrace, "Expected '}' after set literal")?;
                    Ok(Expr::Set(elements))
                }
            }
            TokenType::LParen => {
                // Could be: grouped expression, lambda, or tuple
                if self.is_lambda_ahead(1) {
                    let params = self.parse_lambda_params()?;
                    self.consume(&TokenType::Arrow, "Expected '->' after lambda params")?;
                    let body = self.parse_block()?;
                    Ok(Expr::Lambda {
                        params,
                        body: Box::new(body),
                        is_async: false,
                    })
                } else {
                    // Check for tuple: (expr, expr, ...) or ()
                    if self.check(&TokenType::RParen) {
                        self.advance();
                        Ok(Expr::Tuple(Vec::new()))
                    } else {
                        let first_expr = self.parse_expression(Precedence::None)?;
                        if self.check(&TokenType::Comma) {
                            // Tuple
                            let mut elements = Vec::new();
                            elements.push(first_expr);
                            while self.match_token(&[TokenType::Comma]) {
                                if self.check(&TokenType::RParen) {
                                    break;
                                }
                                elements.push(self.parse_expression(Precedence::None)?);
                            }
                            self.consume(&TokenType::RParen, "Expected ')' after tuple")?;
                            Ok(Expr::Tuple(elements))
                        } else {
                            // Single expression in parentheses
                            self.consume(&TokenType::RParen, "Expected ')' after expression")?;
                            Ok(Expr::Group(Box::new(first_expr)))
                        }
                    }
                }
            }
            TokenType::LSquare => {
                let mut elements = Vec::new();
                if !self.check(&TokenType::RSquare) {
                    loop {
                        elements.push(self.parse_expression(Precedence::None)?);
                        if self.match_token(&[TokenType::Comma]) || self.match_token(&[TokenType::Semicolon]) {
                            // continue parsing
                        } else {
                            break;
                        }
                    }
                }
                self.consume(&TokenType::RSquare, "Expected ']'")?;
                Ok(Expr::List(elements))
            }
            _ => Err(self.error_at(&token, format!(
                "Expected expression but found {:?} ('{}')",
                token.typ, token.lexeme
            ))),
        }
    }

    /// Parse an interpolated string and desugar to concatenation.
    fn parse_interpolated_string(&mut self, first_part: Expr) -> Result<Expr> {
        let mut result = first_part;
        let plus_token = Token {
            typ: TokenType::Plus,
            lexeme: "+".to_string(),
            line: 0,
            column: 0,
        };

        // We're already past the INTERPOLATION_START - parse the expression
        loop {
            // Parse the expression inside the interpolation
            let expr = self.parse_expression(Precedence::None)?;

            // Expect INTERPOLATION_END
            self.consume(
                &TokenType::InterpolationEnd,
                "Expected '}' after interpolated expression",
            )?;

            // Concatenate: result + expr
            result = Expr::Binary {
                left: Box::new(result),
                op: plus_token.clone(),
                right: Box::new(expr),
            };

            // Check if there's more string content or another interpolation
            if self.check(&TokenType::KwString) {
                let string_token = self.advance();
                let string_value = self.unescape_string(&string_token.lexeme);
                result = Expr::Binary {
                    left: Box::new(result),
                    op: plus_token.clone(),
                    right: Box::new(Expr::Literal(Value::String(string_value))),
                };

                if self.check(&TokenType::InterpolationStart) {
                    self.advance(); // consume INTERPOLATION_START
                    continue;
                } else {
                    break;
                }
            } else if self.check(&TokenType::InterpolationStart) {
                self.advance(); // consume INTERPOLATION_START
                continue;
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// Unescape a string literal.
    fn unescape_string(&self, s: &str) -> String {
        // s is the full lexeme including quotes, so we strip them
        let s = &s[1..s.len() - 1];
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('t') => result.push('\t'),
                    Some('r') => result.push('\r'),
                    Some('"') => result.push('"'),
                    Some('\\') => result.push('\\'),
                    Some('$') => result.push('$'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if hex.chars().all(|c| c.is_ascii_hexdigit()) {
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    result.push(ch);
                                }
                            }
                        }
                    }
                    Some(c) => result.push(c),
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Parse lambda parameters (used in both parenthesized and arrow lambdas).
    fn parse_lambda_params(&mut self) -> Result<Vec<Param>> {
        self.advance(); // consume (
        let mut params = Vec::new();

        if !self.check(&TokenType::RParen) {
            loop {
                let param_annotations = self.parse_annotations()?;
                let param_name =
                    self.consume(&TokenType::Identifier, "Expected parameter name")?;
                let param_type = if self.match_token(&[TokenType::Colon]) {
                    Some(self.parse_type()?)
                } else {
                    None
                };
                let default = if self.match_token(&[TokenType::Assign]) {
                    Some(self.parse_expression(Precedence::None)?)
                } else {
                    None
                };
                params.push(Param {
                    annotations: param_annotations,
                    name: param_name,
                    type_annot: param_type,
                    default,
                });
                if !self.match_token(&[TokenType::Comma]) {
                    break;
                }
            }
        }

        self.consume(&TokenType::RParen, "Expected ')' after lambda parameters")?;
        Ok(params)
    }

    /// Check if a lambda is ahead (lookahead for (params) -> pattern).
    /// Looks for the pattern: LParen ... RParen -> Arrow
    fn is_lambda_ahead(&self, starting_depth: usize) -> bool {
        let mut depth = starting_depth;
        let mut i = self.current;

        while i < self.tokens.len() && depth > 0 {
            match self.tokens[i].typ {
                TokenType::LParen => depth += 1,
                TokenType::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching ) - check if next is ->
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].typ == TokenType::Arrow;
                    }
                }
                TokenType::Eof => return false,
                _ => {}
            }
            i += 1;
        }

        false
    }

    /// Check if an async lambda is ahead: async (params) -> pattern.
    /// This is called after consuming 'async', with current at '('.
    fn is_async_lambda_ahead(&self) -> bool {
        // We know: previous token is 'async', current is '('
        // Find the matching ')' and check if '->' follows
        let mut depth = 0; // Start at the '(' - first '(' increments to 1
        let mut i = self.current;

        while i < self.tokens.len() {
            match self.tokens[i].typ {
                TokenType::LParen => depth += 1,
                TokenType::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        // Found the matching ) of the lambda params
                        // Return whether the next token is ->
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].typ == TokenType::Arrow;
                    }
                }
                TokenType::Eof => return false,
                _ => {}
            }
            i += 1;
        }

        false
    }

    // === Helper methods ===

    /// Check if we've reached the end of the token stream.
    fn is_at_end(&self) -> bool {
        self.peek().typ == TokenType::Eof
    }

    /// Get the current token without consuming it.
    fn peek(&self) -> &Token {
        self.tokens.get(self.current).unwrap_or_else(|| {
            // This is safe because we always add an EOF token at the end
            panic!("No more tokens");
        })
    }

    /// Get the previous token.
    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    /// Advance and return the previous token.
    fn advance(&mut self) -> Token {
        if self.is_at_end() {
            // Already at EOF, return EOF without moving
            return self.peek().clone();
        }
        self.current += 1;
        self.previous().clone()
    }

    /// Check if the current token matches any of the given types.
    fn check(&self, typ: &TokenType) -> bool {
        !self.is_at_end() && &self.peek().typ == typ
    }

    /// Check if the current token matches any of the given types, and advance if so.
    fn match_token(&mut self, types: &[TokenType]) -> bool {
        for typ in types {
            if self.check(typ) {
                self.advance();
                return true;
            }
        }
        false
    }

    /// Consume a token of the expected type, or return an error.
    fn consume(&mut self, typ: &TokenType, message: &str) -> Result<Token> {
        if self.check(typ) {
            Ok(self.advance())
        } else {
            let token = self.peek();
            Err(Error::Parse {
                message: format!("{}, found '{}'", message, token.lexeme),
                span: Span { line: token.line, column: token.column },
            })
        }
    }

    /// Create a parse error at the current token position.
    fn parse_error(&self, message: impl Into<String>) -> Error {
        let token = self.peek();
        Error::Parse {
            message: message.into(),
            span: Span { line: token.line, column: token.column },
        }
    }

    /// Create a parse error at a specific token's position.
    fn error_at(&self, token: &Token, message: impl Into<String>) -> Error {
        Error::Parse {
            message: message.into(),
            span: Span { line: token.line, column: token.column },
        }
    }

    /// Check ahead by offset.
    fn check_ahead(&self, offset: usize, typ: &TokenType) -> bool {
        let idx = self.current + offset;
        if idx >= self.tokens.len() {
            return false;
        }
        self.tokens[idx].typ == *typ
    }

    /// Check if a token type can start an expression (prefix parsing).
    fn is_expression_starter(&self, typ: TokenType) -> bool {
        matches!(
            typ,
            TokenType::Identifier
                | TokenType::KwInt
                | TokenType::KwDouble
                | TokenType::KwString
                | TokenType::KwTrue
                | TokenType::KwFalse
                | TokenType::KwNull
                | TokenType::LParen
                | TokenType::LSquare
                | TokenType::LBrace
                | TokenType::Minus
                | TokenType::Bang
                | TokenType::KwNot
                | TokenType::Increment
                | TokenType::Decrement
                | TokenType::KwAsync
                | TokenType::KwAwait
                | TokenType::KwSpawn
                | TokenType::InterpolationStart
        )
    }

    /// Synchronize the parser after an error (skip tokens until a synchronization point).
    fn synchronize(&mut self) {
        self.advance();

        while !self.is_at_end() {
            // Synchronize on statement keywords
            match self.peek().typ {
                TokenType::KwLet
                | TokenType::KwConst
                | TokenType::KwIf
                | TokenType::KwWhile
                | TokenType::KwFor
                | TokenType::KwFn
                | TokenType::KwReturn
                | TokenType::KwClass
                | TokenType::KwEnum
                | TokenType::KwImport
                | TokenType::KwTable
                | TokenType::KwConfig
                | TokenType::Semicolon => return,
                _ => {}
            }
            self.advance();
        }
    }
}

impl Precedence {
    fn to_u8(&self) -> u8 {
        match self {
            Precedence::None => 0,
            Precedence::Assignment => 10,
            Precedence::Ternary => 14,
            Precedence::Elvis => 15,
            Precedence::Or => 20,
            Precedence::And => 30,
            Precedence::Is => 35,
            Precedence::Has => 40,
            Precedence::Comparison => 45,
            Precedence::Range => 55,
            Precedence::Term => 60,
            Precedence::Factor => 70,
            Precedence::Unary => 90,
            Precedence::Call => 100,
            Precedence::Primary => 110,
        }
    }

    fn from(val: u8) -> Self {
        match val {
            0 => Precedence::None,
            10 => Precedence::Assignment,
            14 => Precedence::Ternary,
            15 => Precedence::Elvis,
            20 => Precedence::Or,
            30 => Precedence::And,
            35 => Precedence::Is,
            40 => Precedence::Has,
            45 => Precedence::Comparison,
            55 => Precedence::Range,
            60 => Precedence::Term,
            70 => Precedence::Factor,
            90 => Precedence::Unary,
            100 => Precedence::Call,
            110 => Precedence::Primary,
            _ => Precedence::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Vec<Stmt> {
        let tokens = crate::inklang::lexer::tokenize(source);
        Parser::new(tokens, None).parse().unwrap()
    }

    #[test]
    fn test_parse_literal() {
        let stmts = parse("42");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Literal(Value::Int(42)))));
    }

    #[test]
    fn test_parse_string_literal() {
        let stmts = parse("\"hello\"");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Literal(Value::String(s))) if s == "hello"
        ));
    }

    #[test]
    fn test_parse_boolean_literals() {
        let stmts = parse("true");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Literal(Value::Boolean(true)))
        ));

        let stmts = parse("false");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Literal(Value::Boolean(false)))
        ));
    }

    #[test]
    fn test_parse_null() {
        let stmts = parse("null");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Literal(Value::Null))));
    }

    #[test]
    fn test_parse_binary_expr() {
        let stmts = parse("1 + 2");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Binary { .. })));
    }

    #[test]
    fn test_parse_unary_expr() {
        let stmts = parse("-5");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Unary { op, .. }) if op.typ == TokenType::Minus
        ));

        let stmts = parse("!x");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Unary { op, .. }) if op.typ == TokenType::Bang
        ));

        let stmts = parse("not x");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Unary { op, .. }) if op.typ == TokenType::KwNot
        ));
    }

    #[test]
    fn test_parse_let_statement() {
        let stmts = parse("let x = 5");
        assert!(matches!(&stmts[0], Stmt::Let { name, .. } if name.lexeme == "x"));
    }

    #[test]
    fn test_parse_let_with_type() {
        let stmts = parse("let x: int = 5");
        assert!(matches!(&stmts[0], Stmt::Let { type_annot: Some(_), .. }));
    }

    #[test]
    fn test_parse_const_statement() {
        let stmts = parse("const PI = 3.14");
        assert!(matches!(&stmts[0], Stmt::Const { name, .. } if name.lexeme == "PI"));
    }

    #[test]
    fn test_parse_function() {
        let source = "fn add(a: int, b: int) -> int { a + b }";
        let stmts = parse(source);
        assert!(matches!(&stmts[0], Stmt::Fn { name, .. } if name.lexeme == "add"));
    }

    #[test]
    fn test_parse_function_no_params() {
        let source = "fn foo() { }";
        let stmts = parse(source);
        assert!(matches!(&stmts[0], Stmt::Fn { name, .. } if name.lexeme == "foo"));
    }

    #[test]
    fn test_parse_function_with_default_param() {
        let source = "fn foo(x: int = 5) { x }";
        let stmts = parse(source);
        assert!(matches!(&stmts[0], Stmt::Fn { params, .. } if params[0].default.is_some()));
    }

    #[test]
    fn test_parse_if_statement() {
        let stmts = parse("if x > 5 { 1 } else { 2 }");
        assert!(matches!(&stmts[0], Stmt::If { .. }));
    }

    #[test]
    fn test_parse_if_else_if() {
        let stmts = parse("if x > 5 { 1 } else if x > 3 { 2 } else { 3 }");
        assert!(matches!(&stmts[0], Stmt::If { else_branch: Some(_), .. }));
    }

    #[test]
    fn test_parse_while_statement() {
        let stmts = parse("while x < 10 { x = x + 1 }");
        assert!(matches!(&stmts[0], Stmt::While { .. }));
    }

    #[test]
    fn test_parse_for_statement() {
        let stmts = parse("for i in 0..10 { i }");
        assert!(matches!(&stmts[0], Stmt::For { variable, .. } if variable.lexeme == "i"));
    }

    #[test]
    fn test_parse_return_statement() {
        let stmts = parse("return 42");
        assert!(matches!(&stmts[0], Stmt::Return(Some(_))));
    }

    #[test]
    fn test_parse_return_no_value() {
        let stmts = parse("return");
        assert!(matches!(&stmts[0], Stmt::Return(None)));
    }

    #[test]
    fn test_parse_break_statement() {
        let stmts = parse("break");
        assert!(matches!(&stmts[0], Stmt::Break));
    }

    #[test]
    fn test_parse_next_statement() {
        let stmts = parse("next");
        assert!(matches!(&stmts[0], Stmt::Next));
    }

    #[test]
    fn test_parse_class() {
        let stmts = parse("class Foo { }");
        assert!(matches!(&stmts[0], Stmt::Class { name, .. } if name.lexeme == "Foo"));
    }

    #[test]
    fn test_parse_class_with_extends() {
        let stmts = parse("class Foo extends Bar { }");
        assert!(matches!(&stmts[0], Stmt::Class { superclass: Some(_), .. }));
    }

    #[test]
    fn test_parse_enum() {
        let stmts = parse("enum Color { Red, Green, Blue }");
        assert!(matches!(&stmts[0], Stmt::Enum { name, .. } if name.lexeme == "Color"));
    }

    #[test]
    fn test_parse_assignment() {
        let stmts = parse("x = 5");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Assign { .. })));
    }

    #[test]
    fn test_parse_compound_assignment() {
        let stmts = parse("x += 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Assign { op, .. }) => {
                assert_eq!(op.typ, TokenType::AddEquals);
            }
            _ => panic!("Expected Assign expression"),
        }
    }

    #[test]
    fn test_parse_logical_or() {
        let stmts = parse("x or y");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => {
                assert_eq!(op.typ, TokenType::KwOr);
            }
            _ => panic!("Expected Binary expression"),
        }
    }

    #[test]
    fn test_parse_logical_and() {
        let stmts = parse("x and y");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => {
                assert_eq!(op.typ, TokenType::KwAnd);
            }
            _ => panic!("Expected Binary expression"),
        }
    }

    #[test]
    fn test_parse_ternary() {
        let stmts = parse("x ? 1 : 2");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Ternary { .. })));
    }

    #[test]
    fn test_parse_elvis() {
        let stmts = parse("x ?? 0");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Elvis { .. })));
    }

    #[test]
    fn test_parse_is_expression() {
        let stmts = parse("x is Int");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Is { .. })));
    }

    #[test]
    fn test_parse_has_expression() {
        let stmts = parse("obj has \"field\"");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Has { .. })));
    }

    #[test]
    fn test_parse_comparison() {
        let stmts = parse("x == 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::EqEq),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x != 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::BangEq),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x < 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Lt),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x > 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Gt),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x <= 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Lte),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x >= 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Gte),
            _ => panic!("Expected Binary expression"),
        }
    }

    #[test]
    fn test_parse_range() {
        let stmts = parse("0..10");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::DotDot),
            _ => panic!("Expected Binary expression"),
        }
    }

    #[test]
    fn test_parse_factor() {
        let stmts = parse("x * 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Star),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x / 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Slash),
            _ => panic!("Expected Binary expression"),
        }

        let stmts = parse("x % 5");
        match &stmts[0] {
            Stmt::Expr(Expr::Binary { op, .. }) => assert_eq!(op.typ, TokenType::Percent),
            _ => panic!("Expected Binary expression"),
        }
    }

    #[test]
    fn test_parse_group_expression() {
        let stmts = parse("(1 + 2) * 3");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Binary { .. })));
    }

    #[test]
    fn test_parse_tuple() {
        let stmts = parse("(1, 2, 3)");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Tuple(_))));

        let stmts = parse("()");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Tuple(_))));
    }

    #[test]
    fn test_parse_list() {
        let stmts = parse("[1, 2, 3]");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::List(_))));
    }

    #[test]
    fn test_parse_set() {
        let stmts = parse("{1, 2, 3}");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Set(_))));
    }

    #[test]
    fn test_parse_map() {
        let stmts = parse("{\"key\": 42}");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Map(_))));
    }

    #[test]
    fn test_parse_call_expr() {
        let stmts = parse("foo(1, 2)");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Call { .. })));
    }

    #[test]
    fn test_parse_get_expr() {
        let stmts = parse("obj.field");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Get { .. })));
    }

    #[test]
    fn test_parse_index_expr() {
        let stmts = parse("arr[0]");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Index { .. })));
    }

    #[test]
    fn test_parse_safe_call() {
        let stmts = parse("obj?.field");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::SafeCall { .. })));
    }

    #[test]
    fn test_parse_lambda() {
        let stmts = parse("(x) -> { x }");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Lambda { .. })));
    }

    #[test]
    fn test_parse_async_lambda() {
        let stmts = parse("async (x) -> { x }");
        match &stmts[0] {
            Stmt::Expr(Expr::Lambda { is_async, .. }) => assert!(*is_async),
            _ => panic!("Expected Lambda expression"),
        }
    }

    #[test]
    fn test_parse_block() {
        let stmts = parse("{ 1; 2; 3 }");
        assert!(matches!(&stmts[0], Stmt::Block(_)));
    }

    #[test]
    fn test_parse_annotation() {
        let stmts = parse("@annotationName(x: 5) fn foo() { }");
        match &stmts[0] {
            Stmt::Fn { annotations, .. } => assert!(!annotations.is_empty()),
            _ => panic!("Expected Fn statement"),
        }
    }

    #[test]
    fn test_parse_interpolation() {
        let stmts = parse("\"hello ${name} world\"");
        // Should be desugared to string concatenation
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Binary { .. })));
    }

    #[test]
    fn test_parse_named_argument() {
        let stmts = parse("foo(x = 5)");
        match &stmts[0] {
            Stmt::Expr(Expr::Call { arguments, .. }) => {
                assert!(matches!(&arguments[0], Expr::NamedArg { .. }));
            }
            _ => panic!("Expected Call expression"),
        }
    }

    #[test]
    fn test_parse_config() {
        let stmts = parse("config Settings { port: int = 8080 }");
        match &stmts[0] {
            Stmt::Config { name, .. } => assert_eq!(name.lexeme, "Settings"),
            _ => panic!("Expected Config statement"),
        }
    }

    #[test]
    fn test_parse_table() {
        let stmts = parse("table Users { key id: int, name: string }");
        match &stmts[0] {
            Stmt::Table { name, .. } => assert_eq!(name.lexeme, "Users"),
            _ => panic!("Expected Table statement"),
        }
    }

    #[test]
    fn test_parse_import() {
        let stmts = parse("import foo");
        assert!(matches!(&stmts[0], Stmt::Import(_)));
    }

    #[test]
    fn test_parse_import_from() {
        // Correct Inklang syntax: import items from namespace
        let stmts = parse("import bar, baz from foo");
        assert!(matches!(&stmts[0], Stmt::ImportFrom { .. }));
    }

    #[test]
    fn test_parse_await() {
        let stmts = parse("await x");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Await(_))));
    }

    #[test]
    fn test_parse_spawn() {
        let stmts = parse("spawn x");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Spawn { virtual_: false, .. })));
    }

    #[test]
    fn test_parse_spawn_virtual() {
        let stmts = parse("spawn virtual x");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Spawn { virtual_: true, .. })));
    }

    #[test]
    fn test_parse_precedence() {
        // Test that * binds tighter than +
        let stmts = parse("1 + 2 * 3");
        if let Stmt::Expr(Expr::Binary { left, op: _, right }) = &stmts[0] {
            assert!(matches!(&**left, Expr::Literal(Value::Int(1))));
            assert!(matches!(&**right, Expr::Binary { .. }));
        } else {
            panic!("Expected binary expression");
        }

        // Test that parentheses override precedence
        let stmts = parse("(1 + 2) * 3");
        if let Stmt::Expr(Expr::Binary { left, .. }) = &stmts[0] {
            assert!(matches!(&**left, Expr::Group(_)));
        } else {
            panic!("Expected binary expression");
        }
    }

    #[test]
    fn test_parse_comments_are_ignored() {
        // Comments should be stripped by lexer, so this should parse fine
        let stmts = parse("5 // this is a comment\n6");
        assert!(stmts.len() >= 2);
    }
}
