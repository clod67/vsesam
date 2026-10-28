//! The `nanch` tokenizer.
//!
//! Works on a `Vec<char>` so multi-byte Chinese/Arabic keywords are handled
//! as naturally as ASCII. Identifiers are runs of Unicode-alphabetic chars
//! (CJK and Arabic letters both report `is_alphabetic() == true`), digits and
//! underscore; a handful of them are then promoted to reserved words.

use super::token::{Spanned, Tok};

pub struct LexError {
    pub message: String,
    pub line: usize,
}

pub fn lex(src: &str) -> Result<Vec<Spanned>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();

    while i < chars.len() {
        let c = chars[i];

        // whitespace
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        // line comment: ؟ ... end-of-line   (Arabic question mark)
        if c == '؟' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // the signature statement terminator: ؛ (Arabic semicolon)
        if c == '؛' {
            out.push(Spanned { tok: Tok::Semi, line });
            i += 1;
            continue;
        }

        // string literal
        if c == '"' {
            i += 1;
            let mut s = String::new();
            loop {
                if i >= chars.len() {
                    return Err(LexError {
                        message: "unterminated string literal".into(),
                        line,
                    });
                }
                let ch = chars[i];
                if ch == '"' {
                    i += 1;
                    break;
                }
                if ch == '\\' {
                    i += 1;
                    if i >= chars.len() {
                        return Err(LexError {
                            message: "unterminated escape".into(),
                            line,
                        });
                    }
                    let esc = chars[i];
                    s.push(match esc {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '"' => '"',
                        '\\' => '\\',
                        '0' => '\0',
                        other => other,
                    });
                    i += 1;
                    continue;
                }
                if ch == '\n' {
                    line += 1;
                }
                s.push(ch);
                i += 1;
            }
            out.push(Spanned { tok: Tok::Str(s), line });
            continue;
        }

        // number (ASCII digits, decimal point)
        if c.is_ascii_digit() {
            let mut n = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                n.push(chars[i]);
                i += 1;
            }
            match n.parse::<f64>() {
                Ok(v) => out.push(Spanned { tok: Tok::Num(v), line }),
                Err(_) => {
                    return Err(LexError {
                        message: format!("bad number literal `{}`", n),
                        line,
                    })
                }
            }
            continue;
        }

        // identifier / keyword
        if is_ident_start(c) {
            let mut word = String::new();
            while i < chars.len() && is_ident_part(chars[i]) {
                word.push(chars[i]);
                i += 1;
            }
            let tok = match word.as_str() {
                "设" => Tok::Let,
                "函" => Tok::Func,
                "回" => Tok::Return,
                "如果" => Tok::If,
                "否则" => Tok::Else,
                "طالما" => Tok::While,
                "真" => Tok::True,
                "假" => Tok::False,
                "空" => Tok::Nil,
                _ => Tok::Ident(word),
            };
            out.push(Spanned { tok, line });
            continue;
        }

        // operators and punctuation
        let two = |a: char, b: char| i + 1 < chars.len() && chars[i] == a && chars[i + 1] == b;
        let (tok, width) = if two('=', '=') {
            (Tok::EqEq, 2)
        } else if two('!', '=') {
            (Tok::NeEq, 2)
        } else if two('<', '=') {
            (Tok::Le, 2)
        } else if two('>', '=') {
            (Tok::Ge, 2)
        } else if two('&', '&') {
            (Tok::AmpAmp, 2)
        } else if two('|', '|') {
            (Tok::PipePipe, 2)
        } else {
            let t = match c {
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                '{' => Tok::LBrace,
                '}' => Tok::RBrace,
                ',' => Tok::Comma,
                '=' => Tok::Assign,
                '+' => Tok::Plus,
                '-' => Tok::Minus,
                '*' => Tok::Star,
                '/' => Tok::Slash,
                '%' => Tok::Percent,
                '!' => Tok::Bang,
                '<' => Tok::Lt,
                '>' => Tok::Gt,
                other => {
                    return Err(LexError {
                        message: format!("unexpected character `{}` (U+{:04X})", other, other as u32),
                        line,
                    })
                }
            };
            (t, 1)
        };
        out.push(Spanned { tok, line });
        i += width;
    }

    out.push(Spanned { tok: Tok::Eof, line });
    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}
