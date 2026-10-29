//! The nanch tree-walking interpreter — and the bridge between the language
//! and the VSESAM engine.
//!
//! The interpreter *owns* the `World` and the login `Session`, so nanch native
//! functions can spawn entities, move them, wire up AI brains and read/write
//! the player's persistent session store. A game is just a nanch program: the
//! top level runs once as setup, an optional global `每拍` hook runs every
//! tick, and any entity whose brain is a nanch function becomes an autonomous
//! AI agent driven by that function each frame.

use std::rc::Rc;

use crate::ai::{self, Behavior};
use crate::auth::Session;
use crate::engine::world::{Brain, EntityId, World};

use super::ast::{BinOp, Expr, LogicOp, Stmt, UnOp};
use super::lexer::lex;
use super::parser::Parser;
use super::value::{
    env_assign, env_define, env_get, fmt_num, new_child, new_global, Env, Func, Value,
};

/// The per-tick global update hook a script may define.
const TICK_HOOK: &str = "每拍";

/// Every native function name. Kept in lockstep with `call_native`.
const NATIVES: &[&str] = &[
    // output / conversion
    "喊", "字", "数值", // print, to-string, parse-number
    // spawning / lifetime
    "生", "代理", "毁", // spawn, agent, destroy
    // movement
    "动", // move
    // stats
    "血", "满血", "治", "伤", "阵", "设阵", "名", "标", "活",
    // spatial queries
    "近", "谁", "距", "横", "纵", "数量", "敌数", "友数", "时",
    // AI
    "脑", // set brain
    // math
    "数", "底", "绝", "根", "最小", "最大",
    // session (login state embedded into the engine)
    "记", "取", "我",
];

pub struct RuntimeError {
    pub message: String,
}

fn rt<S: Into<String>>(message: S) -> RuntimeError {
    RuntimeError {
        message: message.into(),
    }
}

type RResult<T> = Result<T, RuntimeError>;

enum Flow {
    Normal,
    Return(Value),
}

pub struct Interp {
    pub global: Env,
    pub world: World,
    pub session: Session,
    pub tick: u64,
    rng: u64,
    depth: usize,
    log: Vec<String>,
}

impl Interp {
    pub fn new(world: World, session: Session) -> Interp {
        let global = new_global();
        let mut it = Interp {
            global,
            world,
            session,
            tick: 0,
            rng: 0,
            depth: 0,
            log: Vec::new(),
        };
        it.rng = it.seed_from_session();
        it.install_natives();
        it
    }

    fn install_natives(&mut self) {
        for name in NATIVES {
            env_define(&self.global, name.to_string(), Value::Native(name));
        }
    }

    fn seed_from_session(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325; // FNV-1a offset basis
        for b in self.session.token.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h | 1 // never zero (xorshift requirement)
    }

    // ---- RNG (deterministic xorshift64*) -------------------------------

    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    // ---- public driver API ---------------------------------------------

    /// Lex, parse and execute a nanch program as engine setup (runs once).
    pub fn run_source(&mut self, src: &str) -> Result<(), String> {
        let stmts = self.compile(src)?;
        let env = self.global.clone();
        for s in &stmts {
            self.exec(s, &env).map_err(|e| format!("runtime error: {}", e.message))?;
        }
        Ok(())
    }

    /// Evaluate a single REPL line. A bare expression's value is returned for
    /// display; statements are executed for their effects.
    pub fn eval_line(&mut self, src: &str) -> Result<Option<String>, String> {
        let stmts = self.compile(src)?;
        let env = self.global.clone();
        let mut last: Option<String> = None;
        for s in &stmts {
            match s {
                Stmt::Expr(e) => {
                    let v = self
                        .eval(e, &env)
                        .map_err(|e| format!("runtime error: {}", e.message))?;
                    last = if matches!(v, Value::Nil) {
                        None
                    } else {
                        Some(v.display())
                    };
                }
                _ => {
                    self.exec(s, &env)
                        .map_err(|e| format!("runtime error: {}", e.message))?;
                    last = None;
                }
            }
        }
        Ok(last)
    }

