//! S-expressions, as KiCad files are written: lists in parentheses of bare words, quoted strings
//! and more lists.
//!
//! A file is read one top-level item at a time ([`Reader::next_item`]), so the memory in use
//! follows the largest item, not the file. Nesting and the size of one item are bounded.
//!
//! Adapted from CircuitCraft's `crates/import/src/sexp.rs` (MIT OR Apache-2.0); see ATTRIBUTION.md.

use crate::Error;
use std::borrow::Cow;

#[cfg(test)]
mod tests;

/// How deep lists may nest. KiCad schematics nest about ten deep.
pub(crate) const MAX_DEPTH: usize = 64;

/// The most words and lists one top-level item may hold. A schematic's largest item is its
/// `lib_symbols` list, which holds every symbol the file draws with.
pub(crate) const MAX_NODES: usize = 8_000_000;

/// A word (bare or quoted) or a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node<'a> {
    Atom(Cow<'a, str>),
    List(Vec<Node<'a>>),
}

impl<'a> Node<'a> {
    /// The parts of a list; none for a word.
    fn parts(&self) -> &[Node<'a>] {
        match self {
            Node::Atom(_) => &[],
            Node::List(parts) => parts,
        }
    }

    /// This node as a word; `None` for a list.
    pub(crate) fn word(&self) -> Option<&str> {
        match self {
            Node::Atom(word) => Some(word),
            Node::List(_) => None,
        }
    }

    /// The word a list starts with: what kind of thing it is. `None` for a word or an empty list.
    pub(crate) fn head(&self) -> Option<&str> {
        self.parts().first().and_then(Node::word)
    }

    /// What follows the head.
    pub(crate) fn args(&self) -> &[Node<'a>] {
        self.parts().get(1..).unwrap_or_default()
    }

    /// The `i`-th argument, if it is a word.
    pub(crate) fn text(&self, i: usize) -> Option<&str> {
        self.args().get(i).and_then(Node::word)
    }

    /// The `i`-th argument, if it is a finite number.
    pub(crate) fn number(&self, i: usize) -> Option<f64> {
        self.text(i).and_then(|word| word.parse::<f64>().ok()).filter(|n| n.is_finite())
    }

    /// The lists among the arguments that start with `name`.
    pub(crate) fn all<'n>(&'n self, name: &'n str) -> impl Iterator<Item = &'n Node<'a>> + 'n {
        self.args().iter().filter(move |part| part.head() == Some(name))
    }

    /// The first list among the arguments that starts with `name`.
    pub(crate) fn child(&self, name: &str) -> Option<&Node<'a>> {
        self.args().iter().find(|part| part.head() == Some(name))
    }

    /// Whether one of the arguments is the bare word `word`.
    pub(crate) fn flag(&self, word: &str) -> bool {
        self.args().iter().any(|part| part.word() == Some(word))
    }
}

/// Reads the items of one outer list, one at a time.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
    text: &'a str,
    at: usize,
    /// How many words and lists the item being read has so far.
    nodes: usize,
}

impl<'a> Reader<'a> {
    /// Starts reading `text`, which must be one list, and returns the word it starts with. A
    /// leading byte-order mark is skipped. Fails when the text doesn't start with `(` and a word.
    pub(crate) fn open(text: &'a str) -> Result<(Reader<'a>, Cow<'a, str>), Error> {
        let mut reader = Reader { text: text.strip_prefix('\u{feff}').unwrap_or(text), at: 0, nodes: 0 };
        reader.skip_space();
        if reader.peek() != Some(b'(') {
            return Err(reader.error("expected `(`"));
        }
        reader.at += 1;
        reader.skip_space();
        match reader.atom()? {
            Some(head) => Ok((reader, head)),
            None => Err(reader.error("expected a word after `(`")),
        }
    }

    /// The next item of the outer list, or `None` at its end. Anything after the end, an
    /// unclosed list or string, nesting beyond [`MAX_DEPTH`] or an item of more than
    /// [`MAX_NODES`] parts is an error.
    pub(crate) fn next_item(&mut self) -> Result<Option<Node<'a>>, Error> {
        self.nodes = 0;
        self.skip_space();
        if self.peek() == Some(b')') {
            self.at += 1;
            self.skip_space();
            return if self.peek().is_none() { Ok(None) } else { Err(self.error("text after the end of the file's list")) };
        }
        self.node(0).map(Some)
    }

    /// The byte at the current place; `None` at the end.
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    /// Move past ASCII whitespace.
    fn skip_space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    /// An error at the current place, with its line.
    fn error(&self, message: &str) -> Error {
        let before = self.text.as_bytes().get(..self.at.min(self.text.len())).unwrap_or_default();
        let line = before.iter().filter(|&&b| b == b'\n').count().saturating_add(1);
        Error::Syntax { line, message: message.to_owned() }
    }

    /// A word or a list, `depth` lists deep. Counts toward the item's node limit.
    fn node(&mut self, depth: usize) -> Result<Node<'a>, Error> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(self.error("one item of the file is too large to read"));
        }
        if self.peek() != Some(b'(') {
            return match self.atom()? {
                Some(word) => Ok(Node::Atom(word)),
                None => Err(self.error("unexpected end of the file")),
            };
        }
        if depth >= MAX_DEPTH {
            return Err(self.error("lists nested too deeply"));
        }
        self.at += 1;
        let mut parts = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                None => return Err(self.error("a list is not closed")),
                Some(b')') => {
                    self.at += 1;
                    return Ok(Node::List(parts));
                }
                Some(_) => parts.push(self.node(depth + 1)?),
            }
        }
    }

    /// A bare or quoted word at the current place; `None` at a parenthesis or the end.
    fn atom(&mut self) -> Result<Option<Cow<'a, str>>, Error> {
        match self.peek() {
            None | Some(b'(' | b')') => Ok(None),
            Some(b'"') => self.quoted().map(Some),
            Some(_) => {
                let start = self.at;
                while self.peek().is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'(' | b')' | b'"')) {
                    self.at += 1;
                }
                // Both ends are at ASCII bytes, so the slice is on character boundaries.
                Ok(self.text.get(start..self.at).map(Cow::Borrowed))
            }
        }
    }

    /// A quoted string, with `\"`, `\\`, `\n`, `\r` and `\t` read as what they stand for and any
    /// other escaped character kept as itself. Fails when the string is not closed.
    fn quoted(&mut self) -> Result<Cow<'a, str>, Error> {
        self.at += 1;
        let mut owned: Option<String> = None;
        let mut copied = self.at;
        loop {
            match self.peek() {
                None => return Err(self.error("a quoted string is not closed")),
                Some(b'"') => {
                    let rest = self.text.get(copied..self.at).unwrap_or_default();
                    self.at += 1;
                    return Ok(match owned {
                        Some(mut s) => {
                            s.push_str(rest);
                            Cow::Owned(s)
                        }
                        None => Cow::Borrowed(rest),
                    });
                }
                Some(b'\\') => {
                    let s = owned.get_or_insert_with(String::new);
                    s.push_str(self.text.get(copied..self.at).unwrap_or_default());
                    let Some(escaped) = self.text.get(self.at + 1..).and_then(|rest| rest.chars().next()) else {
                        return Err(self.error("a quoted string is not closed"));
                    };
                    s.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                    self.at += 1 + escaped.len_utf8();
                    copied = self.at;
                }
                Some(_) => self.at += 1,
            }
        }
    }
}
