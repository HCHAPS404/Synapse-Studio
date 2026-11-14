//! Recursive-descent parser (spec/grammar/neuron.ebnf).
//!
//! The parser accepts the union of all dialects; dialect restrictions are checked
//! later. It recovers from errors by skipping to the next line or block so that one
//! mistake produces one diagnostic.
//!
//! ```
//! use neuron_syntax::parse;
//! let out = parse("module blink for mcu\ntask t every 500 ms:\n  led.toggle()\n");
//! assert!(out.diagnostics.is_empty());
//! assert_eq!(out.file.items.len(), 1);
//! ```

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::lex;
use crate::span::Span;
use crate::token::{Keyword as Kw, Punct as P, Token, TokenKind as K};

/// Result of parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The syntax tree (partial when there are errors).
    pub file: File,
    /// Lexical and syntax diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a whole `.neuron` source.
pub fn parse(src: &str) -> Parsed {
    let lexed = lex(src);
    let mut docs = Vec::new();
    let mut toks = Vec::new();
    let mut pending_doc: Option<String> = None;
    for t in &lexed.tokens {
        match t.kind {
            K::DocComment => {
                let text = t.text(src).trim_start_matches('#').trim().to_string();
                pending_doc = Some(match pending_doc.take() {
                    Some(prev) => format!("{prev}\n{text}"),
                    None => text,
                });
            }
            k if k.is_trivia() => {}
            _ => {
                docs.push(pending_doc.take());
                toks.push(*t);
            }
        }
    }
    let mut p = Parser {
        src,
        toks,
        docs,
        pos: 0,
        diags: lexed.diagnostics,
    };
    let file = p.file();
    Parsed {
        file,
        diagnostics: p.diags,
    }
}

type PResult<T> = Result<T, ()>;

/// `true` for a plain numeric literal, possibly negated (`40`, `-40`, `0.5`).
fn is_plain_number(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(Lit::Int(_) | Lit::Real(_)) => true,
        ExprKind::Unary(UnOp::Neg, inner) => is_plain_number(inner),
        _ => false,
    }
}