    fn compile(&self, src: &str) -> Result<Vec<Stmt>, String> {
        let toks = lex(src).map_err(|e| format!("lex error (line {}): {}", e.line, e.message))?;
        let mut p = Parser::new(toks);
        p.parse_program()
            .map_err(|e| format!("parse error (line {}): {}", e.line, e.message))
    }

    /// Advance the simulation by one tick: run the global hook, run every AI
    /// agent (scripted then built-in), integrate movement, resolve combat.
    pub fn tick_world(&mut self) {
        self.tick += 1;

        // 1. global per-tick hook, if the script defined one
        if let Some(Value::Func(f)) = env_get(&self.global, TICK_HOOK) {
            if let Err(e) = self.call_func(&f, vec![]) {
                self.log.push(format!("⚠ 每拍: {}", e.message));
            }
        }

        // 2. scripted agent brains: f(id) each tick
        for (id, f) in self.world.script_brains() {
            if self.world.alive(id) {
                if let Err(e) = self.call_func(&f, vec![Value::Num(id as f64)]) {
                    self.log.push(format!("⚠ 脑 #{}: {}", id, e.message));
                }
            }
        }

        // 3. built-in behaviours → this tick's velocity
        for (id, beh) in self.world.builtin_brains() {
            if !self.world.alive(id) {
                continue;
            }
            let noise = self.next_u64();
            let (dx, dy) = ai::decide(&self.world, id, beh, noise, self.tick);
            if let Some(e) = self.world.get_mut(id) {
                e.vx = dx;
                e.vy = dy;
            }
        }

        // 4. physics
        self.world.integrate();

        // 5. combat
        for ev in self.world.resolve_combat() {
            if ev.killed {
                let name = self
                    .world
                    .entities
                    .get(ev.victim)
                    .map(|e| e.kind.clone())
                    .unwrap_or_default();
                self.log.push(format!(
                    "☠ #{} ({}) сражён #{} (−{} урона)",
                    ev.victim,
                    name,
                    ev.attacker,
                    fmt_num(ev.dmg)
                ));
            }
        }
    }

    pub fn drain_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    // ---- statement / expression evaluation -----------------------------

    fn exec_block(&mut self, stmts: &[Stmt], env: &Env) -> RResult<Flow> {
        for s in stmts {
            match self.exec(s, env)? {
                Flow::Return(v) => return Ok(Flow::Return(v)),
                Flow::Normal => {}
            }
        }
        Ok(Flow::Normal)
    }

