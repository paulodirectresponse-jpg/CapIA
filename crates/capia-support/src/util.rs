//! Utilidades internas: relógio em ms inteiros, ids aleatórios, remoção de caminhos pessoais.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milissegundos desde a época Unix (inteiro; nunca float de segundos).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// 8 bytes aleatórios em hexadecimal (16 caracteres).
pub(crate) fn random_hex() -> String {
    let mut b = [0_u8; 8];
    if getrandom::fill(&mut b).is_err() {
        b = now_ms().to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Remove do texto o diretório pessoal do usuário (e seu nome nos caminhos) — privacidade, além da
/// redação de segredos. Não há como o diagnóstico revelar `C:\Users\<nome>\...`.
pub fn scrub_paths(text: &str) -> String {
    let mut out = text.to_owned();
    for var in ["USERPROFILE", "HOME"] {
        if let Some(home) = std::env::var_os(var) {
            let home = home.to_string_lossy().into_owned();
            if home.len() > 3 && home != "/" {
                out = out.replace(&home, "~");
                // variante com barras invertidas escapadas (texto JSON no Windows)
                out = out.replace(&home.replace('\\', "\\\\"), "~");
            }
        }
    }
    out
}

/// Redação completa de qualquer texto que saia do processo para suporte: segredos + caminhos pessoais.
pub fn sanitize(text: &str) -> String {
    scrub_paths(&capia_secrets::redact_global(text))
}

/// Trunca em fronteira de caractere.
pub(crate) fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &s[..end])
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;

    /// Diretório temporário único, removido no `Drop`.
    #[derive(Debug)]
    pub(crate) struct Tmp(pub PathBuf);
    impl Tmp {
        pub(crate) fn new(tag: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("capia-support-{tag}-{}", super::random_hex()));
            std::fs::create_dir_all(&p).unwrap_or_else(|e| panic!("tmp: {e}"));
            Self(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
