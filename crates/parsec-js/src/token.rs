// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Tokens: a kind, a few flags and a span. Nothing in one points into the
//! heap; what a token says is in the source, at its span.

use parsec_base::Span;

/// One token: what it is, what the lexer noticed about it, and where it
/// is. 12 bytes, with no allocation behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: Kind,
    pub flags: Flags,
    pub span: Span,
}

impl Token {
    /// The token's text in `src`, the source it came from.
    pub fn text(self, src: &str) -> &str {
        self.span.text(src)
    }

    /// Whether a line terminator, or a comment holding one, came between
    /// this token and the last one that wasn't a comment. Automatic
    /// semicolon insertion and restricted productions like `return`
    /// ⟨newline⟩ turn on it.
    pub fn newline_before(self) -> bool {
        self.flags.contains(Flags::NEWLINE_BEFORE)
    }

    pub fn is_trivia(self) -> bool {
        self.kind.is_trivia()
    }
}

/// What the lexer noticed about a token that its kind doesn't say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u8);

impl Flags {
    /// A line terminator came before the token; see [`Token::newline_before`].
    pub const NEWLINE_BEFORE: Flags = Flags(1);
    /// An identifier, string or template part with a backslash escape in
    /// it, so its value isn't its text. An escaped identifier is never a
    /// [`Kind::Keyword`]: `\u0076ar` is [`Kind::Ident`] with this flag,
    /// which the parser has to reject where `var` would be reserved.
    pub const ESCAPED: Flags = Flags(2);
    /// A legacy octal literal (`017`), a decimal literal with a leading
    /// zero (`08`), or a string with a legacy octal escape (`"\01"`,
    /// `"\8"`): allowed in sloppy scripts, a syntax error in strict code.
    /// The lexer can't tell whether a script is strict, so it flags these
    /// and leaves the error to the parser; in a module, which is always
    /// strict, it reports them itself.
    pub const LEGACY_OCTAL: Flags = Flags(4);
    /// A template part with an escape that isn't one (`` `\01` ``,
    /// `` `\xg` ``). A tagged template allows it, and its cooked value is
    /// `undefined`; anywhere else it's a syntax error, which the parser
    /// reports, since only it knows whether the template is tagged.
    pub const INVALID_ESCAPE: Flags = Flags(8);

    pub const fn empty() -> Flags {
        Flags(0)
    }

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    pub fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }
}

/// What kind of token it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The end of the input; the lexer gives it again on every call after.
    Eof,
    /// `#!` at the very start of the input, to the end of the line.
    Hashbang,
    /// `// ...`, to the end of the line.
    LineComment,
    /// `/* ... */`.
    BlockComment,
    /// Annex B's `<!--`, to the end of the line, in scripts only.
    HtmlOpenComment,
    /// Annex B's `-->` at the start of a line, to its end, in scripts only.
    HtmlCloseComment,
    /// An IdentifierName that isn't a keyword spelled plainly.
    Ident,
    /// An IdentifierName spelled as one of these words, with no escapes.
    Keyword(Keyword),
    /// `#name`, a private name.
    PrivateName,
    /// A NumericLiteral that isn't a BigInt.
    Number,
    /// A NumericLiteral ending in `n`.
    BigInt,
    /// A StringLiteral, quotes included.
    String,
    /// A NoSubstitutionTemplate: `` `...` ``.
    Template,
    /// A TemplateHead: `` `...${ ``.
    TemplateHead,
    /// A TemplateMiddle: `}...${`.
    TemplateMiddle,
    /// A TemplateTail: `` }...` ``.
    TemplateTail,
    /// A RegularExpressionLiteral, `/body/flags`. The lexer checks the
    /// flags; the body is checked against the pattern grammar later.
    RegExp,
    Punct(Punct),
    /// A character that starts no token, or a `\` that starts no escape.
    /// The lexer has reported it.
    Invalid,
}

impl Kind {
    /// Comments and the hashbang: the tokens a parser skips.
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Kind::Hashbang
                | Kind::LineComment
                | Kind::BlockComment
                | Kind::HtmlOpenComment
                | Kind::HtmlCloseComment
        )
    }
}

macro_rules! words {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $text:literal,)* }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            /// Every one, in order.
            #[allow(dead_code)] // Only Keyword's is used, to build its hash table.
            pub(crate) const ALL: &'static [$name] = &[$($name::$variant,)*];

            /// Its spelling.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }

            /// The one spelled `text`, if there is one.
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(text: &str) -> Option<$name> {
                Some(match text {
                    $($text => $name::$variant,)*
                    _ => return None,
                })
            }
        }
    };
}

