//! Lexical units of the Neuron language (spec/language/02-lexical.md).

use crate::span::Span;

macro_rules! keywords {
    ($($variant:ident => $text:literal),* $(,)?) => {
        /// Keywords reserved in every dialect (spec 02 §2.5).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Keyword {
            $(#[doc = concat!("`", $text, "`")] $variant,)*
        }

        impl Keyword {
            /// Every reserved keyword, in declaration order.
            pub const ALL: &'static [Keyword] = &[$(Keyword::$variant),*];

            /// Source text of the keyword.
            pub fn as_str(self) -> &'static str {
                match self { $(Keyword::$variant => $text,)* }
            }

            /// Looks up a reserved keyword by its exact text.
            ///
            /// ```
            /// use neuron_syntax::token::Keyword;
            /// assert_eq!(Keyword::lookup("task"), Some(Keyword::Task));
            /// assert_eq!(Keyword::lookup("step"), None); // contextual, not reserved
            /// ```
            pub fn lookup(text: &str) -> Option<Keyword> {
                match text { $($text => Some(Keyword::$variant),)* _ => None }
            }
        }
    };
}

keywords! {
    And => "and", As => "as", Assert => "assert", Atomic => "atomic",
    Break => "break", Cell => "cell", Class => "class", Const => "const",
    Continue => "continue", Elif => "elif", Else => "else", Emit => "emit",
    Enum => "enum", Extern => "extern", Fault => "fault", Fn => "fn",
    For => "for", If => "if", In => "in",
    Interface => "interface", Io => "io", Is => "is", Let => "let",
    Loop => "loop", Match => "match", Module => "module",
    Mut => "mut", Not => "not", On => "on", Or => "or",
    Pass => "pass", Pub => "pub", Requires => "requires", Ensures => "ensures",
    Invariant => "invariant", Return => "return", Safety => "safety",
    SelfValue => "self", Signal => "signal",
    Struct => "struct", Task => "task", Test => "test", Then => "then",
    Trusted => "trusted", Type => "type", Use => "use", Var => "var",
    Verify => "verify", When => "when", While => "while", True => "true",
    False => "false", NoneLit => "none",
}

/// Words that are keywords only in some dialects or constructs (spec 02 §2.5).
/// The lexer emits them as identifiers; the parser interprets them in context.
pub const CONTEXTUAL_KEYWORDS: &[&str] = &[
    "max",
    "result",
    "old",
    "every",
    "offset",
    "prio",
    "deadline",
    "core",
    "reads",
    "writes",
    "stack",
    "interrupt",
    "after",
    "fsm",
    "state",
    "initial",
    "final",
    "entry",
    "exit",
    "do",
    "sequence",
    "step",
    "parallel",
    "branch",
    "run",
    "until",
    "timeout",
    "wait",
    "safe_state",
    "watchdog",
    "rate",
    "reset",
    "never",
    "always",
    "implies",
    "eventually_within",
    "unit",
    "clocks",
    "from",
    "regs",
    "uses",
    "asm",
    "scan",
    "inputs",
    "outputs",
    "memory",
    "retain",
    "rung",
    "set",
    "clock",
    "circuit",
    "reg",
    "wire",
    "comb",
    "rising",
    "falling",
    "sync",
    "via",
    "instance",
    "out",
    "inout",
    "frame",
    "tool",
    "point",
    "move",
    "ptp",
    "linear",
    "arc",
    "to",
    "speed",
    "accel",
    "blend",
    "display",
    "screen",
    "bind",
    "at",
    "size",
    "row",
    "column",
    "grid",
    "theme",
    "device",
    "link",
    "within",
    "zone",
    "mutex",
    "between",
    "place",
    "event",
    "tag",
];

macro_rules! puncts {
    ($($variant:ident => $text:literal),* $(,)?) => {
        /// Operators and punctuation (spec 02 §2.9).
        ///
        /// Declaration order is the lexer's matching order (longest first).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Punct {
            $(#[doc = concat!("`", $text, "`")] $variant,)*
        }

        impl Punct {
            /// Every punctuation mark, longest first.
            pub const ALL: &'static [Punct] = &[$(Punct::$variant),*];

            /// Source text.
            pub fn as_str(self) -> &'static str {
                match self { $(Punct::$variant => $text,)* }
            }
        }
    };
}

