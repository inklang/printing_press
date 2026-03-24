//! Error types for the Inklang compiler.

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

    /// General parse error with a message.
    Parse(String),

    /// Lexer error.
    Lexer(String),

    /// Compilation error.
    Compile(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnexpectedToken(token) => write!(f, "Unexpected token: {}", token),
            Error::ExpectedToken { expected, found } => {
                write!(f, "Expected {} but found {}", expected, found)
            }
            Error::UnterminatedString => write!(f, "Unterminated string"),
            Error::Parse(msg) => write!(f, "Parse error: {}", msg),
            Error::Lexer(msg) => write!(f, "Lexer error: {}", msg),
            Error::Compile(msg) => write!(f, "Compilation error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}
