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
pub mod token;
pub mod value;

pub use interp::Interp;
