//! Login, accounts, and sessions.
//!
//! A session's token is derived deterministically from the account, so logging
//! back in reattaches you to the *same* persistent key/value store. That store
//! is reachable from nanch via `记`/`取`, which is how a player's session state
//! gets "grabbed" and embedded straight into the running engine: your score,
//! inventory and flags survive across runs and ride into the simulation.

pub mod sha256;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha256::{sha256, sha256_hex};

const ROOT: &str = ".vsesam";

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn users_file() -> PathBuf {
    root().join("users.db")
}

fn sessions_dir() -> PathBuf {
    root().join("sessions")
}

fn current_file() -> PathBuf {
    root().join("current")
}

fn ensure_dirs() -> Result<(), String> {
    fs::create_dir_all(sessions_dir()).map_err(|e| format!("mkdir .vsesam: {e}"))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

struct Account {
    name: String,
    salt: String,
    hash: String,
}

fn load_accounts() -> Vec<Account> {
    let text = match fs::read_to_string(users_file()) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            Some(Account {
                name: parts.next()?.to_string(),
                salt: parts.next()?.to_string(),
                hash: parts.next()?.to_string(),
            })
        })
        .collect()
}

fn save_accounts(accts: &[Account]) -> Result<(), String> {
    ensure_dirs()?;
    let mut out = String::new();
    for a in accts {
        out.push_str(&format!("{}\t{}\t{}\n", a.name, a.salt, a.hash));
    }
    fs::write(users_file(), out).map_err(|e| format!("write users.db: {e}"))
}

fn gen_salt(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{name}:{nanos}:{}", std::process::id());
    sha256_hex(seed.as_bytes())[..32].to_string()
}

/// Password KDF: salted SHA-256 with 4096 reinforcing rounds. Not production
/// crypto, but honest and dependency-free.
fn hash_pw(salt: &str, pass: &str) -> String {
    let mut h = sha256(format!("{salt}{pass}").as_bytes()).to_vec();
    for _ in 0..4096 {
        let mut buf = h.clone();
        buf.extend_from_slice(pass.as_bytes());
        buf.extend_from_slice(salt.as_bytes());
        h = sha256(&buf).to_vec();
    }
    to_hex(&h)
}

/// Stable per-account session token. Same account ⇒ same token ⇒ same KV store.
fn token_for(name: &str, salt: &str) -> String {
    sha256_hex(format!("vsesam-session\u{0}{name}\u{0}{salt}").as_bytes())[..40].to_string()
}

/// Register a new account. Fails if the name already exists.
pub fn register(name: &str, pass: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("empty user name".into());
    }
    if name.contains('\t') || name.contains('\n') {
        return Err("user name may not contain tab/newline".into());
    }
    let mut accts = load_accounts();
    if accts.iter().any(|a| a.name == name) {
        return Err(format!("user `{name}` already exists"));
    }
    let salt = gen_salt(name);
    let hash = hash_pw(&salt, pass);
    accts.push(Account {
        name: name.to_string(),
        salt,
        hash,
    });
    save_accounts(&accts)
}

/// A live, engine-embeddable session.
pub struct Session {
    pub user: String,
    pub token: String,
    pub kv: HashMap<String, String>,
    pub persistent: bool,
}

impl Session {
    /// Anonymous session — nothing is written to disk.
    pub fn guest() -> Session {
        Session {
            user: "гость".to_string(),
            token: "guest".to_string(),
            kv: HashMap::new(),
            persistent: false,
        }
    }

    /// Authenticate and attach to the account's persistent store.
    pub fn login(name: &str, pass: &str) -> Result<Session, String> {
        let acct = load_accounts()
            .into_iter()
            .find(|a| a.name == name)
            .ok_or_else(|| format!("no such user `{name}`"))?;
        if hash_pw(&acct.salt, pass) != acct.hash {
            return Err("wrong password".into());
        }
        let token = token_for(&acct.name, &acct.salt);
        let kv = load_session_kv(&token);
        Ok(Session {
            user: acct.name,
            token,
            kv,
            persistent: true,
        })
    }

    /// Reattach to an existing session purely by its token (no password).
    pub fn by_token(token: &str) -> Result<Session, String> {
        if token == "guest" {
            return Ok(Session::guest());
        }
        let path = sessions_dir().join(format!("{token}.sess"));
        let text = fs::read_to_string(&path)
            .map_err(|_| format!("no live session for token `{}…`", short(token)))?;
        let mut lines = text.lines();
        let user = lines.next().unwrap_or("гость").to_string();
        let mut kv = HashMap::new();
        for l in lines {
            if l.is_empty() {
                continue;
            }
            if let Some((k, v)) = l.split_once('\t') {
                kv.insert(k.to_string(), unescape(v));
            }
        }
        Ok(Session {
            user,
            token: token.to_string(),
            kv,
            persistent: true,
        })
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.kv.get(key)
    }

