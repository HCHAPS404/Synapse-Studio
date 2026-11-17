//! The canonical formatter (`neuron fmt`, spec chapter 13).
//!
//! The formatter works on the token stream, so it never changes the meaning of a program
//! (property P-FMT-2): it only changes the white space between tokens, the indentation of
//! lines and the blank lines between them. After formatting, the result is lexed and
//! parsed again and compared with the input; when the check fails the input is returned
//! unchanged and [`Formatted::fell_back`] is set.
//!
//! Not implemented yet: breaking lines longer than 88 columns (chapter 13.2). The
//! formatter never joins or breaks lines.
//!
//! ```
//! use neuron_syntax::format::format_source;
//!
//! let messy = "module m for mcu\nio led:   gpio.output   @ D13\ntask t every  500 ms:\n    led.toggle( )\n";
//! let out = format_source(messy).expect("valid program");
//! assert_eq!(out.text, "module m for mcu\nio led: gpio.output @ D13\ntask t every 500 ms:\n  led.toggle()\n");
//! ```

use crate::diag::Diagnostic;
use crate::lexer::lex;
use crate::parser::parse;
use crate::token::{Keyword, Punct, Token, TokenKind};

/// Result of formatting a program that parses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    /// The canonical text.
    pub text: String,
    /// `true` when the layout engine produced something that failed the self-check and
    /// the input was returned unchanged. A bug when it happens.
    pub fell_back: bool,
}

/// Formats `src`.
///
/// # Errors
/// Returns the syntax errors when the source does not parse; such a source is never
/// changed (property P-FMT-5).
pub fn format_source(src: &str) -> Result<Formatted, Vec<Diagnostic>> {
    let norm = normalize(src);
    let parsed = parse(&norm);
    let errors: Vec<Diagnostic> = parsed
        .diagnostics
        .into_iter()
        .filter(Diagnostic::is_error)
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    let text = layout(&norm);
    let stable = signature(&norm) == signature(&text)
        && !parse(&text).diagnostics.iter().any(Diagnostic::is_error);
    if stable {
        Ok(Formatted {
            text,
            fell_back: false,
        })
    } else {
        Ok(Formatted {
            text: norm,
            fell_back: true,
        })
    }
}

/// Removes the byte order mark and uses `\n` as the line terminator.
fn normalize(src: &str) -> String {
    src.strip_prefix('\u{feff}')
        .unwrap_or(src)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// Kinds and texts of the significant tokens: what the parser sees.
pub fn signature(src: &str) -> Vec<(TokenKind, String)> {
    let lexed = lex(src);
    lexed
        .tokens
        .iter()
        .filter(|t| !t.kind.is_trivia() && !t.kind.is_layout())
        .map(|t| (t.kind, t.text(src).to_string()))
        .collect()
}

/// Number of comments in `src`.
pub fn comment_count(src: &str) -> usize {
    lex(src)
        .tokens
        .iter()
        .filter(|t| {
            matches!(
                t.kind,
                TokenKind::LineComment | TokenKind::DocComment | TokenKind::BlockComment
            )
        })
        .count()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    Blank,
    Comment,
    Start,
    Cont,
    Skip,
}

struct Line {
    kind: LineKind,
    depth: usize,
    /// Brackets open before the first token (continuation lines).
    brackets: usize,
    text: String,
    col: usize,
    at_byte: Option<usize>,
    ends_colon: bool,
    align: bool,
}

fn is_comment(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::LineComment | TokenKind::DocComment | TokenKind::BlockComment
    )
}

fn comment_text(raw: &str) -> String {
    let t = raw.trim_end();
    if t.starts_with("#[") {
        return t.to_string();
    }
    if let Some(rest) = t.strip_prefix("##") {
        return if rest.is_empty() || rest.starts_with(' ') {
            t.to_string()
        } else {
            format!("## {rest}")
        };
    }
    let rest = t.strip_prefix('#').unwrap_or(t);
    if rest.is_empty() || rest.starts_with(' ') {
        t.to_string()
    } else {
        format!("# {rest}")
    }
}

/// Operators that always have one space on each side.
fn spaced(p: Punct) -> bool {
    use Punct as P;
    matches!(
        p,
        P::Eq
            | P::EqEq
            | P::NotEq
            | P::Le
            | P::Ge
            | P::PlusEq
            | P::MinusEq
            | P::StarEq
            | P::SlashEq
            | P::PercentEq
            | P::AmpEq
            | P::BarEq
            | P::CaretEq
            | P::ShlEq
            | P::ShrEq
            | P::Arrow
            | P::FatArrow
            | P::Pipeline
            | P::Coalesce
    )
}

