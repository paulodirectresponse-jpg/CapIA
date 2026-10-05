//! Despachante de webhooks (Trilha B): assinatura HMAC, retry/backoff, dead-letter e log de entrega
//! sobre as tabelas `events`/`deliveries` do `ServerDb`.
//!
//! Garantias (ver `docs/phase6/IMPL_MCP_WEBHOOKS.md`):
//! * **At-least-once.** Uma entrega só vira `delivered` com um 2xx. Queda do processo no meio de uma
//!   tentativa deixa a linha `delivering`, que a reabertura devolve a `retrying`: o receptor pode ver
//!   o mesmo evento duas vezes e deduplica por `X-CapIA-Event-Id` (estável entre tentativas).
//! * **Nunca bloqueia a Run/export.** O despachante só lê o banco (preenchido pela `pump`) e acorda
//!   por `Core::wake`; um endpoint fora do ar só produz linhas `retrying`/`dead`.
//! * **Sem espera em fila única.** Cada entrega é uma tarefa num runtime multi-thread pequeno; um
//!   endpoint lento ocupa uma vaga (até `MAX_INFLIGHT`), nunca a fila inteira.
//! * **O destino é revalidado a CADA tentativa** (política de URL + resolvedor filtrado), nenhum
//!   redirect é seguido e o corpo da resposta nunca é lido.
//! * **O segredo vive só no cofre** (`capia/server/webhook/<id>`); nunca no banco, no log ou numa
//!   mensagem de erro.

use crate::auth::now_ms;
use crate::core::Core;
use crate::mac::{constant_time_eq, hmac_sha256_hex, random_bytes};
use capia_secrets::CredentialRef;
use capia_store::{DeliveryRow, ServerEventRow};
use serde_json::json;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Janela de replay aceita pelo receptor (cinco minutos, nos dois sentidos).
pub const REPLAY_TOLERANCE_SECS: u64 = 300;
/// Versão do formato do corpo (campo `version`).
pub const PAYLOAD_VERSION: u32 = 1;
/// Esquema de assinatura (`X-CapIA-Signature: v1=<hex>`).
pub const SIGNATURE_SCHEME: &str = "v1";
/// Tentativas simultâneas no máximo (backpressure: nada de spawn ilimitado).
const MAX_INFLIGHT: usize = 32;
/// Resolução do laço quando nada acorda o despachante.
const TICK: Duration = Duration::from_millis(100);

// ---- assinatura / verificação -----------------------------------------------------------------

/// Valor do cabeçalho `X-CapIA-Signature` (`v1=<hex>`): HMAC-SHA256 de `"<timestamp>.<corpo cru>"`.
pub fn sign(secret: &str, timestamp: u64, body: &[u8]) -> String {
    format!("{SIGNATURE_SCHEME}={}", signature_hex(secret, timestamp, body))
}

fn signature_hex(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut msg = Vec::with_capacity(body.len() + 21);
    msg.extend_from_slice(timestamp.to_string().as_bytes());
    msg.push(b'.');
    msg.extend_from_slice(body);
    hmac_sha256_hex(secret.as_bytes(), &msg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// Cabeçalho ausente de `v1=<64 hex>` bem formado.
    Malformed,
    /// `|agora - timestamp|` acima da tolerância (replay ou relógio errado).
    TimestampOutsideTolerance,
    /// Corpo, timestamp ou segredo não conferem.
    SignatureMismatch,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed X-CapIA-Signature header",
            Self::TimestampOutsideTolerance => "timestamp outside the replay window",
            Self::SignatureMismatch => "signature mismatch",
        })
    }
}

impl std::error::Error for VerifyError {}