/// Applies the unit of the upper bound to a unit-less lower bound.
fn share_unit(lo: Expr, hi: &Expr) -> Expr {
    match &hi.kind {
        ExprKind::Quantity(_, unit) if is_plain_number(&lo) => {
            let span = lo.span;
            Expr::new(ExprKind::Quantity(Box::new(lo), unit.clone()), span)
        }
        _ => lo,
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    docs: Vec<Option<String>>,
    pos: usize,
    diags: Vec<Diagnostic>,
}

/// Contextual words that end an expression in headers (`every`, `max`, ...).
const STOP_WORDS: &[&str] = &[
    "every", "offset", "after", "timeout", "via", "within", "from", "for", "max", "speed", "accel",
    "blend", "tool", "frame", "at", "size", "bind", "range", "font", "color", "theme", "implies",
];

impl<'a> Parser<'a> {
    // ---- cursor ---------------------------------------------------------

    fn tok(&self) -> Token {
        self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn kind(&self) -> K {
        self.tok().kind
    }

    fn nth(&self, n: usize) -> K {
        self.toks[(self.pos + n).min(self.toks.len() - 1)].kind
    }

    fn text(&self) -> &'a str {
        self.tok().span.text(self.src)
    }

    fn nth_text(&self, n: usize) -> &'a str {
        self.toks[(self.pos + n).min(self.toks.len() - 1)]
            .span
            .text(self.src)
    }

    fn span(&self) -> Span {
        self.tok().span
    }

    /// End offset of the last consumed token that carries source text
    /// (layout tokens are skipped so spans never include trailing newlines).
    fn prev_end(&self) -> usize {
        let mut i = self.pos;
        while i > 0 {
            i -= 1;
            let t = self.toks[i];
            if !matches!(t.kind, K::Newline | K::Indent | K::Dedent | K::Eof) {
                return t.span.end;
            }
        }
        0
    }

    fn since(&self, start: usize) -> Span {
        Span {
            start,
            end: self.prev_end().max(start),
        }
    }

    fn bump(&mut self) -> Token {
        let t = self.tok();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at(&self, k: K) -> bool {
        self.kind() == k
    }

    fn at_p(&self, p: P) -> bool {
        self.kind() == K::Punct(p)
    }

    fn at_kw(&self, k: Kw) -> bool {
        self.kind() == K::Keyword(k)
    }

    /// Identifier (including contextual keywords) with exact text.
    fn at_word(&self, w: &str) -> bool {
        self.kind() == K::Ident && self.text() == w
    }

    fn eat_p(&mut self, p: P) -> bool {
        if self.at_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn error_here(&mut self, expected: &str) {
        let t = self.tok();
        let found = t.kind.describe();
        if t.kind == K::Error {
            return; // the lexer already reported it
        }
        self.diags.push(Diagnostic::error(
            "NE0003",
            format!("expected {expected}, found {found}"),
            if t.span.is_empty() {
                Span {
                    start: t.span.start,
                    end: t.span.start,
                }
            } else {
                t.span
            },
        ));
    }

    fn expect_p(&mut self, p: P) -> PResult<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.error_here(&format!("`{}`", p.as_str()));
            Err(())
        }
    }

    /// Expects `>`, splitting a `>>` token so nested generics close correctly.
    fn expect_gt(&mut self) -> PResult<()> {
        if self.at_p(P::Shr) {
            let i = self.pos;
            let s = self.toks[i].span;
            self.toks[i] = Token {
                kind: K::Punct(P::Gt),
                span: Span {
                    start: s.start + 1,
                    end: s.end,
                },
            };
            return Ok(());
        }
        self.expect_p(P::Gt)
    }

    fn expect_word(&mut self, w: &str) -> PResult<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            self.error_here(&format!("`{w}`"));
            Err(())
        }
    }

    fn expect_newline(&mut self) -> PResult<()> {
        if self.at(K::Newline) {
            self.bump();
            Ok(())
        } else if self.at(K::Eof) || self.at(K::Dedent) {
            Ok(())
        } else {
            self.error_here("end of line");
            Err(())
        }
    }

    fn ident(&mut self) -> PResult<Ident> {
        let id = self.unit_ident()?;
        if !id.name.is_ascii() {
            self.diags.push(
                Diagnostic::error(
                    "NE0015",
                    format!("`{}` is not a valid name", id.name),
                    id.span,
                )
                .with_help("non-ASCII characters are only allowed inside unit names"),
            );
        }
        Ok(id)
    }

    /// Identifier that may contain the unit characters (micro, ohm, degree).
    fn unit_ident(&mut self) -> PResult<Ident> {
        if self.at(K::Ident) {
            let t = self.bump();
            let name = t.span.text(self.src).trim_matches('`').to_string();
            Ok(Ident { name, span: t.span })
        } else {
            self.error_here("a name");
            Err(())
        }
    }

    /// Identifier or keyword used as a name in a path segment after `.` (e.g. `x.in`).
    fn member_name(&mut self) -> PResult<Ident> {
        match self.kind() {
            K::Ident | K::Keyword(_) | K::Int | K::IecAddr => {
                let t = self.bump();
                Ok(Ident {
                    name: t.span.text(self.src).trim_matches('`').to_string(),
                    span: t.span,
                })
            }
            _ => {
                self.error_here("a field or method name");
                Err(())
            }
        }
    }

    fn path(&mut self) -> PResult<Path> {
        let start = self.span().start;
        let mut segments = vec![self.ident()?];
        while self.at_p(P::Dot)
            && matches!(self.nth(1), K::Ident | K::Keyword(_))
            && self.nth(1) != K::Punct(P::LBrace)
        {
            self.bump();
            segments.push(self.member_name()?);
        }
        Ok(Path {
            segments,
            span: self.since(start),
        })
    }

    /// Skips to the end of the current line, including a nested block.
    fn recover_line(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.kind() {
                K::Eof => return,
                K::Indent => depth += 1,
                K::Dedent => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                K::Newline if depth == 0 => {
                    self.bump();
                    if self.at(K::Indent) {
                        self.skip_block();
                    }
                    return;
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn skip_block(&mut self) {
        if !self.at(K::Indent) {
            return;
        }
        let mut depth = 0usize;
        loop {
            match self.kind() {
                K::Eof => return,
                K::Indent => depth += 1,
                K::Dedent => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    // ---- file -----------------------------------------------------------

    fn file(&mut self) -> File {
        let start = 0;
        let header = self.header();
        let cell = header.dialect == Dialect::Cell;
        let mut items = Vec::new();
        if cell {
            if self.at(K::Indent) {
                self.bump();
            } else if !self.at(K::Eof) {
                let s = self.span();
                self.diags.push(
                    Diagnostic::error(
                        "NE0005",
                        "expected an indented block after the cell header",
                        s,
                    )
                    .with_help("indent every cell item by 2 spaces"),
                );
            }
        }
        let mut cell_open = cell;
        loop {
            if self.at(K::Eof) {
                break;
            }
            if self.at(K::Dedent) {
                self.bump();
                cell_open = false;
                continue;
            }
            if cell && !cell_open && !self.at(K::Newline) {
                let s = self.span();
                self.diags.push(
                    Diagnostic::error("NE0013", "content after the cell block", s).with_help(
                        "a cell file holds one `cell` block; put other code in module files",
                    ),
                );
                self.recover_line();
                continue;
            }
            if self.at(K::Newline) {
                self.bump();
                continue;
            }
            if self.at(K::Indent) {
                let s = self.span();
                self.diags.push(
                    Diagnostic::error("NE0009", "unexpected indentation", s)
                        .with_help("top-level items start at column 1"),
                );
                self.skip_block();
                continue;
            }
            match self.item() {
                Ok(item) => items.push(item),
                Err(()) => self.recover_line(),
            }
        }
        File {
            header,
            items,
            span: Span {
                start,
                end: self.src.len(),
            },
        }
    }

    fn header(&mut self) -> Header {
        let start = self.span().start;
        let fallback = Header {
            name: Path {
                segments: Vec::new(),
                span: Span::at(start),
            },
            dialect: Dialect::Core,
            target: None,
            span: Span::at(start),
        };
        let result: PResult<Header> = (|| {
            if self.eat_kw(Kw::Cell) {
                let name = self.ident()?;
                let path = Path {
                    span: name.span,
                    segments: vec![name],
                };
                let span = self.since(start);
                self.expect_newline()?;
                return Ok(Header {
                    name: path,
                    dialect: Dialect::Cell,
                    target: None,
                    span,
                });
            }
            if !self.eat_kw(Kw::Module) {
                self.error_here("`module` or `cell` header");
                return Err(());
            }
            let name = self.path()?;
            let mut dialect = Dialect::Core;
            if self.eat_kw(Kw::For) {
                let d = self.ident()?;
                dialect = match Dialect::from_name(&d.name) {
                    Some(x) => x,
                    None => {
                        self.diags.push(
                            Diagnostic::error(
                                "NE0010",
                                format!("unknown dialect `{}`", d.name),
                                d.span,
                            )
                            .with_help("dialects: core, mcu, plc, hdl, robot, ui, host"),
                        );
                        Dialect::Core
                    }
                };
            }
            let target = if self.eat_kw(Kw::On) {
                Some(self.path()?)
            } else {
                None
            };
            let span = self.since(start);
            self.expect_newline()?;
            Ok(Header {
                name,
                dialect,
                target,
                span,
            })
        })();
        match result {
            Ok(h) => h,
            Err(()) => {
                self.recover_line();
                fallback
            }
        }
    }

    // ---- items ----------------------------------------------------------

    fn item(&mut self) -> PResult<Item> {
        let start = self.span().start;
        let doc = self.docs.get(self.pos).cloned().flatten();
        let mut attrs = Vec::new();
        while self.at_p(P::At) {
            attrs.push(self.attribute()?);
        }
        let public = self.eat_kw(Kw::Pub);
        let kind = self.item_kind()?;
        Ok(Item {
            attrs,
            public,
            doc,
            kind,
            span: self.since(start),
        })
    }

    fn attribute(&mut self) -> PResult<Attribute> {
        let start = self.span().start;
        self.expect_p(P::At)?;
        let name = self.ident()?;
        let args = if self.at_p(P::LParen) {
            self.args()?
        } else {
            Vec::new()
        };
        let span = self.since(start);
        self.expect_newline()?;
        Ok(Attribute { name, args, span })
    }

    fn item_kind(&mut self) -> PResult<ItemKind> {
        match self.kind() {
            K::Keyword(Kw::Use) => self.use_decl(),
            K::Keyword(Kw::Const) => {
                self.bump();
                let name = self.ident()?;
                let ty = if self.eat_p(P::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect_p(P::Eq)?;
                let value = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Const { name, ty, value })
            }
            K::Keyword(Kw::Var | Kw::Let) => Ok(ItemKind::Var(self.var_decl(false)?)),
            K::Keyword(Kw::Signal) => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = self.ty()?;
                let init = if self.eat_p(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(ItemKind::Signal { name, ty, init })
            }
            K::Keyword(Kw::Struct) => self.struct_decl(),
            K::Keyword(Kw::Enum) => self.enum_decl(),
            K::Keyword(Kw::Class) => self.class_decl(),
            K::Keyword(Kw::Interface) => self.interface_decl(),
            K::Keyword(Kw::Type) => {
                self.bump();
                let name = self.ident()?;
                let generics = self.generics()?;
                self.expect_p(P::Eq)?;
                let ty = self.ty()?;
                self.expect_newline()?;
                Ok(ItemKind::Alias { name, generics, ty })
            }
            K::Keyword(Kw::Fn) => Ok(ItemKind::Fn(self.fn_decl()?)),
            K::Keyword(Kw::Task) => self.task_decl(),
            K::Keyword(Kw::On) if self.nth_text(1) == "interrupt" => {
                self.bump();
                self.bump();
                let source = self.expr_stop()?;
                let attrs = self.task_attrs()?;
                let body = self.colon_block()?;
                Ok(ItemKind::Interrupt {
                    source,
                    attrs,
                    body,
                })
            }
            K::Keyword(Kw::When) => Ok(ItemKind::Rule(self.rule()?)),
            K::Keyword(Kw::Safety) => self.safety_decl(),
            K::Keyword(Kw::Verify) => self.verify_decl(),
            K::Keyword(Kw::Test) => {
                self.bump();
                let name = self.string_lit()?;
                let body = self.colon_block()?;
                Ok(ItemKind::Test { name, body })
            }
            K::Keyword(Kw::Io) => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let kind = self.expr()?;
                self.expect_p(P::At)?;
                let location = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Io {
                    name,
                    kind,
                    location,
                })
            }
            K::Keyword(Kw::Trusted) => self.trusted_item(),
            K::Keyword(Kw::Extern) => {
                self.bump();
                let abi = self.string_lit()?;
                let func = self.fn_decl()?;
                Ok(ItemKind::ExternFn { abi, func })
            }
            K::Ident => self.contextual_item(),
            _ => {
                self.error_here("an item (`fn`, `task`, `io`, `var`, ...)");
                Err(())
            }
        }
    }

    fn contextual_item(&mut self) -> PResult<ItemKind> {
        let w = self.text();
        let next = self.nth(1);
        let followed_by_name = next == K::Ident;
        match w {
            "unit" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Eq)?;
                let def = self.unit_expr()?.ok_or(())?;
                self.expect_newline()?;
                Ok(ItemKind::Unit { name, def })
            }
            "event" if followed_by_name => self.event_decl(),
            "fsm" if followed_by_name => Ok(ItemKind::Fsm(self.fsm_decl()?)),
            "sequence" if followed_by_name => Ok(ItemKind::Sequence(self.sequence_decl()?)),
            "clocks" if next == K::Punct(P::Colon) => self.clocks_decl(),
            "uses" if followed_by_name => {
                self.bump();
                let mut paths = vec![self.path()?];
                while self.eat_p(P::Comma) {
                    paths.push(self.path()?);
                }
                self.expect_newline()?;
                Ok(ItemKind::Uses(paths))
            }
            "scan" => {
                self.bump();
                let e = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Scan(e))
            }
            "inputs" | "outputs" | "memory" if next == K::Punct(P::Colon) => self.plc_section(),
            "rung" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                let body = self.colon_block()?;
                Ok(ItemKind::Rung { name, body })
            }
            "retain" => Ok(ItemKind::Var(self.var_decl(false)?)),
            "clock" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let freq = self.expr()?;
                let pin = if self.eat_p(P::At) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(ItemKind::Clock { name, freq, pin })
            }
            "circuit" if followed_by_name => Ok(ItemKind::Circuit(self.circuit_decl()?)),
            "sync" => {
                self.bump();
                let from = self.expr_until_arrow()?;
                self.expect_p(P::Arrow)?;
                let to = self.expr_stop()?;
                self.expect_word("via")?;
                let via = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Sync { from, to, via })
            }
            "frame" | "tool" | "point" if followed_by_name => {
                let kind = match w {
                    "frame" => GeometryKind::Frame,
                    "tool" => GeometryKind::Tool,
                    _ => GeometryKind::Point,
                };
                self.bump();
                let name = self.ident()?;
                let ty = if self.eat_p(P::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect_p(P::Eq)?;
                let value = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Geometry {
                    kind,
                    name,
                    ty,
                    value,
                })
            }
            "display" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let driver = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Display { name, driver })
            }
            "screen" if followed_by_name => self.screen_decl(),
            "theme" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                let body = self.colon_block()?;
                Ok(ItemKind::Theme { name, body })
            }
            "device" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = self.path()?;
                let address = if self.eat_p(P::At) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(ItemKind::Device { name, ty, address })
            }
            "place" if followed_by_name => {
                self.bump();
                let module = self.path()?;
                if !self.eat_kw(Kw::On) {
                    self.error_here("`on`");
                    return Err(());
                }
                let device = self.ident()?;
                self.expect_newline()?;
                Ok(ItemKind::Place { module, device })
            }
            "link" => {
                self.bump();
                let from = self.expr_until_arrow()?;
                self.expect_p(P::Arrow)?;
                let to = self.expr_stop()?;
                self.expect_word("via")?;
                let via = self.expr_stop()?;
                let every = if self.eat_word("every") {
                    Some(self.expr_stop()?)
                } else {
                    None
                };
                let within = if self.eat_word("within") {
                    Some(self.expr_stop()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(ItemKind::Link(LinkDecl {
                    from,
                    to,
                    via,
                    every,
                    within,
                }))
            }
            "tag" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = self.ty()?;
                self.expect_p(P::At)?;
                let mut locations = vec![self.expr()?];
                while self.eat_p(P::Comma) {
                    locations.push(self.expr()?);
                }
                self.expect_newline()?;
                Ok(ItemKind::Tag {
                    name,
                    ty,
                    locations,
                })
            }
            "zone" if followed_by_name => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let shape = self.expr()?;
                self.expect_newline()?;
                Ok(ItemKind::Zone { name, shape })
            }
            "mutex" if followed_by_name => {
                self.bump();
                let zone = self.ident()?;
                self.expect_word("between")?;
                let mut parties = vec![self.expr()?];
                while self.eat_p(P::Comma) {
                    parties.push(self.expr()?);
                }
                self.expect_newline()?;
                Ok(ItemKind::Mutex { zone, parties })
            }
            _ => {
                self.error_here("an item (`fn`, `task`, `io`, `var`, ...)");
                Err(())
            }
        }
    }

    fn use_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let start = self.span().start;
        let mut segments = vec![self.ident()?];
        let mut names = Vec::new();
        while self.eat_p(P::Dot) {
            if self.eat_p(P::LBrace) {
                names.push(self.ident()?);
                while self.eat_p(P::Comma) {
                    if self.at_p(P::RBrace) {
                        break;
                    }
                    names.push(self.ident()?);
                }
                self.expect_p(P::RBrace)?;
                break;
            }
            segments.push(self.ident()?);
        }
        let path = Path {
            segments,
            span: self.since(start),
        };
        let alias = if self.eat_kw(Kw::As) {
            Some(self.ident()?)
        } else {
            None
        };
        self.expect_newline()?;
        Ok(ItemKind::Use { path, names, alias })
    }

    fn var_decl(&mut self, plc_section: bool) -> PResult<VarDecl> {
        let start = self.span().start;
        let retain = self.eat_word("retain");
        let mutable = if (plc_section && self.at(K::Ident)) || self.eat_kw(Kw::Var) {
            true
        } else if self.eat_kw(Kw::Let) {
            false
        } else {
            self.error_here("`var` or `let`");
            return Err(());
        };
        let name = self.ident()?;
        let ty = if self.eat_p(P::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        let init = if self.eat_p(P::Eq) {
            Some(self.expr()?)
        } else {
            None
        };
        let at = if self.eat_p(P::At) {
            Some(self.expr()?)
        } else {
            None
        };
        let span = self.since(start);
        self.expect_newline()?;
        Ok(VarDecl {
            mutable,
            retain,
            name,
            ty,
            init,
            at,
            span,
        })
    }

    fn event_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let mut fields = Vec::new();
        if self.eat_p(P::Colon) {
            loop {
                let f = self.ident()?;
                let ty = if self.eat_p(P::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                fields.push((f, ty));
                if !self.eat_p(P::Comma) {
                    break;
                }
            }
        }
        let from = if self.eat_word("from") {
            Some(self.ident()?)
        } else {
            None
        };
        self.expect_newline()?;
        Ok(ItemKind::Event { name, fields, from })
    }

    fn generics(&mut self) -> PResult<Vec<GenericParam>> {
        let mut out = Vec::new();
        if !self.eat_p(P::Lt) {
            return Ok(out);
        }
        loop {
            let gstart = self.span().start;
            if self.eat_kw(Kw::Const) {
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = self.ty()?;
                out.push(GenericParam::Const {
                    name,
                    ty,
                    span: self.since(gstart),
                });
            } else {
                let name = self.ident()?;
                let mut bounds = Vec::new();
                if self.eat_p(P::Colon) {
                    bounds.push(self.path()?);
                    while self.eat_p(P::Plus) {
                        bounds.push(self.path()?);
                    }
                }
                out.push(GenericParam::Type {
                    name,
                    bounds,
                    span: self.since(gstart),
                });
            }
            if !self.eat_p(P::Comma) {
                break;
            }
        }
        self.expect_gt()?;
        Ok(out)
    }

    /// `:` NEWLINE INDENT ... DEDENT, calling `f` for each member line.
    fn members<T>(&mut self, mut f: impl FnMut(&mut Self) -> PResult<T>) -> PResult<Vec<T>> {
        self.expect_p(P::Colon)?;
        self.expect_newline()?;
        let mut out = Vec::new();
        if !self.at(K::Indent) {
            let s = self.span();
            self.diags.push(Diagnostic::error(
                "NE0005",
                "expected an indented block after `:`",
                s,
            ));
            return Err(());
        }
        self.bump();
        while !self.at(K::Dedent) && !self.at(K::Eof) {
            if self.at(K::Newline) {
                self.bump();
                continue;
            }
            match f(self) {
                Ok(m) => out.push(m),
                Err(()) => self.recover_line(),
            }
        }
        self.eat_dedent();
        Ok(out)
    }

    /// Like [`members`](Self::members) but records the span of every member.
    fn members_sp<T>(
        &mut self,
        mut f: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<Spanned<T>>> {
        self.members(|p| {
            let start = p.span().start;
            let node = f(p)?;
            Ok(Spanned {
                node,
                span: p.since(start),
            })
        })
    }

    fn eat_dedent(&mut self) {
        if self.at(K::Dedent) {
            self.bump();
        }
    }

    fn struct_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let generics = self.generics()?;
        let members = self.members(|p| {
            let fstart = p.span().start;
            let name = p.ident()?;
            p.expect_p(P::Colon)?;
            let ty = p.ty()?;
            let default = if p.eat_p(P::Eq) {
                Some(p.expr()?)
            } else {
                None
            };
            p.expect_newline()?;
            Ok(Field {
                name,
                ty,
                default,
                span: p.since(fstart),
            })
        })?;
        Ok(ItemKind::Struct(TypeDef {
            name,
            generics,
            repr: None,
            members,
        }))
    }

    fn enum_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let generics = self.generics()?;
        let repr = if self.at_p(P::Colon) && self.nth(1) != K::Newline {
            self.bump();
            Some(self.ty()?)
        } else {
            None
        };
        let members = self.members(|p| {
            let vstart = p.span().start;
            let name = p.ident()?;
            let mut payload = Vec::new();
            if p.eat_p(P::LParen) {
                if !p.at_p(P::RParen) {
                    payload.push(p.ty()?);
                    while p.eat_p(P::Comma) {
                        payload.push(p.ty()?);
                    }
                }
                p.expect_p(P::RParen)?;
            }
            let value = if p.eat_p(P::Eq) {
                Some(p.expr()?)
            } else {
                None
            };
            p.expect_newline()?;
            Ok(Variant {
                name,
                payload,
                value,
                span: p.since(vstart),
            })
        })?;
        Ok(ItemKind::Enum(TypeDef {
            name,
            generics,
            repr,
            members,
        }))
    }

    fn class_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let generics = self.generics()?;
        let mut interfaces = Vec::new();
        if self.eat_kw(Kw::Is) {
            interfaces.push(self.path()?);
            while self.eat_p(P::Comma) {
                interfaces.push(self.path()?);
            }
        }
        let mut vars = Vec::new();
        let mut methods = Vec::new();
        let mut invariants = Vec::new();
        self.members(|p| {
            if p.eat_kw(Kw::Invariant) {
                invariants.push(p.expr()?);
                return p.expect_newline();
            }
            if p.at_kw(Kw::Var) || p.at_kw(Kw::Let) {
                vars.push(p.var_decl(false)?);
                return Ok(());
            }
            methods.push(p.item()?);
            Ok(())
        })?;
        Ok(ItemKind::Class(ClassDecl {
            name,
            generics,
            interfaces,
            vars,
            methods,
            invariants,
        }))
    }

    fn interface_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let generics = self.generics()?;
        let methods = self.members(|p| {
            let sig = p.fn_sig()?;
            p.expect_newline()?;
            Ok(sig)
        })?;
        Ok(ItemKind::Interface {
            name,
            generics,
            methods,
        })
    }

    fn fn_sig(&mut self) -> PResult<FnSig> {
        let start = self.span().start;
        if !self.eat_kw(Kw::Fn) {
            self.error_here("`fn`");
            return Err(());
        }
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_p(P::LParen)?;
        let mut params = Vec::new();
        while !self.at_p(P::RParen) {
            let pstart = self.span().start;
            let mutable = self.eat_kw(Kw::Mut);
            if self.at_kw(Kw::SelfValue) {
                let t = self.bump();
                params.push(Param {
                    mutable,
                    name: Ident {
                        name: "self".into(),
                        span: t.span,
                    },
                    ty: None,
                    default: None,
                    span: self.since(pstart),
                });
            } else {
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = Some(self.ty()?);
                let default = if self.eat_p(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                params.push(Param {
                    mutable,
                    name,
                    ty,
                    default,
                    span: self.since(pstart),
                });
            }
            if !self.eat_p(P::Comma) {
                break;
            }
        }
        self.expect_p(P::RParen)?;
        let ret = if self.eat_p(P::Arrow) {
            Some(self.ty()?)
        } else {
            None
        };
        Ok(FnSig {
            name,
            generics,
            params,
            ret,
            span: self.since(start),
        })
    }

    fn fn_decl(&mut self) -> PResult<FnDecl> {
        let sig = self.fn_sig()?;
        if self.eat_p(P::Eq) {
            let e = self.expr()?;
            self.expect_newline()?;
            return Ok(FnDecl {
                sig,
                contracts: Vec::new(),
                body: FnBody::Expr(e),
            });
        }
        let mut body = self.colon_block_with(true)?;
        let contracts = std::mem::take(&mut body.contracts);
        Ok(FnDecl {
            sig,
            contracts,
            body: FnBody::Block(body),
        })
    }

    fn task_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let trigger = self.trigger()?;
        let attrs = self.task_attrs()?;
        let body = self.colon_block()?;
        Ok(ItemKind::Task(TaskDecl {
            name,
            trigger,
            attrs,
            body,
        }))
    }

    fn trigger(&mut self) -> PResult<Trigger> {
        if self.eat_word("every") {
            let period = self.expr_stop()?;
            let offset = if self.eat_word("offset") {
                Some(self.expr_stop()?)
            } else {
                None
            };
            Ok(Trigger::Every { period, offset })
        } else if self.eat_kw(Kw::On) {
            Ok(Trigger::On(self.expr_stop()?))
        } else {
            self.error_here("`every <period>` or `on <event>`");
            Err(())
        }
    }

    fn task_attrs(&mut self) -> PResult<Vec<Spanned<TaskAttr>>> {
        let mut out = Vec::new();
        if !self.eat_p(P::LParen) {
            return Ok(out);
        }
        while !self.at_p(P::RParen) {
            let astart = self.span().start;
            let w = self.text();
            if self.kind() != K::Ident {
                self.error_here(
                    "a task attribute (`prio`, `deadline`, `core`, `stack`, `reads`, `writes`)",
                );
                return Err(());
            }
            self.bump();
            let attr = match w {
                "prio" => TaskAttr::Prio(self.expr()?),
                "deadline" => TaskAttr::Deadline(self.expr()?),
                "core" => TaskAttr::Core(self.expr()?),
                "stack" => TaskAttr::Stack(self.expr()?),
                "reads" | "writes" => {
                    let mut names = Vec::new();
                    while self.at(K::Ident) {
                        names.push(self.ident()?);
                    }
                    if w == "reads" {
                        TaskAttr::Reads(names)
                    } else {
                        TaskAttr::Writes(names)
                    }
                }
                other => {
                    let s = self.toks[self.pos - 1].span;
                    self.diags.push(
                        Diagnostic::error("NE0003", format!("unknown task attribute `{other}`"), s)
                            .with_help("attributes: prio, deadline, core, stack, reads, writes"),
                    );
                    return Err(());
                }
            };
            out.push(Spanned {
                node: attr,
                span: self.since(astart),
            });
            if !self.eat_p(P::Comma) {
                break;
            }
        }
        self.expect_p(P::RParen)?;
        Ok(out)
    }

    fn rule(&mut self) -> PResult<Rule> {
        let start = self.span().start;
        self.bump(); // when
        let cond = self.expr_stop()?;
        let hold = if self.eat_kw(Kw::For) {
            Some(self.expr()?)
        } else {
            None
        };
        let body = self.colon_block()?;
        Ok(Rule {
            cond,
            hold,
            body,
            span: self.since(start),
        })
    }

    fn fsm_decl(&mut self) -> PResult<FsmDecl> {
        self.bump();
        let name = self.ident()?;
        let trigger = if self.at_word("every") || self.at_kw(Kw::On) {
            Some(self.trigger()?)
        } else {
            None
        };
        let mut vars = Vec::new();
        let mut states = Vec::new();
        self.members(|p| {
            if p.at_kw(Kw::Var) || p.at_kw(Kw::Let) {
                vars.push(p.var_decl(false)?);
                return Ok(());
            }
            states.push(p.state()?);
            Ok(())
        })?;
        Ok(FsmDecl {
            name,
            trigger,
            vars,
            states,
        })
    }

    fn state(&mut self) -> PResult<State> {
        let start = self.span().start;
        let initial = self.eat_word("initial");
        let is_final = self.eat_word("final");
        self.expect_word("state")?;
        let name = self.ident()?;
        let mut st = State {
            initial,
            is_final,
            name,
            entry: None,
            exit: None,
            during: None,
            transitions: Vec::new(),
            span: Span::at(start),
        };
        self.members(|p| {
            if p.at_p(P::Arrow) {
                st.transitions.push(p.transition()?);
                return Ok(());
            }
            let which = p.text().to_string();
            if p.at(K::Ident)
                && matches!(which.as_str(), "entry" | "exit" | "do")
                && p.nth(1) == K::Punct(P::Colon)
            {
                p.bump();
                let b = p.colon_block()?;
                match which.as_str() {
                    "entry" => st.entry = Some(b),
                    "exit" => st.exit = Some(b),
                    _ => st.during = Some(b),
                }
                return Ok(());
            }
            p.error_here("`entry:`, `exit:`, `do:` or a transition `-> State`");
            Err(())
        })?;
        st.span = self.since(start);
        Ok(st)
    }

    fn transition(&mut self) -> PResult<Transition> {
        let start = self.span().start;
        self.expect_p(P::Arrow)?;
        let target = self.ident()?;
        let mut when = None;
        let mut after = None;
        if self.eat_kw(Kw::When) {
            when = Some(self.expr_stop()?);
            if self.at_kw(Kw::Or) && self.nth_text(1) == "after" {
                self.bump();
            }
        }
        if self.eat_word("after") {
            after = Some(self.expr_stop()?);
        }
        let action = if self.at_p(P::Colon) {
            Some(self.colon_block()?)
        } else {
            self.expect_newline()?;
            None
        };
        Ok(Transition {
            target,
            when,
            after,
            action,
            span: self.since(start),
        })
    }

    fn sequence_decl(&mut self) -> PResult<SequenceDecl> {
        self.bump();
        let name = self.ident()?;
        let mut params = Vec::new();
        if self.eat_p(P::LParen) {
            while !self.at_p(P::RParen) {
                let pstart = self.span().start;
                let mutable = self.eat_kw(Kw::Mut);
                let n = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = Some(self.ty()?);
                let default = if self.eat_p(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                params.push(Param {
                    mutable,
                    name: n,
                    ty,
                    default,
                    span: self.since(pstart),
                });
                if !self.eat_p(P::Comma) {
                    break;
                }
            }
            self.expect_p(P::RParen)?;
        }
        let on = if self.eat_kw(Kw::On) {
            Some(self.ident()?)
        } else {
            None
        };
        let items: Vec<SeqItem> = self
            .members(|p| p.seq_item())?
            .into_iter()
            .flatten()
            .collect();
        Ok(SequenceDecl {
            name,
            params,
            on,
            items,
        })
    }

    /// One logical line of a sequence: a step, a parallel block, or one item per
    /// `;`-separated statement.
    fn seq_item(&mut self) -> PResult<Vec<SeqItem>> {
        let start = self.span().start;
        if (self.at_word("initial") && self.nth_text(1) == "step")
            || (self.at_word("step") && self.nth(1) == K::Ident)
        {
            let initial = self.eat_word("initial");
            self.expect_word("step")?;
            let name = self.ident()?;
            let body = self.seq_body()?;
            return Ok(vec![SeqItem::Step {
                initial,
                name,
                body,
                span: self.since(start),
            }]);
        }
        if self.at_word("parallel") && self.nth(1) == K::Punct(P::Colon) {
            self.bump();
            let branches = self.members(|p| {
                p.expect_word("branch")?;
                let n = p.ident()?;
                let body = p.seq_body()?;
                Ok((n, body))
            })?;
            return Ok(vec![SeqItem::Parallel {
                branches,
                span: self.since(start),
            }]);
        }
        Ok(self.stmt_line()?.into_iter().map(SeqItem::Stmt).collect())
    }

    /// Body of a step or branch: either one line after `:` or an indented list.
    fn seq_body(&mut self) -> PResult<Vec<SeqItem>> {
        self.expect_p(P::Colon)?;
        if !self.at(K::Newline) {
            let stmts = self.simple_stmts_line()?;
            return Ok(stmts.into_iter().map(SeqItem::Stmt).collect());
        }
        self.bump();
        if !self.at(K::Indent) {
            let s = self.span();
            self.diags.push(Diagnostic::error(
                "NE0005",
                "expected an indented block after `:`",
                s,
            ));
            return Err(());
        }
        self.bump();
        let mut out = Vec::new();
        while !self.at(K::Dedent) && !self.at(K::Eof) {
            if self.at(K::Newline) {
                self.bump();
                continue;
            }
            match self.seq_item() {
                Ok(i) => out.extend(i),
                Err(()) => self.recover_line(),
            }
        }
        self.eat_dedent();
        Ok(out)
    }

    fn safety_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let members = self.members_sp(|p| {
            if p.at_kw(Kw::When) {
                return Ok(SafetyMember::Rule(p.rule()?));
            }
            if p.at_kw(Kw::On) && p.nth_text(1) == "fault" {
                p.bump();
                p.bump();
                let fault = if p.at(K::Ident) {
                    Some(p.path()?)
                } else {
                    None
                };
                let body = p.colon_block()?;
                return Ok(SafetyMember::OnFault { fault, body });
            }
            let w = p.text();
            if p.at(K::Ident) {
                match w {
                    "rate" | "watchdog" => {
                        p.bump();
                        let e = p.expr()?;
                        p.expect_newline()?;
                        return Ok(if w == "rate" {
                            SafetyMember::Rate(e)
                        } else {
                            SafetyMember::Watchdog(e)
                        });
                    }
                    "safe_state" => {
                        p.bump();
                        return Ok(SafetyMember::SafeState(p.colon_block()?));
                    }
                    "reset" => {
                        p.bump();
                        if !p.eat_kw(Kw::When) {
                            p.error_here("`when`");
                            return Err(());
                        }
                        let e = p.expr()?;
                        p.expect_newline()?;
                        return Ok(SafetyMember::Reset(e));
                    }
                    _ => {}
                }
            }
            p.error_here("`safe_state:`, `when`, `watchdog`, `rate`, `on fault:` or `reset when`");
            Err(())
        })?;
        Ok(ItemKind::Safety(members))
    }

    fn verify_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let props = self.members_sp(|p| {
            let w = p.text();
            let prop = match w {
                "never" => {
                    p.bump();
                    Property::Never(p.expr()?)
                }
                "always" => {
                    p.bump();
                    let cond = p.expr_stop()?;
                    let implies = if p.eat_word("implies") {
                        Some(p.expr()?)
                    } else {
                        None
                    };
                    Property::Always { cond, implies }
                }
                "eventually_within" => {
                    p.bump();
                    let within = p.expr_stop()?;
                    p.expect_word("after")?;
                    let after = p.expr_stop()?;
                    p.expect_p(P::Colon)?;
                    let goal = p.expr()?;
                    Property::EventuallyWithin {
                        within,
                        after,
                        goal,
                    }
                }
                _ => {
                    p.error_here("`never`, `always` or `eventually_within`");
                    return Err(());
                }
            };
            p.expect_newline()?;
            Ok(prop)
        })?;
        Ok(ItemKind::Verify(props))
    }

    fn clocks_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let defs = self.members(|p| {
            let cstart = p.span().start;
            let name = p.ident()?;
            p.expect_p(P::Eq)?;
            let value = p.expr_stop()?;
            let from = if p.eat_word("from") {
                Some(p.expr()?)
            } else {
                None
            };
            p.expect_newline()?;
            Ok(ClockDef {
                name,
                value,
                from,
                span: p.since(cstart),
            })
        })?;
        Ok(ItemKind::Clocks(defs))
    }

    fn trusted_item(&mut self) -> PResult<ItemKind> {
        self.bump(); // trusted
        if self.eat_kw(Kw::Extern) {
            let abi = self.string_lit()?;
            let fns = self.members(|p| {
                let s = p.fn_sig()?;
                p.expect_newline()?;
                Ok(s)
            })?;
            return Ok(ItemKind::ExternBlock { abi, fns });
        }
        if self.at_word("asm") {
            return Ok(ItemKind::Asm(self.asm_block()?));
        }
        self.error_here("`asm` or `extern` after `trusted`");
        Err(())
    }

    fn asm_block(&mut self) -> PResult<AsmBlock> {
        self.bump(); // asm
        let arch = self.ident()?;
        let mut operands = String::new();
        if self.at_p(P::LParen) {
            let open = self.bump().span.end;
            let mut depth = 1usize;
            while depth > 0 && !self.at(K::Eof) {
                if self.at_p(P::LParen) || self.at_p(P::LBracket) {
                    depth += 1;
                } else if self.at_p(P::RParen) || self.at_p(P::RBracket) {
                    depth -= 1;
                }
                let t = self.bump();
                if depth == 0 {
                    operands = self.src[open..t.span.start].trim().to_string();
                }
            }
        }
        self.expect_p(P::Colon)?;
        self.expect_newline()?;
        if !self.at(K::Indent) {
            let s = self.span();
            self.diags.push(Diagnostic::error(
                "NE0005",
                "expected an indented block after `:`",
                s,
            ));
            return Err(());
        }
        // raw lines: take the source text of each line of the block
        let start = self.span().start;
        self.skip_block();
        let raw = Span {
            start,
            end: self.prev_end(),
        }
        .text(self.src);
        let lines = raw
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(AsmBlock {
            arch,
            operands,
            lines,
        })
    }

    fn plc_section(&mut self) -> PResult<ItemKind> {
        let section = match self.text() {
            "inputs" => PlcSection::Inputs,
            "outputs" => PlcSection::Outputs,
            _ => PlcSection::Memory,
        };
        self.bump();
        let vars = self.members(|p| p.var_decl(true))?;
        Ok(ItemKind::PlcSection { section, vars })
    }

    fn circuit_decl(&mut self) -> PResult<CircuitDecl> {
        self.bump();
        let name = self.ident()?;
        self.expect_p(P::LParen)?;
        let mut ports = Vec::new();
        while !self.at_p(P::RParen) {
            let pstart = self.span().start;
            let pname = self.ident()?;
            self.expect_p(P::Colon)?;
            let dir = if self.eat_kw(Kw::In) {
                PortDir::In
            } else if self.eat_word("out") {
                PortDir::Out
            } else if self.eat_word("inout") {
                PortDir::InOut
            } else {
                self.error_here("a port direction (`in`, `out`, `inout`)");
                return Err(());
            };
            let ty = self.ty()?;
            let pin = if self.eat_p(P::At) {
                Some(self.expr()?)
            } else {
                None
            };
            ports.push(Port {
                name: pname,
                dir,
                ty,
                pin,
                span: self.since(pstart),
            });
            if !self.eat_p(P::Comma) {
                break;
            }
        }
        self.expect_p(P::RParen)?;
        let clock = if self.eat_kw(Kw::On) {
            Some(self.ident()?)
        } else {
            None
        };
        let members = self.members_sp(|p| p.circuit_member())?;
        Ok(CircuitDecl {
            name,
            ports,
            clock,
            members,
        })
    }

    fn circuit_member(&mut self) -> PResult<CircuitMember> {
        if self.eat_kw(Kw::Const) {
            let name = self.ident()?;
            let ty = if self.eat_p(P::Colon) {
                Some(self.ty()?)
            } else {
                None
            };
            self.expect_p(P::Eq)?;
            let value = self.expr()?;
            self.expect_newline()?;
            return Ok(CircuitMember::Const { name, ty, value });
        }
        if self.at_kw(Kw::On) && matches!(self.nth_text(1), "rising" | "falling") {
            self.bump();
            let rising = self.text() == "rising";
            self.bump();
            self.expect_p(P::LParen)?;
            let clock = self.ident()?;
            self.expect_p(P::RParen)?;
            let body = self.colon_block()?;
            return Ok(CircuitMember::OnEdge {
                rising,
                clock,
                body,
            });
        }
        let w = self.text();
        match w {
            "reg" | "wire" if self.nth(1) == K::Ident => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Colon)?;
                let ty = self.ty()?;
                if w == "wire" {
                    self.expect_newline()?;
                    return Ok(CircuitMember::Wire { name, ty });
                }
                let init = if self.eat_p(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.expect_newline()?;
                Ok(CircuitMember::Reg { name, ty, init })
            }
            "instance" => {
                self.bump();
                let name = self.ident()?;
                self.expect_p(P::Eq)?;
                let of = self.expr()?;
                self.expect_newline()?;
                Ok(CircuitMember::Instance { name, of })
            }
            "comb" if self.nth(1) == K::Punct(P::Colon) => {
                self.bump();
                Ok(CircuitMember::Comb(self.colon_block()?))
            }
            "fsm" => Ok(CircuitMember::Fsm(self.fsm_decl()?)),
            _ => {
                self.error_here(
                    "`reg`, `wire`, `const`, `instance`, `comb:`, `on rising(clk):` or `fsm`",
                );
                Err(())
            }
        }
    }

    fn screen_decl(&mut self) -> PResult<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let display = if self.eat_kw(Kw::On) {
            Some(self.ident()?)
        } else {
            None
        };
        let widgets = self.members(|p| p.widget())?;
        Ok(ItemKind::Screen {
            name,
            display,
            widgets,
        })
    }

    fn widget(&mut self) -> PResult<Widget> {
        let start = self.span().start;
        let kind = self.ident()?;
        let name = if self.at(K::Ident) && !STOP_WORDS.contains(&self.text()) {
            Some(self.ident()?)
        } else {
            None
        };
        let text = if matches!(self.kind(), K::Str) {
            Some(self.string_lit()?)
        } else {
            None
        };
        let mut options = Vec::new();
        while self.at(K::Ident) && STOP_WORDS.contains(&self.text()) {
            let opt = self.ident()?;
            let value = self.expr_stop()?;
            options.push((opt, value));
        }
        let mut children = Vec::new();
        let mut events = Vec::new();
        if self.at_p(P::Colon) {
            self.members(|p| {
                if p.at_kw(Kw::On) {
                    p.bump();
                    let ev = p.ident()?;
                    let body = p.colon_block()?;
                    events.push((ev, body));
                } else {
                    children.push(p.widget()?);
                }
                Ok(())
            })?;
        } else {
            self.expect_newline()?;
        }
        Ok(Widget {
            kind,
            name,
            text,
            options,
            children,
            events,
            span: self.since(start),
        })
    }

    fn string_lit(&mut self) -> PResult<String> {
        if matches!(self.kind(), K::Str | K::RawStr) {
            let t = self.bump();
            Ok(t.span.text(self.src).to_string())
        } else {
            self.error_here("a string");
            Err(())
        }
    }

    // ---- types ----------------------------------------------------------

    fn ty(&mut self) -> PResult<Type> {
        let start = self.span().start;
        let kind = if self.eat_p(P::LBracket) {
            let inner = self.ty()?;
            if self.eat_p(P::Semi) {
                let n = self.expr()?;
                self.expect_p(P::RBracket)?;
                TypeKind::Array(Box::new(inner), Box::new(n))
            } else {
                self.expect_p(P::RBracket)?;
                TypeKind::Slice(Box::new(inner))
            }
        } else if self.eat_p(P::LParen) {
            let mut items = vec![self.ty()?];
            while self.eat_p(P::Comma) {
                items.push(self.ty()?);
            }
            self.expect_p(P::RParen)?;
            TypeKind::Tuple(items)
        } else {
            let path = self.path()?;
            let mut args = Vec::new();
            if self.eat_p(P::Lt) {
                loop {
                    let a = if self.at(K::Ident)
                        && !matches!(
                            self.nth(1),
                            K::Punct(P::Plus | P::Minus | P::Star | P::Slash)
                        ) {
                        TypeArg::Type(self.ty()?)
                    } else {
                        TypeArg::Expr(self.add_expr()?)
                    };
                    args.push(a);
                    if !self.eat_p(P::Comma) {
                        break;
                    }
                }
                self.expect_gt()?;
            }
            TypeKind::Named { path, args }
        };
        let unit = if self.unit_follows() {
            self.unit_expr()?
        } else {
            None
        };
        let optional = self.eat_p(P::Question);
        Ok(Type {
            kind,
            unit,
            optional,
            span: self.since(start),
        })
    }

    /// `true` if a unit expression starts here (an identifier or `%` on the same line).
    fn unit_follows(&self) -> bool {
        match self.kind() {
            K::Ident => !STOP_WORDS.contains(&self.text()) && !self.text().starts_with('`'),
            // `%` is a unit unless an operand follows (`7 % 2` is a remainder).
            K::Punct(P::Percent) => !self.starts_operand(1),
            _ => false,
        }
    }

    /// `true` if the token `n` positions ahead can start an operand.
    fn starts_operand(&self, n: usize) -> bool {
        match self.nth(n) {
            K::Int
            | K::Real
            | K::Str
            | K::RawStr
            | K::ByteStr
            | K::Char
            | K::IecAddr
            | K::BitsLit => true,
            K::Ident => !STOP_WORDS.contains(&self.nth_text(n)),
            K::Keyword(
                Kw::True | Kw::False | Kw::NoneLit | Kw::SelfValue | Kw::Not | Kw::Fn | Kw::If,
            ) => true,
            K::Punct(P::LParen | P::LBracket | P::Minus | P::Tilde) => true,
            _ => false,
        }
    }

    fn unit_expr(&mut self) -> PResult<Option<UnitExpr>> {
        let start = self.span().start;
        let mut factors = Vec::new();
        let mut sign = 1;
        loop {
            let name = if self.at_p(P::Percent) {
                let t = self.bump();
                Ident {
                    name: "%".into(),
                    span: t.span,
                }
            } else if self.at(K::Ident) {
                self.unit_ident()?
            } else {
                self.error_here("a unit");
                return Err(());
            };
            let mut exp = 1;
            if self.at_p(P::Caret) && matches!(self.nth(1), K::Int | K::Punct(P::Minus)) {
                self.bump();
                let neg = self.eat_p(P::Minus);
                let t = self.bump();
                let n: i32 = t.span.text(self.src).parse().map_err(|_| ())?;
                exp = if neg { -n } else { n };
            }
            factors.push((name, sign * exp));
            if self.at_p(P::Slash) && matches!(self.nth(1), K::Ident) {
                self.bump();
                sign = -1;
            } else if self.at_p(P::Star) && matches!(self.nth(1), K::Ident) {
                self.bump();
                sign = 1;
            } else {
                break;
            }
        }
        Ok(Some(UnitExpr {
            factors,
            span: self.since(start),
        }))
    }

    // ---- statements -----------------------------------------------------

    /// `:` followed by an indented block or by simple statements on the same line.
    fn colon_block(&mut self) -> PResult<Block> {
        self.colon_block_with(false)
    }

    /// Like `colon_block`; contracts are accepted only when `allow_contracts` is set
    /// (function bodies).
    fn colon_block_with(&mut self, allow_contracts: bool) -> PResult<Block> {
        let start = self.span().start;
        self.expect_p(P::Colon)?;
        if !self.at(K::Newline) {
            let stmts = self.simple_stmts_line()?;
            return Ok(Block {
                contracts: Vec::new(),
                stmts,
                span: self.since(start),
            });
        }
        self.indented_block(start, allow_contracts)
    }

    /// Cursor on NEWLINE: parses the indented block that follows.
    fn indented_block(&mut self, start: usize, allow_contracts: bool) -> PResult<Block> {
        self.bump();
        if !self.at(K::Indent) {
            let s = self.span();
            self.diags.push(
                Diagnostic::error("NE0005", "expected an indented block", s)
                    .with_help("indent the block by 2 spaces"),
            );
            return Err(());
        }
        self.bump();
        let mut contracts = Vec::new();
        let mut stmts = Vec::new();
        while !self.at(K::Dedent) && !self.at(K::Eof) {
            if self.at(K::Newline) {
                self.bump();
                continue;
            }
            let kind = match self.kind() {
                K::Keyword(Kw::Requires) => Some(ContractKind::Requires),
                K::Keyword(Kw::Ensures) => Some(ContractKind::Ensures),
                K::Keyword(Kw::Invariant) => Some(ContractKind::Invariant),
                _ => None,
            };
            if let Some(kind) = kind {
                let cstart = self.span().start;
                self.bump();
                let misplaced =
                    !allow_contracts || !stmts.is_empty() || kind == ContractKind::Invariant;
                if misplaced {
                    let (msg, help) = if kind == ContractKind::Invariant {
                        (
                            "`invariant` is only allowed in a class body",
                            "declare it directly inside the class",
                        )
                    } else {
                        (
                            "contracts are only allowed at the start of a function body",
                            "move `requires` and `ensures` to the first lines of the function",
                        )
                    };
                    let at = Span {
                        start: cstart,
                        end: self.prev_end(),
                    };
                    self.diags
                        .push(Diagnostic::error("NE0012", msg, at).with_help(help));
                }
                let parsed = self.expr();
                let cspan = self.since(cstart);
                match parsed.and_then(|cond| self.expect_newline().map(|()| cond)) {
                    Ok(cond) => {
                        if !misplaced {
                            contracts.push(Contract {
                                kind,
                                cond,
                                span: cspan,
                            });
                        }
                    }
                    Err(()) => self.recover_line(),
                }
                continue;
            }
            match self.stmt_line() {
                Ok(mut s) => stmts.append(&mut s),
                Err(()) => self.recover_line(),
            }
        }
        self.eat_dedent();
        Ok(Block {
            contracts,
            stmts,
            span: self.since(start),
        })
    }

    /// One logical line: a compound statement or `;`-separated simple statements.
    fn stmt_line(&mut self) -> PResult<Vec<Stmt>> {
        if let Some(s) = self.compound_stmt()? {
            return Ok(vec![s]);
        }
        self.simple_stmts_line()
    }

    fn simple_stmts_line(&mut self) -> PResult<Vec<Stmt>> {
        let mut out = vec![self.simple_stmt()?];
        while self.eat_p(P::Semi) {
            if self.at(K::Newline) {
                break;
            }
            out.push(self.simple_stmt()?);
        }
        self.expect_newline()?;
        Ok(out)
    }

    fn compound_stmt(&mut self) -> PResult<Option<Stmt>> {
        let start = self.span().start;
        let kind = match self.kind() {
            K::Keyword(Kw::If) => {
                self.bump();
                let mut branches = Vec::new();
                let c = self.expr()?;
                branches.push((c, self.colon_block()?));
                let mut otherwise = None;
                loop {
                    if self.eat_kw(Kw::Elif) {
                        let c = self.expr()?;
                        branches.push((c, self.colon_block()?));
                    } else if self.eat_kw(Kw::Else) {
                        otherwise = Some(self.colon_block()?);
                        break;
                    } else {
                        break;
                    }
                }
                StmtKind::If {
                    branches,
                    otherwise,
                }
            }
            K::Keyword(Kw::Match) => {
                self.bump();
                let value = self.expr()?;
                let arms = self.members(|p| {
                    let s = p.span().start;
                    let pattern = p.pattern()?;
                    let guard = if p.eat_kw(Kw::If) {
                        Some(p.expr()?)
                    } else {
                        None
                    };
                    p.expect_p(P::FatArrow)?;
                    let body = if p.at(K::Newline) {
                        let bs = p.span().start;
                        p.indented_block(bs, false)?
                    } else {
                        let bs = p.span().start;
                        let stmts = p.simple_stmts_line()?;
                        Block {
                            contracts: Vec::new(),
                            stmts,
                            span: p.since(bs),
                        }
                    };
                    Ok(Arm {
                        pattern,
                        guard,
                        body,
                        span: p.since(s),
                    })
                })?;
                StmtKind::Match { value, arms }
            }
            K::Keyword(Kw::For) => {
                self.bump();
                let pattern = self.pattern()?;
                if !self.eat_kw(Kw::In) {
                    self.error_here("`in`");
                    return Err(());
                }
                let iter = self.expr()?;
                StmtKind::For {
                    pattern,
                    iter,
                    body: self.colon_block()?,
                }
            }
            K::Keyword(Kw::While) => {
                self.bump();
                let pattern = if self.eat_kw(Kw::Let) {
                    let p = self.pattern()?;
                    self.expect_p(P::Eq)?;
                    Some(p)
                } else {
                    None
                };
                let cond = self.expr_stop()?;
                let max = if self.eat_word("max") {
                    Some(self.expr()?)
                } else {
                    None
                };
                StmtKind::While {
                    pattern,
                    cond,
                    max,
                    body: self.colon_block()?,
                }
            }
            K::Keyword(Kw::Loop) => {
                self.bump();
                let max = if self.eat_word("max") {
                    Some(self.expr()?)
                } else {
                    None
                };
                StmtKind::Loop {
                    max,
                    body: self.colon_block()?,
                }
            }
            K::Keyword(Kw::Atomic) => {
                self.bump();
                StmtKind::Atomic(self.colon_block()?)
            }
            K::Keyword(Kw::When) => StmtKind::Rule(self.rule()?),
            K::Keyword(Kw::Trusted) if self.nth_text(1) == "asm" => {
                self.bump();
                StmtKind::Asm(self.asm_block()?)
            }
            _ => return Ok(None),
        };
        Ok(Some(Stmt {
            kind,
            span: self.since(start),
        }))
    }

    fn simple_stmt(&mut self) -> PResult<Stmt> {
        let start = self.span().start;
        let kind = match self.kind() {
            K::Keyword(Kw::Let | Kw::Var) => {
                let mutable = self.bump().kind == K::Keyword(Kw::Var);
                let pattern = self.pattern()?;
                let ty = if self.eat_p(P::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                self.expect_p(P::Eq)?;
                let value = self.expr()?;
                StmtKind::Let {
                    mutable,
                    pattern,
                    ty,
                    value,
                }
            }
            K::Keyword(Kw::Pass) => {
                self.bump();
                StmtKind::Pass
            }
            K::Keyword(Kw::Return) => {
                self.bump();
                let v = if self.at(K::Newline) || self.at_p(P::Semi) || self.at(K::Eof) {
                    None
                } else {
                    Some(self.expr()?)
                };
                StmtKind::Return(v)
            }
            K::Keyword(Kw::Break) => {
                self.bump();
                StmtKind::Break
            }
            K::Keyword(Kw::Continue) => {
                self.bump();
                StmtKind::Continue
            }
            K::Keyword(Kw::Fault) => {
                self.bump();
                let name = self.path()?;
                let args = if self.at_p(P::LParen) {
                    self.args()?
                } else {
                    Vec::new()
                };
                StmtKind::Fault { name, args }
            }
            K::Keyword(Kw::Emit) => {
                self.bump();
                let event = self.path()?;
                let args = if self.at_p(P::LParen) {
                    self.args()?
                } else {
                    Vec::new()
                };
                StmtKind::Emit { event, args }
            }
            K::Keyword(Kw::Assert) => {
                self.bump();
                let cond = self.expr()?;
                let msg = if self.eat_p(P::Comma) {
                    Some(self.string_lit()?)
                } else {
                    None
                };
                StmtKind::Assert { cond, msg }
            }
            K::Punct(P::Arrow) => {
                self.bump();
                StmtKind::Jump(self.ident()?)
            }
            K::Ident
                if matches!(self.text(), "set" | "reset") && matches!(self.nth(1), K::Ident) =>
            {
                let set = self.text() == "set";
                self.bump();
                let place = self.expr_stop_when()?;
                if !self.eat_kw(Kw::When) {
                    self.error_here("`when`");
                    return Err(());
                }
                let when = self.expr()?;
                StmtKind::Coil { set, place, when }
            }
            K::Ident
                if self.text() == "wait"
                    && !matches!(self.nth(1), K::Punct(P::LParen | P::Dot | P::Eq)) =>
            {
                self.bump();
                StmtKind::Wait(self.expr()?)
            }
            K::Ident if self.text() == "until" => {
                self.bump();
                let cond = self.expr_stop()?;
                let mut timeout = None;
                let mut on_timeout = None;
                if self.at_kw(Kw::Or) && self.nth_text(1) == "timeout" {
                    self.bump();
                }
                if self.eat_word("timeout") {
                    timeout = Some(self.expr_until_arrow()?);
                    if self.eat_p(P::Arrow) {
                        on_timeout = Some(self.ident()?);
                    }
                }
                StmtKind::Until {
                    cond,
                    timeout,
                    on_timeout,
                }
            }
            K::Ident
                if self.text() == "move"
                    && matches!(self.nth_text(1), "ptp" | "linear" | "arc") =>
            {
                self.bump();
                let kind = match self.bump().span.text(self.src) {
                    "ptp" => MoveKind::Ptp,
                    "linear" => MoveKind::Linear,
                    _ => MoveKind::Arc,
                };
                self.expect_word("to")?;
                let to = self.expr_stop()?;
                let via = if self.eat_word("via") {
                    Some(self.expr_stop()?)
                } else {
                    None
                };
                let mut options = Vec::new();
                while self.at(K::Ident)
                    && matches!(self.text(), "speed" | "accel" | "blend" | "tool" | "frame")
                {
                    let o = self.ident()?;
                    options.push((o, self.expr_stop()?));
                }
                StmtKind::Move(Move {
                    kind,
                    to,
                    via,
                    options,
                })
            }
            _ => {
                let lhs = self.expr_stop_when()?;
                let op = match self.kind() {
                    K::Punct(P::Eq) => Some(AssignOp::Set),
                    K::Punct(P::PlusEq) => Some(AssignOp::Compound(BinOp::Add)),
                    K::Punct(P::MinusEq) => Some(AssignOp::Compound(BinOp::Sub)),
                    K::Punct(P::StarEq) => Some(AssignOp::Compound(BinOp::Mul)),
                    K::Punct(P::SlashEq) => Some(AssignOp::Compound(BinOp::Div)),
                    K::Punct(P::PercentEq) => Some(AssignOp::Compound(BinOp::Rem)),
                    K::Punct(P::AmpEq) => Some(AssignOp::Compound(BinOp::BitAnd)),
                    K::Punct(P::BarEq) => Some(AssignOp::Compound(BinOp::BitOr)),
                    K::Punct(P::CaretEq) => Some(AssignOp::Compound(BinOp::BitXor)),
                    K::Punct(P::ShlEq) => Some(AssignOp::Compound(BinOp::Shl)),
                    K::Punct(P::ShrEq) => Some(AssignOp::Compound(BinOp::Shr)),
                    _ => None,
                };
                match op {
                    Some(op) => {
                        self.bump();
                        let value = self.expr_stop_when()?;
                        let when = if self.eat_kw(Kw::When) {
                            Some(self.expr()?)
                        } else {
                            None
                        };
                        StmtKind::Assign {
                            place: lhs,
                            op,
                            value,
                            when,
                        }
                    }
                    None => StmtKind::Expr(lhs),
                }
            }
        };
        Ok(Stmt {
            kind,
            span: self.since(start),
        })
    }

    // ---- patterns -------------------------------------------------------

    fn pattern(&mut self) -> PResult<Pattern> {
        let start = self.span().start;
        let first = self.pattern_atom()?;
        if !self.at_p(P::Bar) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat_p(P::Bar) {
            alts.push(self.pattern_atom()?);
        }
        Ok(Pattern {
            kind: PatternKind::Or(alts),
            span: self.since(start),
        })
    }

    fn pattern_atom(&mut self) -> PResult<Pattern> {
        let start = self.span().start;
        let kind = match self.kind() {
            K::Ident if self.text() == "_" => {
                self.bump();
                PatternKind::Wildcard
            }
            K::Punct(P::LParen) => {
                self.bump();
                let mut items = vec![self.pattern()?];
                while self.eat_p(P::Comma) {
                    items.push(self.pattern()?);
                }
                self.expect_p(P::RParen)?;
                PatternKind::Tuple(items)
            }
            K::Ident => {
                let path = self.path()?;
                if self.eat_p(P::LParen) {
                    let mut args = Vec::new();
                    let mut fields = Vec::new();
                    while !self.at_p(P::RParen) {
                        if self.at(K::Ident) && self.nth(1) == K::Punct(P::Colon) {
                            let f = self.ident()?;
                            self.bump();
                            fields.push((f, self.pattern()?));
                        } else {
                            args.push(self.pattern()?);
                        }
                        if !self.eat_p(P::Comma) {
                            break;
                        }
                    }
                    self.expect_p(P::RParen)?;
                    PatternKind::Constructor { path, args, fields }
                } else if path.segments.len() == 1 {
                    PatternKind::Bind(path.segments[0].clone())
                } else {
                    PatternKind::Path(path)
                }
            }
            _ => {
                let lo = self.unary_expr()?;
                if self.at_p(P::DotDot) || self.at_p(P::DotDotEq) {
                    let inclusive = self.bump().kind == K::Punct(P::DotDotEq);
                    let hi = self.unary_expr()?;
                    PatternKind::Range { lo, hi, inclusive }
                } else {
                    PatternKind::Lit(lo)
                }
            }
        };
        Ok(Pattern {
            kind,
            span: self.since(start),
        })
    }

    // ---- expressions ----------------------------------------------------

    /// Full expression (stop words are still respected so headers parse cleanly).
    fn expr(&mut self) -> PResult<Expr> {
        self.pipe_expr()
    }

    /// Expression in a header position: stops before contextual words.
    fn expr_stop(&mut self) -> PResult<Expr> {
        self.pipe_expr()
    }

    /// Expression that must not consume `when` (assignments, coils).
    fn expr_stop_when(&mut self) -> PResult<Expr> {
        self.pipe_expr()
    }

    /// Expression that stops before `->` (links, sync, timeouts); `->` is not an
    /// expression operator outside call arguments, so a plain expression stops there.
    fn expr_until_arrow(&mut self) -> PResult<Expr> {
        self.pipe_expr()
    }

    fn pipe_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let mut lhs = self.cond_expr()?;
        while self.eat_p(P::Pipeline) {
            let rhs = self.postfix_expr()?;
            lhs = Expr::new(
                ExprKind::Pipe(Box::new(lhs), Box::new(rhs)),
                self.since(start),
            );
        }
        Ok(lhs)
    }

    fn cond_expr(&mut self) -> PResult<Expr> {
        if self.at_kw(Kw::If) {
            let start = self.span().start;
            self.bump();
            let c = self.expr()?;
            if !self.eat_kw(Kw::Then) {
                self.error_here("`then`");
                return Err(());
            }
            let a = self.expr()?;
            if !self.eat_kw(Kw::Else) {
                self.error_here("`else`");
                return Err(());
            }
            let b = self.expr()?;
            return Ok(Expr::new(
                ExprKind::IfElse(Box::new(c), Box::new(a), Box::new(b)),
                self.since(start),
            ));
        }
        self.coalesce_expr()
    }

    fn coalesce_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let lhs = self.or_expr()?;
        if self.eat_p(P::Coalesce) {
            let rhs = self.coalesce_expr()?;
            return Ok(Expr::new(
                ExprKind::Binary(BinOp::Coalesce, Box::new(lhs), Box::new(rhs)),
                self.since(start),
            ));
        }
        Ok(lhs)
    }

    fn or_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let mut lhs = self.and_expr()?;
        // `or after` / `or timeout` belong to the enclosing transition or `until`
        while self.at_kw(Kw::Or)
            && !(self.nth(1) == K::Ident && STOP_WORDS.contains(&self.nth_text(1)))
        {
            self.bump();
            let rhs = self.and_expr()?;
            lhs = Expr::new(
                ExprKind::Binary(BinOp::Or, Box::new(lhs), Box::new(rhs)),
                self.since(start),
            );
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let mut lhs = self.not_expr()?;
        while self.eat_kw(Kw::And) {
            let rhs = self.not_expr()?;
            lhs = Expr::new(
                ExprKind::Binary(BinOp::And, Box::new(lhs), Box::new(rhs)),
                self.since(start),
            );
        }
        Ok(lhs)
    }

    fn not_expr(&mut self) -> PResult<Expr> {
        if self.at_kw(Kw::Not) {
            let start = self.span().start;
            self.bump();
            let e = self.not_expr()?;
            return Ok(Expr::new(
                ExprKind::Unary(UnOp::Not, Box::new(e)),
                self.since(start),
            ));
        }
        self.cmp_expr()
    }

    fn cmp_op(&self) -> Option<BinOp> {
        Some(match self.kind() {
            K::Punct(P::EqEq) => BinOp::Eq,
            K::Punct(P::NotEq) => BinOp::Ne,
            K::Punct(P::Lt) => BinOp::Lt,
            K::Punct(P::Le) => BinOp::Le,
            K::Punct(P::Gt) => BinOp::Gt,
            K::Punct(P::Ge) => BinOp::Ge,
            K::Keyword(Kw::In) => BinOp::In,
            _ => return None,
        })
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let first = self.range_expr()?;
        let mut rest = Vec::new();
        while let Some(op) = self.cmp_op() {
            self.bump();
            rest.push((op, self.range_expr()?));
        }
        if rest.is_empty() {
            return Ok(first);
        }
        if rest.len() == 1 {
            let (op, rhs) = rest.remove(0);
            return Ok(Expr::new(
                ExprKind::Binary(op, Box::new(first), Box::new(rhs)),
                self.since(start),
            ));
        }
        Ok(Expr::new(
            ExprKind::Compare(Box::new(first), rest),
            self.since(start),
        ))
    }

    fn range_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let lo = self.bor_expr()?;
        if self.at_p(P::DotDot) || self.at_p(P::DotDotEq) {
            let inclusive = self.bump().kind == K::Punct(P::DotDotEq);
            let hi = self.bor_expr()?;
            // A unit written after the upper bound applies to both bounds:
            // `0..120 degC` means `0 degC..120 degC` (spec 03, ranges).
            let lo = share_unit(lo, &hi);
            return Ok(Expr::new(
                ExprKind::Range {
                    lo: Box::new(lo),
                    hi: Box::new(hi),
                    inclusive,
                },
                self.since(start),
            ));
        }
        Ok(lo)
    }

    fn binary_level(
        &mut self,
        next: fn(&mut Self) -> PResult<Expr>,
        ops: &[(P, BinOp)],
    ) -> PResult<Expr> {
        let start = self.span().start;
        let mut lhs = next(self)?;
        'outer: loop {
            for (p, op) in ops {
                if self.at_p(*p) {
                    self.bump();
                    let rhs = next(self)?;
                    lhs = Expr::new(
                        ExprKind::Binary(*op, Box::new(lhs), Box::new(rhs)),
                        self.since(start),
                    );
                    continue 'outer;
                }
            }
            return Ok(lhs);
        }
    }

    fn bor_expr(&mut self) -> PResult<Expr> {
        self.binary_level(Self::bxor_expr, &[(P::Bar, BinOp::BitOr)])
    }

    fn bxor_expr(&mut self) -> PResult<Expr> {
        self.binary_level(Self::band_expr, &[(P::Caret, BinOp::BitXor)])
    }

    fn band_expr(&mut self) -> PResult<Expr> {
        self.binary_level(Self::shift_expr, &[(P::Amp, BinOp::BitAnd)])
    }

    fn shift_expr(&mut self) -> PResult<Expr> {
        self.binary_level(
            Self::add_expr,
            &[(P::Shl, BinOp::Shl), (P::Shr, BinOp::Shr)],
        )
    }

    fn add_expr(&mut self) -> PResult<Expr> {
        self.binary_level(
            Self::mul_expr,
            &[
                (P::Plus, BinOp::Add),
                (P::Minus, BinOp::Sub),
                (P::PlusWrap, BinOp::AddWrap),
                (P::MinusWrap, BinOp::SubWrap),
                (P::PlusSat, BinOp::AddSat),
                (P::MinusSat, BinOp::SubSat),
                (P::PlusPlus, BinOp::Concat),
            ],
        )
    }

    fn mul_expr(&mut self) -> PResult<Expr> {
        self.binary_level(
            Self::unary_expr,
            &[
                (P::Star, BinOp::Mul),
                (P::Slash, BinOp::Div),
                (P::Percent, BinOp::Rem),
                (P::StarWrap, BinOp::MulWrap),
                (P::StarSat, BinOp::MulSat),
            ],
        )
    }

    fn unary_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let op = match self.kind() {
            K::Punct(P::Minus) => Some(UnOp::Neg),
            K::Punct(P::Tilde) => Some(UnOp::BitNot),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary_expr()?;
            return Ok(Expr::new(
                ExprKind::Unary(op, Box::new(e)),
                self.since(start),
            ));
        }
        let base = self.cast_expr()?;
        if self.eat_p(P::StarStar) {
            let exp = self.unary_expr()?;
            return Ok(Expr::new(
                ExprKind::Binary(BinOp::Pow, Box::new(base), Box::new(exp)),
                self.since(start),
            ));
        }
        Ok(base)
    }

    fn cast_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let mut e = self.quantity_expr()?;
        while self.eat_kw(Kw::As) {
            let t = self.ty()?;
            e = Expr::new(ExprKind::Cast(Box::new(e), t), self.since(start));
        }
        Ok(e)
    }

    /// A postfix expression optionally followed by a unit (`5 ms`, `xyz(1,2,3) mm`).
    fn quantity_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let numeric_start = matches!(self.kind(), K::Int | K::Real);
        let e = self.postfix_expr()?;
        let unit_ok = numeric_start
            || matches!(
                &e.kind,
                ExprKind::Call(..) | ExprKind::Tuple(..) | ExprKind::Range { .. }
            );
        if unit_ok && self.unit_follows() {
            let from = self.prev_end();
            let gap = self.src.get(from..self.span().start).unwrap_or("");
            if !gap.is_empty() && gap.chars().all(|c| c == ' ' || c == '\t') {
                if gap != " " {
                    let at = Span {
                        start: from,
                        end: self.span().start,
                    };
                    self.diags.push(
                        Diagnostic::error(
                            "NE0014",
                            "a number and its unit are separated by exactly one space",
                            at,
                        )
                        .with_help("write one space between the number and the unit: `5 ms`"),
                    );
                }
                let u = self.unit_expr()?.ok_or(())?;
                return Ok(Expr::new(
                    ExprKind::Quantity(Box::new(e), u),
                    self.since(start),
                ));
            }
        }
        Ok(e)
    }

    fn postfix_expr(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let mut e = self.primary()?;
        loop {
            // calls and indexing require `(`/`[` directly after the callee:
            // `f(x)` is a call, `f (x)` is two expressions (e.g. a task header).
            let adjacent = self.prev_end() == self.span().start;
            match self.kind() {
                K::Punct(P::LParen | P::LBracket) if !adjacent => return Ok(e),
                K::Punct(P::LParen) => {
                    let args = self.args()?;
                    e = Expr::new(ExprKind::Call(Box::new(e), args), self.since(start));
                }
                K::Punct(P::LBracket) => {
                    self.bump();
                    let i = self.expr()?;
                    let hi = if self.eat_p(P::Colon) {
                        Some(Box::new(self.expr()?))
                    } else {
                        None
                    };
                    self.expect_p(P::RBracket)?;
                    e = Expr::new(
                        ExprKind::Index(Box::new(e), Box::new(i), hi),
                        self.since(start),
                    );
                }
                K::Punct(P::Dot) if !matches!(self.nth(1), K::Punct(_)) => {
                    self.bump();
                    let f = self.member_name()?;
                    e = Expr::new(ExprKind::Field(Box::new(e), f), self.since(start));
                }
                K::Punct(P::Question) => {
                    self.bump();
                    e = Expr::new(ExprKind::Try(Box::new(e)), self.since(start));
                }
                _ => return Ok(e),
            }
        }
    }

    fn args(&mut self) -> PResult<Vec<Arg>> {
        self.expect_p(P::LParen)?;
        let mut out = Vec::new();
        while !self.at_p(P::RParen) {
            let astart = self.span().start;
            let mutable = self.eat_kw(Kw::Mut);
            let name = if matches!(self.kind(), K::Ident | K::Keyword(_))
                && self.nth(1) == K::Punct(P::Colon)
            {
                let n = self.member_name()?;
                self.bump();
                Some(n)
            } else {
                None
            };
            let start = self.span().start;
            let mut value = self.expr()?;
            if self.eat_p(P::Arrow) {
                let to = self.expr()?;
                value = Expr::new(
                    ExprKind::Map(Box::new(value), Box::new(to)),
                    self.since(start),
                );
            }
            out.push(Arg {
                mutable,
                name,
                value,
                span: self.since(astart),
            });
            if !self.eat_p(P::Comma) {
                break;
            }
        }
        self.expect_p(P::RParen)?;
        Ok(out)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span().start;
        let t = self.tok();
        let text = t.span.text(self.src).to_string();
        let lit = |l| Ok(Expr::new(ExprKind::Lit(l), t.span));
        match t.kind {
            K::Int => {
                self.bump();
                lit(Lit::Int(text))
            }
            K::Real => {
                self.bump();
                lit(Lit::Real(text))
            }
            K::Str => {
                self.bump();
                lit(Lit::Str(text))
            }
            K::RawStr => {
                self.bump();
                lit(Lit::RawStr(text))
            }
            K::ByteStr => {
                self.bump();
                lit(Lit::ByteStr(text))
            }
            K::Char => {
                self.bump();
                lit(Lit::Char(text))
            }
            K::IecAddr => {
                self.bump();
                lit(Lit::Iec(text))
            }
            K::BitsLit => {
                self.bump();
                lit(Lit::Bits(text))
            }
            K::Keyword(Kw::True) => {
                self.bump();
                lit(Lit::Bool(true))
            }
            K::Keyword(Kw::False) => {
                self.bump();
                lit(Lit::Bool(false))
            }
            K::Keyword(Kw::NoneLit) => {
                self.bump();
                lit(Lit::None)
            }
            K::Keyword(Kw::SelfValue) => {
                self.bump();
                Ok(Expr::new(ExprKind::SelfValue, t.span))
            }
            K::Keyword(Kw::Fn) => {
                self.bump();
                self.expect_p(P::LParen)?;
                let mut params = Vec::new();
                while !self.at_p(P::RParen) {
                    params.push(self.ident()?);
                    if !self.eat_p(P::Comma) {
                        break;
                    }
                }
                self.expect_p(P::RParen)?;
                self.expect_p(P::FatArrow)?;
                let body = self.expr()?;
                Ok(Expr::new(
                    ExprKind::Lambda(params, Box::new(body)),
                    self.since(start),
                ))
            }
            K::Punct(P::LParen) => {
                self.bump();
                if self.eat_p(P::RParen) {
                    return Ok(Expr::new(ExprKind::Tuple(Vec::new()), self.since(start)));
                }
                let first = self.expr()?;
                if self.eat_p(P::RParen) {
                    return Ok(first);
                }
                let mut items = vec![first];
                while self.eat_p(P::Comma) {
                    if self.at_p(P::RParen) {
                        break;
                    }
                    items.push(self.expr()?);
                }
                self.expect_p(P::RParen)?;
                Ok(Expr::new(ExprKind::Tuple(items), self.since(start)))
            }
            K::Punct(P::LBracket) => {
                self.bump();
                let mut items = Vec::new();
                if !self.at_p(P::RBracket) {
                    let first = self.expr()?;
                    if self.eat_p(P::Semi) {
                        let n = self.expr()?;
                        self.expect_p(P::RBracket)?;
                        return Ok(Expr::new(
                            ExprKind::ArrayRepeat(Box::new(first), Box::new(n)),
                            self.since(start),
                        ));
                    }
                    items.push(first);
                    while self.eat_p(P::Comma) {
                        if self.at_p(P::RBracket) {
                            break;
                        }
                        items.push(self.expr()?);
                    }
                }
                self.expect_p(P::RBracket)?;
                Ok(Expr::new(ExprKind::Array(items), self.since(start)))
            }
            K::Ident => match text.as_str() {
                "result" => {
                    self.bump();
                    Ok(Expr::new(ExprKind::Result, t.span))
                }
                "old" if self.nth(1) == K::Punct(P::LParen) => {
                    self.bump();
                    self.bump();
                    let e = self.expr()?;
                    self.expect_p(P::RParen)?;
                    Ok(Expr::new(ExprKind::Old(Box::new(e)), self.since(start)))
                }
                "run"
                    if !matches!(
                        self.nth(1),
                        K::Punct(P::LParen | P::Dot | P::Eq) | K::Newline
                    ) =>
                {
                    self.bump();
                    let e = self.postfix_expr()?;
                    Ok(Expr::new(ExprKind::Run(Box::new(e)), self.since(start)))
                }
                _ => {
                    let id = self.ident()?;
                    Ok(Expr::new(ExprKind::Name(id), t.span))
                }
            },
            _ => {
                self.error_here("an expression");
                Err(())
            }
        }
    }
}
