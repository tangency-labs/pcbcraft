//! Project-independent object-query syntax. Parsing neither resolves objects nor edits a session.
//!
//! The grammar and byte-offset diagnostic contract are documented in `docs/object-queries.md`.

use std::{fmt, ops::Range};

use pcbcraft_units::quantity::Quantity;

/// Maximum UTF-8 input size, including whitespace and quoted text.
pub const MAX_QUERY_BYTES: usize = 16_384;
/// Maximum number of non-end tokens consumed by a parse.
pub const MAX_QUERY_TOKENS: usize = 4_096;
/// Maximum combined nesting of parentheses and unary negations.
pub const MAX_QUERY_DEPTH: usize = 32;

/// A complete parsed expression, independent of any project, board or revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub expression: Expression,
}

/// An expression and its half-open UTF-8 byte range in the original input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expression {
    pub span: Range<usize>,
    pub kind: ExpressionKind,
}

/// Boolean syntax; repeated operators at one precedence level share one vector.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExpressionKind {
    Constant(bool),
    Predicate(Predicate),
    Not(Box<Expression>),
    And(Vec<Expression>),
    Or(Vec<Expression>),
}

/// Text selectors use exact text; numeric properties retain normalized decimal digits.
/// Resolution, property availability and type checking belong to a future snapshot consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Predicate {
    Text { field: TextField, comparison: Comparison, value: String },
    Number { property: String, comparison: Comparison, value: String },
}

/// Text-addressed query selectors. A reference names a placed part instance, not a definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextField {
    Kind,
    Layer,
    Net,
    NetClass,
    PartReference,
    PartGroup,
    Footprint,
}

/// Comparisons of numeric values; text predicates permit only equality and inequality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// Stable categories for syntax diagnostics, separate from their human-readable explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    UnexpectedToken,
    UnknownField,
    InvalidComparison,
    InvalidString,
    InvalidNumber,
    LimitExceeded,
}

/// A refused query. `offset` is a zero-based UTF-8 byte boundary, or the input length at EOF.
/// No partial tree is returned on any error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    /// Include the byte position without imposing a GUI's line or character indexing convention.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse a complete query with `not` before `and` before `or`; comparisons form predicates.
/// Word operators are lowercase and symbol equivalents are `!`, `&&` and `||`.
/// Refuses malformed input, unsupported syntax and resource limits without accessing state.
pub fn parse(text: &str) -> Result<Query, ParseError> {
    if text.len() > MAX_QUERY_BYTES {
        return Err(error(ParseErrorKind::LimitExceeded, 0, "Query exceeds 16384 UTF-8 bytes."));
    }
    let mut parser = Parser { text, cursor: 0, tokens: 0, current: Token { kind: TokenKind::End, span: 0..0 } };
    parser.advance()?;
    let expression = parser.boolean(false, 0)?;
    if !matches!(parser.current.kind, TokenKind::End) {
        return Err(parser.expected("Expected 'and', 'or' or end of query."));
    }
    Ok(Query { expression })
}

