//! VSESAM — a Rust game engine driven by the `nanch` scripting language, with
//! built-in AI agents and session-embedded login.
//!
//! Usage:
//!   vsesam                         show the banner + help
//!   vsesam register <user> <pass>  create an account
//!   vsesam login <user> <pass>     log in (prints + remembers a session token)
//!   vsesam logout                  forget the current session
//!   vsesam whoami [--as <token>]   show the active session
//!   vsesam run  <script.nanch> [--ticks N] [--size WxH] [--as <token>]
//!   vsesam play <script.nanch> [--ticks N] [--size WxH] [--as <token>]
//!   vsesam repl [--as <token>]     interactive nanch prompt
//!   vsesam demo                    run the bundled arena demo

mod ai;
mod auth;
mod engine;
mod nanch;

use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

use auth::Session;
use engine::render;
use engine::world::World;
use nanch::Interp;

/// The bundled demo, compiled into the binary so `vsesam demo` always works.
const ARENA: &str = include_str!("../scripts/arena.nanch");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    match cmd {
        "" | "help" | "-h" | "--help" => {
            banner();
            help();
            if !auth::initialized() {
                println!("аккаунтов пока нет — начни с:  vsesam register <user> <pass>");
            } else if let Some(tok) = auth::current_token() {
                println!("текущая сессия: {}…  (vsesam whoami)", auth::short(&tok));
            }
            0
        }
        "register" => cmd_register(rest),
        "login" => cmd_login(rest),
        "logout" => {
            auth::logout();
            println!("вышел. сессия забыта.");
            0
        }
        "whoami" => cmd_whoami(rest),
        "run" => cmd_run(rest, false),
        "play" => cmd_run(rest, true),
        "demo" => cmd_demo(rest),
        "repl" => cmd_repl(rest),
        other => {
            eprintln!("неизвестная команда `{other}`. попробуй `vsesam help`.");
            2
        }
    }
}

// ---- banner / help -----------------------------------------------------

fn banner() {
    println!(
        r#"
 ╦  ╦╔═╗╔═╗╦ ╦╔═╗╔═╗╔╦╗   VSESAM — движок на Rust + язык nanch
 ╚╗╔╝╚═╗║╣ ╚═╗║ ║║ ║║║║   арабо-китайский синтаксис · AI-агенты · сессии
  ╚╝ ╚═╝╚═╝╚═╝╚═╝╚═╝╩ ╩   всё сам. ؛"#
    );
}

fn help() {
    println!(
        r#"
команды:
  register <user> <pass>      создать аккаунт
  login    <user> <pass>      войти (печатает и запоминает токен сессии)
  logout                      забыть текущую сессию
  whoami   [--as <token>]     показать активную сессию
  run  <script> [opts]        прогнать сцену и печатать кадры (не анимировано)
  play <script> [opts]        анимированный прогон (очистка экрана, кадры)
  demo [opts]                 встроенная арена с AI-агентами
  repl [--as <token>]         интерактивный nanch

опции run/play/demo:
  --ticks N        число тиков симуляции (по умолчанию 40 / play 80)
  --size WxH       размер арены (по умолчанию 48x20)
  --as <token>     запустить под конкретной сессией (иначе — текущая/гость)

язык nanch (кратко):
  设 x = 5؛                объявить переменную,   ؛ завершает выражение
  函 имя(a,b) {{ 回 a+b؛ }}  функция и возврат (回)
  如果 (c) {{…}} 否则 {{…}}   if / else
  طالما (c) {{…}}            while
  喊("привет", x)؛          печать
  生("goblin", 5, 5)؛       заспавнить сущность -> id
  脑(id, "think")؛          назначить AI-мозг (think/seek/flee/hunt/wander/patrol)
  脑(id, мояФункция)؛       или собственный мозг-агент на nanch
  记("score", 10)؛ 取("score")؛   сессия: сохранить / прочитать (встраивается в движок)
"#
    );
}

// ---- account commands --------------------------------------------------

