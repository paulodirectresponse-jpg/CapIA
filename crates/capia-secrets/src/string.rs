use core::fmt;
use zeroize::Zeroize;

/// Segredo em memória. Não implementa `Serialize`/`Clone` implícito; `Debug`/`Display` redigidos.
pub struct SecretString {
    inner: String,
}

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            inner: value.into(),
        }
    }

    /// Acesso ao valor em claro. Chame apenas no ponto de montar a requisição e descarte logo.
    pub fn expose(&self) -> &str {
        &self.inner
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Cópia explícita (nunca `Clone` por acidente).
    pub fn duplicate(&self) -> Self {
        Self::new(self.inner.clone())
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.inner.zeroize();
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn debug_and_display_never_reveal() {
        let s = SecretString::new("sk-live-supersecret");
        assert!(!format!("{s:?}").contains("supersecret"));
        assert!(!format!("{s}").contains("supersecret"));
        assert_eq!(s.expose(), "sk-live-supersecret");
    }
}