puncts! {
    ShlEq => "<<=", ShrEq => ">>=", DotDotEq => "..=",
    StarStar => "**", PlusWrap => "+%", MinusWrap => "-%", StarWrap => "*%",
    PlusSat => "+|", MinusSat => "-|", StarSat => "*|", Pipeline => "|>",
    Arrow => "->", FatArrow => "=>", EqEq => "==", NotEq => "!=", Le => "<=",
    Ge => ">=", Shl => "<<", Shr => ">>", DotDot => "..", PlusPlus => "++",
    Coalesce => "??", PlusEq => "+=", MinusEq => "-=", StarEq => "*=",
    SlashEq => "/=", PercentEq => "%=", AmpEq => "&=", BarEq => "|=", CaretEq => "^=",
    Plus => "+", Minus => "-", Star => "*", Slash => "/", Percent => "%",
    Amp => "&", Bar => "|", Caret => "^", Tilde => "~", Lt => "<", Gt => ">",
    Eq => "=", Question => "?", At => "@", Colon => ":", Comma => ",",
    Semi => ";", Dot => ".", LParen => "(", RParen => ")", LBracket => "[",
    RBracket => "]", LBrace => "{", RBrace => "}",
}

impl Punct {
    /// `true` for `(`, `[` and `{`.
    pub fn is_open(self) -> bool {
        matches!(self, Punct::LParen | Punct::LBracket | Punct::LBrace)
    }

    /// `true` for `)`, `]` and `}`.
    pub fn is_close(self) -> bool {
        matches!(self, Punct::RParen | Punct::RBracket | Punct::RBrace)
    }
}

/// Kind of a lexical unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Spaces, tabs and newlines that are not significant.
    Whitespace,
    /// `# ...`
    LineComment,
    /// `## ...` documentation comment.
    DocComment,
    /// `#[ ... ]#` (nestable).
    BlockComment,
    /// End of a logical line.
    Newline,
    /// Start of an indented block (zero width).
    Indent,
    /// End of an indented block (zero width).
    Dedent,
    /// End of file (zero width).
    Eof,
    /// Identifier, including contextual keywords and escaped names.
    Ident,
    /// Reserved keyword.
    Keyword(Keyword),
    /// Integer literal (`42`, `0x2A`, `0b1010`, `0o755`).
    Int,
    /// Real literal (`3.14`, `1e-3`).
    Real,
    /// String literal.
    Str,
    /// Raw string `r"..."`.
    RawStr,
    /// Byte string `b"..."`.
    ByteStr,
    /// Character literal `'A'`.
    Char,
    /// IEC 61131-3 direct address (`%I0.0`, `%MD10`, `%DB1.DBX0.0`).
    IecAddr,
    /// Sized bit-vector literal (`8'b1010_0000`, `16'hBEEF`).
    BitsLit,
    /// Operator or punctuation.
    Punct(Punct),
    /// Invalid input (a diagnostic was reported).
    Error,
}

impl TokenKind {
    /// `true` for kinds the parser skips (whitespace and comments).
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace
                | TokenKind::LineComment
                | TokenKind::DocComment
                | TokenKind::BlockComment
        )
    }

    /// `true` for layout kinds (newline, indent, dedent, end of file).
    pub fn is_layout(self) -> bool {
        matches!(
            self,
            TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
        )
    }

    /// Human-readable description used in diagnostics.
    pub fn describe(self) -> String {
        match self {
            TokenKind::Whitespace => "whitespace".into(),
            TokenKind::LineComment => "comment".into(),
            TokenKind::DocComment => "doc comment".into(),
            TokenKind::BlockComment => "block comment".into(),
            TokenKind::Newline => "end of line".into(),
            TokenKind::Indent => "indentation".into(),
            TokenKind::Dedent => "end of block".into(),
            TokenKind::Eof => "end of file".into(),
            TokenKind::Ident => "identifier".into(),
            TokenKind::Keyword(k) => format!("keyword `{}`", k.as_str()),
            TokenKind::Int => "integer".into(),
            TokenKind::Real => "real number".into(),
            TokenKind::Str | TokenKind::RawStr => "string".into(),
            TokenKind::ByteStr => "byte string".into(),
            TokenKind::Char => "character".into(),
            TokenKind::IecAddr => "IEC address".into(),
            TokenKind::BitsLit => "bit-vector literal".into(),
            TokenKind::Punct(p) => format!("`{}`", p.as_str()),
            TokenKind::Error => "invalid input".into(),
        }
    }
}

/// A lexical unit with its source span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    /// Kind.
    pub kind: TokenKind,
    /// Byte range in the source.
    pub span: Span,
}

impl Token {
    /// Source text.
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        self.span.text(src)
    }
}
