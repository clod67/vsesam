//! A faithful nanch pretty-printer and an AST-driven auto-commenter.
//!
//! `pretty` renders a parsed program back to nanch with precedence-correct
//! (no redundant) parentheses. `annotate` does the same but prepends a
//! generated `؟` comment to every top-level statement — meaning is read off
//! the AST, so the comments are real static analysis, not guesswork.

use super::ast::{BinOp, Expr, LogicOp, Stmt, UnOp};
use super::value::fmt_num;

pub fn pretty(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    print_stmts(stmts, 0, &mut out);
    out
}

/// Render the program with an explanatory `؟` comment above each top-level
/// statement.
pub fn annotate(stmts: &[Stmt]) -> String {
    let mut out = String::new();
    out.push_str("؟ авто-комментарии VSESAM (из AST, не вайбкод) — 6767\n\n");
    for s in stmts {
        out.push_str(&format!("؟ {}\n", describe(s)));
        print_stmt(s, 0, &mut out);
        out.push('\n');
    }
    out
}

// ---- statement printing -------------------------------------------------

fn pad(indent: usize) -> String {
    "  ".repeat(indent)
}

fn print_stmts(stmts: &[Stmt], indent: usize, out: &mut String) {
    for s in stmts {
        print_stmt(s, indent, out);
    }
}

fn print_stmt(s: &Stmt, indent: usize, out: &mut String) {
    let p = pad(indent);
    match s {
        Stmt::Let(name, e) => {
            out.push_str(&format!("{p}设 {name} = {}؛\n", expr(e)));
        }
        Stmt::Func { name, params, body } => {
            out.push_str(&format!("{p}函 {name}({}) {{\n", params.join(", ")));
            print_stmts(body, indent + 1, out);
            out.push_str(&format!("{p}}}\n"));
        }
        Stmt::Return(Some(e)) => out.push_str(&format!("{p}回 {}؛\n", expr(e))),
        Stmt::Return(None) => out.push_str(&format!("{p}回؛\n")),
        Stmt::If { cond, then, els } => {
            out.push_str(&format!("{p}如果 ({}) {{\n", expr(cond)));
            print_stmts(then, indent + 1, out);
            if els.is_empty() {
                out.push_str(&format!("{p}}}\n"));
            } else if els.len() == 1 && matches!(els[0], Stmt::If { .. }) {
                // else-if chain
                out.push_str(&format!("{p}}} 否则 "));
                let mut inner = String::new();
                print_stmt(&els[0], indent, &mut inner);
                out.push_str(inner.trim_start());
            } else {
                out.push_str(&format!("{p}}} 否则 {{\n"));
                print_stmts(els, indent + 1, out);
                out.push_str(&format!("{p}}}\n"));
            }
        }
        Stmt::While { cond, body } => {
            out.push_str(&format!("{p}طالما ({}) {{\n", expr(cond)));
            print_stmts(body, indent + 1, out);
            out.push_str(&format!("{p}}}\n"));
        }
        Stmt::Block(body) => {
            out.push_str(&format!("{p}{{\n"));
            print_stmts(body, indent + 1, out);
            out.push_str(&format!("{p}}}\n"));
        }
        Stmt::Expr(e) => out.push_str(&format!("{p}{}؛\n", expr(e))),
    }
}

// ---- expression printing (precedence-aware) -----------------------------

fn expr(e: &Expr) -> String {
    with_prec(e, 0)
}

/// Print `e`, wrapping in parens if its own precedence is below `min`.
fn with_prec(e: &Expr, min: u8) -> String {
    let (s, p) = render(e);
    if p < min {
        format!("({s})")
    } else {
        s
    }
}

fn render(e: &Expr) -> (String, u8) {
    match e {
        Expr::Num(n) => (fmt_num(*n), 10),
        Expr::Str(s) => (format!("\"{}\"", escape(s)), 10),
        Expr::Bool(b) => ((if *b { "真" } else { "假" }).to_string(), 10),
        Expr::Nil => ("空".to_string(), 10),
        Expr::Ident(n) => (n.clone(), 10),
        Expr::Unary(op, x) => {
            let sym = match op {
                UnOp::Neg => "-",
                UnOp::Not => "!",
            };
            (format!("{sym}{}", with_prec(x, 8)), 8)
        }
        Expr::Binary(op, a, b) => {
            let (sym, prec) = binop(*op);
            (
                format!("{} {sym} {}", with_prec(a, prec), with_prec(b, prec + 1)),
                prec,
            )
        }
        Expr::Logic(op, a, b) => {
            let (sym, prec) = match op {
                LogicOp::Or => ("||", 2u8),
                LogicOp::And => ("&&", 3u8),
            };
            (
                format!("{} {sym} {}", with_prec(a, prec), with_prec(b, prec + 1)),
                prec,
            )
        }
        Expr::Assign(n, v) => (format!("{n} = {}", with_prec(v, 1)), 1),
        Expr::Call(callee, args) => {
            let a = args.iter().map(expr).collect::<Vec<_>>().join(", ");
            (format!("{}({a})", with_prec(callee, 9)), 9)
        }
    }
}

