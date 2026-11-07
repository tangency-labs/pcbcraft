//! A small Markdown reader for assistant replies: paragraphs, headings, lists, quotes, fenced
//! code, rules, and bold, italic, code and link spans. Tables are kept as preformatted text.
//!
//! It takes untrusted text and never fails: anything it does not understand stays text, and the
//! amount of work and output is bounded.

/// Longest input parsed; the rest is ignored.
pub const MAX_INPUT: usize = 256 * 1024;
/// Most blocks produced.
pub const MAX_BLOCKS: usize = 4000;
/// Longest link target kept.
const MAX_URL: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    /// A link with an `http` or `https` target; any other target is shown as its label only.
    Link {
        label: String,
        url: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Code {
        lang: String,
        text: String,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<Span>>,
    },
    Quote(Vec<Span>),
    /// Preformatted text (tables).
    Pre(String),
    Rule,
}

/// Parse `src` into blocks.
pub fn parse(src: &str) -> Vec<Block> {
    let src = cut(src, MAX_INPUT);
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<Block> = Vec::new();
    let mut i = 0;
    let mut para: Vec<&str> = Vec::new();
    macro_rules! flush {
        () => {
            if !para.is_empty() {
                out.push(Block::Paragraph(spans(&para.join(" "))));
                para.clear();
            }
        };
    }
    while i < lines.len() && out.len() < MAX_BLOCKS {
        let line = lines[i];
        let t = line.trim_start();
        if let Some(fence) = fence_of(t) {
            flush!();
            let lang = t[fence.len()..].trim().to_string();
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(&fence) {
                body.push(lines[i]);
                i += 1;
            }
            i += 1;
            out.push(Block::Code { lang, text: body.join("\n") });
            continue;
        }
        if t.is_empty() {
            flush!();
            i += 1;
            continue;
        }
        if is_rule(t) {
            flush!();
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if let Some((level, rest)) = heading(t) {
            flush!();
            out.push(Block::Heading { level, spans: spans(rest) });
            i += 1;
            continue;
        }
        if t.starts_with('|') {
            flush!();
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(lines[i].trim_end());
                i += 1;
            }
            out.push(Block::Pre(rows.join("\n")));
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            flush!();
            let mut q = vec![rest.trim()];
            i += 1;
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                q.push(lines[i].trim_start()[1..].trim());
                i += 1;
            }
            out.push(Block::Quote(spans(&q.join(" "))));
            continue;
        }
        if let Some((ordered, n, rest)) = list_item(t) {
            flush!();
            let mut items = vec![spans(rest)];
            i += 1;
            while i < lines.len() {
                let l = lines[i].trim_start();
                match list_item(l) {
                    Some((o, _, r)) if o == ordered => {
                        items.push(spans(r));
                        i += 1;
                    }
                    // A wrapped line of the previous item.
                    None if !l.is_empty() && lines[i].starts_with(' ') && fence_of(l).is_none() && !l.starts_with('|') => {
                        if let Some(last) = items.last_mut() {
                            last.push(Span::Text(" ".into()));
                            last.extend(spans(l));
                        }
                        i += 1;
                    }
                    _ => break,
                }
                if items.len() >= MAX_BLOCKS {
                    break;
                }
            }
            out.push(Block::List { ordered, start: n, items });
            continue;
        }
        para.push(t);
        i += 1;
    }
    flush!();
    out
}

/// `s` cut to at most `max` bytes on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut e = max;
    while e > 0 && !s.is_char_boundary(e) {
        e -= 1;
    }
    &s[..e]
}

/// The code fence a line opens (three or more backticks or tildes), if any.
fn fence_of(t: &str) -> Option<String> {
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then(|| c.to_string().repeat(n))
}

/// Whether a line is a thematic break: three or more of one of `-`, `*` or `_`.
fn is_rule(t: &str) -> bool {
    let s: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    s.len() >= 3 && (s.chars().all(|c| c == '-') || s.chars().all(|c| c == '*') || s.chars().all(|c| c == '_'))
}

