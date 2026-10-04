use capia_time::Ticks;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Relógio monotônico do preview, em **ticks** (nunca float). O sistema usa `Instant`; os testes
/// usam [`ManualClock`] e controlam a cadência quadro a quadro.
pub trait Clock: Send + Sync {
    fn now(&self) -> Ticks;
    /// `true` ⇒ o tempo só avança quando o dono chama `advance` (o scheduler faz *polling* curto).
    fn is_manual(&self) -> bool {
        false
    }
}

#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Ticks {
        let ns = self.origin.elapsed().as_nanos();
        let t = ns * (capia_time::TICKS_PER_SECOND as u128) / 1_000_000_000;
        Ticks(i64::try_from(t).unwrap_or(i64::MAX))
    }
}

/// Relógio controlado por quem testa.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Mutex<i64>,
    cv: Condvar,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Ticks) {
        let mut g = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g = g.saturating_add(by.0);
        self.cv.notify_all();
    }

    pub fn set(&self, to: Ticks) {
        let mut g = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g = to.0;
        self.cv.notify_all();
    }

    /// Espera (no máx. `timeout`) o relógio passar de `after`.
    pub fn wait_past(&self, after: Ticks, timeout: Duration) -> bool {
        let g = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |n| *n <= after.0)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *g > after.0
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Ticks {
        Ticks(
            *self
                .now
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn is_manual(&self) -> bool {
        true
    }
}