    pub fn set(&mut self, key: String, value: String) {
        self.kv.insert(key, value);
    }

    /// Flush the KV store to disk and mark this token as current.
    pub fn save(&self) -> Result<(), String> {
        if !self.persistent {
            return Ok(());
        }
        ensure_dirs()?;
        let mut out = String::new();
        out.push_str(&self.user);
        out.push('\n');
        let mut keys: Vec<&String> = self.kv.keys().collect();
        keys.sort();
        for k in keys {
            let v = &self.kv[k];
            out.push_str(&format!("{}\t{}\n", k, escape(v)));
        }
        let path = sessions_dir().join(format!("{}.sess", self.token));
        fs::write(&path, out).map_err(|e| format!("write session: {e}"))?;
        fs::write(current_file(), &self.token).ok();
        Ok(())
    }
}

fn load_session_kv(token: &str) -> HashMap<String, String> {
    let path = sessions_dir().join(format!("{token}.sess"));
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    let mut kv = HashMap::new();
    for l in text.lines().skip(1) {
        if let Some((k, v)) = l.split_once('\t') {
            kv.insert(k.to_string(), unescape(v));
        }
    }
    kv
}

/// Token recorded by the most recent `login`, if any.
pub fn current_token() -> Option<String> {
    fs::read_to_string(current_file())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn logout() {
    let _ = fs::remove_file(current_file());
}

pub fn short(token: &str) -> String {
    token.chars().take(10).collect()
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Does the account store exist yet?
pub fn initialized() -> bool {
    Path::new(&users_file()).exists()
}

// ---- SSO via a coding-assistant session --------------------------------
//
// "Login by your Claude Code / OpenAI Codex session." IMPORTANT, on purpose:
// this reads only NON-SECRET identifiers from environment variables (or an
// explicit value you pass), hashes them locally into an opaque account handle,
// stores only that locally, and transmits NOTHING — the whole engine makes no
// network calls. It never reads secret token files and never impersonates you
// to any provider. It is single-sign-on by local identity, not credential
// theft.

/// Detect a coding-assistant session identity from the environment.
/// Returns `(provider_label, raw_identity)` or `None` if nothing is present.
pub fn detect_sso() -> Option<(String, String)> {
    use std::env::var;

    // 1. Explicit override wins.
    if let Ok(s) = var("VSESAM_AI_SESSION") {
        if !s.is_empty() {
            let prov = var("VSESAM_AI_PROVIDER").unwrap_or_else(|_| "ai".into());
            return Some((prov, s));
        }
    }

    // 2. Claude Code.
    for m in ["CLAUDE_SESSION_ID", "CLAUDE_CODE_SESSION_ID", "ANTHROPIC_SESSION_ID"] {
        if let Ok(s) = var(m) {
            if !s.is_empty() {
                return Some(("claude".into(), s));
            }
        }
    }
    if var("CLAUDECODE").is_ok() || var("CLAUDE_CODE_ENTRYPOINT").is_ok() {
        let who = var("USER").or_else(|_| var("USERNAME")).unwrap_or_else(|_| "local".into());
        return Some(("claude".into(), format!("claudecode:{who}")));
    }

    // 3. OpenAI Codex / similar.
    for m in ["CODEX_SESSION_ID", "OPENAI_SESSION_ID", "CODEX_SESSION"] {
        if let Ok(s) = var(m) {
            if !s.is_empty() {
                return Some(("codex".into(), s));
            }
        }
    }
    if var("CODEX").is_ok() || var("OPENAI_CODEX").is_ok() {
        let who = var("USER").or_else(|_| var("USERNAME")).unwrap_or_else(|_| "local".into());
        return Some(("codex".into(), format!("codex:{who}")));
    }

    None
}

/// Build (or reattach to) a passwordless SSO session from a provider identity.
/// The identity is hashed; the raw value is never stored.
pub fn sso_login(provider: &str, raw_identity: &str) -> Session {
    let handle = format!("{provider}:{}", &sha256_hex(raw_identity.as_bytes())[..12]);
    let token = token_for_sso(provider, raw_identity);
    let kv = load_session_kv(&token);
    Session {
        user: handle,
        token,
        kv,
        persistent: true,
    }
}

fn token_for_sso(provider: &str, raw: &str) -> String {
    sha256_hex(format!("vsesam-sso\u{0}{provider}\u{0}{raw}").as_bytes())[..40].to_string()
}