/// A heading line's level (1 to 6) and text.
fn heading(t: &str) -> Option<(u8, &str)> {
    let n = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) && t[n..].starts_with(' ') { Some((n as u8, t[n..].trim())) } else { None }
}

/// A list item's kind (ordered or not), its number and its text.
fn list_item(t: &str) -> Option<(bool, u64, &str)> {
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            return Some((false, 1, r.trim()));
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &t[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((true, t[..digits].parse().unwrap_or(1), r.trim()));
        }
    }
    None
}

/// For each position `j` of `c` (and its end), the first position at or after `j` where `at`
/// holds, or `usize::MAX`: one pass, so every opener finds its closer in constant time.
fn next_where(c: &[char], at: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut next = vec![usize::MAX; c.len() + 1];
    for j in (0..c.len()).rev() {
        next[j] = if at(j) { j } else { next[j + 1] };
    }
    next
}

/// Where each closer `spans` looks for is, computed once per paragraph so that unclosed
/// openers (`*a *a …`, `[[[…`) cost linear rather than quadratic time.
struct Closers {
    /// `**` and `__` pairs.
    bold: [Vec<usize>; 2],
    /// A `*` or `_` that can end an italic span.
    italic: [Vec<usize>; 2],
    bracket: Vec<usize>,
    paren: Vec<usize>,
    /// The start of every maximal run of backticks, by run length.
    ticks: std::collections::HashMap<usize, Vec<usize>>,
    /// How many backticks run from each position, so an unclosed run isn't recounted at each
    /// of its characters.
    tick_len: Vec<usize>,
}

impl Closers {
    /// Every closer table of `c`, in linear passes.
    fn new(c: &[char]) -> Self {
        let pair = |d: char| next_where(c, |j| c[j] == d && c.get(j + 1) == Some(&d));
        let italic = |d: char| {
            next_where(c, |j| c[j] == d && j > 0 && !c[j - 1].is_whitespace() && (d == '*' || c.get(j + 1).is_none_or(|n| !n.is_alphanumeric())))
        };
        let mut tick_len = vec![0; c.len() + 1];
        for j in (0..c.len()).rev() {
            tick_len[j] = if c[j] == '`' { tick_len[j + 1] + 1 } else { 0 };
        }
        let mut ticks: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
        let mut j = 0;
        while j < c.len() {
            let run = tick_len[j];
            if run > 0 {
                ticks.entry(run).or_default().push(j);
            }
            j += run.max(1);
        }
        Self {
            bold: [pair('*'), pair('_')],
            italic: [italic('*'), italic('_')],
            bracket: next_where(c, |j| c[j] == ']'),
            paren: next_where(c, |j| c[j] == ')'),
            ticks,
            tick_len,
        }
    }

    fn index(d: char) -> usize {
        usize::from(d == '_')
    }

    /// The first position at or after `from` in a [`next_where`] table.
    fn at(table: &[usize], from: usize) -> Option<usize> {
        table.get(from).copied().filter(|j| *j != usize::MAX)
    }

    /// The start of the next run of exactly `n` backticks at or after `from`.
    fn run(&self, from: usize, n: usize) -> Option<usize> {
        let starts = self.ticks.get(&n)?;
        starts.get(starts.partition_point(|start| *start < from)).copied()
    }
}

