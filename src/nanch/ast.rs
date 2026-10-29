//! Abstract syntax tree for `nanch`.

use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Copy, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug)]
pub enum LogicOp {
    And,
    Or,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Nil,
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logic(LogicOp, Box<Expr>, Box<Expr>),
    /// `name = expr`
    Assign(String, Box<Expr>),
    /// `callee(args...)`
    Call(Box<Expr>, Vec<Expr>),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// `设 name = expr ؛`
    Let(String, Expr),
    /// `函 name(params) { body }`
    Func {
        name: String,
        params: Vec<String>,
        body: Rc<Vec<Stmt>>,
    },
    /// `回 expr? ؛`
    Return(Option<Expr>),
    /// `如果 (cond) { then } 否则 { els }`
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
    /// `طالما (cond) { body }`
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),
    /// `expr ؛`
    Expr(Expr),
}
