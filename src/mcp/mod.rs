//! VSESAM MCP server — JSON-RPC 2.0 over stdio (newline-delimited), so an
//! external AI client (Claude, Codex, …) can drive the engine through tools:
//! spawn entities, tick the simulation, eval nanch, seal scripts, annotate.
//!
//! This turns "AI agents" inside-out: besides nanch-scripted brains *in* the
//! world, a live model *outside* can now orchestrate the whole world over MCP.
//! The transport is stdio only — no network, consistent with the rest of VSESAM.

pub mod json;

use std::io::{self, BufRead, Write};

use json::Json;

use crate::auth::Session;
use crate::engine::render;
use crate::engine::world::World;
use crate::nanch::{self, printer, Interp};
use crate::vault;

pub const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "vsesam";
const SERVER_VERSION: &str = "0.1.0";

/// Run the MCP server loop until stdin closes.
pub fn serve(session: Session) -> i32 {
    let mut state = State::new(session);
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!(
        "VSESAM MCP server · протокол {PROTOCOL_VERSION} · сессия {} · жду JSON-RPC на stdin",
        state.interp.session.user
    );

    let mut buf = String::new();
    loop {
        buf.clear();
        match stdin.lock().read_line(&mut buf) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("ошибка чтения stdin: {e}");
                break;
            }
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let req = match json::parse(line) {
            Ok(j) => j,
            Err(e) => {
                send(&mut stdout, error_msg(Json::Null, -32700, &format!("parse error: {e}")));
                continue;
            }
        };

        let is_notification = req.get("id").is_none();
        let id = req.get("id").cloned().unwrap_or(Json::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let params = req.get("params").cloned().unwrap_or(Json::Obj(vec![]));

        match state.handle(&method, &params) {
            Ok(Some(result)) => {
                if !is_notification {
                    send(&mut stdout, result_msg(id, result));
                }
            }
            Ok(None) => { /* notification: no reply */ }
            Err((code, msg)) => {
                if !is_notification {
                    send(&mut stdout, error_msg(id, code, &msg));
                }
            }
        }
    }
    0
}

fn send(out: &mut impl Write, msg: Json) {
    let _ = writeln!(out, "{}", msg.to_string());
    let _ = out.flush();
}

fn result_msg(id: Json, result: Json) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::s("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn error_msg(id: Json, code: i64, message: &str) -> Json {
    Json::obj(vec![
        ("jsonrpc", Json::s("2.0")),
        ("id", id),
        (
            "error",
            Json::obj(vec![("code", Json::Num(code as f64)), ("message", Json::s(message))]),
        ),
    ])
}

struct State {
    interp: Interp,
}

impl State {
    fn new(session: Session) -> State {
        State {
            interp: Interp::new(World::new(48, 20), session),
        }
    }

    /// Returns Ok(Some(result)) for a request, Ok(None) for a handled
    /// notification, or Err((code, msg)) for a protocol error.
    fn handle(&mut self, method: &str, params: &Json) -> Result<Option<Json>, (i64, String)> {
        match method {
            "initialize" => Ok(Some(Json::obj(vec![
                ("protocolVersion", Json::s(PROTOCOL_VERSION)),
                ("capabilities", Json::obj(vec![("tools", Json::obj(vec![]))])),
                (
                    "serverInfo",
                    Json::obj(vec![
                        ("name", Json::s(SERVER_NAME)),
                        ("version", Json::s(SERVER_VERSION)),
                    ]),
                ),
            ]))),
            "notifications/initialized" | "initialized" => Ok(None),
            "ping" => Ok(Some(Json::Obj(vec![]))),
            "tools/list" => Ok(Some(Json::obj(vec![("tools", tool_defs())]))),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or((-32602, "missing tool name".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Json::Obj(vec![]));
                match self.run_tool(name, &args) {
                    Some(Ok(text)) => Ok(Some(tool_content(&text, false))),
                    Some(Err(msg)) => Ok(Some(tool_content(&msg, true))),
                    None => Err((-32602, format!("unknown tool `{name}`"))),
                }
            }
            other => Err((-32601, format!("method not found: {other}"))),
        }
    }

