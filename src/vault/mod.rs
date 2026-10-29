//! VSESAM Vault — 67-слойное шифрование nanch-скриптов.
//!
//! Исходник прогоняется через `ROUNDS` (= 67) раундов: в каждом — потоковый
//! XOR-шифр в режиме счётчика (keystream из SHA-256) плюс ключезависимый сдвиг
//! всего буфера. Ключ раундов выводится из пароля + соли. По умолчанию пароль —
//! это токен твоей сессии, поэтому «код открывается твоим логином».
//!
//! Первая строка хранится в открытом виде как бесплатный preview — остальное
//! заперто (см. `view`/`open`). Всё локально, оффлайн: ни одного сетевого
//! вызова во всём движке.

use crate::auth::sha256::sha256;

/// Сколько слоёв шифрования накладывать. 67, как заказано.
pub const ROUNDS: usize = 67;
/// Версия формата.
pub const FORMAT: &str = "6767";
const MAGIC: [u8; 4] = *b"VS67";

/// Ключ одного раунда: SHA-256(пароль ‖ соль ‖ номер-раунда ‖ тег).
fn round_key(pass: &[u8], salt: &[u8], round: usize) -> [u8; 32] {
    let mut buf = Vec::with_capacity(pass.len() + salt.len() + 24);
    buf.extend_from_slice(pass);
    buf.extend_from_slice(salt);
    buf.extend_from_slice(&(round as u64).to_le_bytes());
    buf.extend_from_slice(b"vsesam-vault-6767");
    sha256(&buf)
}

/// Потоковый XOR в режиме счётчика: блок = SHA-256(round_key ‖ индекс-блока).
/// XOR — собственная инверсия, так что та же функция и шифрует, и расшифровывает.
fn xor_ctr(data: &mut [u8], rk: &[u8; 32]) {
    let mut pos = 0;
    while pos < data.len() {
        let blk_idx = (pos / 32) as u64;
        let mut buf = [0u8; 40];
        buf[..32].copy_from_slice(rk);
        buf[32..].copy_from_slice(&blk_idx.to_le_bytes());
        let block = sha256(&buf);
        let n = (data.len() - pos).min(32);
        for j in 0..n {
            data[pos + j] ^= block[j];
        }
        pos += 32;
    }
}

/// Обрамление: MAGIC ‖ checksum(4) ‖ len(4) ‖ plaintext. Длина постоянна во
/// всех раундах, поэтому сдвиги обратимы один-в-один.
fn frame(plaintext: &[u8]) -> Vec<u8> {
    let sum = sha256(plaintext);
    let mut f = Vec::with_capacity(12 + plaintext.len());
    f.extend_from_slice(&MAGIC);
    f.extend_from_slice(&sum[..4]);
    f.extend_from_slice(&(plaintext.len() as u32).to_le_bytes());
    f.extend_from_slice(plaintext);
    f
}

fn unframe(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 {
        return Err("неверный ключ/сессия (повреждённый заголовок)".into());
    }
    if bytes[..4] != MAGIC {
        return Err("неверный ключ/сессия — не открылось".into());
    }
    let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let payload = &bytes[12..];
    if payload.len() != len {
        return Err("неверный ключ/сессия (длина не сходится)".into());
    }
    if sha256(payload)[..4] != bytes[4..8] {
        return Err("неверный ключ/сессия (контрольная сумма)".into());
    }
    Ok(payload.to_vec())
}

/// Наложить 67 слоёв шифрования.
fn seal_bytes(plaintext: &[u8], pass: &[u8], salt: &[u8]) -> Vec<u8> {
    let mut data = frame(plaintext);
    for r in 0..ROUNDS {
        let rk = round_key(pass, salt, r);
        xor_ctr(&mut data, &rk);
        if !data.is_empty() {
            let rot = (rk[0] as usize) % data.len();
            data.rotate_left(rot);
        }
    }
    data
}

/// Снять 67 слоёв (в обратном порядке).
fn open_bytes(cipher: &[u8], pass: &[u8], salt: &[u8]) -> Result<Vec<u8>, String> {
    let mut data = cipher.to_vec();
    for r in (0..ROUNDS).rev() {
        let rk = round_key(pass, salt, r);
        if !data.is_empty() {
            let rot = (rk[0] as usize) % data.len();
            data.rotate_right(rot);
        }
        xor_ctr(&mut data, &rk);
    }
    unframe(&data)
}

fn gen_salt() -> [u8; 8] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("salt:{nanos}:{}", std::process::id());
    let h = sha256(seed.as_bytes());
    let mut s = [0u8; 8];
    s.copy_from_slice(&h[..8]);
    s
}

/// Запечатанный скрипт плюс открытый preview первой строки.
pub struct Vault {
    pub salt: Vec<u8>,
    pub lines: usize,
    pub free: String,
    pub body: Vec<u8>,
}

impl Vault {
    /// Запечатать исходник паролем.
    pub fn seal(source: &str, pass: &[u8]) -> Vault {
        let salt = gen_salt();
        let body = seal_bytes(source.as_bytes(), pass, &salt);
        let free = source.lines().next().unwrap_or("").to_string();
        Vault {
            salt: salt.to_vec(),
            lines: source.lines().count().max(1),
            free,
            body,
        }
    }

