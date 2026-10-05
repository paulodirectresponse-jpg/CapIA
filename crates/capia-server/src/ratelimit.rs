//! Rate limit por token e classe de operação (token bucket). 429 estruturado com `Retry-After`.

use crate::catalog::Class;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// (capacidade em rajada, reposição por segundo).
fn budget(c: Class) -> (f64, f64) {
    match c {
        Class::Read => (240.0, 40.0),
        Class::Write => (60.0, 10.0),
        Class::Upload => (12.0, 0.5),
        Class::RunStart => (6.0, 0.2),
        Class::Approve => (30.0, 2.0),
        Class::Export => (6.0, 0.2),
        Class::Admin => (20.0, 1.0),
    }
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Debug)]
pub struct RateLimiter {
    scale: f64,
    buckets: Mutex<HashMap<(String, Class), Bucket>>,
}

impl RateLimiter {
    pub fn new(scale: f64) -> Self {
        Self {
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// `Ok` consome uma unidade; `Err(segundos)` = quanto esperar.
    pub fn check(&self, token_id: &str, class: Class) -> Result<(), u64> {
        self.check_at(token_id, class, Instant::now())
    }

    pub fn check_at(&self, token_id: &str, class: Class, now: Instant) -> Result<(), u64> {
        let (cap, rate) = budget(class);
        let (cap, rate) = ((cap * self.scale).max(1.0), (rate * self.scale).max(0.001));
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // tabela limitada: um token revogado/rotacionado não deixa lixo para sempre
        if map.len() > 4096 {
            map.retain(|_, b| now.duration_since(b.last).as_secs() < 300);
        }
        let b = map.entry((token_id.to_owned(), class)).or_insert(Bucket {
            tokens: cap,
            last: now,
        });
        let dt = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + dt * rate).min(cap);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - b.tokens) / rate).ceil().max(1.0) as u64)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bursts_then_throttles_then_refills_per_token_and_class() {
        let rl = RateLimiter::new(1.0);
        let t0 = Instant::now();
        for _ in 0..6 {
            assert!(rl.check_at("a", Class::RunStart, t0).is_ok());
        }
        let wait = rl.check_at("a", Class::RunStart, t0).unwrap_err();
        assert!(wait >= 1);
        // outro token e outra classe têm baldes próprios
        assert!(rl.check_at("b", Class::RunStart, t0).is_ok());
        assert!(rl.check_at("a", Class::Read, t0).is_ok());
        // reposição: 0,2/s => 5 s devolvem 1 unidade
        assert!(
            rl.check_at("a", Class::RunStart, t0 + Duration::from_secs(5))
                .is_ok()
        );
    }

    #[test]
    fn a_non_finite_scale_falls_back_to_the_default() {
        let rl = RateLimiter::new(f64::NAN);
        assert!(rl.check("x", Class::Admin).is_ok());
    }
}