words! {
    /// The words a parser treats specially: every ReservedWord, the ones
    /// strict code reserves, and the contextual ones (`async`, `of`,
    /// `get` and the rest), which are identifiers wherever they aren't
    /// keywords. Which a word is, where, is the parser's to decide;
    /// [`Keyword::is_reserved`] says which are ReservedWords.
    Keyword {
        Await = "await",
        Break = "break",
        Case = "case",
        Catch = "catch",
        Class = "class",
        Const = "const",
        Continue = "continue",
        Debugger = "debugger",
        Default = "default",
        Delete = "delete",
        Do = "do",
        Else = "else",
        Enum = "enum",
        Export = "export",
        Extends = "extends",
        False = "false",
        Finally = "finally",
        For = "for",
        Function = "function",
        If = "if",
        Import = "import",
        In = "in",
        Instanceof = "instanceof",
        New = "new",
        Null = "null",
        Return = "return",
        Super = "super",
        Switch = "switch",
        This = "this",
        Throw = "throw",
        True = "true",
        Try = "try",
        Typeof = "typeof",
        Var = "var",
        Void = "void",
        While = "while",
        With = "with",
        Yield = "yield",
        // Reserved in strict code.
        Let = "let",
        Static = "static",
        Implements = "implements",
        Interface = "interface",
        Package = "package",
        Private = "private",
        Protected = "protected",
        Public = "public",
        // Contextual.
        As = "as",
        Async = "async",
        From = "from",
        Get = "get",
        Meta = "meta",
        Of = "of",
        Set = "set",
        Target = "target",
    }
}

impl Keyword {
    /// Whether it's one of the spec's ReservedWords. `await` and `yield`
    /// are among them, though each is an identifier in some places.
    pub const fn is_reserved(self) -> bool {
        (self as u8) <= (Keyword::Yield as u8)
    }

    /// The keyword spelled `b`, found with one table lookup and one
    /// comparison: the lexer asks this of every identifier it reads.
    pub(crate) fn from_bytes(b: &[u8]) -> Option<Keyword> {
        if !(2..=10).contains(&b.len()) {
            return None;
        }
        let k = KEYWORDS[keyword_hash(b)]?;
        (k.as_str().as_bytes() == b).then_some(k)
    }

    /// Whether strict code reserves it beyond the ReservedWords.
    pub const fn is_strict_reserved(self) -> bool {
        (self as u8) > (Keyword::Yield as u8) && (self as u8) <= (Keyword::Public as u8)
    }
}

/// A hash of a word of two bytes or more that no two keywords share,
/// which building [`KEYWORDS`] checks.
const fn keyword_hash(b: &[u8]) -> usize {
    (b[0] as usize + b[1] as usize * 6 + b[b.len() - 1] as usize * 21 + b.len()) & 255
}

/// Each keyword at its hash.
static KEYWORDS: [Option<Keyword>; 256] = {
    let mut table = [None; 256];
    let mut i = 0;
    while i < Keyword::ALL.len() {
        let k = Keyword::ALL[i];
        let h = keyword_hash(k.as_str().as_bytes());
        assert!(table[h].is_none(), "two keywords share a hash");
        table[h] = Some(k);
        i += 1;
    }
    table
};

words! {
    /// A Punctuator, DivPunctuator or RightBracePunctuator.
    Punct {
        LBrace = "{",
        RBrace = "}",
        LParen = "(",
        RParen = ")",
        LBracket = "[",
        RBracket = "]",
        Dot = ".",
        Ellipsis = "...",
        Semicolon = ";",
        Comma = ",",
        Lt = "<",
        Gt = ">",
        LtEq = "<=",
        GtEq = ">=",
        EqEq = "==",
        NotEq = "!=",
        EqEqEq = "===",
        NotEqEq = "!==",
        Plus = "+",
        Minus = "-",
        Star = "*",
        Slash = "/",
        Percent = "%",
        StarStar = "**",
        PlusPlus = "++",
        MinusMinus = "--",
        Shl = "<<",
        Shr = ">>",
        UShr = ">>>",
        Amp = "&",
        Pipe = "|",
        Caret = "^",
        Bang = "!",
        Tilde = "~",
        AmpAmp = "&&",
        PipePipe = "||",
        QuestionQuestion = "??",
        Question = "?",
        QuestionDot = "?.",
        Colon = ":",
        Eq = "=",
        PlusEq = "+=",
        MinusEq = "-=",
        StarEq = "*=",
        SlashEq = "/=",
        PercentEq = "%=",
        StarStarEq = "**=",
        ShlEq = "<<=",
        ShrEq = ">>=",
        UShrEq = ">>>=",
        AmpEq = "&=",
        PipeEq = "|=",
        CaretEq = "^=",
        AmpAmpEq = "&&=",
        PipePipeEq = "||=",
        QuestionQuestionEq = "??=",
        Arrow = "=>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_by_hash() {
        for &k in Keyword::ALL {
            assert_eq!(Keyword::from_bytes(k.as_str().as_bytes()), Some(k));
        }
        for word in [
            "",
            "a",
            "x",
            "lets",
            "yiel",
            "constructor",
            "Await",
            "awaiT",
            "of_",
            "ofs",
        ] {
            assert_eq!(
                Keyword::from_bytes(word.as_bytes()),
                Keyword::from_str(word),
                "{word}"
            );
        }
    }
}
