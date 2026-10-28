//! Minimal, dependency-free JSON for the MCP server. Order-preserving objects
//! (JSON-RPC cares about nothing here, but stable key order keeps output
//! readable and tests deterministic).

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn s<S: Into<String>>(s: S) -> Json {
        Json::Str(s.into())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    // Used by the MCP test suite to introspect tool/list results.
    #[allow(dead_code)]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    /// Compact serialization.
    pub fn to_string(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => {
                if n.is_finite() {
                    if n.fract() == 0.0 && n.abs() < 1e15 {
                        let _ = write!(out, "{}", *n as i64);
                    } else {
                        let _ = write!(out, "{}", n);
                    }
                } else {
                    out.push_str("null"); // JSON has no NaN/Inf
                }
            }
            Json::Str(s) => write_json_string(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    it.write(out);
                }
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---- parser -------------------------------------------------------------

pub fn parse(src: &str) -> Result<Json, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut p = P { chars, i: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.chars.len() {
        return Err(format!("trailing junk at position {}", p.i));
    }
    Ok(v)
}

struct P {
    chars: Vec<char>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Json::Str(self.string()?)),
            Some('t') | Some('f') => self.boolean(),
            Some('n') => self.null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected char `{c}` at {}", self.i)),
            None => Err("unexpected end of input".into()),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.i += 1; // {
        let mut pairs = Vec::new();
        self.ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(Json::Obj(pairs));
        }
        loop {
            self.ws();
            if self.peek() != Some('"') {
                return Err(format!("expected string key at {}", self.i));
            }
            let key = self.string()?;
            self.ws();
            if self.peek() != Some(':') {
                return Err(format!("expected `:` at {}", self.i));
            }
            self.i += 1;
            let val = self.value()?;
            pairs.push((key, val));
            self.ws();
            match self.peek() {
                Some(',') => {
                    self.i += 1;
                }
                Some('}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected `,` or `}}` at {}", self.i)),
            }
        }
        Ok(Json::Obj(pairs))
    }

    fn array(&mut self) -> Result<Json, String> {
        self.i += 1; // [
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            let val = self.value()?;
            items.push(val);
            self.ws();
            match self.peek() {
                Some(',') => {
                    self.i += 1;
                }
                Some(']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected `,` or `]` at {}", self.i)),
            }
        }
        Ok(Json::Arr(items))
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some('"') => {
                    self.i += 1;
                    break;
                }
                Some('\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some('/') => s.push('/'),
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('b') => s.push('\u{0008}'),
                        Some('f') => s.push('\u{000C}'),
                        Some('u') => {
                            let cp = self.hex4()?;
                            // basic BMP handling (no surrogate pairs — fine for our use)
                            if let Some(c) = char::from_u32(cp) {
                                s.push(c);
                            }
                            continue; // hex4 advanced i past the 4 digits
                        }
                        Some(c) => return Err(format!("bad escape `\\{c}`")),
                        None => return Err("unterminated escape".into()),
                    }
                    self.i += 1;
                }
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(s)
    }

    fn hex4(&mut self) -> Result<u32, String> {
        self.i += 1; // past 'u'
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.peek().ok_or("short \\u escape")?;
            let d = c.to_digit(16).ok_or("bad \\u hex digit")?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    fn boolean(&mut self) -> Result<Json, String> {
        if self.starts_with("true") {
            self.i += 4;
            Ok(Json::Bool(true))
        } else if self.starts_with("false") {
            self.i += 5;
            Ok(Json::Bool(false))
        } else {
            Err(format!("invalid literal at {}", self.i))
        }
    }

    fn null(&mut self) -> Result<Json, String> {
        if self.starts_with("null") {
            self.i += 4;
            Ok(Json::Null)
        } else {
            Err(format!("invalid literal at {}", self.i))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                self.i += 1;
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.i].iter().collect();
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number `{text}`"))
    }

    fn starts_with(&self, kw: &str) -> bool {
        kw.chars()
            .enumerate()
            .all(|(k, c)| self.chars.get(self.i + k) == Some(&c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_serialize() {
        let v = parse(r#"{"a":1,"b":[true,null,"x"],"c":{"d":-2.5}}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_f64(), Some(1.0));
        assert_eq!(v.get("b").unwrap().as_array().unwrap().len(), 3);
        assert_eq!(v.get("c").unwrap().get("d").unwrap().as_f64(), Some(-2.5));
    }

    #[test]
    fn escapes_roundtrip() {
        let v = parse(r#"{"k":"line1\nline2\t\"q\"A"}"#).unwrap();
        assert_eq!(v.get("k").unwrap().as_str(), Some("line1\nline2\t\"q\"A"));
        let out = v.to_string();
        let reparsed = parse(&out).unwrap();
        assert_eq!(v, reparsed);
    }

    #[test]
    fn rejects_trailing_junk() {
        assert!(parse("{} nonsense").is_err());
        assert!(parse("[1,2,]").is_err());
    }

    #[test]
    fn empty_containers() {
        assert_eq!(parse("{}").unwrap(), Json::Obj(vec![]));
        assert_eq!(parse("[]").unwrap(), Json::Arr(vec![]));
    }
}