/// Verificação do receptor. `header` aceita várias assinaturas separadas por vírgula (`v1=a,v1=b`,
/// útil na rotação de segredo); basta uma bater. Comparação em tempo constante; a janela de replay
/// é checada **antes** de qualquer HMAC. Para deduplicar replays DENTRO da janela o receptor deve
/// guardar `X-CapIA-Event-Id` (ver docs).
pub fn verify(
    secret: &str,
    timestamp: u64,
    body: &[u8],
    header: &str,
    now: u64,
    tolerance_secs: u64,
) -> Result<(), VerifyError> {
    let candidates: Vec<&str> = header
        .split(',')
        .filter_map(|p| p.trim().strip_prefix("v1="))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    if candidates.is_empty() {
        return Err(VerifyError::Malformed);
    }
    if now.abs_diff(timestamp) > tolerance_secs {
        return Err(VerifyError::TimestampOutsideTolerance);
    }
    let expected = signature_hex(secret, timestamp, body);
    let mut ok = false;
    for c in candidates {
        // sem curto-circuito entre candidatos
        ok |= constant_time_eq(c.to_ascii_lowercase().as_bytes(), expected.as_bytes());
    }
    if ok {
        Ok(())
    } else {
        Err(VerifyError::SignatureMismatch)
    }
}

// ---- corpo ------------------------------------------------------------------------------------

/// `1970-01-01T00:00:00.000Z` a partir de milissegundos Unix (UTC, sem dependência de calendário).
pub fn iso8601_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // civil_from_days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        ms % 1000
    )
}

/// Corpo cru enviado ao receptor (as chaves saem ordenadas: bytes estáveis entre tentativas).
pub fn build_body(ev: &ServerEventRow) -> Vec<u8> {
    json!({
        "id": ev.event_id,
        "type": ev.kind,
        "version": PAYLOAD_VERSION,
        "occurred_at": iso8601_utc(ev.occurred_ms),
        "occurred_ms": ev.occurred_ms,
        "project_id": ev.project_id,
        "run_id": ev.run_id,
        "export_id": ev.export_id,
        "data": ev.data,
    })
    .to_string()
    .into_bytes()
}

// ---- backoff ----------------------------------------------------------------------------------

/// `base * 2^(attempt-1)` limitado por `cap`, com jitter de ±20% (`jitter` em `[-1, 1]`); o
/// resultado nunca passa de `cap`.
pub fn backoff(base: Duration, cap: Duration, attempt: u32, jitter: f64) -> Duration {
    let exp = attempt.saturating_sub(1).min(30);
    let raw = base
        .checked_mul(1u32 << exp)
        .unwrap_or(cap)
        .min(cap)
        .as_secs_f64();
    let j = jitter.clamp(-1.0, 1.0);
    Duration::from_secs_f64((raw * 0.2f64.mul_add(j, 1.0)).max(0.0)).min(cap)
}

fn random_jitter() -> f64 {
    random_bytes(2).map_or(0.0, |b| {
        let n = u16::from_le_bytes([b[0], b[1]]);
        f64::from(n) / f64::from(u16::MAX) * 2.0 - 1.0
    })
}

// ---- despacho ---------------------------------------------------------------------------------

enum Outcome {
    Delivered {
        status: u16,
        latency_ms: u64,
    },
    Retry {
        status: Option<u16>,
        latency_ms: u64,
        error: String,
    },
    Dead {
        status: Option<u16>,
        latency_ms: u64,
        error: String,
    },
}

fn dead(error: impl Into<String>) -> Outcome {
    Outcome::Dead {
        status: None,
        latency_ms: 0,
        error: error.into(),
    }
}

