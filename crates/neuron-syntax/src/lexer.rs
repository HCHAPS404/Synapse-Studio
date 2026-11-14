//! Lexer for Neuron source text (spec/language/02-lexical.md).
//!
//! The lexer is lossless: the concatenated text of all produced units equals the
//! source (property LEX-1). Layout units (`Newline`, `Indent`, `Dedent`, `Eof`) are
//! zero width except `Newline`, which covers the line terminator when present.
//!
//! ```
//! use neuron_syntax::lexer::lex;
//! use neuron_syntax::token::TokenKind;
//!
//! let out = lex("task t every 10 ms:\n  led.toggle()\n");
//! assert!(out.diagnostics.is_empty());
//! assert!(out.tokens.iter().any(|t| t.kind == TokenKind::Indent));
//! ```

use crate::diag::Diagnostic;
use crate::span::Span;
use crate::token::{Keyword, Punct, Token, TokenKind};

/// Result of lexing a source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexed {
    /// Every unit, including trivia, covering the source without gaps.
    pub tokens: Vec<Token>,
    /// Problems found while lexing.
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    /// The units the parser consumes (trivia removed).
    pub fn significant(&self) -> impl Iterator<Item = &Token> + '_ {
        self.tokens.iter().filter(|t| !t.kind.is_trivia())
    }
}

/// Lexes `src` into a lossless stream.
pub fn lex(src: &str) -> Lexed {
    let mut lx = Lexer {
        src,
        pos: 0,
        tokens: Vec::new(),
        diags: Vec::new(),
        indent_stack: vec![0],
        bracket_depth: 0,
        open_brackets: Vec::new(),
        line_has_token: false,
        pending: None,
        last_atom: None,
    };
    lx.run();
    Lexed {
        tokens: lx.tokens,
        diagnostics: lx.diags,
    }
}

/// `true` if `c` may start an identifier (ASCII letters, `_`, and the unit alias
/// characters micro, ohm and degree).
pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic()
        || c == '_'
        || matches!(c, '\u{b5}' | '\u{3bc}' | '\u{3a9}' | '\u{2126}' | '\u{b0}')
}

/// `true` if `c` may continue an identifier.
pub fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