/// Inline spans of `s`.
pub fn spans(s: &str) -> Vec<Span> {
    let c: Vec<char> = s.chars().collect();
    let closers = Closers::new(&c);
    let mut out: Vec<Span> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    /// End the plain text run so far as a text span.
    fn flush(out: &mut Vec<Span>, text: &mut String) {
        if !text.is_empty() {
            out.push(Span::Text(std::mem::take(text)));
        }
    }
    while i < c.len() {
        let rest = &c[i..];
        // Inline code.
        if c[i] == '`' {
            let n = closers.tick_len[i];
            if let Some(end) = closers.run(i + n, n) {
                flush(&mut out, &mut text);
                out.push(Span::Code(c[i + n..end].iter().collect::<String>().trim().to_string()));
                i = end + n;
                continue;
            }
        }
        // Bold.
        if (rest.starts_with(&['*', '*']) || rest.starts_with(&['_', '_'])) && rest.len() > 4 {
            let d = c[i];
            if let Some(end) = Closers::at(&closers.bold[Closers::index(d)], i + 2)
                && end > i + 2
            {
                flush(&mut out, &mut text);
                out.push(Span::Bold(c[i + 2..end].iter().collect()));
                i = end + 2;
                continue;
            }
        }
        // Italic (not inside words for `_`, so snake_case stays text).
        if (c[i] == '*' || (c[i] == '_' && (i == 0 || !c[i - 1].is_alphanumeric()))) && rest.len() > 2 && !rest[1].is_whitespace() {
            let d = c[i];
            if let Some(end) = Closers::at(&closers.italic[Closers::index(d)], i + 1)
                && end > i + 1
            {
                flush(&mut out, &mut text);
                out.push(Span::Italic(c[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        }
        // Link.
        if c[i] == '['
            && let Some(close) = Closers::at(&closers.bracket, i + 1)
            && c.get(close + 1) == Some(&'(')
            && let Some(end) = Closers::at(&closers.paren, close + 2)
        {
            let label: String = c[i + 1..close].iter().collect();
            let url: String = c[close + 2..end].iter().collect::<String>().trim().to_string();
            flush(&mut out, &mut text);
            let lower = url.to_ascii_lowercase();
            let web = lower.starts_with("https://") || lower.starts_with("http://");
            if web && url.len() <= MAX_URL && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
                out.push(Span::Link { label, url });
            } else {
                out.push(Span::Text(label));
            }
            i = end + 1;
            continue;
        }
        // A backslash escapes the next marker.
        if c[i] == '\\' && c.get(i + 1).is_some_and(|n| "\\`*_[]()#-".contains(*n)) {
            text.push(c[i + 1]);
            i += 2;
            continue;
        }
        text.push(c[i]);
        i += 1;
    }
    flush(&mut out, &mut text);
    out
}

/// The plain text of spans, for copying and for exports.
pub fn plain(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|s| match s {
            Span::Text(t) | Span::Bold(t) | Span::Italic(t) | Span::Code(t) => t.clone(),
            Span::Link { label, url } => format!("{label} ({url})"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Span {
        Span::Text(s.into())
    }

    /// Blocks.
    #[test]
    fn blocks() {
        let md = "# Title\n\nSome *emphasis* and **bold** with `code`.\nSecond line.\n\n- one\n- two\n  wrapped\n\n1. a\n2. b\n\n> quoted\n\n---\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let b = parse(md);
        assert_eq!(b[0], Block::Heading { level: 1, spans: vec![t("Title")] });
        assert_eq!(
            b[1],
            Block::Paragraph(vec![
                t("Some "),
                Span::Italic("emphasis".into()),
                t(" and "),
                Span::Bold("bold".into()),
                t(" with "),
                Span::Code("code".into()),
                t(". Second line.")
            ])
        );
        assert_eq!(b[2], Block::List { ordered: false, start: 1, items: vec![vec![t("one")], vec![t("two"), t(" "), t("wrapped")]] });
        assert_eq!(b[3], Block::List { ordered: true, start: 1, items: vec![vec![t("a")], vec![t("b")]] });
        assert_eq!(b[4], Block::Quote(vec![t("quoted")]));
        assert_eq!(b[5], Block::Rule);
        assert_eq!(b[6], Block::Code { lang: "rust".into(), text: "let x = 1;".into() });
        assert!(matches!(&b[7], Block::Pre(s) if s.lines().count() == 3));
        assert_eq!(b.len(), 8);
    }

    #[test]
    fn snake_case_and_stray_markers_stay_text() {
        assert_eq!(spans("use set_parameter and body_topology"), vec![t("use set_parameter and body_topology")]);
        assert_eq!(spans("2 * 3 * 4"), vec![t("2 * 3 * 4")]);
        assert_eq!(spans("unclosed **bold"), vec![t("unclosed **bold")]);
        assert_eq!(spans("a `b"), vec![t("a `b")]);
        assert_eq!(spans(r"\*not italic\*"), vec![t("*not italic*")]);
    }

    /// Only web links are links.
    #[test]
    fn only_web_links_are_links() {
        assert_eq!(spans("[docs](https://example.com/a)"), vec![Span::Link { label: "docs".into(), url: "https://example.com/a".into() }]);
        for bad in ["[x](javascript:alert(1))", "[x](file:///etc/passwd)", "[x](ftp://a)", "[x](/relative)", "[x](https://a b)"] {
            let s = spans(bad);
            assert_eq!(s[0], t("x"), "{bad}");
            assert!(!s.iter().any(|x| matches!(x, Span::Link { .. })), "{bad}");
        }
    }

    #[test]
    fn code_blocks_keep_everything() {
        let b = parse("```\n# not a heading\n- not a list\n```");
        assert_eq!(b, vec![Block::Code { lang: String::new(), text: "# not a heading\n- not a list".into() }]);
        // An unterminated fence runs to the end.
        let b = parse("```\nstill code");
        assert_eq!(b, vec![Block::Code { lang: String::new(), text: "still code".into() }]);
    }

    /// Hostile input is bounded and never panics.
    #[test]
    fn hostile_input_is_bounded_and_never_panics() {
        let mut seed = 0x1234_5678_u32;
        let alphabet: Vec<char> = "*_`[]()#->|\\~ \n\t1.ä🌍a".chars().collect();
        for _ in 0..300 {
            let mut s = String::new();
            for _ in 0..200 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                s.push(alphabet[(seed >> 16) as usize % alphabet.len()]);
            }
            let _ = parse(&s);
        }
        let huge = "- x\n".repeat(MAX_INPUT);
        let blocks = parse(&huge);
        assert!(blocks.len() <= MAX_BLOCKS);
        let nested = "*".repeat(100_000);
        let _ = parse(&nested);
        let _ = parse(&"[".repeat(100_000));
        let _ = parse(&"`a".repeat(50_000));
    }

    /// Unclosed openers in one long paragraph cost linear time: each `*`, `_`, `[` or backtick
    /// run that never closes once scanned the rest of the paragraph.
    #[test]
    fn unclosed_openers_take_linear_time() {
        let start = std::time::Instant::now();
        for hostile in ["*a ".repeat(80_000), "_a ".repeat(80_000), "[".repeat(200_000), "[a](".repeat(50_000)] {
            let parsed = spans(&hostile);
            assert_eq!(plain(&parsed).chars().count(), hostile.chars().count(), "nothing closes, so it all stays text");
        }
        let decreasing: String = (1..=600).rev().map(|n| "`".repeat(n) + "a").collect();
        let _ = spans(&decreasing);
        let unclosed_run = format!("a{}", "`".repeat(200_000));
        assert_eq!(plain(&spans(&unclosed_run)), unclosed_run, "a run with no partner stays text");
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
    }

    /// The linear matcher finds exactly the spans the previous one did.
    #[test]
    fn linear_spans_match_the_reference() {
        let mut seed = 0x9e37_79b9_u32;
        let alphabet: Vec<char> = "*_`[]() \\ab1ä".chars().collect();
        for _ in 0..3000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let len = (seed >> 24) as usize % 60;
            let mut s = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                s.push(alphabet[(seed >> 16) as usize % alphabet.len()]);
            }
            assert_eq!(spans(&s), reference_spans(&s), "{s:?}");
        }
        for s in ["**a** *b* _c_ snake_case `d` ``e`` [f](https://g.h) [i](j) \\*k*", "``a` b``c`", "*a **b** c*", "__a_b__"] {
            assert_eq!(spans(s), reference_spans(s), "{s:?}");
        }
    }

    /// The previous, quadratic matcher: the reference the linear one must agree with.
    fn reference_spans(s: &str) -> Vec<Span> {
        let c: Vec<char> = s.chars().collect();
        let mut out: Vec<Span> = Vec::new();
        let mut text = String::new();
        let mut i = 0;
        /// End the plain text run so far as a text span.
        fn flush(out: &mut Vec<Span>, text: &mut String) {
            if !text.is_empty() {
                out.push(Span::Text(std::mem::take(text)));
            }
        }
        while i < c.len() {
            let rest = &c[i..];
            // Inline code.
            if c[i] == '`' {
                let n = rest.iter().take_while(|x| **x == '`').count();
                if let Some(end) = find_run(&c, i + n, '`', n) {
                    flush(&mut out, &mut text);
                    out.push(Span::Code(c[i + n..end].iter().collect::<String>().trim().to_string()));
                    i = end + n;
                    continue;
                }
            }
            // Bold.
            if (rest.starts_with(&['*', '*']) || rest.starts_with(&['_', '_'])) && rest.len() > 4 {
                let d = c[i];
                if let Some(end) = find_pair(&c, i + 2, d)
                    && end > i + 2
                {
                    flush(&mut out, &mut text);
                    out.push(Span::Bold(c[i + 2..end].iter().collect()));
                    i = end + 2;
                    continue;
                }
            }
            // Italic (not inside words for `_`, so snake_case stays text).
            if (c[i] == '*' || (c[i] == '_' && (i == 0 || !c[i - 1].is_alphanumeric()))) && rest.len() > 2 && !rest[1].is_whitespace() {
                let d = c[i];
                let closes = |j: &usize| c[*j] == d && !c[*j - 1].is_whitespace() && (d == '*' || c.get(*j + 1).is_none_or(|n| !n.is_alphanumeric()));
                if let Some(end) = (i + 1..c.len()).find(closes)
                    && end > i + 1
                {
                    flush(&mut out, &mut text);
                    out.push(Span::Italic(c[i + 1..end].iter().collect()));
                    i = end + 1;
                    continue;
                }
            }
            // Link.
            if c[i] == '['
                && let Some(close) = c[i + 1..].iter().position(|x| *x == ']').map(|p| p + i + 1)
                && c.get(close + 1) == Some(&'(')
                && let Some(end) = c[close + 2..].iter().position(|x| *x == ')').map(|p| p + close + 2)
            {
                let label: String = c[i + 1..close].iter().collect();
                let url: String = c[close + 2..end].iter().collect::<String>().trim().to_string();
                flush(&mut out, &mut text);
                let lower = url.to_ascii_lowercase();
                let web = lower.starts_with("https://") || lower.starts_with("http://");
                if web && url.len() <= MAX_URL && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
                    out.push(Span::Link { label, url });
                } else {
                    out.push(Span::Text(label));
                }
                i = end + 1;
                continue;
            }
            // A backslash escapes the next marker.
            if c[i] == '\\' && c.get(i + 1).is_some_and(|n| "\\`*_[]()#-".contains(*n)) {
                text.push(c[i + 1]);
                i += 2;
                continue;
            }
            text.push(c[i]);
            i += 1;
        }
        flush(&mut out, &mut text);
        out
    }

    /// The index of the next run of exactly `n` `ch` at or after `from`.
    fn find_run(c: &[char], from: usize, ch: char, n: usize) -> Option<usize> {
        let mut j = from;
        while j < c.len() {
            if c[j] == ch {
                let run = c[j..].iter().take_while(|x| **x == ch).count();
                if run == n {
                    return Some(j);
                }
                j += run;
            } else {
                j += 1;
            }
        }
        None
    }

    /// The next pair of `d` at or after `from`.
    fn find_pair(c: &[char], from: usize, d: char) -> Option<usize> {
        (from..c.len().saturating_sub(1)).find(|&j| c[j] == d && c[j + 1] == d)
    }

    #[test]
    fn plain_text_of_spans() {
        assert_eq!(plain(&spans("**a** [b](https://c.d) `e`")), "a b (https://c.d) e");
    }
}