async fn attempt(core: &Core, row: &DeliveryRow) -> Outcome {
    let hook = match core.db.webhook_get(&row.webhook_id) {
        Ok(Some(h)) => h,
        Ok(None) => return dead("webhook no longer exists"),
        Err(e) => {
            return Outcome::Retry {
                status: None,
                latency_ms: 0,
                error: format!("server database unavailable: {}", e.message),
            };
        }
    };
    if !hook.enabled {
        return dead("webhook disabled");
    }
    let ev = match core.db.event_get(row.event_seq) {
        Ok(Some(e)) => e,
        Ok(None) => return dead("event no longer exists"),
        Err(e) => {
            return Outcome::Retry {
                status: None,
                latency_ms: 0,
                error: format!("server database unavailable: {}", e.message),
            };
        }
    };
    // o destino é revalidado a cada tentativa (política de URL pode ter mudado; DNS é filtrado
    // de novo na conexão pelo resolvedor guardado do cliente)
    if let Err(e) = core.webhook_client.check_url(&hook.url) {
        return dead(format!("URL_REJECTED: {}", e.message));
    }
    let secret = match CredentialRef::new(hook.secret_ref.clone())
        .map_err(|e| e.to_string())
        .and_then(|r| core.cfg.secrets.get(&r).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(_) => {
            return dead(
                "SECRET_UNAVAILABLE: the signing secret is not in the secret store; rotate it \
                 (POST /v1/webhooks/{id}/rotate-secret) to requeue",
            );
        }
    };
    let body = build_body(&ev);
    let ts = now_ms() / 1000;
    let headers = vec![
        ("X-CapIA-Timestamp".to_owned(), ts.to_string()),
        (
            "X-CapIA-Signature".to_owned(),
            sign(secret.expose(), ts, &body),
        ),
        ("X-CapIA-Event-Id".to_owned(), ev.event_id.clone()),
        ("X-CapIA-Delivery".to_owned(), row.id.to_string()),
        (
            "X-CapIA-Attempt".to_owned(),
            (row.attempt.saturating_add(1)).to_string(),
        ),
    ];
    drop(secret);
    let d = core.webhook_client.post(&hook.url, &headers, body).await;
    if d.delivered() {
        return Outcome::Delivered {
            status: d.status.unwrap_or(200),
            latency_ms: d.latency_ms,
        };
    }
    let redirect = d.status.is_some_and(|s| (300..400).contains(&s));
    if d.permanent() || redirect {
        let error = match (&d.error, d.status) {
            (Some(e), _) => e.clone(),
            (None, Some(s)) => format!("the endpoint answered {s} (permanent client error)"),
            (None, None) => "permanent failure".to_owned(),
        };
        return Outcome::Dead {
            status: d.status,
            latency_ms: d.latency_ms,
            error,
        };
    }
    let error = match (&d.error, d.status) {
        (Some(e), _) => e.clone(),
        (None, Some(s)) => format!("the endpoint answered {s}"),
        (None, None) => "delivery failed".to_owned(),
    };
    Outcome::Retry {
        status: d.status,
        latency_ms: d.latency_ms,
        error,
    }
}

fn record(core: &Core, row: &DeliveryRow, outcome: Outcome) {
    let now = now_ms();
    let n = row.attempt.saturating_add(1);
    let res = match outcome {
        Outcome::Delivered { status, latency_ms } => core.db.delivery_finish(
            row.id,
            "delivered",
            n,
            0,
            Some(status),
            Some(latency_ms),
            None,
            now,
        ),
        Outcome::Dead {
            status,
            latency_ms,
            error,
        } => core.db.delivery_finish(
            row.id,
            "dead",
            n,
            0,
            status,
            Some(latency_ms),
            Some(&error),
            now,
        ),
        Outcome::Retry {
            status,
            latency_ms,
            error,
        } => {
            if n >= core.cfg.webhook_max_attempts {
                core.db.delivery_finish(
                    row.id,
                    "dead",
                    n,
                    0,
                    status,
                    Some(latency_ms),
                    Some(&format!("gave up after {n} attempts: {error}")),
                    now,
                )
            } else {
                let wait = backoff(
                    core.cfg.webhook_backoff_base,
                    core.cfg.webhook_backoff_cap,
                    n,
                    random_jitter(),
                );
                let next = now.saturating_add(u64::try_from(wait.as_millis()).unwrap_or(u64::MAX));
                core.db.delivery_finish(
                    row.id,
                    "retrying",
                    n,
                    next,
                    status,
                    Some(latency_ms),
                    Some(&error),
                    now,
                )
            }
        }
    };
    if let Err(e) = res {
        // a linha continua `delivering`: a reabertura a devolve à fila (at-least-once)
        eprintln!("capia-server: delivery log write failed: {}", e.message);
    }
}

