//! Pontos de falha da autonomia para os testes de crash (feature `failpoints`; **fora** do build
//! normal). `CAPIA_FAILPOINT=<ponto>` seleciona onde parar e `CAPIA_FAILPOINT_MODE` o que fazer:
//! `abort` (padrão — `process::abort`, sem destrutores nem *flush*) ou `park` (imprime
//! `FAILPOINT_REACHED <ponto>` e dorme para o processo pai matar de fora).
//!
//! Para testes em processo existe também [`arm`]: o ponto devolve erro uma vez (simula a morte do
//! driver no ponto sem derrubar o processo de teste).

#[cfg(feature = "failpoints")]
mod imp {
    use std::collections::BTreeSet;
    use std::io::Write as _;
    use std::sync::Mutex;

    static ARMED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

    pub fn arm(name: &str) {
        if let Ok(mut a) = ARMED.lock() {
            a.insert(name.to_owned());
        }
    }

    /// O ponto ainda está armado (não disparou).
    pub fn is_armed(name: &str) -> bool {
        ARMED.lock().is_ok_and(|a| a.contains(name))
    }

    pub fn disarm_all() {
        if let Ok(mut a) = ARMED.lock() {
            a.clear();
        }
    }

    /// `true` ⇒ o ponto armado em processo disparou (o chamador devolve erro e **para**).
    pub fn hit(name: &str) -> bool {
        if let Ok(mut a) = ARMED.lock()
            && a.remove(name)
        {
            return true;
        }
        if std::env::var("CAPIA_FAILPOINT").ok().as_deref() != Some(name) {
            return false;
        }
        match std::env::var("CAPIA_FAILPOINT_MODE").ok().as_deref() {
            Some("park") => {
                let mut out = std::io::stdout();
                let _ = writeln!(out, "FAILPOINT_REACHED {name}");
                let _ = out.flush();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            _ => std::process::abort(),
        }
    }
}

#[cfg(feature = "failpoints")]
pub use imp::{arm, disarm_all, hit, is_armed};

/// `fp!("nome")?` — devolve `Err(FAILPOINT)` se o ponto estiver armado em processo; aborta/estaciona
/// se for o ponto do ambiente; no-op sem a feature.
macro_rules! fp {
    ($name:expr) => {{
        #[cfg(feature = "failpoints")]
        {
            if $crate::autonomy::failpoint::hit($name) {
                return Err($crate::error::IntelError::new(
                    "FAILPOINT",
                    format!("failpoint {} fired", $name),
                ));
            }
        }
    }};
}
pub(crate) use fp;
