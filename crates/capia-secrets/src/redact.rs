//! Redação central. Duas camadas: (1) valores **registrados** (exatos, também em base64/url-encoded);
//! (2) padrões conhecidos (cabeçalhos de auth, `key=`/`token=` em query, prefixos `sk-…`, `AIza…`).

use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

const MASK: &str = "[REDACTED]";
const MIN_SECRET_LEN: usize = 6;

#[derive(Default)]
pub struct SecretRegistry {
    values: Mutex<BTreeSet<String>>,
}

impl core::fmt::Debug for SecretRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecretRegistry").finish_non_exhaustive()
    }
}

impl SecretRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra um valor e suas formas codificadas comuns.
    pub fn register(&self, value: &str) {
        if value.len() < MIN_SECRET_LEN {
            return;
        }
        let Ok(mut set) = self.values.lock() else {
            return;
        };
        set.insert(value.to_owned());
        set.insert(base64_std(value.as_bytes()));
        set.insert(percent_encode(value));
    }

    /// Só os valores **registrados** (e suas codificações), sem os padrões heurísticos: para quem
    /// grava dados de usuário (projeto/app DB) e não pode alterar texto legítimo.
    pub fn redact_registered(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Ok(set) = self.values.lock() {
            let mut vals: Vec<&String> = set.iter().collect();
            vals.sort_by_key(|v| core::cmp::Reverse(v.len()));
            for v in vals {
                if out.contains(v.as_str()) {
                    out = out.replace(v.as_str(), MASK);
                }
            }
        }
        out
    }

    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Ok(set) = self.values.lock() {
            // mais longos primeiro, para não deixar sufixos de um valor maior
            let mut vals: Vec<&String> = set.iter().collect();
            vals.sort_by_key(|v| core::cmp::Reverse(v.len()));
            for v in vals {
                if out.contains(v.as_str()) {
                    out = out.replace(v.as_str(), MASK);
                }
            }
        }
        scrub_patterns(&out)
    }
}

fn global() -> &'static SecretRegistry {
    static G: OnceLock<SecretRegistry> = OnceLock::new();
    G.get_or_init(SecretRegistry::new)
}

/// Registra um segredo no registro do processo (chamado ao carregar/gravar uma credencial).
pub fn register_global(value: &str) {
    global().register(value);
}

/// Redige com o registro do processo + padrões.
pub fn redact_global(text: &str) -> String {
    global().redact(text)
}

/// Só os valores registrados no processo (sem heurística de padrões).
pub fn redact_registered_global(text: &str) -> String {
    global().redact_registered(text)
}

/// Atalho: padrões conhecidos + registro do processo.
pub fn redact(text: &str) -> String {
    redact_global(text)
}

const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
];
const SENSITIVE_QUERY: &[&str] = &[
    "key",
    "api_key",
    "apikey",
    "token",
    "access_token",
    "secret",
    "signature",
    "x-goog-api-key",
];

fn scrub_patterns(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&scrub_line(line));
    }
    out
}

fn scrub_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut s = line.to_owned();
    // `Header: value` e `"header": "value"`
    for h in SENSITIVE_HEADERS {
        let mut from = 0usize;
        while let Some(pos) = lower[from..].find(h) {
            let at = from + pos;
            let after = at + h.len();
            let rest = &line[after..];
            let trimmed = rest.trim_start_matches(['"', '\'', ' ']);
            if trimmed.starts_with(':') || trimmed.starts_with('=') {
                s = mask_value_after(&s, h);
                break;
            }
            from = after;
        }
    }
    // Bearer <token>
    s = mask_bearer(&s);
    // query: ?key=VALUE&token=VALUE
    s = mask_query(&s);
    // prefixos conhecidos de chave
    s = mask_prefixed(&s, "sk-", 16);
    s = mask_prefixed(&s, "sk_", 16);
    s = mask_prefixed(&s, "AIza", 20);
    s
}