    fn exec(&mut self, stmt: &Stmt, env: &Env) -> RResult<Flow> {
        match stmt {
            Stmt::Let(name, e) => {
                let v = self.eval(e, env)?;
                env_define(env, name.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Func { name, params, body } => {
                let f = Rc::new(Func {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    closure: env.clone(),
                });
                env_define(env, name.clone(), Value::Func(f));
                Ok(Flow::Normal)
            }
            Stmt::Return(opt) => {
                let v = match opt {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(v))
            }
            Stmt::If { cond, then, els } => {
                if self.eval(cond, env)?.truthy() {
                    let scope = new_child(env);
                    self.exec_block(then, &scope)
                } else {
                    let scope = new_child(env);
                    self.exec_block(els, &scope)
                }
            }
            Stmt::While { cond, body } => {
                let mut guard: u64 = 0;
                while self.eval(cond, env)?.truthy() {
                    guard += 1;
                    if guard > 10_000_000 {
                        return Err(rt("طالما loop exceeded 10M iterations (runaway?)"));
                    }
                    let scope = new_child(env);
                    match self.exec_block(body, &scope)? {
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Normal => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Block(stmts) => {
                let scope = new_child(env);
                self.exec_block(stmts, &scope)
            }
            Stmt::Expr(e) => {
                self.eval(e, env)?;
                Ok(Flow::Normal)
            }
        }
    }

    fn eval(&mut self, expr: &Expr, env: &Env) -> RResult<Value> {
        match expr {
            Expr::Num(n) => Ok(Value::Num(*n)),
            Expr::Str(s) => Ok(Value::str(s.clone())),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Ident(name) => {
                env_get(env, name).ok_or_else(|| rt(format!("undefined name `{}`", name)))
            }
            Expr::Unary(op, e) => {
                let v = self.eval(e, env)?;
                match op {
                    UnOp::Not => Ok(Value::Bool(!v.truthy())),
                    UnOp::Neg => match v {
                        Value::Num(n) => Ok(Value::Num(-n)),
                        other => Err(rt(format!("cannot negate {}", other.type_name()))),
                    },
                }
            }
            Expr::Logic(op, a, b) => {
                let left = self.eval(a, env)?;
                match op {
                    LogicOp::And => {
                        if left.truthy() {
                            self.eval(b, env)
                        } else {
                            Ok(left)
                        }
                    }
                    LogicOp::Or => {
                        if left.truthy() {
                            Ok(left)
                        } else {
                            self.eval(b, env)
                        }
                    }
                }
            }
            Expr::Binary(op, a, b) => {
                let l = self.eval(a, env)?;
                let r = self.eval(b, env)?;
                self.binop(*op, l, r)
            }
            Expr::Assign(name, e) => {
                let v = self.eval(e, env)?;
                if env_assign(env, name, v.clone()) {
                    Ok(v)
                } else {
                    Err(rt(format!("assign to undeclared name `{}` (use 设 first)", name)))
                }
            }
            Expr::Call(callee, args) => {
                let callee_v = self.eval(callee, env)?;
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.eval(a, env)?);
                }
                match callee_v {
                    Value::Func(f) => self.call_func(&f, argv),
                    Value::Native(n) => self.call_native(n, argv),
                    other => Err(rt(format!("{} is not callable", other.type_name()))),
                }
            }
        }
    }

    fn binop(&self, op: BinOp, l: Value, r: Value) -> RResult<Value> {
        use BinOp::*;
        match op {
            Add => match (&l, &r) {
                (Value::Num(a), Value::Num(b)) => Ok(Value::Num(a + b)),
                // string concatenation if either side is a string
                (Value::Str(_), _) | (_, Value::Str(_)) => {
                    Ok(Value::str(format!("{}{}", l.display(), r.display())))
                }
                _ => Err(rt(format!(
                    "cannot add {} and {}",
                    l.type_name(),
                    r.type_name()
                ))),
            },
            Sub | Mul | Div | Mod => {
                let a = as_num(&l)?;
                let b = as_num(&r)?;
                Ok(Value::Num(match op {
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    Mod => a % b,
                    _ => unreachable!(),
                }))
            }
            Eq => Ok(Value::Bool(l.equals(&r))),
            Ne => Ok(Value::Bool(!l.equals(&r))),
            Lt | Gt | Le | Ge => {
                let ord = match (&l, &r) {
                    (Value::Num(a), Value::Num(b)) => a.partial_cmp(b),
                    (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
                    _ => {
                        return Err(rt(format!(
                            "cannot compare {} and {}",
                            l.type_name(),
                            r.type_name()
                        )))
                    }
                };
                let ord = match ord {
                    Some(o) => o,
                    None => return Ok(Value::Bool(false)), // NaN
                };
                use std::cmp::Ordering::*;
                Ok(Value::Bool(match op {
                    Lt => ord == Less,
                    Gt => ord == Greater,
                    Le => ord != Greater,
                    Ge => ord != Less,
                    _ => unreachable!(),
                }))
            }
        }
    }

    fn call_func(&mut self, f: &Rc<Func>, args: Vec<Value>) -> RResult<Value> {
        self.depth += 1;
        if self.depth > 512 {
            self.depth -= 1;
            return Err(rt("call stack too deep (recursion runaway?)"));
        }
        let scope = new_child(&f.closure);
        for (i, p) in f.params.iter().enumerate() {
            env_define(&scope, p.clone(), args.get(i).cloned().unwrap_or(Value::Nil));
        }
        let result = self.exec_block(&f.body, &scope);
        self.depth -= 1;
        match result? {
            Flow::Return(v) => Ok(v),
            Flow::Normal => Ok(Value::Nil),
        }
    }

    // ---- native functions (the nanch ↔ engine bridge) ------------------

    fn call_native(&mut self, name: &str, args: Vec<Value>) -> RResult<Value> {
        let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
        match name {
            // ---- output / conversion ----
            "喊" => {
                let line = args
                    .iter()
                    .map(|v| v.display())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.log.push(line);
                Ok(Value::Nil)
            }
            "字" => Ok(Value::str(arg(0).display())),
            "数值" => Ok(match arg(0) {
                Value::Num(n) => Value::Num(n),
                other => match other.display().trim().parse::<f64>() {
                    Ok(n) => Value::Num(n),
                    Err(_) => Value::Nil,
                },
            }),

            // ---- spawning / lifetime ----
            "生" => {
                let kind = arg(0).display();
                let x = as_num(&arg(1))?;
                let y = as_num(&arg(2))?;
                Ok(Value::Num(self.world.spawn(&kind, x, y) as f64))
            }
            "代理" => {
                let kind = arg(0).display();
                let x = as_num(&arg(1))?;
                let y = as_num(&arg(2))?;
                let id = self.world.spawn(&kind, x, y);
                let brain = arg(3);
                self.set_brain(id, brain)?;
                Ok(Value::Num(id as f64))
            }
            "毁" => {
                self.world.kill(as_id(&arg(0))?);
                Ok(Value::Nil)
            }

            // ---- movement ----
            "动" => {
                let id = as_id(&arg(0))?;
                let dx = as_num(&arg(1))?;
                let dy = as_num(&arg(2))?;
                if let Some(e) = self.world.get_mut(id) {
                    e.vx = dx;
                    e.vy = dy;
                }
                Ok(Value::Nil)
            }

            // ---- stats ----
            "血" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::Num(e.hp))
                .unwrap_or(Value::Nil)),
            "满血" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::Num(e.max_hp))
                .unwrap_or(Value::Nil)),
            "治" => {
                let id = as_id(&arg(0))?;
                let amt = as_num(&arg(1))?;
                if let Some(e) = self.world.get_mut(id) {
                    e.hp = (e.hp + amt).min(e.max_hp);
                }
                Ok(Value::Nil)
            }
            "伤" => {
                let id = as_id(&arg(0))?;
                let amt = as_num(&arg(1))?;
                let mut killed = false;
                if let Some(e) = self.world.get_mut(id) {
                    e.hp -= amt;
                    killed = e.hp <= 0.0;
                }
                if killed {
                    self.world.kill(id);
                }
                Ok(Value::Bool(killed))
            }
            "阵" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::Num(e.faction as f64))
                .unwrap_or(Value::Nil)),
            "设阵" => {
                let id = as_id(&arg(0))?;
                let team = as_num(&arg(1))? as i32;
                if let Some(e) = self.world.get_mut(id) {
                    e.faction = team;
                }
                Ok(Value::Nil)
            }
            "名" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::str(e.kind.clone()))
                .unwrap_or(Value::Nil)),
            "标" => {
                let id = as_id(&arg(0))?;
                if args.len() >= 2 {
                    let text = arg(1).display();
                    if let Some(e) = self.world.get_mut(id) {
                        e.label = text;
                    }
                    Ok(Value::Nil)
                } else {
                    Ok(self
                        .world
                        .get(id)
                        .map(|e| Value::str(e.label.clone()))
                        .unwrap_or(Value::Nil))
                }
            }
            "活" => Ok(Value::Bool(self.world.alive(as_id(&arg(0))?))),

            // ---- spatial queries ----
            "近" => Ok(self
                .world
                .nearest_enemy(as_id(&arg(0))?)
                .map(|id| Value::Num(id as f64))
                .unwrap_or(Value::Nil)),
            "谁" => Ok(self
                .world
                .nearest_any(as_id(&arg(0))?)
                .map(|id| Value::Num(id as f64))
                .unwrap_or(Value::Nil)),
            "距" => Ok(self
                .world
                .distance(as_id(&arg(0))?, as_id(&arg(1))?)
                .map(Value::Num)
                .unwrap_or(Value::Nil)),
            "横" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::Num(e.x))
                .unwrap_or(Value::Nil)),
            "纵" => Ok(self
                .world
                .get(as_id(&arg(0))?)
                .map(|e| Value::Num(e.y))
                .unwrap_or(Value::Nil)),
            "数量" => Ok(Value::Num(self.world.alive_count() as f64)),
            "敌数" => Ok(Value::Num(self.world.count_faction(1) as f64)),
            "友数" => Ok(Value::Num(self.world.count_faction(0) as f64)),
            "时" => Ok(Value::Num(self.tick as f64)),

            // ---- AI ----
            "脑" => {
                let id = as_id(&arg(0))?;
                if args.len() < 2 {
                    // getter: report the entity's current brain as a string
                    let name = match self.world.get(id).map(|e| &e.brain) {
                        Some(Brain::Builtin(b)) => b.as_str().to_string(),
                        Some(Brain::Script(_)) => "<函>".to_string(),
                        Some(Brain::None) => "空".to_string(),
                        None => return Ok(Value::Nil),
                    };
                    return Ok(Value::str(name));
                }
                let brain = arg(1);
                self.set_brain(id, brain)?;
                Ok(Value::Nil)
            }

            // ---- math ----
            "数" => {
                let u = self.next_unit();
                let v = match args.len() {
                    0 => u,
                    1 => u * as_num(&arg(0))?,
                    _ => {
                        let lo = as_num(&arg(0))?;
                        let hi = as_num(&arg(1))?;
                        lo + u * (hi - lo)
                    }
                };
                Ok(Value::Num(v))
            }
            "底" => Ok(Value::Num(as_num(&arg(0))?.floor())),
            "绝" => Ok(Value::Num(as_num(&arg(0))?.abs())),
            "根" => Ok(Value::Num(as_num(&arg(0))?.sqrt())),
            "最小" => Ok(Value::Num(as_num(&arg(0))?.min(as_num(&arg(1))?))),
            "最大" => Ok(Value::Num(as_num(&arg(0))?.max(as_num(&arg(1))?))),

            // ---- session (login state embedded in the engine) ----
            "记" => {
                let key = arg(0).display();
                let val = arg(1).display();
                self.session.set(key, val);
                Ok(Value::Nil)
            }
            "取" => Ok(self
                .session
                .get(&arg(0).display())
                .map(|s| Value::str(s.clone()))
                .unwrap_or(Value::Nil)),
            "我" => Ok(Value::str(self.session.user.clone())),

            other => Err(rt(format!("unknown native `{}`", other))),
        }
    }

    fn set_brain(&mut self, id: EntityId, b: Value) -> RResult<()> {
        let brain = match &b {
            Value::Str(s) => match Behavior::from_str(s) {
                Some(beh) => Brain::Builtin(beh),
                None => return Err(rt(format!("unknown behavior `{}`", s))),
            },
            Value::Func(f) => Brain::Script(f.clone()),
            Value::Nil => Brain::None,
            other => {
                return Err(rt(format!(
                    "脑 expects a behavior name or 函, got {}",
                    other.type_name()
                )))
            }
        };
        if let Some(e) = self.world.get_mut(id) {
            e.brain = brain;
        }
        Ok(())
    }
}

