//! Tokens de API: formato `capia_<64 hex>` (256 bits do CSPRNG), mostrado **uma única vez**; o
//! servidor guarda só o SHA-256. Revogação, expiração e rotação atômica são do `ServerDb`.

use crate::error::{ApiErr, ApiResult};
use crate::mac::{random_hex, sha256_hex};
use crate::scope::{Scope, parse_scopes};
use capia_store::{ServerDb, TokenRow};
use std::collections::BTreeSet;

pub const TOKEN_PREFIX: &str = "capia_";

#[derive(Clone, Debug)]
pub struct Principal {
    pub token_id: String,
    pub name: String,
    pub scopes: BTreeSet<Scope>,
}

impl Principal {
    pub fn has(&self, s: Scope) -> bool {
        self.scopes.contains(&s)
    }

    /// Ator do Command Engine para gravações externas (`Actor::Api`).
    pub fn actor(&self) -> capia_commands::Actor {
        capia_commands::Actor::api(format!("token:{}", self.token_id))
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Cria `(linha, segredo)`. O segredo nunca é persistido.
pub fn mint(
    name: &str,
    scopes: &BTreeSet<Scope>,
    expires_ms: Option<u64>,
    rotated_from: Option<String>,
    now: u64,
) -> ApiResult<(TokenRow, String)> {
    let secret = format!(
        "{TOKEN_PREFIX}{}",
        random_hex(32).map_err(ApiErr::internal)?
    );
    let id = format!("tok_{}", random_hex(6).map_err(ApiErr::internal)?);
    let row = TokenRow {
        id,
        name: name.to_owned(),
        secret_hash: sha256_hex(secret.as_bytes()),
        prefix: secret.chars().take(TOKEN_PREFIX.len() + 4).collect(),
        scopes: scopes.iter().map(|s| s.as_str().to_owned()).collect(),
        created_ms: now,
        expires_ms,
        last_used_ms: None,
        revoked_ms: None,
        rotated_from,
    };
    capia_secrets::register_global(&secret);
    Ok((row, secret))
}

/// Cria e persiste um token (CLI offline `capia-server token create` e o bootstrap).
pub fn create_token(
    db: &ServerDb,
    name: &str,
    scopes: &[String],
    expires_in_s: Option<u64>,
) -> ApiResult<(TokenRow, String)> {
    let set = parse_scopes(scopes).map_err(ApiErr::invalid)?;
    let now = now_ms();
    let (row, secret) = mint(
        name,
        &set,
        expires_in_s.map(|s| now.saturating_add(s.saturating_mul(1000))),
        None,
        now,
    )?;
    db.token_insert(&row)?;
    Ok((row, secret))
}

/// Resolve um cabeçalho `Authorization: Bearer …`. A mensagem é a mesma para desconhecido,
/// revogado e expirado (nada de oráculo de existência).
pub fn authenticate(db: &ServerDb, bearer: &str) -> ApiResult<Principal> {
    let deny = || ApiErr::unauthorized("missing, invalid, expired or revoked token");
    if !bearer.starts_with(TOKEN_PREFIX) || bearer.len() > 128 {
        return Err(deny());
    }
    let row = db
        .token_by_hash(&sha256_hex(bearer.as_bytes()))?
        .ok_or_else(deny)?;
    // um segredo apresentado e reconhecido passa a ser "conhecido" do redator, mesmo que tenha sido
    // criado por outro processo (ex.: CLI offline): nunca ecoa nem é gravado em dado de usuário
    capia_secrets::register_global(bearer);
    let now = now_ms();
    if row.revoked_ms.is_some() || row.expires_ms.is_some_and(|e| e <= now) {
        return Err(deny());
    }
    // `last_used` com granularidade de 30 s: leituras não viram uma escrita no banco por chamada
    if row
        .last_used_ms
        .is_none_or(|l| now.saturating_sub(l) > 30_000)
    {
        let _ = db.token_touch(&row.id, now);
    }
    let scopes = row
        .scopes
        .iter()
        .filter_map(|s| Scope::parse(s))
        .collect::<BTreeSet<_>>();
    Ok(Principal {
        token_id: row.id,
        name: row.name,
        scopes,
    })
}

/// Visão pública de um token (sem hash).
pub fn public_view(t: &TokenRow) -> serde_json::Value {
    serde_json::json!({
        "id": t.id, "name": t.name, "prefix": t.prefix, "scopes": t.scopes,
        "created_ms": t.created_ms, "expires_ms": t.expires_ms,
        "last_used_ms": t.last_used_ms, "revoked_ms": t.revoked_ms,
        "rotated_from": t.rotated_from,
        "active": t.revoked_ms.is_none() && t.expires_ms.is_none_or(|e| e > now_ms()),
    })
}
