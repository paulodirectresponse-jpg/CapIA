//! Leitura de recursos do próprio processo (Linux: `/proc/self`). Nas demais plataformas as
//! funções devolvem `None` e os testes que dependem delas **se declaram skip com motivo**.

use std::fs;

fn status_field(name: &str) -> Option<String> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix(':'))
        .map(|v| v.trim().to_owned())
}

/// Memória residente (VmRSS) em KiB.
pub fn rss_kib() -> Option<u64> {
    status_field("VmRSS")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Pico de memória residente (VmHWM) em KiB.
pub fn peak_rss_kib() -> Option<u64> {
    status_field("VmHWM")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Quantidade de threads do processo.
pub fn thread_count() -> Option<usize> {
    status_field("Threads")?.parse().ok()
}

/// Quantidade de descritores de arquivo abertos (exclui o descritor usado para listar).
pub fn fd_count() -> Option<usize> {
    let n = fs::read_dir("/proc/self/fd").ok()?.count();
    Some(n.saturating_sub(1))
}

/// UID efetivo (`0` ⇒ root: testes de permissão negada não têm efeito e devem pular).
pub fn effective_uid() -> Option<u32> {
    status_field("Uid")?.split_whitespace().nth(1)?.parse().ok()
}

/// Linux com `/proc` legível: as medições acima são reais.
pub fn available() -> bool {
    rss_kib().is_some() && fd_count().is_some()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn reads_real_numbers_on_linux() {
        assert!(rss_kib().is_some_and(|k| k > 100));
        assert!(fd_count().is_some_and(|n| n >= 3));
        assert!(thread_count().is_some_and(|n| n >= 1));
        assert!(effective_uid().is_some());
        // abrir um arquivo aumenta a contagem e fechar devolve
        let before = fd_count().unwrap();
        let f = fs::File::open("/proc/self/status").unwrap();
        assert_eq!(fd_count().unwrap(), before + 1);
        drop(f);
        assert_eq!(fd_count().unwrap(), before);
    }
}