/// Laço do despachante até o shutdown. Nunca bloqueia a conclusão de uma Run: lê só o banco.
pub fn run_dispatcher(core: &Arc<Core>) {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("capia-webhook-io")
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("capia-server: webhook runtime failed to start: {e}");
            return;
        }
    };
    let inflight = Arc::new(AtomicUsize::new(0));
    while !core.shutting_down.load(Ordering::SeqCst) {
        let seen = core.wake.current();
        let free = MAX_INFLIGHT.saturating_sub(inflight.load(Ordering::SeqCst));
        if free > 0 {
            match core
                .db
                .deliveries_claim_due(now_ms(), u32::try_from(free).unwrap_or(1))
            {
                Ok(rows) => {
                    for row in rows {
                        inflight.fetch_add(1, Ordering::SeqCst);
                        let (c, inf) = (Arc::clone(core), Arc::clone(&inflight));
                        rt.spawn(async move {
                            let outcome = attempt(&c, &row).await;
                            record(&c, &row, outcome);
                            inf.fetch_sub(1, Ordering::SeqCst);
                            // uma nova tentativa pode já estar vencida
                            c.wake.notify();
                        });
                    }
                }
                Err(e) => eprintln!("capia-server: delivery claim failed: {}", e.message),
            }
        }
        core.wake.wait(seen, TICK);
    }
    // shutdown: dá uma chance curta às tentativas em voo; o que sobrar fica `delivering` e é
    // devolvido a `retrying` na próxima abertura
    let t0 = std::time::Instant::now();
    while inflight.load(Ordering::SeqCst) > 0 && t0.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    rt.shutdown_background();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip_and_reject_tampering() {
        let h = sign("whsec_x", 1_700_000_000, b"{\"a\":1}");
        assert!(h.starts_with("v1="));
        assert_eq!(
            verify("whsec_x", 1_700_000_000, b"{\"a\":1}", &h, 1_700_000_100, 300),
            Ok(())
        );
        assert_eq!(
            verify("whsec_x", 1_700_000_000, b"{\"a\":2}", &h, 1_700_000_100, 300),
            Err(VerifyError::SignatureMismatch)
        );
        assert_eq!(
            verify("whsec_y", 1_700_000_000, b"{\"a\":1}", &h, 1_700_000_100, 300),
            Err(VerifyError::SignatureMismatch)
        );
        assert_eq!(
            verify("whsec_x", 1_700_000_001, b"{\"a\":1}", &h, 1_700_000_100, 300),
            Err(VerifyError::SignatureMismatch)
        );
        assert_eq!(
            verify("whsec_x", 1_700_000_000, b"{\"a\":1}", &h, 1_700_000_301, 300),
            Err(VerifyError::TimestampOutsideTolerance)
        );
        assert_eq!(
            verify("whsec_x", 1_700_000_000, b"{\"a\":1}", &h, 1_699_999_000, 300),
            Err(VerifyError::TimestampOutsideTolerance)
        );
        assert_eq!(
            verify("whsec_x", 1, b"x", "sha256=abc", 1, 300),
            Err(VerifyError::Malformed)
        );
        // rotação: várias assinaturas, basta uma
        let two = format!("v1={},{h}", "0".repeat(64));
        assert_eq!(
            verify("whsec_x", 1_700_000_000, b"{\"a\":1}", &two, 1_700_000_000, 300),
            Ok(())
        );
    }

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn backoff_doubles_is_capped_and_jittered_within_twenty_percent() {
        let (b, c) = (Duration::from_secs(5), Duration::from_secs(60));
        assert_eq!(backoff(b, c, 1, 0.0), Duration::from_secs(5));
        assert_eq!(backoff(b, c, 2, 0.0), Duration::from_secs(10));
        assert_eq!(backoff(b, c, 3, 0.0), Duration::from_secs(20));
        assert_eq!(backoff(b, c, 9, 0.0), c);
        assert_eq!(backoff(b, c, 1000, 1.0), c);
        assert_eq!(backoff(b, c, 1, 1.0), Duration::from_secs(6));
        assert_eq!(backoff(b, c, 1, -1.0), Duration::from_secs(4));
    }
}