fn cmd_register(args: &[String]) -> i32 {
    let (pos, _flags) = split_flags(args);
    if pos.len() < 2 {
        eprintln!("использование: vsesam register <user> <pass>");
        return 2;
    }
    match auth::register(&pos[0], &pos[1]) {
        Ok(()) => {
            println!("аккаунт `{}` создан. теперь: vsesam login {} <pass>", pos[0], pos[0]);
            0
        }
        Err(e) => {
            eprintln!("ошибка регистрации: {e}");
            1
        }
    }
}

fn cmd_login(args: &[String]) -> i32 {
    let (pos, _flags) = split_flags(args);
    if pos.len() < 2 {
        eprintln!("использование: vsesam login <user> <pass>");
        return 2;
    }
    match Session::login(&pos[0], &pos[1]) {
        Ok(session) => {
            if let Err(e) = session.save() {
                eprintln!("не смог сохранить сессию: {e}");
                return 1;
            }
            println!("вошёл как `{}`.", session.user);
            println!("токен сессии: {}", session.token);
            println!("он запомнен — команды run/play/repl подхватят его сами.");
            println!("сохранённых ключей в сессии: {}", session.kv.len());
            0
        }
        Err(e) => {
            eprintln!("не вошёл: {e}");
            1
        }
    }
}

fn cmd_whoami(args: &[String]) -> i32 {
    let (_pos, flags) = split_flags(args);
    let session = resolve_session(&flags);
    println!("пользователь: {}", session.user);
    println!("токен:        {}", session.token);
    println!("ключей:       {}", session.kv.len());
    if !session.kv.is_empty() {
        let mut keys: Vec<&String> = session.kv.keys().collect();
        keys.sort();
        for k in keys {
            println!("  {} = {}", k, session.kv[k]);
        }
    }
    0
}

// ---- running scripts ---------------------------------------------------

fn cmd_run(args: &[String], animated: bool) -> i32 {
    let (pos, flags) = split_flags(args);
    let path = match pos.first() {
        Some(p) => p,
        None => {
            eprintln!("использование: vsesam {} <script.nanch> [opts]", if animated { "play" } else { "run" });
            return 2;
        }
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("не прочитал `{path}`: {e}");
            return 1;
        }
    };
    run_scene(&src, path, &flags, animated)
}

fn cmd_demo(args: &[String]) -> i32 {
    let (_pos, flags) = split_flags(args);
    run_scene(ARENA, "<встроенная арена>", &flags, flag_has(&flags, "play"))
}

fn run_scene(src: &str, label: &str, flags: &[(String, Option<String>)], animated: bool) -> i32 {
    let (w, h) = parse_size(flags).unwrap_or((48, 20));
    let default_ticks = if animated { 80 } else { 40 };
    let ticks = parse_usize(flags, "ticks").unwrap_or(default_ticks);

    let mut session = resolve_session(flags);
    let keys_before = session.kv.len();

    let world = World::new(w, h);
    let mut interp = Interp::new(world, session_take(&mut session));

    println!("── VSESAM ── сцена: {label}");
    println!("сессия: {} · ключей на входе: {}", interp.session.user, keys_before);

    if let Err(e) = interp.run_source(src) {
        eprintln!("\n{e}");
        return 1;
    }

    // setup frame
    if animated {
        print!("{}", render::clear());
    }
    print_frame(&interp, 0, ticks);
    flush_log(&mut interp);

    let sample = (ticks / 12).max(1);
    for t in 1..=ticks {
        interp.tick_world();
        let show = animated || t == ticks || t % sample == 0;
        if show {
            if animated {
                print!("{}", render::clear());
            }
            print_frame(&interp, t, ticks);
        }
        flush_log(&mut interp);
        if animated {
            let _ = io::stdout().flush();
            thread::sleep(Duration::from_millis(90));
        }
    }

    // Persist the session: the state the script wrote via 记 is now embedded
    // and will ride back into the engine on the next login.
    match interp.session.save() {
        Ok(()) if interp.session.persistent => {
            println!("\n✔ сессия `{}` сохранена ({} ключей).", interp.session.user, interp.session.kv.len());
        }
        Ok(()) => {
            println!("\n(гостевая сессия — не сохраняется. войди, чтобы прогресс встроился в движок.)");
        }
        Err(e) => eprintln!("не сохранил сессию: {e}"),
    }
    0
}

