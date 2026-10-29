//! Lexical tokens for the `nanch` language.
//!
//! nanch is deliberately foreign-looking: statements end with the Arabic
//! semicolon `؛` (U+061B), comments start with the Arabic question mark `؟`
//! (U+061F), and the reserved words are a mix of Chinese ideographs and an
//! Arabic loop keyword. Everything else (engine verbs like `喊`, `生`, `脑`…)
//! is an ordinary identifier that resolves to a native function at runtime.

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    // literals
    Num(f64),
    Str(String),
    Ident(String),

    // reserved words (Chinese + one Arabic)
    Let,    // 设   — declare a binding
    Func,   // 函   — define a function
    Return, // 回   — return from a function
    If,     // 如果 — if
    Else,   // 否则 — else
    While,  // طالما — while (Arabic: "as long as")
    True,   // 真
    False,  // 假
    Nil,    // 空

    // punctuation
    LParen,  // (
    RParen,  // )
    LBrace,  // {
    RBrace,  // }
    Comma,   // ,
    Semi,    // ؛  (statement terminator)

    // operators
    Assign,    // =
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Bang,      // !
    EqEq,      // ==
    NeEq,      // !=
    Lt,        // <
    Gt,        // >
    Le,        // <=
    Ge,        // >=
    AmpAmp,    // &&
    PipePipe,  // ||

    Eof,
}

/// A token paired with the 1-based source line it was scanned on, for errors.
#[derive(Clone, Debug)]
pub struct Spanned {
    pub tok: Tok,
    pub line: usize,
}