/// The white space between two tokens of one line.
fn separator(
    a: &Token,
    b: &Token,
    a_first: bool,
    inner_a: Option<Punct>,
    gap_empty: bool,
) -> &'static str {
    use Punct as P;
    use TokenKind as K;
    if is_comment(b.kind) {
        return "  ";
    }
    let orig = if gap_empty { "" } else { " " };
    match (a.kind, b.kind) {
        (_, K::Punct(P::Comma | P::Semi)) => "",
        (K::Punct(P::Comma | P::Semi), K::Punct(P::RParen | P::RBracket | P::RBrace)) => "",
        (K::Punct(P::Comma | P::Semi), _) => " ",
        (K::Punct(P::LParen | P::LBracket | P::LBrace), _) => "",
        (_, K::Punct(P::RParen | P::RBracket | P::RBrace)) => "",
        (_, K::Punct(P::Dot)) | (K::Punct(P::Dot), _) => "",
        (K::Punct(P::At), _) if a_first => "",
        (_, K::Punct(P::At)) | (K::Punct(P::At), _) => " ",
        (K::Punct(P::Colon), _) => {
            if inner_a == Some(P::LBracket) {
                orig
            } else {
                " "
            }
        }
        (_, K::Punct(P::Colon)) => {
            if inner_a == Some(P::LBracket) {
                orig
            } else {
                ""
            }
        }
        (K::Punct(p), _) | (_, K::Punct(p)) if spaced(p) => " ",
        _ => orig,
    }
}

/// Per-token facts computed in one pass.
struct Facts {
    depth: Vec<usize>,
    brackets: Vec<usize>,
    inner: Vec<Option<Punct>>,
    logical_start: Vec<bool>,
}

fn facts(toks: &[Token]) -> Facts {
    let mut f = Facts {
        depth: vec![0; toks.len()],
        brackets: vec![0; toks.len()],
        inner: vec![None; toks.len()],
        logical_start: vec![false; toks.len()],
    };
    let mut depth = 0usize;
    let mut stack: Vec<Punct> = Vec::new();
    let mut after_layout = true;
    for (i, t) in toks.iter().enumerate() {
        match t.kind {
            TokenKind::Indent => depth += 1,
            TokenKind::Dedent => depth = depth.saturating_sub(1),
            _ => {}
        }
        f.depth[i] = depth;
        f.brackets[i] = stack.len();
        f.inner[i] = stack.last().copied();
        f.logical_start[i] = after_layout;
        match t.kind {
            TokenKind::Indent | TokenKind::Dedent | TokenKind::Newline => after_layout = true,
            k if k.is_trivia() || k == TokenKind::Whitespace => {}
            TokenKind::Punct(p) => {
                after_layout = false;
                if p.is_open() {
                    stack.push(p);
                } else if p.is_close() {
                    stack.pop();
                }
            }
            _ => after_layout = false,
        }
    }
    f
}