fn print_frame(interp: &Interp, tick: usize, total: usize) {
    println!("{}", render::frame(&interp.world));
    let friends = interp.world.count_faction(0);
    let foes = interp.world.count_faction(1);
    println!(
        "тик {tick}/{total} · сущностей {} · друзья {friends} враги {foes} · сессия {}",
        interp.world.alive_count(),
        interp.session.user
    );
}

fn flush_log(interp: &mut Interp) {
    for line in interp.drain_log() {
        println!("  {line}");
    }
}

// ---- REPL --------------------------------------------------------------

fn cmd_repl(args: &[String]) -> i32 {
    let (_pos, flags) = split_flags(args);
    let mut session = resolve_session(&flags);
    let world = World::new(48, 20);
    let mut interp = Interp::new(world, session_take(&mut session));

    banner();
    println!("\nnanch REPL · сессия: {} · `:тик` = шаг симуляции, `:кадр` = отрисовать, `:выход`", interp.session.user);
    println!("пример:  设 g = 生(\"goblin\", 10, 5)؛   потом   脑(g, \"wander\")؛   потом   :тик\n");

    let stdin = io::stdin();
    loop {
        print!("nanch؛ ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("ошибка ввода: {e}");
                break;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed {
            ":выход" | ":q" | ":quit" => break,
            ":кадр" | ":frame" => {
                println!("{}", render::frame(&interp.world));
                continue;
            }
            ":тик" | ":tick" => {
                interp.tick_world();
                flush_log(&mut interp);
                println!("{}", render::frame(&interp.world));
                continue;
            }
            ":сохр" | ":save" => {
                match interp.session.save() {
                    Ok(()) => println!("сессия сохранена."),
                    Err(e) => eprintln!("{e}"),
                }
                continue;
            }
            _ => {}
        }
        match interp.eval_line(trimmed) {
            Ok(Some(v)) => println!("=> {v}"),
            Ok(None) => {}
            Err(e) => eprintln!("{e}"),
        }
        flush_log(&mut interp);
    }
    let _ = interp.session.save();
    println!("пока.");
    0
}

// ---- session plumbing --------------------------------------------------

fn resolve_session(flags: &[(String, Option<String>)]) -> Session {
    if let Some(tok) = flag_value(flags, "as") {
        match Session::by_token(&tok) {
            Ok(s) => return s,
            Err(e) => {
                eprintln!("(--as: {e}; продолжаю гостем)");
                return Session::guest();
            }
        }
    }
    if let Some(tok) = auth::current_token() {
        if let Ok(s) = Session::by_token(&tok) {
            return s;
        }
    }
    Session::guest()
}

/// Move a Session out of a holder, leaving a guest placeholder behind.
fn session_take(slot: &mut Session) -> Session {
    std::mem::replace(slot, Session::guest())
}

// ---- argument parsing --------------------------------------------------

/// Split argv into positional args and `--flag[=value | value]` pairs.
fn split_flags(args: &[String]) -> (Vec<String>, Vec<(String, Option<String>)>) {
    let mut pos = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if let Some((k, v)) = name.split_once('=') {
                flags.push((k.to_string(), Some(v.to_string())));
            } else if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                flags.push((name.to_string(), Some(args[i + 1].clone())));
                i += 1;
            } else {
                flags.push((name.to_string(), None));
            }
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    (pos, flags)
}

fn flag_value(flags: &[(String, Option<String>)], key: &str) -> Option<String> {
    flags.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.clone())
}

fn flag_has(flags: &[(String, Option<String>)], key: &str) -> bool {
    flags.iter().any(|(k, _)| k == key)
}

fn parse_usize(flags: &[(String, Option<String>)], key: &str) -> Option<usize> {
    flag_value(flags, key).and_then(|v| v.parse().ok())
}

fn parse_size(flags: &[(String, Option<String>)]) -> Option<(usize, usize)> {
    let s = flag_value(flags, "size")?;
    let (w, h) = s.split_once('x').or_else(|| s.split_once('X'))?;
    Some((w.parse().ok()?, h.parse().ok()?))
}