fn binop(op: BinOp) -> (&'static str, u8) {
    match op {
        BinOp::Add => ("+", 6),
        BinOp::Sub => ("-", 6),
        BinOp::Mul => ("*", 7),
        BinOp::Div => ("/", 7),
        BinOp::Mod => ("%", 7),
        BinOp::Eq => ("==", 4),
        BinOp::Ne => ("!=", 4),
        BinOp::Lt => ("<", 5),
        BinOp::Gt => (">", 5),
        BinOp::Le => ("<=", 5),
        BinOp::Ge => (">=", 5),
    }
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

// ---- comment generation (meaning read from the AST) ---------------------

fn describe(s: &Stmt) -> String {
    match s {
        Stmt::Let(name, e) => match call_name(e) {
            Some(native) => format!("`{name}` ← {}", verb(native, e)),
            None => format!("объявляет переменную `{name}`"),
        },
        Stmt::Func { name, params, .. } => {
            if params.is_empty() {
                format!("функция `{name}()`")
            } else {
                format!("функция `{name}({})`", params.join(", "))
            }
        }
        Stmt::Return(_) => "возврат из функции".to_string(),
        Stmt::If { .. } => "условие (如果)".to_string(),
        Stmt::While { .. } => "цикл (طالما), пока условие истинно".to_string(),
        Stmt::Block(_) => "блок".to_string(),
        Stmt::Expr(e) => match call_name(e) {
            Some(native) => verb(native, e),
            None => "вычисляет выражение".to_string(),
        },
    }
}

/// If `e` is a call whose callee is a bare identifier, return that name.
fn call_name(e: &Expr) -> Option<&str> {
    if let Expr::Call(callee, _) = e {
        if let Expr::Ident(name) = &**callee {
            return Some(name);
        }
    }
    None
}

/// A human verb phrase for a known native call, else a generic one.
fn verb(name: &str, _e: &Expr) -> String {
    match name {
        "喊" => "печатает в журнал".to_string(),
        "生" => "спавнит сущность → id".to_string(),
        "代理" => "создаёт AI-агента с мозгом".to_string(),
        "毁" => "уничтожает сущность".to_string(),
        "动" => "задаёт смещение сущности на этот тик".to_string(),
        "脑" => "назначает AI-мозг сущности".to_string(),
        "伤" => "наносит урон".to_string(),
        "治" => "лечит сущность".to_string(),
        "记" => "пишет значение в сессию (встраивается в движок)".to_string(),
        "取" => "читает значение из сессии".to_string(),
        "近" => "ищет ближайшего врага".to_string(),
        "距" => "считает дистанцию".to_string(),
        "数" => "случайное число".to_string(),
        "我" => "имя текущего игрока".to_string(),
        other => format!("вызывает `{other}(…)`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nanch::lexer::lex;
    use crate::nanch::parser::Parser;

    fn program(src: &str) -> Vec<Stmt> {
        Parser::new(lex(src).unwrap()).parse_program().unwrap()
    }

    #[test]
    fn pretty_is_reparseable_and_precedence_correct() {
        let src = "设 x = 2 + 3 * 4؛ 设 y = (2 + 3) * 4؛ 喊(x, y)؛";
        let out = pretty(&program(src));
        // round-trips through the parser
        let re = program(&out);
        assert_eq!(pretty(&re), out);
        assert!(out.contains("2 + 3 * 4"));
        assert!(out.contains("(2 + 3) * 4"));
    }

    #[test]
    fn annotate_adds_comments() {
        let src = "设 g = 生(\"goblin\", 5, 5)؛\n脑(g, \"think\")؛";
        let out = annotate(&program(src));
        assert!(out.contains("؟"));
        assert!(out.contains("спавнит сущность"));
        assert!(out.contains("назначает AI-мозг"));
    }
}
