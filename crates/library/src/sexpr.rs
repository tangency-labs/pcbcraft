use std::ops::Range;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("KiCad syntax at byte {offset}: {message}")]
    Syntax { offset: usize, message: String },
    #[error("KiCad input exceeds {0}")]
    Limit(&'static str),
    #[error("Invalid KiCad library: {0}")]
    Invalid(String),
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub bytes: usize,
    pub nodes: usize,
    pub depth: usize,
    pub atom_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: 32 * 1024 * 1024,
            nodes: 1_000_000,
            depth: 64,
            atom_bytes: 256 * 1024,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub atom: Option<String>,
    pub children: Vec<Node>,
    pub span: Range<usize>,
}
impl Node {
    pub fn name(&self) -> &str {
        self.children
            .first()
            .and_then(|x| x.atom.as_deref())
            .unwrap_or("")
    }
    pub fn value(&self, n: usize) -> &str {
        self.children
            .get(n)
            .and_then(|x| x.atom.as_deref())
            .unwrap_or("")
    }
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|x| x.name() == name)
    }
    pub fn field(&self, name: &str) -> &str {
        self.child(name).map(|x| x.value(1)).unwrap_or("")
    }
}
pub(crate) fn parse(text: &str) -> Result<Node, Error> {
    parse_limits(text, Limits::default())
}
pub(crate) fn parse_limits(text: &str, limits: Limits) -> Result<Node, Error> {
    if text.len() > limits.bytes {
        return Err(Error::Limit("source byte limit"));
    }
    let mut parser = Parser {
        text,
        pos: 0,
        count: 0,
        limits,
    };
    let node = parser.node(0)?;
    parser.space();
    if parser.pos != text.len() {
        return Err(parser.syntax("trailing input"));
    }
    Ok(node)
}
struct Parser<'a> {
    text: &'a str,
    pos: usize,
    count: usize,
    limits: Limits,
}

impl Parser<'_> {
    fn syntax(&self, message: &str) -> Error {
        Error::Syntax {
            offset: self.pos,
            message: message.to_owned(),
        }
    }
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.pos += 1;
        }
    }
    fn node(&mut self, depth: usize) -> Result<Node, Error> {
        self.space();
        if depth > self.limits.depth {
            return Err(Error::Limit("nesting depth"));
        }
        self.count += 1;
        if self.count > self.limits.nodes {
            return Err(Error::Limit("node count"));
        }
        let start = self.pos;
        match self.text.as_bytes().get(self.pos).copied() {
            Some(b'(') => {
                self.pos += 1;
                let mut children = Vec::new();
                loop {
                    self.space();
                    match self.text.as_bytes().get(self.pos) {
                        Some(b')') => {
                            self.pos += 1;
                            break;
                        }
                        None => return Err(self.syntax("unclosed list")),
                        _ => children.push(self.node(depth + 1)?),
                    }
                }
                Ok(Node {
                    atom: None,
                    children,
                    span: start..self.pos,
                })
            }
            Some(b'"') => {
                self.pos += 1;
                let mut value = String::new();
                loop {
                    let rest = self
                        .text
                        .get(self.pos..)
                        .ok_or_else(|| self.syntax("invalid UTF-8 boundary"))?;
                    let c = rest
                        .chars()
                        .next()
                        .ok_or_else(|| self.syntax("unclosed string"))?;
                    self.pos += c.len_utf8();
                    if c == '"' {
                        break;
                    }
                    if c == '\\' {
                        let c = self
                            .text
                            .get(self.pos..)
                            .and_then(|s| s.chars().next())
                            .ok_or_else(|| self.syntax("unclosed escape"))?;
                        self.pos += c.len_utf8();
                        value.push(match c {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        value.push(c);
                    }
                    if value.len() > self.limits.atom_bytes {
                        return Err(Error::Limit("atom byte limit"));
                    }
                }
                Ok(Node {
                    atom: Some(value),
                    children: Vec::new(),
                    span: start..self.pos,
                })
            }
            Some(b')') | None => Err(self.syntax("expected expression")),
            _ => {
                while self
                    .text
                    .as_bytes()
                    .get(self.pos)
                    .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'(' && *b != b')')
                {
                    self.pos += 1;
                }
                if self.pos - start > self.limits.atom_bytes {
                    return Err(Error::Limit("atom byte limit"));
                }
                Ok(Node {
                    atom: Some(self.text[start..self.pos].to_owned()),
                    children: Vec::new(),
                    span: start..self.pos,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_node_depth_and_atom_budgets() {
        let defaults = Limits::default();
        assert!(matches!(
            parse_limits(
                "(a)",
                Limits {
                    bytes: 2,
                    ..defaults
                }
            ),
            Err(Error::Limit(_))
        ));
        assert!(matches!(
            parse_limits(
                "(a b c)",
                Limits {
                    nodes: 3,
                    ..defaults
                }
            ),
            Err(Error::Limit(_))
        ));
        assert!(matches!(
            parse_limits(
                "((a))",
                Limits {
                    depth: 1,
                    ..defaults
                }
            ),
            Err(Error::Limit(_))
        ));
        assert!(matches!(
            parse_limits(
                "(abcd)",
                Limits {
                    atom_bytes: 3,
                    ..defaults
                }
            ),
            Err(Error::Limit(_))
        ));
        assert!(matches!(
            parse_limits(
                "(\"abcd\")",
                Limits {
                    atom_bytes: 3,
                    ..defaults
                }
            ),
            Err(Error::Limit(_))
        ));
    }
    #[test]
    fn unicode_escaped_strings_and_malformed_input() -> Result<(), Error> {
        let tree = parse("(name \"µΩ\\\"\\n\")")?;
        assert_eq!(tree.value(1), "µΩ\"\n");
        for input in ["(\"unterminated)", "(\"escape\\", "(a) (b)", ")"] {
            assert!(parse(input).is_err());
        }
        Ok(())
    }
}
