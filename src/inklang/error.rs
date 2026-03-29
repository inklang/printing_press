//! Error types for the Inklang compiler.

/// A source location (line and column, both 1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Result type for parser operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Parse error types.
#[derive(Debug, Clone)]
pub enum Error {
    /// Unexpected token encountered.
    UnexpectedToken(String),

    /// Expected a specific token type but found something else.
    ExpectedToken {
        expected: String,
        found: String,
    },

    /// Unterminated string literal.
    UnterminatedString,

    /// General parse error with a message and source location.
    Parse {
        message: String,
        span: Span,
    },

    /// Lexer error.
    Lexer(String),

    /// Compilation error.
    Compile(String),
}

impl Error {
    /// Return the span associated with this error, if any.
    pub fn span(&self) -> Option<Span> {
        match self {
            Error::Parse { span, .. } => Some(*span),
            _ => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnexpectedToken(token) => write!(f, "Unexpected token: {}", token),
            Error::ExpectedToken { expected, found } => {
                write!(f, "Expected {} but found {}", expected, found)
            }
            Error::UnterminatedString => write!(f, "Unterminated string"),
            Error::Parse { message, span } => {
                write!(f, "{} at {}", message, span)
            }
            Error::Lexer(msg) => write!(f, "Lexer error: {}", msg),
            Error::Compile(msg) => write!(f, "Compilation error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}