fn as_num(v: &Value) -> RResult<f64> {
    match v {
        Value::Num(n) => Ok(*n),
        other => Err(rt(format!("expected 数 (number), got {}", other.type_name()))),
    }
}

fn as_id(v: &Value) -> RResult<EntityId> {
    let n = as_num(v)?;
    if n < 0.0 || !n.is_finite() {
        return Err(rt(format!("bad entity id {}", fmt_num(n))));
    }
    Ok(n as EntityId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Session;

    fn interp() -> Interp {
        Interp::new(World::new(40, 20), Session::guest())
    }

    #[test]
    fn arithmetic_precedence() {
        let mut it = interp();
        assert_eq!(it.eval_line("2 + 3 * 4؛").unwrap().as_deref(), Some("14"));
        assert_eq!(it.eval_line("(2 + 3) * 4؛").unwrap().as_deref(), Some("20"));
        assert_eq!(it.eval_line("10 % 3؛").unwrap().as_deref(), Some("1"));
        assert_eq!(it.eval_line("真 && !假؛").unwrap().as_deref(), Some("真"));
    }

    #[test]
    fn recursion_and_functions() {
        let mut it = interp();
        it.run_source("函 факт(n) { 如果 (n <= 1) { 回 1؛ } 回 n * факт(n - 1)؛ }")
            .unwrap();
        assert_eq!(it.eval_line("факт(6)؛").unwrap().as_deref(), Some("720"));
    }

    #[test]
    fn while_loop_and_assignment() {
        let mut it = interp();
        it.run_source("设 s = 0؛ 设 i = 1؛ طالما (i <= 5) { s = s + i؛ i = i + 1؛ }")
            .unwrap();
        assert_eq!(it.eval_line("s؛").unwrap().as_deref(), Some("15"));
    }

    #[test]
    fn string_concat() {
        let mut it = interp();
        assert_eq!(
            it.eval_line("\"a\" + 1 + \"b\"؛").unwrap().as_deref(),
            Some("a1b")
        );
    }

    #[test]
    fn spawn_brain_and_tick_moves_agent() {
        let mut it = interp();
        it.run_source(
            r#"
            设 g = 生("goblin", 2, 10)؛
            脑(g, "seek")؛
            生("player", 30, 10)؛
            "#,
        )
        .unwrap();
        assert_eq!(it.world.alive_count(), 2);
        assert_eq!(it.eval_line("脑(0)؛").unwrap().as_deref(), Some("seek"));
        let x0 = it.world.get(0).unwrap().x;
        for _ in 0..5 {
            it.tick_world();
        }
        let x1 = it.world.get(0).unwrap().x;
        assert!(x1 > x0, "seek brain should move the goblin toward the player");
    }

    #[test]
    fn session_kv_roundtrip() {
        let mut it = interp();
        it.run_source(r#"记("score", 42)؛"#).unwrap();
        assert_eq!(it.eval_line(r#"取("score")؛"#).unwrap().as_deref(), Some("42"));
        assert_eq!(
            it.eval_line(r#"数值(取("score")) + 8؛"#).unwrap().as_deref(),
            Some("50")
        );
    }

    #[test]
    fn script_brain_runs_each_tick() {
        // A nanch function as a brain: nudge the agent right by 1 each tick.
        let mut it = interp();
        it.run_source(
            r#"
            函 вправо(id) { 动(id, 1, 0)؛ }
            设 a = 代理("agent", 5, 5, вправо)؛
            "#,
        )
        .unwrap();
        let x0 = it.world.get(0).unwrap().x;
        it.tick_world();
        it.tick_world();
        let x1 = it.world.get(0).unwrap().x;
        assert_eq!(x1, x0 + 2.0);
    }

    #[test]
    fn combat_kills_and_reports() {
        let mut it = interp();
        it.run_source(
            r#"
            设 p = 生("player", 10, 10)؛
            设 s = 生("slime", 10, 10)؛
            "#,
        )
        .unwrap();
        // slime has 5 hp, player does 3 dmg/tick → dead within 2 ticks
        it.tick_world();
        it.tick_world();
        assert!(!it.world.alive(1), "slime should be dead");
        assert!(it.drain_log().iter().any(|l| l.contains("сражён")));
    }

    #[test]
    fn undefined_name_is_error() {
        let mut it = interp();
        assert!(it.eval_line("нет_такого؛").is_err());
    }
}
