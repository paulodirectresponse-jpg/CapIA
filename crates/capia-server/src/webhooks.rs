//! Despachante de webhooks (Trilha B): assinatura HMAC, retry/backoff, dead-letter e log de entrega
//! sobre as tabelas `events`/`deliveries` do `ServerDb`. **Stub do contrato**: a implementação
//! substitui `run_dispatcher`; a assinatura da função e o uso de `Core` (db, cofre de segredos,
//! `webhook_client`, `wake`) são o contrato estável da Trilha A.

use crate::core::Core;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Laço do despachante até o shutdown. Nunca bloqueia a conclusão de uma Run: lê só o banco.
pub fn run_dispatcher(core: &Arc<Core>) {
    while !core.shutting_down.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}