    /// `None` = unknown tool; `Some(Ok)` = success text; `Some(Err)` = tool error.
    fn run_tool(&mut self, name: &str, args: &Json) -> Option<Result<String, String>> {
        Some(match name {
            "vsesam_eval" => {
                let src = match args.get("source").and_then(|s| s.as_str()) {
                    Some(s) => s.to_string(),
                    None => return Some(Err("нужен аргумент `source`".into())),
                };
                match self.interp.run_source(&src) {
                    Ok(()) => Ok(self.snapshot()),
                    Err(e) => Err(e),
                }
            }
            "vsesam_spawn" => {
                let kind = args.get("kind").and_then(|s| s.as_str()).unwrap_or("goblin");
                let x = args.get("x").and_then(|n| n.as_f64()).unwrap_or(0.0);
                let y = args.get("y").and_then(|n| n.as_f64()).unwrap_or(0.0);
                let id = self.interp.world.spawn(kind, x, y);
                Ok(format!("заспавнен `{kind}` id={id} в ({x}, {y})\n{}", self.snapshot()))
            }
            "vsesam_tick" => {
                let count = args
                    .get("count")
                    .and_then(|n| n.as_f64())
                    .unwrap_or(1.0)
                    .clamp(1.0, 1000.0) as u64;
                for _ in 0..count {
                    self.interp.tick_world();
                }
                Ok(format!("прогнал {count} тик(ов)\n{}", self.snapshot()))
            }
            "vsesam_frame" => Ok(self.snapshot()),
            "vsesam_stats" => Ok(self.stats()),
            "vsesam_reset" => {
                let w = args.get("width").and_then(|n| n.as_f64()).unwrap_or(48.0) as usize;
                let h = args.get("height").and_then(|n| n.as_f64()).unwrap_or(20.0) as usize;
                let session = std::mem::replace(&mut self.interp.session, Session::guest());
                self.interp = Interp::new(World::new(w.max(4), h.max(4)), session);
                Ok(format!("мир сброшен ({w}x{h})\n{}", self.snapshot()))
            }
            "vsesam_annotate" => {
                let src = match args.get("source").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => return Some(Err("нужен аргумент `source`".into())),
                };
                match nanch::parse_program(src) {
                    Ok(stmts) => Ok(printer::annotate(&stmts)),
                    Err(e) => Err(e),
                }
            }
            "vsesam_seal" => {
                let src = match args.get("source").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => return Some(Err("нужен аргумент `source`".into())),
                };
                let pass = args
                    .get("pass")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.interp.session.token.clone());
                let v = vault::Vault::seal(src, pass.as_bytes());
                Ok(v.to_text())
            }
            _ => return None,
        })
    }

    fn snapshot(&mut self) -> String {
        let mut s = String::new();
        for l in self.interp.drain_log() {
            s.push_str("• ");
            s.push_str(&l);
            s.push('\n');
        }
        s.push_str(&render::frame(&self.interp.world));
        s.push('\n');
        s.push_str(&format!(
            "тик {} · сущностей {} · друзья {} враги {} · сессия {}",
            self.interp.tick,
            self.interp.world.alive_count(),
            self.interp.world.count_faction(0),
            self.interp.world.count_faction(1),
            self.interp.session.user
        ));
        s
    }

    fn stats(&self) -> String {
        let mut s = format!(
            "сущностей: {} · друзья: {} · враги: {} · тик: {}\n",
            self.interp.world.alive_count(),
            self.interp.world.count_faction(0),
            self.interp.world.count_faction(1),
            self.interp.tick
        );
        for e in self.interp.world.entities.iter().filter(|e| e.alive) {
            s.push_str(&format!(
                "  #{} {:7} ({:>4.1},{:>4.1}) hp {:.0}/{:.0} фракция {}{}\n",
                e.id,
                e.kind,
                e.x,
                e.y,
                e.hp,
                e.max_hp,
                e.faction,
                if e.label.is_empty() {
                    String::new()
                } else {
                    format!(" «{}»", e.label)
                }
            ));
        }
        s
    }
}

// ---- tool result + schema helpers --------------------------------------

fn tool_content(text: &str, is_error: bool) -> Json {
    Json::obj(vec![
        (
            "content",
            Json::Arr(vec![Json::obj(vec![
                ("type", Json::s("text")),
                ("text", Json::s(text)),
            ])]),
        ),
        ("isError", Json::Bool(is_error)),
    ])
}

fn prop(ty: &str, desc: &str) -> Json {
    Json::obj(vec![("type", Json::s(ty)), ("description", Json::s(desc))])
}

fn schema(props: Vec<(&str, Json)>, required: &[&str]) -> Json {
    Json::obj(vec![
        ("type", Json::s("object")),
        ("properties", Json::obj(props)),
        (
            "required",
            Json::Arr(required.iter().map(|r| Json::s(*r)).collect()),
        ),
    ])
}

fn tool(name: &str, desc: &str, input: Json) -> Json {
    Json::obj(vec![
        ("name", Json::s(name)),
        ("description", Json::s(desc)),
        ("inputSchema", input),
    ])
}

