//! Saída de *crash* sem segredo: o gancho de pânico padrão imprime a mensagem crua (que pode conter
//! um valor interpolado por engano). Este gancho passa a mensagem pelo redator do processo antes de
//! escrever em qualquer destino. Sem *backtrace* automático (caminhos de arquivo do usuário).

use crate::redact::redact_global;

/// Instala o gancho que escreve em `sink` (padrão: stderr) a mensagem **redigida**.
pub fn install_panic_hook_with(sink: Box<dyn Fn(&str) + Send + Sync>) {
    std::panic::set_hook(Box::new(move |info| {
        sink(&redact_global(&info.to_string()));
    }));
}

/// Gancho de produção: stderr redigido.
pub fn install_redacting_panic_hook() {
    install_panic_hook_with(Box::new(|m| eprintln!("{m}")));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_panic_that_carries_a_secret_is_written_redacted() {
        const CANARY: &str = "CNRY-panic-7c1d9e0b2a4f6e8d";
        crate::redact::register_global(CANARY);
        let out = Arc::new(Mutex::new(String::new()));
        let o = out.clone();
        let prev = std::panic::take_hook();
        install_panic_hook_with(Box::new(move |m| o.lock().unwrap().push_str(m)));
        let r = std::panic::catch_unwind(|| {
            panic!(
                "provider failed with key {CANARY} and header Authorization: Bearer abcdefghijkl"
            )
        });
        std::panic::set_hook(prev);
        assert!(r.is_err());
        let text = out.lock().unwrap().clone();
        assert!(text.contains("provider failed"), "{text}");
        assert!(!text.contains(CANARY), "{text}");
        assert!(!text.contains("abcdefghijkl"), "{text}");
    }
}
