//! Recursive-descent parser turning a nanch token stream into an AST.

use std::rc::Rc;

use super::ast::{BinOp, Expr, LogicOp, Stmt, UnOp};
use super::token::{Spanned, Tok};

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

pub struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
}

type PResult<T> = Result<T, ParseError>;

impl Parser {
    pub fn new(toks: Vec<Spanned>) -> Self {
        Parser { toks, pos: 0 }
    }

    pub fn parse_program(&mut self) -> PResult<Vec<Stmt>> {
        let mut stmts = Vec::new();
        while !self.check(&Tok::Eof) {
            stmts.push(self.statement()?);
        }
        Ok(stmts)
    }

    // ---- token helpers -------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn line(&self) -> usize {
        self.toks[self.pos].line
    }

    fn check(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn matches(&mut self, t: &Tok) -> bool {
        if self.check(t) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> PResult<()> {
        if self.check(t) {
            self.advance();
            Ok(())
        } else {
            Err(self.err(format!("expected {}, found {:?}", what, self.peek())))
        }
    }

    fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            line: self.line(),
        }
    }

    // ---- statements ----------------------------------------------------

    fn statement(&mut self) -> PResult<Stmt> {
        match self.peek() {
            Tok::Let => self.let_stmt(),
            Tok::Func => self.func_stmt(),
            Tok::Return => self.return_stmt(),
            Tok::If => self.if_stmt(),
            Tok::While => self.while_stmt(),
            Tok::LBrace => Ok(Stmt::Block(self.block()?)),
            _ => self.expr_stmt(),
        }
    }

    fn let_stmt(&mut self) -> PResult<Stmt> {
        self.advance(); // 设
        let name = self.ident("binding name after 设")?;
        self.expect(&Tok::Assign, "`=` in 设 declaration")?;
        let value = self.expression()?;
        self.expect(&Tok::Semi, "`؛` after declaration")?;
        Ok(Stmt::Let(name, value))
    }

    fn func_stmt(&mut self) -> PResult<Stmt> {
        self.advance(); // 函
        let name = self.ident("function name after 函")?;
        self.expect(&Tok::LParen, "`(` after function name")?;
        let mut params = Vec::new();
        if !self.check(&Tok::RParen) {
            loop {
                params.push(self.ident("parameter name")?);
                if !self.matches(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(&Tok::RParen, "`)` after parameters")?;
        let body = self.block()?;
        Ok(Stmt::Func {
            name,
            params,
            body: Rc::new(body),
        })
    }

    fn return_stmt(&mut self) -> PResult<Stmt> {
        self.advance(); // 回
        if self.matches(&Tok::Semi) {
            return Ok(Stmt::Return(None));
        }
        let value = self.expression()?;
        self.expect(&Tok::Semi, "`؛` after 回 value")?;
        Ok(Stmt::Return(Some(value)))
    }

    fn if_stmt(&mut self) -> PResult<Stmt> {
        self.advance(); // 如果
        self.expect(&Tok::LParen, "`(` after 如果")?;
        let cond = self.expression()?;
        self.expect(&Tok::RParen, "`)` after condition")?;
        let then = self.block()?;
        let els = if self.matches(&Tok::Else) {
            if self.check(&Tok::If) {
                // 否则 如果 ... — chained else-if
                vec![self.if_stmt()?]
            } else {
                self.block()?
            }
        } else {
            Vec::new()
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn while_stmt(&mut self) -> PResult<Stmt> {
        self.advance(); // طالما
        self.expect(&Tok::LParen, "`(` after طالما")?;
        let cond = self.expression()?;
        self.expect(&Tok::RParen, "`)` after condition")?;
        let body = self.block()?;
        Ok(Stmt::While { cond, body })
    }

    fn block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(&Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while !self.check(&Tok::RBrace) && !self.check(&Tok::Eof) {
            stmts.push(self.statement()?);
        }
        self.expect(&Tok::RBrace, "`}`")?;
        Ok(stmts)
    }

    fn expr_stmt(&mut self) -> PResult<Stmt> {
        let e = self.expression()?;
        self.expect(&Tok::Semi, "`؛` after expression")?;
        Ok(Stmt::Expr(e))
    }

    fn ident(&mut self, what: &str) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(self.err(format!("expected {}, found {:?}", what, other))),
        }
    }

    // ---- expressions (precedence climbing) -----------------------------

    fn expression(&mut self) -> PResult<Expr> {
        self.assignment()
    }

    fn assignment(&mut self) -> PResult<Expr> {
        let expr = self.logic_or()?;
        if self.check(&Tok::Assign) {
            self.advance();
            let value = self.assignment()?;
            if let Expr::Ident(name) = expr {
                return Ok(Expr::Assign(name, Box::new(value)));
            }
            return Err(self.err("invalid assignment target".into()));
        }
        Ok(expr)
    }

    fn logic_or(&mut self) -> PResult<Expr> {
        let mut left = self.logic_and()?;
        while self.check(&Tok::PipePipe) {
            self.advance();
            let right = self.logic_and()?;
            left = Expr::Logic(LogicOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn logic_and(&mut self) -> PResult<Expr> {
        let mut left = self.equality()?;
        while self.check(&Tok::AmpAmp) {
            self.advance();
            let right = self.equality()?;
            left = Expr::Logic(LogicOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn equality(&mut self) -> PResult<Expr> {
        let mut left = self.comparison()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NeEq => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let right = self.comparison()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn comparison(&mut self) -> PResult<Expr> {
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.term()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn term(&mut self) -> PResult<Expr> {
        let mut left = self.factor()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.factor()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn factor(&mut self) -> PResult<Expr> {
        let mut left = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> PResult<Expr> {
        match self.peek() {
            Tok::Bang => {
                self.advance();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)))
            }
            Tok::Minus => {
                self.advance();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)))
            }
            _ => self.call(),
        }
    }

    fn call(&mut self) -> PResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            if self.check(&Tok::LParen) {
                self.advance();
                let mut args = Vec::new();
                if !self.check(&Tok::RParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.matches(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RParen, "`)` after call arguments")?;
                expr = Expr::Call(Box::new(expr), args);
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> PResult<Expr> {
        match self.peek().clone() {
            Tok::Num(n) => {
                self.advance();
                Ok(Expr::Num(n))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Tok::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Tok::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Tok::Nil => {
                self.advance();
                Ok(Expr::Nil)
            }
            Tok::Ident(name) => {
                self.advance();
                Ok(Expr::Ident(name))
            }
            Tok::LParen => {
                self.advance();
                let e = self.expression()?;
                self.expect(&Tok::RParen, "`)` after grouped expression")?;
                Ok(e)
            }
            other => Err(self.err(format!("unexpected token in expression: {:?}", other))),
        }
    }
}
