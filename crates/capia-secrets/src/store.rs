use crate::SecretString;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Ponteiro opaco para uma credencial no cofre (`capia/provider/<id>`). É o único dado que vai ao app DB.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialRef(String);

impl CredentialRef {
    /// Só `[A-Za-z0-9._/-]`, até 128 bytes: nada de espaços/controle que enganem o backend.
    pub fn new(value: impl Into<String>) -> Result<Self, StoreError> {
        let v = value.into();
        let ok = !v.is_empty()
            && v.len() <= 128
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'));
        if ok {
            Ok(Self(v))
        } else {
            Err(StoreError::InvalidRef)
        }
    }

    /// Referência canônica de um provider.
    pub fn for_provider(provider_id: &str) -> Result<Self, StoreError> {
        Self::new(format!("capia/provider/{provider_id}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    InvalidRef,
    NotFound,
    /// Nenhum backend seguro nesta plataforma (nunca cai para arquivo em claro).
    NoSecureBackend,
    /// Falha do backend; a mensagem nunca contém o segredo.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRef => f.write_str("invalid credential reference"),
            Self::NotFound => f.write_str("credential not found"),
            Self::NoSecureBackend => f.write_str("no secure credential backend on this platform"),
            Self::Backend(m) => write!(f, "credential backend error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

pub trait SecretStore: Send + Sync + fmt::Debug {
    fn put(&self, key: &CredentialRef, secret: SecretString) -> Result<(), StoreError>;
    fn get(&self, key: &CredentialRef) -> Result<SecretString, StoreError>;
    fn delete(&self, key: &CredentialRef) -> Result<(), StoreError>;
    fn exists(&self, key: &CredentialRef) -> Result<bool, StoreError> {
        match self.get(key) {
            Ok(_) => Ok(true),
            Err(StoreError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }
    /// Nome do backend (diagnóstico; nunca contém segredo).
    fn backend(&self) -> &'static str;
}

/// Cofre volátil (testes, devserver, modo sem cofre do SO). Não persiste nada.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl SecretStore for MemoryStore {
    fn put(&self, key: &CredentialRef, secret: SecretString) -> Result<(), StoreError> {
        let mut m = self
            .map
            .lock()
            .map_err(|_| StoreError::Backend("poisoned".into()))?;
        m.insert(key.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }
    fn get(&self, key: &CredentialRef) -> Result<SecretString, StoreError> {
        let m = self
            .map
            .lock()
            .map_err(|_| StoreError::Backend("poisoned".into()))?;
        m.get(key.as_str())
            .map(|v| SecretString::new(v.clone()))
            .ok_or(StoreError::NotFound)
    }
    fn delete(&self, key: &CredentialRef) -> Result<(), StoreError> {
        let mut m = self
            .map
            .lock()
            .map_err(|_| StoreError::Backend("poisoned".into()))?;
        m.remove(key.as_str());
        Ok(())
    }
    fn backend(&self) -> &'static str {
        "memory"
    }
}

#[cfg(windows)]
mod windows_store {
    use super::*;

    /// Windows Credential Manager (credenciais genéricas, DPAPI no perfil do usuário).
    #[derive(Debug, Default)]
    pub struct CredentialManagerStore;

    const SERVICE: &str = "CapIA";

    fn entry(key: &CredentialRef) -> Result<keyring::Entry, StoreError> {
        keyring::Entry::new(SERVICE, key.as_str()).map_err(map_err)
    }

    fn map_err(e: keyring::Error) -> StoreError {
        match e {
            keyring::Error::NoEntry => StoreError::NotFound,
            // a mensagem do keyring descreve a falha do backend, nunca o valor
            other => StoreError::Backend(other.to_string()),
        }
    }

    impl SecretStore for CredentialManagerStore {
        fn put(&self, key: &CredentialRef, secret: SecretString) -> Result<(), StoreError> {
            entry(key)?.set_password(secret.expose()).map_err(map_err)
        }
        fn get(&self, key: &CredentialRef) -> Result<SecretString, StoreError> {
            entry(key)?
                .get_password()
                .map(SecretString::new)
                .map_err(map_err)
        }
        fn delete(&self, key: &CredentialRef) -> Result<(), StoreError> {
            match entry(key)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(map_err(e)),
            }
        }
        fn backend(&self) -> &'static str {
            "windows-credential-manager"
        }
    }
}

#[cfg(windows)]
pub use windows_store::CredentialManagerStore;

/// Backend seguro da plataforma. Windows ⇒ Credential Manager; demais ⇒ `NoSecureBackend`
/// (o chamador decide usar [`MemoryStore`] explicitamente — nunca silenciosamente).
pub fn platform_store() -> Result<Arc<dyn SecretStore>, StoreError> {
    #[cfg(windows)]
    {
        Ok(Arc::new(CredentialManagerStore))
    }
    #[cfg(not(windows))]
    {
        Err(StoreError::NoSecureBackend)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn ref_validation() {
        assert!(CredentialRef::new("capia/provider/p_1").is_ok());
        assert!(CredentialRef::new("").is_err());
        assert!(CredentialRef::new("a b").is_err());
        assert!(CredentialRef::new("a\nb").is_err());
        assert!(CredentialRef::new("x".repeat(129)).is_err());
    }

    #[test]
    fn memory_roundtrip_and_delete() {
        let s = MemoryStore::new();
        let k = CredentialRef::for_provider("p1").unwrap();
        assert_eq!(s.exists(&k), Ok(false));
        s.put(&k, SecretString::new("abc")).unwrap();
        assert_eq!(s.get(&k).unwrap().expose(), "abc");
        assert_eq!(s.exists(&k), Ok(true));
        s.delete(&k).unwrap();
        assert!(matches!(s.get(&k), Err(StoreError::NotFound)));
        assert_eq!(format!("{s:?}"), "MemoryStore { .. }");
    }
}
