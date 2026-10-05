//! Persistência de resultados de IA no `.capia` (tabela `ai_records`, schema 4). Resultados são
//! **derivados e rastreáveis** (chave por conteúdo + parâmetros + modelo); a timeline nunca depende
//! deles. Nada aqui é documento nem undo.

use crate::error::{IntelError, IntelResult};
use capia_store::AiStore;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

pub const KIND_TRANSCRIPT: &str = "transcript";
pub const KIND_MEDIA_ANALYSIS: &str = "media_analysis";
pub const KIND_REFERENCE: &str = "reference_grammar";
pub const KIND_DEMAND: &str = "demand_spec";
pub const KIND_TASK: &str = "assistant_task";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Chave estável: SHA-256 dos componentes separados por `\u{1f}` (sem ambiguidade de junção).
pub fn key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0x1f]);
    }
    let d = h.finalize();
    let mut s = String::with_capacity(32);
    for b in &d[..16] {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[derive(Debug)]
pub struct Records {
    store: AiStore,
}

impl Records {
    /// Abre sobre um projeto já aberto/migrado (`Session`/`Project` mantêm o arquivo).
    pub fn open(project: &Path) -> IntelResult<Self> {
        Ok(Self {
            store: AiStore::open(project, Duration::from_secs(10))?,
        })
    }

    pub fn store(&self) -> &AiStore {
        &self.store
    }

    pub fn put<T: Serialize>(
        &self,
        kind: &str,
        id: &str,
        version: u32,
        parent: Option<&str>,
        value: &T,
    ) -> IntelResult<()> {
        let json = serde_json::to_value(value)
            .map_err(|e| IntelError::new("RECORD_INVALID", e.to_string()))?;
        self.store
            .put(kind, id, version, parent, 1, &json, now_ms())?;
        Ok(())
    }

    pub fn latest<T: DeserializeOwned>(&self, kind: &str, id: &str) -> IntelResult<Option<T>> {
        let Some(row) = self.store.latest(kind, id)? else {
            return Ok(None);
        };
        // um registro ilegível não derruba a tarefa: é tratado como ausente (recalcula)
        Ok(serde_json::from_value::<T>(row.json).ok())
    }

    pub fn latest_json(&self, kind: &str, id: &str) -> IntelResult<Option<Value>> {
        Ok(self.store.latest(kind, id)?.map(|r| r.json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_and_unambiguous() {
        assert_eq!(key(&["a", "bc"]), key(&["a", "bc"]));
        assert_ne!(key(&["ab", "c"]), key(&["a", "bc"]));
        assert_eq!(key(&["x"]).len(), 32);
    }
}