fn tool_defs() -> Json {
    Json::Arr(vec![
        tool(
            "vsesam_eval",
            "Выполнить nanch-код в общем мире (setup). Возвращает журнал и кадр.",
            schema(vec![("source", prop("string", "исходник на nanch"))], &["source"]),
        ),
        tool(
            "vsesam_spawn",
            "Заспавнить сущность (goblin/orc/agent/player/slime/wall/gold). Возвращает id.",
            schema(
                vec![
                    ("kind", prop("string", "вид сущности")),
                    ("x", prop("number", "координата x")),
                    ("y", prop("number", "координата y")),
                ],
                &["kind", "x", "y"],
            ),
        ),
        tool(
            "vsesam_tick",
            "Прогнать симуляцию на N тиков (AI-агенты, движение, бой).",
            schema(vec![("count", prop("number", "число тиков (по умолч. 1)"))], &[]),
        ),
        tool(
            "vsesam_frame",
            "Отрисовать текущий кадр мира с HUD.",
            schema(vec![], &[]),
        ),
        tool(
            "vsesam_stats",
            "Список живых сущностей с координатами, здоровьем и фракцией.",
            schema(vec![], &[]),
        ),
        tool(
            "vsesam_reset",
            "Сбросить мир (новая арена), сессия сохраняется.",
            schema(
                vec![
                    ("width", prop("number", "ширина арены")),
                    ("height", prop("number", "высота арены")),
                ],
                &[],
            ),
        ),
        tool(
            "vsesam_annotate",
            "Авто-комментатор: вернуть nanch-код с комментариями из AST.",
            schema(vec![("source", prop("string", "исходник на nanch"))], &["source"]),
        ),
        tool(
            "vsesam_seal",
            "Зашифровать nanch-код в 67-слойный VSESAM-VAULT (ключ — пароль или токен сессии).",
            schema(
                vec![
                    ("source", prop("string", "исходник на nanch")),
                    ("pass", prop("string", "пароль (по умолч. токен сессии)")),
                ],
                &["source"],
            ),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(Session::guest())
    }

    #[test]
    fn initialize_and_tools_list() {
        let mut st = state();
        let init = st.handle("initialize", &Json::Obj(vec![])).unwrap().unwrap();
        assert_eq!(init.get("protocolVersion").unwrap().as_str(), Some(PROTOCOL_VERSION));
        let list = st.handle("tools/list", &Json::Obj(vec![])).unwrap().unwrap();
        let tools = list.get("tools").unwrap().as_array().unwrap();
        assert!(tools.len() >= 8);
        assert!(tools.iter().any(|t| t.get("name").unwrap().as_str() == Some("vsesam_eval")));
    }

    #[test]
    fn notification_has_no_reply() {
        let mut st = state();
        assert!(st.handle("notifications/initialized", &Json::Obj(vec![])).unwrap().is_none());
    }

    #[test]
    fn tool_call_eval_then_tick() {
        let mut st = state();
        let call = Json::obj(vec![
            ("name", Json::s("vsesam_eval")),
            (
                "arguments",
                Json::obj(vec![("source", Json::s("生(\"goblin\", 2, 2)؛ 脑(0, \"seek\")؛ 生(\"player\", 20, 2)؛"))]),
            ),
        ]);
        let res = st.handle("tools/call", &call).unwrap().unwrap();
        assert_eq!(res.get("isError").unwrap(), &Json::Bool(false));

        let tick = Json::obj(vec![
            ("name", Json::s("vsesam_tick")),
            ("arguments", Json::obj(vec![("count", Json::Num(3.0))])),
        ]);
        let res = st.handle("tools/call", &tick).unwrap().unwrap();
        let text = res.get("content").unwrap().as_array().unwrap()[0]
            .get("text")
            .unwrap()
            .as_str()
            .unwrap();
        assert!(text.contains("тик"));
    }

    #[test]
    fn unknown_tool_is_protocol_error() {
        let mut st = state();
        let call = Json::obj(vec![("name", Json::s("nope"))]);
        assert!(st.handle("tools/call", &call).is_err());
    }

    #[test]
    fn eval_error_is_tool_error_not_protocol_error() {
        let mut st = state();
        let call = Json::obj(vec![
            ("name", Json::s("vsesam_eval")),
            ("arguments", Json::obj(vec![("source", Json::s("это не ؛ валидно {{{"))])),
        ]);
        let res = st.handle("tools/call", &call).unwrap().unwrap();
        assert_eq!(res.get("isError").unwrap(), &Json::Bool(true));
    }
}