    /// Расшифровать (нужен верный пароль/сессия).
    pub fn open(&self, pass: &[u8]) -> Result<String, String> {
        let bytes = open_bytes(&self.body, pass, &self.salt)?;
        String::from_utf8(bytes).map_err(|_| "расшифровано, но это не UTF-8".into())
    }

    /// Сериализовать в текстовый `.vault`-формат.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("VSESAM-VAULT {FORMAT}\n"));
        out.push_str(&format!("rounds: {ROUNDS}\n"));
        out.push_str(&format!("salt: {}\n", hex_encode(&self.salt)));
        out.push_str(&format!("lines: {}\n", self.lines));
        out.push_str(&format!("free: {}\n", self.free.replace('\n', " ")));
        out.push_str("--\n");
        let hex = hex_encode(&self.body);
        for chunk in hex.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).unwrap());
            out.push('\n');
        }
        out
    }

    /// Разобрать текстовый `.vault`.
    pub fn from_text(text: &str) -> Result<Vault, String> {
        let mut salt = Vec::new();
        let mut lines = 1usize;
        let mut free = String::new();
        let mut body_hex = String::new();
        let mut in_body = false;
        let mut saw_magic = false;
        for line in text.lines() {
            if in_body {
                body_hex.push_str(line.trim());
                continue;
            }
            if line.trim() == "--" {
                in_body = true;
                continue;
            }
            if let Some(rest) = line.strip_prefix("VSESAM-VAULT") {
                saw_magic = true;
                let _ = rest;
            } else if let Some(rest) = line.strip_prefix("salt:") {
                salt = hex_decode(rest.trim())?;
            } else if let Some(rest) = line.strip_prefix("lines:") {
                lines = rest.trim().parse().unwrap_or(1);
            } else if let Some(rest) = line.strip_prefix("free:") {
                free = rest.strip_prefix(' ').unwrap_or(rest).to_string();
            }
        }
        if !saw_magic {
            return Err("это не VSESAM-VAULT файл".into());
        }
        let body = hex_decode(&body_hex)?;
        Ok(Vault { salt, lines, free, body })
    }

    /// Фримиум-просмотр: первая строка бесплатно, остальное заперто.
    pub fn preview(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "🔒 VSESAM-VAULT · {ROUNDS} слоёв · {} строк · бесплатно видно 1 строку\n",
            self.lines
        ));
        out.push_str("────────────────────────────────────────\n");
        out.push_str(&format!("1 │ {}\n", self.free));
        let hidden = self.lines.saturating_sub(1);
        for i in 0..hidden {
            let bars = "▓".repeat(((self.free.len().max(8)) % 24) + 6 + (i % 7));
            out.push_str(&format!("{} │ {bars}\n", i + 2));
        }
        out.push_str("────────────────────────────────────────\n");
        out.push_str("разблокируй целиком:  vsesam open <file.vault>   (своей сессией/паролем)\n");
        out
    }
}

// ---- hex ----------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() % 2 != 0 {
        return Err("нечётная длина hex".into());
    }
    let mut out = Vec::with_capacity(clean.len() / 2);
    let val = |c: u8| -> Result<u8, String> {
        (c as char)
            .to_digit(16)
            .map(|d| d as u8)
            .ok_or_else(|| format!("не hex-символ `{}`", c as char))
    };
    for pair in clean.chunks(2) {
        out.push((val(pair[0])? << 4) | val(pair[1])?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_67_layers() {
        let src = "设 x = 6767؛\n喊(\"ssalom\", x)؛\n函 f(n) { 回 n * 2؛ }";
        let v = Vault::seal(src, b"my-session-token");
        assert_eq!(v.open(b"my-session-token").unwrap(), src);
    }

    #[test]
    fn wrong_key_fails() {
        let v = Vault::seal("设 secret = 1؛\n喊(secret)؛", b"right");
        assert!(v.open(b"wrong").is_err());
    }

    #[test]
    fn text_format_roundtrip() {
        let src = "喊(\"привет\")؛\n生(\"goblin\", 5, 5)؛\n脑(0, \"think\")؛";
        let v = Vault::seal(src, b"pass67");
        let text = v.to_text();
        let v2 = Vault::from_text(&text).unwrap();
        assert_eq!(v2.open(b"pass67").unwrap(), src);
        assert!(v2.open(b"nope").is_err());
    }

    #[test]
    fn preview_shows_only_first_line() {
        let src = "设 free_line = 1؛\n设 secret = 2؛\n设 hidden = 3؛";
        let v = Vault::seal(src, b"k");
        let p = v.preview();
        assert!(p.contains("free_line"));
        assert!(!p.contains("secret"));
        assert!(!p.contains("hidden"));
    }

    #[test]
    fn empty_and_unicode() {
        let v = Vault::seal("", b"k");
        assert_eq!(v.open(b"k").unwrap(), "");
        let src = "喊(\"中文 عربي кириллица 🎮\")؛";
        let v2 = Vault::seal(src, b"k");
        assert_eq!(v2.open(b"k").unwrap(), src);
    }
}
