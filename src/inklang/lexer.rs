use super::token::{Token, TokenType};
use std::collections::HashMap;

// Tokens that can end a statement (for ASI)
// Note: R_BRACE is NOT included because it's a block terminator, not a statement terminator
const STATEMENT_ENDERS: &[TokenType] = &[
    TokenType::Identifier,
    TokenType::KwInt,
    TokenType::KwFloat,
    TokenType::KwDouble,
    TokenType::KwString,
    TokenType::KwTrue,
    TokenType::KwFalse,
    TokenType::KwNull,
    TokenType::RParen,
    TokenType::RSquare,
    TokenType::KwBreak,
    TokenType::KwNext,
];

fn keywords() -> HashMap<&'static str, TokenType> {
    HashMap::from([
        ("bool", TokenType::KwBool),
        ("int", TokenType::KwInt),
        ("float", TokenType::KwFloat),
        ("double", TokenType::KwDouble),
        ("string", TokenType::KwString),
        ("true", TokenType::KwTrue),
        ("false", TokenType::KwFalse),
        ("let", TokenType::KwLet),
        ("const", TokenType::KwConst),
        ("if", TokenType::KwIf),
        ("else", TokenType::KwElse),
        ("while", TokenType::KwWhile),
        ("for", TokenType::KwFor),
        ("in", TokenType::KwIn),
        ("fn", TokenType::KwFn),
        ("return", TokenType::KwReturn),
        ("and", TokenType::KwAnd),
        ("or", TokenType::KwOr),
        ("not", TokenType::KwNot),
        ("null", TokenType::KwNull),
        ("break", TokenType::KwBreak),
        ("next", TokenType::KwNext),
        ("enum", TokenType::KwEnum),
        ("class", TokenType::KwClass),
        ("extends", TokenType::KwExtends),
        ("import", TokenType::KwImport),
        ("from", TokenType::KwFrom),
        ("is", TokenType::KwIs),
        ("has", TokenType::KwHas),
        ("table", TokenType::KwTable),
        ("key", TokenType::KwKey),
        ("config", TokenType::KwConfig),
        ("try", TokenType::KwTry),
        ("catch", TokenType::KwCatch),
        ("finally", TokenType::KwFinally),
        ("throw", TokenType::KwThrow),
        ("annotation", TokenType::KwAnnotation),
        ("on", TokenType::KwOn),
        ("event", TokenType::KwEvent),
        ("enable", TokenType::KwEnable),
        ("disable", TokenType::KwDisable),
        ("async", TokenType::KwAsync),
        ("await", TokenType::KwAwait),
        ("spawn", TokenType::KwSpawn),
        ("virtual", TokenType::KwVirtual),
    ])
}

