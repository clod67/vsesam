//! Runtime values and lexical environments for the nanch interpreter.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::ast::Stmt;

/// A callable nanch function (closure capturing its defining scope).
pub struct Func {
    pub name: String,
    pub params: Vec<String>,
    pub body: Rc<Vec<Stmt>>,
    pub closure: Env,
}

#[derive(Clone)]
pub enum Value {
    Num(f64),
    Str(Rc<String>),
    Bool(bool),
    Nil,
    Func(Rc<Func>),
    /// A host builtin, identified by its name; dispatched in the interpreter.
    Native(&'static str),
}

impl Value {
    pub fn str<S: Into<String>>(s: S) -> Value {
        Value::Str(Rc::new(s.into()))
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Nil => false,
            Value::Str(s) => !s.is_empty(),
            Value::Func(_) | Value::Native(_) => true,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Num(_) => "数",
            Value::Str(_) => "字",
            Value::Bool(_) => "真假",
            Value::Nil => "空",
            Value::Func(_) => "函",
            Value::Native(_) => "native",
        }
    }

    /// Display form used by `喊` (print) and string coercion.
    pub fn display(&self) -> String {
        match self {
            Value::Num(n) => fmt_num(*n),
            Value::Str(s) => (**s).clone(),
            Value::Bool(b) => if *b { "真" } else { "假" }.to_string(),
            Value::Nil => "空".to_string(),
            Value::Func(f) => format!("<函 {}>", f.name),
            Value::Native(n) => format!("<native {}>", n),
        }
    }

    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            (Value::Func(a), Value::Func(b)) => Rc::ptr_eq(a, b),
            (Value::Native(a), Value::Native(b)) => a == b,
            _ => false,
        }
    }
}

/// Render a float without a trailing `.0` when it is integral.
pub fn fmt_num(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{}", n);
        s
    }
}

/// A lexical scope with a chain to its parent.
pub struct Scope {
    pub vars: HashMap<String, Value>,
    pub parent: Option<Env>,
}

pub type Env = Rc<RefCell<Scope>>;

pub fn new_global() -> Env {
    Rc::new(RefCell::new(Scope {
        vars: HashMap::new(),
        parent: None,
    }))
}

pub fn new_child(parent: &Env) -> Env {
    Rc::new(RefCell::new(Scope {
        vars: HashMap::new(),
        parent: Some(parent.clone()),
    }))
}

/// Look up a name through the scope chain.
pub fn env_get(env: &Env, name: &str) -> Option<Value> {
    let scope = env.borrow();
    if let Some(v) = scope.vars.get(name) {
        return Some(v.clone());
    }
    match &scope.parent {
        Some(p) => env_get(p, name),
        None => None,
    }
}

/// Define a new binding in the current scope.
pub fn env_define(env: &Env, name: String, v: Value) {
    env.borrow_mut().vars.insert(name, v);
}

/// Assign to an existing binding somewhere up the chain. Returns false if the
/// name was never declared.
pub fn env_assign(env: &Env, name: &str, v: Value) -> bool {
    if env.borrow().vars.contains_key(name) {
        env.borrow_mut().vars.insert(name.to_string(), v);
        return true;
    }
    let parent = env.borrow().parent.clone();
    match parent {
        Some(p) => env_assign(&p, name, v),
        None => false,
    }
}