#[derive(Debug, Clone, Copy)]
struct PendingIndent {
    col: usize,
    line_start: usize,
    tab_at: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StrKind {
    Normal,
    Raw,
    Byte,
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
    indent_stack: Vec<usize>,
    bracket_depth: usize,
    /// Offsets of currently open brackets (for NE0011).
    open_brackets: Vec<usize>,
    line_has_token: bool,
    pending: Option<PendingIndent>,
    last_atom: Option<TokenKind>,
}

impl<'a> Lexer<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn push(&mut self, kind: TokenKind, start: usize) {
        self.tokens.push(Token {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        });
        if !kind.is_trivia() && !kind.is_layout() {
            self.last_atom = Some(kind);
        }
    }

    fn push_zero(&mut self, kind: TokenKind) {
        let at = self.pos;
        self.push(kind, at);
    }

    fn report(
        &mut self,
        code: &'static str,
        msg: impl Into<String>,
        start: usize,
        end: usize,
        help: Option<String>,
    ) {
        let mut d = Diagnostic::error(code, msg, Span { start, end });
        d.help = help;
        self.diags.push(d);
    }

    fn run(&mut self) {
        self.start_line();
        while let Some(c) = self.peek() {
            match c {
                '\n' | '\r' => self.newline(),
                ' ' | '\t' => self.whitespace(),
                '#' => self.comment(),
                _ => {
                    self.before_significant();
                    self.significant(c);
                }
            }
        }
        self.finish();
    }

    // ---- layout -------------------------------------------------------

    fn start_line(&mut self) {
        if self.bracket_depth > 0 {
            self.pending = None;
            return;
        }
        let line_start = self.pos;
        let mut col = 0;
        let mut tab_at = None;
        while let Some(c) = self.peek() {
            match c {
                ' ' => {}
                '\t' => {
                    tab_at.get_or_insert(self.pos);
                }
                _ => break,
            }
            col += 1;
            self.pos += 1;
        }
        if self.pos > line_start {
            self.push(TokenKind::Whitespace, line_start);
        }
        self.pending = Some(PendingIndent {
            col,
            line_start,
            tab_at,
        });
    }

    fn before_significant(&mut self) {
        if let Some(p) = self.pending.take() {
            if let Some(t) = p.tab_at {
                self.report(
                    "NE0001",
                    "tab character used for indentation",
                    t,
                    t + 1,
                    Some("indent with 2 spaces per level".into()),
                );
            }
            self.apply_indent(p.col, p.line_start);
        }
        self.line_has_token = true;
    }

    fn top(&self) -> usize {
        self.indent_stack.last().copied().unwrap_or(0)
    }

    fn apply_indent(&mut self, col: usize, line_start: usize) {
        if col > self.top() {
            self.indent_stack.push(col);
            self.push_zero(TokenKind::Indent);
        } else if col < self.top() {
            while self.indent_stack.len() > 1 && col < self.top() {
                self.indent_stack.pop();
                self.push_zero(TokenKind::Dedent);
            }
            if col != self.top() {
                let end = self.pos;
                self.report(
                    "NE0002",
                    "dedent does not match any enclosing indentation level",
                    line_start,
                    end,
                    Some("align this line with an enclosing block".into()),
                );
            }
        }
    }

    fn newline(&mut self) {
        let start = self.pos;
        self.pos += if self.rest().starts_with("\r\n") {
            2
        } else {
            1
        };
        if self.bracket_depth == 0 && self.line_has_token {
            self.push(TokenKind::Newline, start);
            self.line_has_token = false;
        } else {
            self.push(TokenKind::Whitespace, start);
        }
        self.start_line();
    }

    fn whitespace(&mut self) {
        let start = self.pos;
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
        self.push(TokenKind::Whitespace, start);
    }

    fn finish(&mut self) {
        if let Some(&open) = self.open_brackets.first() {
            self.report(
                "NE0011",
                "unclosed bracket",
                open,
                open + 1,
                Some("close it on the same logical line; lines inside brackets are joined".into()),
            );
        }
        if self.line_has_token {
            self.push_zero(TokenKind::Newline);
            self.line_has_token = false;
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.push_zero(TokenKind::Dedent);
        }
        self.push_zero(TokenKind::Eof);
    }

    // ---- comments -----------------------------------------------------

    fn skip_to_line_end(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' || c == '\r' {
                break;
            }
            self.bump();
        }
    }

    fn comment(&mut self) {
        let start = self.pos;
        if self.rest().starts_with("#[") {
            let mut depth = 0usize;
            loop {
                if self.rest().starts_with("#[") {
                    depth += 1;
                    self.pos += 2;
                } else if self.rest().starts_with("]#") {
                    depth -= 1;
                    self.pos += 2;
                    if depth == 0 {
                        break;
                    }
                } else if self.bump().is_none() {
                    self.report(
                        "NE0004",
                        "unterminated block comment",
                        start,
                        start + 2,
                        Some("close the comment with `]#`".into()),
                    );
                    break;
                }
            }
            self.push(TokenKind::BlockComment, start);
        } else if self.rest().starts_with("##") {
            self.skip_to_line_end();
            self.push(TokenKind::DocComment, start);
        } else {
            self.skip_to_line_end();
            self.push(TokenKind::LineComment, start);
        }
    }

    // ---- significant units --------------------------------------------

    fn significant(&mut self, c: char) {
        let start = self.pos;
        let next = self.peek_at(1);
        if c == 'r' && next == Some('"') {
            self.pos += 1;
            self.string(start, StrKind::Raw);
        } else if c == 'b' && next == Some('"') {
            self.pos += 1;
            self.string(start, StrKind::Byte);
        } else if is_ident_start(c) {
            self.ident(start);
        } else if c.is_ascii_digit() {
            self.number(start);
        } else {
            match c {
                '"' => self.string(start, StrKind::Normal),
                '\'' => self.char_lit(start),
                '`' => self.escaped_ident(start),
                '%' => match iec_address_len(self.rest()) {
                    Some(len) => {
                        self.pos += len;
                        self.push(TokenKind::IecAddr, start);
                    }
                    None => self.punct(start, c),
                },
                _ => self.punct(start, c),
            }
        }
    }

    fn ident(&mut self, start: usize) {
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        let text = &self.src[start..self.pos];
        let kind = match Keyword::lookup(text) {
            Some(k) => TokenKind::Keyword(k),
            None => TokenKind::Ident,
        };
        self.push(kind, start);
    }

    fn escaped_ident(&mut self, start: usize) {
        self.bump();
        let name_start = self.pos;
        while self.peek().is_some_and(is_ident_continue) {
            self.bump();
        }
        if self.pos > name_start && self.peek() == Some('`') {
            self.bump();
            self.push(TokenKind::Ident, start);
        } else {
            let end = self.pos;
            self.report(
                "NE0003",
                "unterminated escaped identifier",
                start,
                end,
                Some("write the name between backticks, like `step`".into()),
            );
            self.push(TokenKind::Error, start);
        }
    }

    fn digits(&mut self, valid: fn(char) -> bool) -> usize {
        let mut n = 0;
        while let Some(c) = self.peek() {
            if valid(c) {
                n += 1;
            } else if c != '_' {
                break;
            }
            self.pos += 1;
        }
        n
    }

    fn number(&mut self, start: usize) {
        let after_dot = self.last_atom == Some(TokenKind::Punct(Punct::Dot));
        let mut kind = TokenKind::Int;
        let mut bad = false;
        let mut reported = false;
        let rest = self.rest().as_bytes();
        let prefix = if rest.first() == Some(&b'0') {
            rest.get(1).map(u8::to_ascii_lowercase)
        } else {
            None
        };
        if matches!(prefix, Some(b'x' | b'b' | b'o')) {
            self.pos += 2;
            let n = match prefix {
                Some(b'x') => self.digits(|c| c.is_ascii_hexdigit()),
                Some(b'b') => self.digits(|c| matches!(c, '0' | '1')),
                _ => self.digits(|c| matches!(c, '0'..='7')),
            };
            bad |= n == 0;
        } else {
            self.digits(|c| c.is_ascii_digit());
            let base = self.peek_at(1);
            if self.peek() == Some('\'') && matches!(base, Some('b' | 'h' | 'd' | 'o')) {
                self.pos += 2;
                let n = match base {
                    Some('b') => self.digits(|c| matches!(c, '0' | '1')),
                    Some('h') => self.digits(|c| c.is_ascii_hexdigit()),
                    Some('o') => self.digits(|c| matches!(c, '0'..='7')),
                    _ => self.digits(|c| c.is_ascii_digit()),
                };
                bad |= n == 0;
                kind = TokenKind::BitsLit;
            } else if !after_dot {
                if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                    self.digits(|c| c.is_ascii_digit());
                    kind = TokenKind::Real;
                } else if self.peek() == Some('.')
                    && self.peek_at(1).is_none_or(|c| {
                        c.is_whitespace() || matches!(c, ')' | ']' | ',' | ':' | ';')
                    })
                {
                    let number = self.src[start..self.pos].to_string();
                    self.pos += 1;
                    let end = self.pos;
                    self.report(
                        "NE0006",
                        "a real number needs a digit after the decimal point",
                        start,
                        end,
                        Some(format!("write `{number}.0`")),
                    );
                    bad = true;
                    reported = true;
                }
                if matches!(self.peek(), Some('e' | 'E')) {
                    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
                    let d1 = self.peek_at(1);
                    let d2 = self.peek_at(2);
                    if digit(d1) || (matches!(d1, Some('+' | '-')) && digit(d2)) {
                        self.pos += if digit(d1) { 1 } else { 2 };
                        self.digits(|c| c.is_ascii_digit());
                        kind = TokenKind::Real;
                    }
                }
            }
        }
        if !bad && has_bad_underscores(&self.src[start..self.pos]) {
            let end = self.pos;
            self.report(
                "NE0006",
                "misplaced `_` in numeric literal",
                start,
                end,
                Some("use a single `_` between digits, for example `1_000`".into()),
            );
            bad = true;
            reported = true;
        }
        if self.peek().is_some_and(is_ident_continue) {
            let suffix_start = self.pos;
            while self.peek().is_some_and(is_ident_continue) {
                self.bump();
            }
            let suffix = &self.src[suffix_start..self.pos];
            let number = &self.src[start..suffix_start];
            let help = if !bad && !suffix.chars().any(|c| c.is_ascii_digit()) {
                Some(format!(
                    "separate the number from its unit: `{number} {suffix}`"
                ))
            } else {
                None
            };
            let end = self.pos;
            if !reported {
                self.report("NE0006", "invalid numeric literal", start, end, help);
            }
            bad = true;
        } else if bad && !reported {
            let end = self.pos;
            self.report(
                "NE0006",
                "invalid numeric literal: no digits after the prefix",
                start,
                end,
                None,
            );
        }
        self.push(if bad { TokenKind::Error } else { kind }, start);
    }

    fn escape(&mut self, in_char: bool) {
        let esc_start = self.pos;
        self.bump();
        let ok = match self.peek() {
            Some('n' | 'r' | 't' | '\\' | '"' | '0') => {
                self.bump();
                true
            }
            Some('\'') if in_char => {
                self.bump();
                true
            }
            Some('{' | '}') if !in_char => {
                self.bump();
                true
            }
            Some('x') => {
                self.bump();
                let mut n = 0;
                while n < 2 && self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.bump();
                    n += 1;
                }
                n == 2
            }
            Some('u') => {
                self.bump();
                if self.peek() == Some('{') {
                    self.bump();
                    let mut n = 0;
                    while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                        self.bump();
                        n += 1;
                    }
                    let closed = self.peek() == Some('}');
                    if closed {
                        self.bump();
                    }
                    closed && (1..=6).contains(&n)
                } else {
                    false
                }
            }
            Some('\n' | '\r') | None => false,
            Some(_) => {
                self.bump();
                false
            }
        };
        if !ok {
            let end = self.pos;
            self.report(
                "NE0007",
                "invalid escape sequence",
                esc_start,
                end,
                Some(r#"valid escapes: \n \r \t \\ \" \0 \xHH \u{HHHH} \{ \}"#.into()),
            );
        }
    }

    fn string(&mut self, start: usize, kind: StrKind) {
        self.bump(); // opening quote
        let mut interp: Option<usize> = None;
        loop {
            match self.peek() {
                None | Some('\n' | '\r') => {
                    let end = self.pos;
                    self.report(
                        "NE0004",
                        "unterminated string",
                        start,
                        end,
                        Some("a string must end on the same line with `\"`".into()),
                    );
                    self.push(TokenKind::Error, start);
                    return;
                }
                Some('"') => {
                    if let Some(open) = interp {
                        self.report(
                            "NE0016",
                            "unterminated `{` in string interpolation",
                            open,
                            open + 1,
                            Some("close it with `}` or write `\\{` for a literal brace".into()),
                        );
                    }
                    self.bump();
                    break;
                }
                Some('\\') if kind != StrKind::Raw => self.escape(false),
                Some(c) => {
                    let at = self.pos;
                    self.bump();
                    if kind == StrKind::Byte && !c.is_ascii() {
                        let end = self.pos;
                        self.report(
                            "NE0008",
                            "byte strings may only contain ASCII",
                            at,
                            end,
                            None,
                        );
                    }
                    if kind == StrKind::Normal {
                        match (c, interp) {
                            ('{', None) => interp = Some(at),
                            ('{', Some(_)) => self.report(
                                "NE0016",
                                "nested `{` in string interpolation",
                                at,
                                at + 1,
                                None,
                            ),
                            ('}', Some(_)) => interp = None,
                            ('}', None) => self.report(
                                "NE0016",
                                "unmatched `}` in string",
                                at,
                                at + 1,
                                Some("write `\\}` for a literal brace".into()),
                            ),
                            _ => {}
                        }
                    }
                }
            }
        }
        let tk = match kind {
            StrKind::Normal => TokenKind::Str,
            StrKind::Raw => TokenKind::RawStr,
            StrKind::Byte => TokenKind::ByteStr,
        };
        self.push(tk, start);
    }

    fn char_lit(&mut self, start: usize) {
        self.bump(); // opening quote
        match self.peek() {
            Some('\\') => self.escape(true),
            Some(c) if c != '\'' && c != '\n' && c != '\r' => {
                let at = self.pos;
                self.bump();
                if !c.is_ascii() {
                    let end = self.pos;
                    self.report("NE0008", "character literals must be ASCII", at, end, None);
                }
            }
            _ => {
                let end = self.pos;
                self.report("NE0008", "empty character literal", start, end, None);
                self.push(TokenKind::Error, start);
                return;
            }
        }
        if self.peek() == Some('\'') {
            self.bump();
            self.push(TokenKind::Char, start);
        } else {
            let end = self.pos;
            self.report("NE0004", "unterminated character literal", start, end, None);
            self.push(TokenKind::Error, start);
        }
    }

    fn punct(&mut self, start: usize, c: char) {
        if c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) {
            let prev = start
                .checked_sub(1)
                .and_then(|i| self.src.as_bytes().get(i))
                .copied();
            let after_operand = prev.is_some_and(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'_' | b')' | b']' | b'`')
            });
            if !after_operand {
                self.bump();
                self.digits(|d| d.is_ascii_digit());
                let end = self.pos;
                let text = self.src[start..end].to_string();
                self.report(
                    "NE0006",
                    "a real number needs a digit before the decimal point",
                    start,
                    end,
                    Some(format!("write `0{text}`")),
                );
                self.push(TokenKind::Error, start);
                return;
            }
        }
        let rest = self.rest();
        if let Some(&p) = Punct::ALL.iter().find(|p| rest.starts_with(p.as_str())) {
            self.pos += p.as_str().len();
            if p.is_open() {
                self.bracket_depth += 1;
                self.open_brackets.push(start);
            } else if p.is_close() {
                self.bracket_depth = self.bracket_depth.saturating_sub(1);
                self.open_brackets.pop();
            }
            self.push(TokenKind::Punct(p), start);
            return;
        }
        self.bump();
        let end = self.pos;
        let help = match c {
            '!' => Some("use `not` for logical negation".to_string()),
            _ => None,
        };
        self.report(
            "NE0003",
            format!("unexpected character `{c}`"),
            start,
            end,
            help,
        );
        self.push(TokenKind::Error, start);
    }
}