struct Lexer<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    start: usize,
    cursor: usize,
    line: usize,
    column: usize,
    interpolation_depth: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Lexer {
            source,
            tokens: Vec::new(),
            start: 0,
            cursor: 0,
            line: 1,
            column: 0,
            interpolation_depth: 0,
        }
    }

    fn tokenize(&mut self) -> Vec<Token> {
        if !self.tokens.is_empty() {
            return std::mem::take(&mut self.tokens);
        }

        while !self.is_at_end() {
            self.start = self.cursor;
            let c = self.advance();

            match c {
                // Grouping & Punctuation
                '(' => self.add_token(TokenType::LParen),
                ')' => self.add_token(TokenType::RParen),
                '{' => self.add_token(TokenType::LBrace),
                '}' => {
                    if self.interpolation_depth > 0 {
                        self.handle_interpolation_end();
                    } else {
                        self.add_token(TokenType::RBrace);
                    }
                }
                '[' => self.add_token(TokenType::LSquare),
                ']' => self.add_token(TokenType::RSquare),
                ',' => self.add_token(TokenType::Comma),
                '.' => {
                    if self.match_char('.') {
                        self.add_token(TokenType::DotDot);
                    } else {
                        self.add_token(TokenType::Dot);
                    }
                }
                ';' => self.add_token(TokenType::Semicolon),
                ':' => self.add_token(TokenType::Colon),
                '@' => self.add_token(TokenType::At),

                // Math & Operators
                '+' => {
                    if self.match_char('+') {
                        self.add_token(TokenType::Increment);
                    } else if self.match_char('=') {
                        self.add_token(TokenType::AddEquals);
                    } else {
                        self.add_token(TokenType::Plus);
                    }
                }

                '-' => {
                    if self.match_char('-') {
                        self.add_token(TokenType::Decrement);
                    } else if self.match_char('=') {
                        self.add_token(TokenType::SubEquals);
                    } else if self.match_char('>') {
                        self.add_token(TokenType::Arrow);
                    } else {
                        self.add_token(TokenType::Minus);
                    }
                }

                '*' => {
                    if self.match_char('*') {
                        self.add_token(TokenType::Pow);
                    } else if self.match_char('=') {
                        self.add_token(TokenType::MulEquals);
                    } else {
                        self.add_token(TokenType::Star);
                    }
                }

                '/' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::DivEquals);
                    } else if self.match_char('/') {
                        // Comment: consume until newline
                        while self.peek() != '\n' && !self.is_at_end() {
                            self.advance();
                        }
                    } else {
                        self.add_token(TokenType::Slash);
                    }
                }

                '%' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::ModEquals);
                    } else {
                        self.add_token(TokenType::Percent);
                    }
                }

                '!' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::BangEq);
                    } else {
                        self.add_token(TokenType::Bang);
                    }
                }
                '=' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::EqEq);
                    } else {
                        self.add_token(TokenType::Assign);
                    }
                }
                '<' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::Lte);
                    } else {
                        self.add_token(TokenType::Lt);
                    }
                }
                '>' => {
                    if self.match_char('=') {
                        self.add_token(TokenType::Gte);
                    } else {
                        self.add_token(TokenType::Gt);
                    }
                }
                '?' => {
                    if self.match_char('.') {
                        self.add_token(TokenType::QuestionDot);
                    } else if self.match_char('?') {
                        self.add_token(TokenType::QuestionQuestion);
                    } else {
                        self.add_token(TokenType::Question);
                    }
                }

                ' ' | '\r' | '\t' => {}

                '\n' => {
                    // Automatic Semicolon Insertion (ASI)
                    if !self.tokens.is_empty()
                        && STATEMENT_ENDERS.contains(&self.tokens.last().unwrap().typ)
                    {
                        self.add_token(TokenType::Semicolon);
                    }
                    self.line += 1;
                    self.column = 0;
                }

                '"' => self.string(),

                _ => {
                    if c.is_ascii_digit() {
                        self.number();
                    } else if c.is_ascii_alphabetic() || c == '_' {
                        self.identifier();
                    }
                }
            }
        }

        self.add_token(TokenType::Eof);
        std::mem::take(&mut self.tokens)
    }

    fn string(&mut self) {
        loop {
            // Check for interpolation start ${
            if self.peek() == '$' && self.peek_next() == '{' {
                // Emit the string part we've accumulated so far
                if self.cursor > self.start + 1 {
                    let start_idx = self.start + 1;
                    let end_idx = self.cursor;
                    if end_idx > start_idx {
                        let value = &self.source[start_idx..end_idx];
                        let lexeme = format!("\"{}\"", value);
                        self.tokens.push(Token {
                            typ: TokenType::KwString,
                            lexeme,
                            line: self.line,
                            column: self.column - value.len(),
                        });
                    }
                }

                // Emit INTERPOLATION_START
                self.advance(); // consume $
                self.advance(); // consume {
                self.tokens.push(Token {
                    typ: TokenType::InterpolationStart,
                    lexeme: "${".to_string(),
                    line: self.line,
                    column: self.column - 1,
                });
                self.interpolation_depth += 1;
                return; // Let normal tokenization handle the expression inside
            }

            if self.peek() == '"' {
                // Closing quote
                break;
            }

            if self.is_at_end() {
                return;
            }

            if self.peek() == '\n' {
                self.line += 1;
                self.column = 0;
            }

            // Handle escape sequences
            if self.peek() == '\\' {
                self.advance(); // consume backslash
                if !self.is_at_end() {
                    self.advance(); // consume escaped char
                }
            } else {
                self.advance();
            }
        }

        if self.is_at_end() {
            return;
        }

        // Closing quote
        self.advance();

        // Trim the surrounding quotes
        let value = &self.source[self.start + 1..self.cursor - 1];
        self.add_token(TokenType::KwString);
    }

    fn handle_interpolation_end(&mut self) {
        // Note: The } has already been consumed by advance() in the main loop
        self.tokens.push(Token {
            typ: TokenType::InterpolationEnd,
            lexeme: "}".to_string(),
            line: self.line,
            column: self.column - 1,
        });
        self.interpolation_depth -= 1;

        // After closing interpolation, we might have more string content
        if self.peek() == '"' {
            // End of the interpolated string
            self.advance(); // consume closing quote
        } else {
            // Continue scanning string content after the interpolation
            self.scan_string_tail();
        }
    }

    fn scan_string_tail(&mut self) {
        // Continue scanning string content after an interpolation
        self.start = self.cursor;

        while self.peek() != '"' && !self.is_at_end() {
            // Check for another interpolation
            if self.peek() == '$' && self.peek_next() == '{' {
                // Emit the string part we've accumulated
                if self.cursor > self.start {
                    let value = &self.source[self.start..self.cursor];
                    let lexeme = format!("\"{}\"", value);
                    self.tokens.push(Token {
                        typ: TokenType::KwString,
                        lexeme,
                        line: self.line,
                        column: self.column - value.len(),
                    });
                }

                // Emit INTERPOLATION_START
                self.advance(); // consume $
                self.advance(); // consume {
                self.tokens.push(Token {
                    typ: TokenType::InterpolationStart,
                    lexeme: "${".to_string(),
                    line: self.line,
                    column: self.column - 1,
                });
                self.interpolation_depth += 1;
                return;
            }

            if self.peek() == '\n' {
                self.line += 1;
                self.column = 0;
            }

            // Handle escape sequences
            if self.peek() == '\\' {
                self.advance(); // consume backslash
                if !self.is_at_end() {
                    self.advance(); // consume escaped char
                }
            } else {
                self.advance();
            }
        }

        if self.is_at_end() {
            return;
        }

        // Closing quote
        self.advance();

        // Emit the final string part (may be empty)
        if self.cursor > self.start + 1 {
            let value = &self.source[self.start..self.cursor - 1];
            let lexeme = format!("\"{}\"", value);
            self.tokens.push(Token {
                typ: TokenType::KwString,
                lexeme,
                line: self.line,
                column: self.column - value.len(),
            });
        }
    }

    fn identifier(&mut self) {
        while self.peek().is_ascii_alphanumeric() || self.peek() == '_' {
            self.advance();
        }
        let text = &self.source[self.start..self.cursor];
        let kw = keywords();
        let typ = *kw.get(text).unwrap_or(&TokenType::Identifier);
        self.add_token(typ);
    }

    fn number(&mut self) {
        while self.peek().is_ascii_digit() {
            self.advance();
        }
        if self.peek() == '.' && self.peek_next().is_ascii_digit() {
            self.advance();
            while self.peek().is_ascii_digit() {
                self.advance();
            }
            self.add_token(TokenType::KwDouble);
        } else {
            self.add_token(TokenType::KwInt);
        }
    }

    fn advance(&mut self) -> char {
        let c = self.source[self.cursor..].chars().next().unwrap();
        self.cursor += 1;
        self.column += 1;
        c
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.source[self.cursor..].chars().next() != Some(expected) {
            return false;
        }
        self.cursor += 1;
        self.column += 1;
        true
    }

    fn peek(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.source[self.cursor..].chars().next().unwrap()
        }
    }

    fn peek_next(&self) -> char {
        if self.cursor + 1 >= self.source.len() {
            '\0'
        } else {
            self.source[self.cursor + 1..].chars().next().unwrap()
        }
    }

    fn is_at_end(&self) -> bool {
        self.cursor >= self.source.len()
    }

    fn add_token(&mut self, typ: TokenType) {
        let text = self.source[self.start..self.cursor].to_string();
        let len = text.len();
        self.tokens.push(Token {
            typ,
            lexeme: text,
            line: self.line,
            column: self.column.saturating_sub(len),
        });
    }
}