#[allow(clippy::needless_range_loop, clippy::too_many_lines)]
fn layout(src: &str) -> String {
    let lexed = lex(src);
    let toks = &lexed.tokens;
    let f = facts(toks);

    let mut starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    let nlines = starts.len();
    let line_of = |pos: usize| match starts.binary_search(&pos) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let mut per_line: Vec<Vec<usize>> = vec![Vec::new(); nlines];
    let mut inside = vec![false; nlines];
    for (i, t) in toks.iter().enumerate() {
        if matches!(
            t.kind,
            TokenKind::Whitespace
                | TokenKind::Newline
                | TokenKind::Indent
                | TokenKind::Dedent
                | TokenKind::Eof
        ) {
            continue;
        }
        let l = line_of(t.span.start);
        per_line[l].push(i);
        let end = line_of(t.span.end.saturating_sub(1).max(t.span.start));
        for flag in inside.iter_mut().take(end + 1).skip(l + 1) {
            *flag = true;
        }
    }

    // Parent section of each depth, to find `inputs:` and `outputs:` groups.
    let mut parents: Vec<String> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    for l in 0..nlines {
        let raw = &src[starts[l]..if l + 1 < nlines {
            starts[l + 1] - 1
        } else {
            src.len()
        }];
        let col = raw.len() - raw.trim_start_matches(' ').len();
        if inside[l] {
            lines.push(Line {
                kind: LineKind::Skip,
                depth: 0,
                brackets: 0,
                text: String::new(),
                col,
                at_byte: None,
                ends_colon: false,
                align: false,
            });
            continue;
        }
        let idxs = &per_line[l];
        if idxs.is_empty() {
            lines.push(Line {
                kind: LineKind::Blank,
                depth: 0,
                brackets: 0,
                text: String::new(),
                col,
                at_byte: None,
                ends_colon: false,
                align: false,
            });
            continue;
        }
        let first = idxs[0];
        let comment_only = idxs.iter().all(|&i| is_comment(toks[i].kind));
        let kind = if f.brackets[first] > 0 {
            LineKind::Cont
        } else if comment_only {
            LineKind::Comment
        } else if f.logical_start[first] {
            LineKind::Start
        } else {
            LineKind::Cont
        };
        let mut text = String::new();
        let mut at_byte = None;
        let mut prev: Option<usize> = None;
        for &ti in idxs {
            let t = &toks[ti];
            if let Some(p) = prev {
                let a = &toks[p];
                let gap_empty = a.span.end == t.span.start;
                let a_first = p == first;
                text.push_str(separator(
                    a,
                    t,
                    a_first && kind == LineKind::Start,
                    f.inner[p],
                    gap_empty,
                ));
            }
            if t.kind == TokenKind::Punct(Punct::At) && prev.is_some() {
                at_byte = Some(text.len());
            }
            if is_comment(t.kind) {
                text.push_str(&comment_text(t.text(src)));
            } else {
                text.push_str(t.text(src));
            }
            prev = Some(ti);
        }
        let last_sig = idxs.iter().rev().find(|&&i| !is_comment(toks[i].kind));
        let ends_colon = last_sig.is_some_and(|&i| toks[i].kind == TokenKind::Punct(Punct::Colon));
        let tok0 = toks[first].text(src).to_string();
        let mut align = false;
        if kind == LineKind::Start {
            let d = f.depth[first];
            parents.truncate(d);
            let parent_is_section = d > 0
                && parents
                    .get(d - 1)
                    .is_some_and(|p| matches!(p.as_str(), "inputs" | "outputs" | "memory"));
            align = toks[first].kind == TokenKind::Keyword(Keyword::Io) || parent_is_section;
            parents.push(tok0);
        }
        lines.push(Line {
            kind,
            depth: f.depth[first],
            brackets: f.brackets[first].saturating_sub(usize::from(
                matches!(toks[first].kind, TokenKind::Punct(p) if p.is_close()),
            )),
            text,
            col,
            at_byte,
            ends_colon,
            align,
        });
    }

    // Depth of comment lines: the next code line, or the level the comment was written at
    // when that level is still open and at least as deep.
    let mut next_depth = vec![0usize; lines.len()];
    let mut cur = 0usize;
    for (i, ln) in lines.iter().enumerate().rev() {
        if ln.kind == LineKind::Start {
            cur = ln.depth;
        }
        next_depth[i] = cur;
    }
    let mut col_stack: Vec<usize> = Vec::new();
    let mut last_start_depth = 0usize;
    for i in 0..lines.len() {
        match lines[i].kind {
            LineKind::Start => {
                let d = lines[i].depth;
                col_stack.truncate(d);
                col_stack.push(lines[i].col);
                last_start_depth = d;
            }
            LineKind::Comment => {
                let want = col_stack
                    .iter()
                    .position(|&c| c == lines[i].col)
                    .filter(|&d| d >= next_depth[i]);
                lines[i].depth = want.unwrap_or(next_depth[i]);
            }
            LineKind::Cont => lines[i].depth = last_start_depth,
            _ => {}
        }
    }

    // Align the `@` of consecutive `io` bindings and of the entries of `inputs:` and
    // `outputs:`.
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == LineKind::Start && lines[i].align && lines[i].at_byte.is_some() {
            let mut j = i;
            while j + 1 < lines.len()
                && lines[j + 1].kind == LineKind::Start
                && lines[j + 1].align
                && lines[j + 1].at_byte.is_some()
                && lines[j + 1].depth == lines[i].depth
            {
                j += 1;
            }
            let cols: Vec<usize> = (i..=j)
                .map(|k| {
                    let ln = &lines[k];
                    ln.at_byte.map_or(0, |b| ln.text[..b].chars().count())
                })
                .collect();
            let target = cols.iter().copied().max().unwrap_or(0);
            for (k, c) in (i..=j).zip(cols) {
                if let Some(b) = lines[k].at_byte {
                    lines[k].text.insert_str(b, &" ".repeat(target - c));
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }

    // Assemble: indentation, blank lines and the blank line after a block item.
    let item_depth = lines
        .iter()
        .find(|l| l.kind == LineKind::Start)
        .map_or(0, |l| {
            usize::from(l.text.starts_with("cell ") || l.text == "cell")
        });
    let mut out: Vec<String> = Vec::new();
    let mut after_header = false;
    let mut prev_block = false;
    for ln in &lines {
        match ln.kind {
            LineKind::Skip => {}
            LineKind::Blank => {
                if !after_header && out.last().is_some_and(|s| !s.is_empty()) {
                    out.push(String::new());
                }
            }
            LineKind::Start | LineKind::Comment | LineKind::Cont => {
                let indent = match ln.kind {
                    LineKind::Cont => 2 * (ln.depth + ln.brackets),
                    _ => 2 * ln.depth,
                };
                if ln.kind != LineKind::Cont && ln.depth == item_depth {
                    if prev_block && out.last().is_some_and(|s| !s.is_empty()) {
                        out.push(String::new());
                    }
                    prev_block = false;
                } else if ln.kind != LineKind::Cont && ln.depth > item_depth {
                    prev_block = true;
                }
                out.push(format!("{}{}", " ".repeat(indent), ln.text));
                after_header = ln.kind == LineKind::Start && ln.ends_colon;
            }
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    if out.is_empty() {
        return String::new();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}