/// `true` if `_` separators are misplaced in a numeric literal.
fn has_bad_underscores(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    t.contains("__")
        || t.ends_with('_')
        || t.contains("_.")
        || t.contains("._")
        || ["0x_", "0b_", "0o_", "'b_", "'h_", "'d_", "'o_"]
            .iter()
            .any(|p| t.contains(*p))
}

/// Length in bytes of an IEC 61131-3 direct address at the start of `s`, if any.
/// Accepts `%I0.0`, `%QX1.7`, `%IW64`, `%MD10`, `%DB1.DBX0.0`.
fn iec_address_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let count_digits = |i: &mut usize| {
        let from = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - from
    };
    let mut i;
    if b.get(1..3) == Some(b"DB") && b.get(3).is_some_and(u8::is_ascii_digit) {
        i = 3;
        count_digits(&mut i);
        if b.get(i) != Some(&b'.') || b.get(i + 1..i + 3) != Some(b"DB") {
            return None;
        }
        i += 3;
        if !matches!(b.get(i), Some(b'X' | b'B' | b'W' | b'D')) {
            return None;
        }
        i += 1;
    } else {
        if !matches!(b.get(1), Some(b'I' | b'Q' | b'M')) {
            return None;
        }
        i = 2;
        if matches!(b.get(2), Some(b'X' | b'B' | b'W' | b'D' | b'L')) {
            i = 3;
        }
    }
    if count_digits(&mut i) == 0 {
        return None;
    }
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        count_digits(&mut i);
    }
    if s[i..].chars().next().is_some_and(is_ident_continue) {
        return None;
    }
    Some(i)
}