pub fn tokenize(source: &str) -> Vec<Token> {
    Lexer::new(source).tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_int() {
        let tokens = tokenize("42");
        assert_eq!(tokens[0].typ, TokenType::KwInt);
        assert_eq!(tokens[0].lexeme, "42");
    }

    #[test]
    fn test_tokenize_string() {
        let tokens = tokenize("\"hello\"");
        assert_eq!(tokens[0].typ, TokenType::KwString);
        assert_eq!(tokens[0].lexeme, "\"hello\"");
    }

    #[test]
    fn test_tokenize_keywords() {
        let tokens = tokenize("let x = 5");
        assert_eq!(tokens[0].typ, TokenType::KwLet);
        assert_eq!(tokens[1].typ, TokenType::Identifier);
        assert_eq!(tokens[1].lexeme, "x");
        assert_eq!(tokens[2].typ, TokenType::Assign);
        assert_eq!(tokens[3].typ, TokenType::KwInt);
        assert_eq!(tokens[3].lexeme, "5");
    }

    #[test]
    fn test_tokenize_operators() {
        let tokens = tokenize("a + b == c");
        // a, +, b, ==, c
        assert_eq!(tokens[1].typ, TokenType::Plus);
        assert_eq!(tokens[3].typ, TokenType::EqEq);
    }

    #[test]
    fn test_tokenize_interpolation() {
        let tokens = tokenize("\"hello ${name} world\"");
        assert!(tokens
            .iter()
            .any(|t| t.typ == TokenType::InterpolationStart));
        assert!(tokens
            .iter()
            .any(|t| t.typ == TokenType::InterpolationEnd));
    }

    #[test]
    fn test_tokenize_bool() {
        let tokens = tokenize("true false");
        assert_eq!(tokens[0].typ, TokenType::KwTrue);
        assert_eq!(tokens[1].typ, TokenType::KwFalse);
    }

    #[test]
    fn test_tokenize_double() {
        let tokens = tokenize("3.14");
        assert_eq!(tokens[0].typ, TokenType::KwDouble);
        assert_eq!(tokens[0].lexeme, "3.14");
    }

    #[test]
    fn test_tokenize_comment() {
        let tokens = tokenize("5 // this is a comment\n6");
        // Should get: 5, SEMICOLON (ASI), 6, EOF
        assert_eq!(tokens[0].typ, TokenType::KwInt);
        assert_eq!(tokens[1].typ, TokenType::Semicolon); // ASI inserts semicolon
        assert_eq!(tokens[2].typ, TokenType::KwInt);
    }

    #[test]
    fn test_tokenize_asi() {
        let tokens = tokenize("5\n6");
        // Should get: 5, SEMICOLON (ASI), 6, EOF
        assert_eq!(tokens[0].typ, TokenType::KwInt);
        assert_eq!(tokens[1].typ, TokenType::Semicolon);
        assert_eq!(tokens[2].typ, TokenType::KwInt);
    }

    #[test]
    fn test_tokenize_question_operators() {
        let tokens = tokenize("a?.b ?? c");
        // a, ?., b, ??, c
        assert_eq!(tokens[1].typ, TokenType::QuestionDot);
        assert_eq!(tokens[3].typ, TokenType::QuestionQuestion);
    }

    #[test]
    fn test_tokenize_arrow() {
        let tokens = tokenize("->");
        assert_eq!(tokens[0].typ, TokenType::Arrow);
    }

    #[test]
    fn test_tokenize_extends() {
        let tokens = tokenize("class Foo extends Bar {}");
        let extends_token = tokens.iter().find(|t| t.lexeme == "extends").unwrap();
        assert_eq!(extends_token.typ, TokenType::KwExtends);
    }

    #[test]
    fn test_tokenize_complex() {
        let source = r#"let greeting = "Hello, ${name}!";"#;
        let tokens = tokenize(source);
        // Should have: let, greeting, =, "Hello, ", ${, name, }, "!", ;
        let has_string = tokens.iter().any(|t| t.typ == TokenType::KwString);
        let has_interp_start = tokens
            .iter()
            .any(|t| t.typ == TokenType::InterpolationStart);
        let has_interp_end = tokens.iter().any(|t| t.typ == TokenType::InterpolationEnd);
        assert!(has_string);
        assert!(has_interp_start);
        assert!(has_interp_end);
    }
}