fn mask_value_after(s: &str, header: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let Some(pos) = lower.find(header) else {
        return s.to_owned();
    };
    let after = pos + header.len();
    let bytes = s.as_bytes();
    let mut i = after;
    while i < bytes.len() && matches!(bytes[i], b'"' | b'\'' | b' ') {
        i += 1;
    }
    if i < bytes.len() && (bytes[i] == b':' || bytes[i] == b'=') {
        i += 1;
    } else {
        return s.to_owned();
    }
    while i < bytes.len() && matches!(bytes[i], b' ' | b'"' | b'\'') {
        i += 1;
    }
    // valor até fim da linha, aspas ou vírgula/ponto-e-vírgula
    let mut j = i;
    while j < bytes.len() && !matches!(bytes[j], b'"' | b'\'' | b',' | b'\r' | b'}') {
        j += 1;
    }
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..i]);
    out.push_str(MASK);
    out.push_str(&s[j..]);
    out
}

fn mask_bearer(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(p) = lower.find("bearer ") else {
            out.push_str(rest);
            break;
        };
        let start = p + "bearer ".len();
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | ')'))
            .unwrap_or(tail.len());
        if end > 0 {
            out.push_str(MASK);
        }
        rest = &tail[end..];
    }
    out
}

fn mask_query(s: &str) -> String {
    let mut out = s.to_owned();
    for k in SENSITIVE_QUERY {
        for sep in ['?', '&'] {
            let needle = format!("{sep}{k}=");
            let mut from = 0usize;
            while let Some(p) = out[from..].to_ascii_lowercase().find(&needle) {
                let vstart = from + p + needle.len();
                let tail = &out[vstart..];
                let end = tail
                    .find(['&', ' ', '"', '\'', '\n', ')', '#'])
                    .unwrap_or(tail.len());
                out.replace_range(vstart..vstart + end, MASK);
                from = vstart + MASK.len();
            }
        }
    }
    out
}

fn mask_prefixed(s: &str, prefix: &str, min_tail: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find(prefix) {
        out.push_str(&rest[..p]);
        let tail = &rest[p + prefix.len()..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(tail.len());
        if end >= min_tail {
            out.push_str(MASK);
        } else {
            out.push_str(prefix);
            out.push_str(&tail[..end]);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn base64_std(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn registered_values_and_encodings_disappear() {
        let r = SecretRegistry::new();
        r.register("CANARY-abc/123+xyz==");
        let t = "k=CANARY-abc/123+xyz== b64=Q0FOQVJZLWFiYy8xMjMreHl6PT0= url=CANARY-abc%2F123%2Bxyz%3D%3D";
        let o = r.redact(t);
        assert!(!o.contains("CANARY"), "{o}");
        assert!(!o.contains("Q0FOQVJZ"), "{o}");
    }

    #[test]
    fn headers_bearer_query_and_prefixes() {
        let r = SecretRegistry::new();
        let o = r.redact("Authorization: Bearer abcdefghijkl\nx-api-key: zzzzzzzz\nGET /v1?key=AIzaSyA1234567890abcdefghij&a=1");
        assert!(!o.contains("abcdefghijkl"), "{o}");
        assert!(!o.contains("zzzzzzzz"), "{o}");
        assert!(!o.contains("AIzaSyA"), "{o}");
        assert!(o.contains("a=1"));
        let o = r.redact(
            r#"{"Authorization":"Bearer tok_123456789","x":"sk-proj-ABCDEFGHIJKLMNOPQRSTUV"}"#,
        );
        assert!(!o.contains("tok_123456789"), "{o}");
        assert!(!o.contains("ABCDEFGHIJKLMNOP"), "{o}");
    }

    #[test]
    fn plain_text_is_untouched() {
        let r = SecretRegistry::new();
        let t = "o produto custa 10 reais e a chave de ouro";
        assert_eq!(r.redact(t), t);
    }
}
