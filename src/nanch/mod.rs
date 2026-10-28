//! `nanch` — VSESAM's esoteric scripting language.
//!
//! Chinese + Arabic keywords, `؛` statement terminators, `؟` comments. The
//! interpreter owns the engine `World` and the login `Session`, so a nanch
//! program *is* a game: it spawns entities, wires AI brains and reads the
//! player's persistent session store.

pub mod ast;
pub mod interp;
pub mod lexer;
pub mod parser;
pub mod printer;
pub mod token;
pub mod value;

pub use interp::Interp;

/// Lex + parse a nanch program into its AST, with a formatted error on failure.
pub fn parse_program(src: &str) -> Result<Vec<ast::Stmt>, String> {
    let toks = lexer::lex(src).map_err(|e| format!("lex error (line {}): {}", e.line, e.message))?;
    parser::Parser::new(toks)
        .parse_program()
        .map_err(|e| format!("parse error (line {}): {}", e.line, e.message))
}
