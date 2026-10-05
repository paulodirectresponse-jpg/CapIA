//! Cancelamento cooperativo com *abort* real da rede: `cancelled().await` entra em `select!` com a
//! requisição; ao disparar, o future da requisição é descartado (conexão fechada).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancelToken {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    flag: AtomicBool,
    notify: Notify,
}

impl core::fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CancelToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Resolve quando cancelado (imediatamente se já estiver).
    pub async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancel_wakes_waiters_and_is_sticky() {
        let t = CancelToken::new();
        let t2 = t.clone();
        let h = tokio::spawn(async move { t2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!h.is_finished());
        t.cancel();
        tokio::time::timeout(Duration::from_secs(1), h)
            .await
            .unwrap()
            .unwrap();
        // pegajoso: um novo waiter resolve na hora
        tokio::time::timeout(Duration::from_millis(50), t.cancelled())
            .await
            .unwrap();
    }
}