/// Construct a diagnostic at a caller-supplied byte boundary.
fn error(kind: ParseErrorKind, offset: usize, message: &str) -> ParseError {
    ParseError { kind, offset, message: message.to_owned() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TokenKind<'a> {
    Word(&'a str),
    Text(String),
    Number(&'a str),
    Comparison(Comparison),
    And,
    Or,
    Not,
    Open,
    Close,
    End,
}

#[derive(Clone, Debug)]
struct Token<'a> {
    kind: TokenKind<'a>,
    span: Range<usize>,
}

struct Parser<'a> {
    text: &'a str,
    cursor: usize,
    tokens: usize,
    current: Token<'a>,
}

impl<'a> Parser<'a> {
    /// Lex one token, bounding scanning and allocation by the input and token limits.
    fn advance(&mut self) -> Result<(), ParseError> {
        while let Some(ch) = self.text[self.cursor..].chars().next().filter(|ch| ch.is_whitespace()) {
            self.cursor += ch.len_utf8();
        }
        let start = self.cursor;
        let bytes = self.text.as_bytes();
        let Some(&byte) = bytes.get(start) else {
            self.current = Token { kind: TokenKind::End, span: start..start };
            return Ok(());
        };
        if self.tokens == MAX_QUERY_TOKENS {
            return Err(error(ParseErrorKind::LimitExceeded, start, "Query exceeds 4096 tokens."));
        }
        self.tokens += 1;
        self.cursor += 1;
        let kind = match byte {
            b'(' => TokenKind::Open,
            b')' => TokenKind::Close,
            b'=' => TokenKind::Comparison(Comparison::Equal),
            b'!' | b'<' | b'>' => {
                let equal = bytes.get(self.cursor) == Some(&b'=');
                self.cursor += usize::from(equal);
                match (byte, equal) {
                    (b'!', true) => TokenKind::Comparison(Comparison::NotEqual),
                    (b'!', false) => TokenKind::Not,
                    (b'<', true) => TokenKind::Comparison(Comparison::LessOrEqual),
                    (b'<', false) => TokenKind::Comparison(Comparison::Less),
                    (b'>', true) => TokenKind::Comparison(Comparison::GreaterOrEqual),
                    _ => TokenKind::Comparison(Comparison::Greater),
                }
            }
            b'&' | b'|' if bytes.get(self.cursor) == Some(&byte) => {
                self.cursor += 1;
                if byte == b'&' { TokenKind::And } else { TokenKind::Or }
            }
            b'"' => TokenKind::Text(self.quoted(start)?),
            b'0'..=b'9' | b'+' | b'-' | b'.' => {
                while bytes.get(self.cursor).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-')) {
                    self.cursor += 1;
                }
                TokenKind::Number(&self.text[start..self.cursor])
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while bytes.get(self.cursor).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                    self.cursor += 1;
                }
                match &self.text[start..self.cursor] {
                    "and" => TokenKind::And,
                    "or" => TokenKind::Or,
                    "not" => TokenKind::Not,
                    word => TokenKind::Word(word),
                }
            }
            _ => return Err(error(ParseErrorKind::UnexpectedToken, start, "Unexpected character.")),
        };
        self.current = Token { kind, span: start..self.cursor };
        Ok(())
    }

    /// Read JSON-style quoted text; malformed escapes, unpaired surrogates and controls fail
    /// at the opening quote. The scanner advances by UTF-8 characters, never into a code point.
    fn quoted(&mut self, start: usize) -> Result<String, ParseError> {
        let mut escaped = false;
        while let Some(ch) = self.text[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
            if !escaped && ch == '"' {
                return serde_json::from_str(&self.text[start..self.cursor])
                    .map_err(|_| error(ParseErrorKind::InvalidString, start, "Invalid quoted text or escape."));
            }
            escaped = !escaped && ch == '\\';
        }
        Err(error(ParseErrorKind::InvalidString, start, "Unterminated quoted text."))
    }

    /// Parse a flat chain at one precedence level, keeping long chains shallow to clone/drop.
    /// `conjunction` selects AND; OR delegates to AND for its operands.
    fn boolean(&mut self, conjunction: bool, depth: usize) -> Result<Expression, ParseError> {
        let first = self.operand(conjunction, depth)?;
        let mut children = vec![first];
        while if conjunction { self.current.kind == TokenKind::And } else { self.current.kind == TokenKind::Or } {
            self.advance()?;
            children.push(self.operand(conjunction, depth)?);
        }
        if children.len() == 1 {
            return Ok(children.remove(0));
        }
        let span = children[0].span.start..children[children.len() - 1].span.end;
        let kind = if conjunction { ExpressionKind::And(children) } else { ExpressionKind::Or(children) };
        Ok(Expression { span, kind })
    }

    /// Delegate one precedence level without increasing the user-controlled nesting depth.
    fn operand(&mut self, conjunction: bool, depth: usize) -> Result<Expression, ParseError> {
        if conjunction { self.unary(depth) } else { self.boolean(true, depth) }
    }

    /// Parse a negation, grouped expression or predicate with a bounded recursive stack.
    fn unary(&mut self, depth: usize) -> Result<Expression, ParseError> {
        let start = self.current.span.start;
        if matches!(self.current.kind, TokenKind::Not | TokenKind::Open) {
            if depth == MAX_QUERY_DEPTH {
                return Err(error(ParseErrorKind::LimitExceeded, start, "Query exceeds 32 nested parentheses or negations."));
            }
            let negate = self.current.kind == TokenKind::Not;
            self.advance()?;
            if negate {
                let child = self.unary(depth + 1)?;
                return Ok(Expression { span: start..child.span.end, kind: ExpressionKind::Not(Box::new(child)) });
            }
            let mut child = self.boolean(false, depth + 1)?;
            if self.current.kind != TokenKind::Close {
                return Err(self.expected("Expected ')'."));
            }
            child.span = start..self.current.span.end;
            self.advance()?;
            return Ok(child);
        }
        self.predicate()
    }

    /// Parse text equality or a numeric property comparison; field and operator errors are
    /// distinguished from missing operands. This does not resolve names against a project.
    fn predicate(&mut self) -> Result<Expression, ParseError> {
        let start = self.current.span.start;
        let TokenKind::Word(word) = self.current.kind else {
            return Err(self.expected("Expected a selector, 'true', 'false', 'not' or '('."));
        };
        if word == "true" || word == "false" {
            let expression = Expression { span: self.current.span.clone(), kind: ExpressionKind::Constant(word == "true") };
            self.advance()?;
            return Ok(expression);
        }
        let field = match word {
            "kind" => Some(TextField::Kind),
            "layer" => Some(TextField::Layer),
            "net" => Some(TextField::Net),
            "net_class" => Some(TextField::NetClass),
            "part_reference" => Some(TextField::PartReference),
            "part_group" => Some(TextField::PartGroup),
            "footprint" => Some(TextField::Footprint),
            "property" => None,
            _ => return Err(error(ParseErrorKind::UnknownField, start, "Unknown selector.")),
        };
        self.advance()?;
        let property = if field.is_none() {
            if self.current.kind != TokenKind::Open {
                return Err(self.expected("Expected '(' after 'property'."));
            }
            self.advance()?;
            let (property, _) = self.text_value()?;
            if property.is_empty() {
                return Err(error(ParseErrorKind::UnexpectedToken, start, "Property identifier must not be empty."));
            }
            if self.current.kind != TokenKind::Close {
                return Err(self.expected("Expected ')' after property identifier."));
            }
            self.advance()?;
            property
        } else {
            String::new()
        };
        let operator_offset = self.current.span.start;
        let TokenKind::Comparison(comparison) = self.current.kind else {
            return Err(self.expected("Expected a comparison operator."));
        };
        if field.is_some() && !matches!(comparison, Comparison::Equal | Comparison::NotEqual) {
            return Err(error(ParseErrorKind::InvalidComparison, operator_offset, "Text selectors accept only '=' and '!='."));
        }
        self.advance()?;
        let (predicate, end) = if let Some(field) = field {
            let (value, end) = self.text_value()?;
            (Predicate::Text { field, comparison, value }, end)
        } else {
            let TokenKind::Number(number) = self.current.kind else {
                return Err(self.expected("Expected a decimal number."));
            };
            let offset = self.current.span.start;
            let value = number_value(number).map_err(|message| error(ParseErrorKind::InvalidNumber, offset, message))?;
            let end = self.current.span.end;
            self.advance()?;
            (Predicate::Number { property, comparison, value }, end)
        };
        Ok(Expression { span: start..end, kind: ExpressionKind::Predicate(predicate) })
    }

    /// Consume quoted text, retaining the token's end before lexing the following token.
    fn text_value(&mut self) -> Result<(String, usize), ParseError> {
        let TokenKind::Text(value) = &self.current.kind else {
            return Err(self.expected("Expected double-quoted text."));
        };
        let value = value.clone();
        let end = self.current.span.end;
        self.advance()?;
        Ok((value, end))
    }

    /// Report an unmet grammar production at the current token, including EOF.
    fn expected(&self, message: &str) -> ParseError {
        error(ParseErrorKind::UnexpectedToken, self.current.span.start, message)
    }
}

/// Require `[+-]?digits[.digits][e[+-]?digits]`, then use the shared exact decimal
/// normalizer without a unit suffix. It caps input at 128 bytes, 36 digits and 18 places.
fn number_value(text: &str) -> Result<String, &'static str> {
    let bytes = text.as_bytes();
    let mut i = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let whole = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == whole {
        return Err("Expected digits before the decimal point.");
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let fraction = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if fraction == i {
            return Err("Expected digits after the decimal point.");
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        i += usize::from(matches!(bytes.get(i), Some(b'+' | b'-')));
        let exponent = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if exponent == i {
            return Err("Expected exponent digits.");
        }
    }
    if i != bytes.len() {
        return Err("Invalid decimal number; unit suffixes are not supported.");
    }
    Quantity::Ratio.parse(text).map(|value| value.value().to_owned()).map_err(|_| "Number exceeds shared decimal precision or exponent limits.")
}

#[cfg(test)]
mod tests;
