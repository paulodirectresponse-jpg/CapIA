//! Assinatura atrás de um trait. Implementação: Ed25519 (`ed25519-dalek`, puro Rust, BSD-3-Clause).
//! A chave pública de produção entra em tempo de compilação (`CAPIA_UPDATE_PUBKEYS`); sem ela o
//! updater fica **não configurado** (nenhum update é aceito). A chave privada nunca está no repositório.

use crate::error::UpdateError;
use ed25519_dalek::{Signature as DalekSig, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub alg: String,
    pub key_id: String,
    /// Assinatura em hexadecimal minúsculo (128 caracteres para Ed25519).
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    UnsupportedAlgorithm(String),
    UnknownKey(String),
    BadEncoding,
    Invalid,
    NoTrustedKeys,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedAlgorithm(a) => write!(f, "unsupported signature algorithm `{a}`"),
            Self::UnknownKey(k) => write!(f, "signing key `{k}` is not trusted"),
            Self::BadEncoding => f.write_str("malformed signature"),
            Self::Invalid => f.write_str("signature does not match the manifest"),
            Self::NoTrustedKeys => f.write_str("no trusted update keys are configured"),
        }
    }
}

impl std::error::Error for VerifyError {}

pub trait Verifier: Send + Sync + fmt::Debug {
    fn verify(&self, payload: &[u8], sig: &Signature) -> Result<(), VerifyError>;
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub(crate) fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedKey {
    pub key_id: String,
    pub public: [u8; 32],
}

impl TrustedKey {
    pub fn from_hex(key_id: &str, hex: &str) -> Result<Self, UpdateError> {
        let bytes = hex_decode(hex)
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .ok_or_else(|| UpdateError::Invalid("public key must be 64 hex characters".into()))?;
        // rejeita pontos inválidos já na configuração
        VerifyingKey::from_bytes(&bytes)
            .map_err(|_| UpdateError::Invalid("public key is not a valid Ed25519 point".into()))?;
        Ok(Self {
            key_id: key_id.to_owned(),
            public: bytes,
        })
    }
}

/// Várias chaves confiáveis permitem rotação (a antiga e a nova coexistem por um tempo).
#[derive(Debug, Clone)]
pub struct Ed25519Verifier {
    keys: Vec<TrustedKey>,
}

impl Ed25519Verifier {
    pub fn new(keys: Vec<TrustedKey>) -> Self {
        Self { keys }
    }

    /// Chaves embutidas na build (`CAPIA_UPDATE_PUBKEYS="id1:hex,id2:hex"`); vazio se não configurado.
    pub fn embedded() -> Self {
        Self::new(parse_key_list(
            option_env!("CAPIA_UPDATE_PUBKEYS").unwrap_or(""),
        ))
    }

    pub fn is_configured(&self) -> bool {
        !self.keys.is_empty()
    }
}

/// `id:hex,id:hex`; entradas inválidas são ignoradas (e a build sem chaves fica "não configurada").
pub fn parse_key_list(list: &str) -> Vec<TrustedKey> {
    list.split(',')
        .filter_map(|e| {
            let (id, hex) = e.trim().split_once(':')?;
            TrustedKey::from_hex(id.trim(), hex.trim()).ok()
        })
        .collect()
}

impl Verifier for Ed25519Verifier {
    fn verify(&self, payload: &[u8], sig: &Signature) -> Result<(), VerifyError> {
        if self.keys.is_empty() {
            return Err(VerifyError::NoTrustedKeys);
        }
        if sig.alg != "ed25519" {
            return Err(VerifyError::UnsupportedAlgorithm(sig.alg.clone()));
        }
        let key = self
            .keys
            .iter()
            .find(|k| k.key_id == sig.key_id)
            .ok_or_else(|| VerifyError::UnknownKey(sig.key_id.clone()))?;
        let raw = hex_decode(&sig.value)
            .and_then(|b| <[u8; 64]>::try_from(b).ok())
            .ok_or(VerifyError::BadEncoding)?;
        let vk = VerifyingKey::from_bytes(&key.public).map_err(|_| VerifyError::BadEncoding)?;
        vk.verify_strict(payload, &DalekSig::from_bytes(&raw))
            .map_err(|_| VerifyError::Invalid)
    }
}

/// Assinador (ferramentas de release e testes). Não faz parte do caminho do app instalado.
pub struct Ed25519Signer {
    key_id: String,
    key: SigningKey,
}

impl fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ed25519Signer({}, <redacted>)", self.key_id)
    }
}

impl Ed25519Signer {
    pub fn from_seed(key_id: &str, seed: [u8; 32]) -> Self {
        Self {
            key_id: key_id.to_owned(),
            key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_hex(&self) -> String {
        hex_encode(self.key.verifying_key().as_bytes())
    }

    pub fn trusted(&self) -> TrustedKey {
        TrustedKey {
            key_id: self.key_id.clone(),
            public: self.key.verifying_key().to_bytes(),
        }
    }

    pub fn sign(&self, payload: &[u8]) -> Signature {
        Signature {
            alg: "ed25519".to_owned(),
            key_id: self.key_id.clone(),
            value: hex_encode(&self.key.sign(payload).to_bytes()),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn pair(id: &str) -> Ed25519Signer {
        let mut seed = [0_u8; 32];
        getrandom::fill(&mut seed).unwrap();
        Ed25519Signer::from_seed(id, seed)
    }

    #[test]
    fn valid_signature_verifies_and_wrong_payload_or_key_does_not() {
        let s = pair("k1");
        let v = Ed25519Verifier::new(vec![s.trusted()]);
        let sig = s.sign(b"payload");
        assert!(v.verify(b"payload", &sig).is_ok());
        assert_eq!(v.verify(b"payloae", &sig), Err(VerifyError::Invalid));
        let other = pair("k1");
        let forged = other.sign(b"payload");
        assert_eq!(v.verify(b"payload", &forged), Err(VerifyError::Invalid));
        let mut unknown = sig.clone();
        unknown.key_id = "k2".into();
        assert!(matches!(
            v.verify(b"payload", &unknown),
            Err(VerifyError::UnknownKey(_))
        ));
        let mut alg = sig.clone();
        alg.alg = "rsa".into();
        assert!(matches!(
            v.verify(b"payload", &alg),
            Err(VerifyError::UnsupportedAlgorithm(_))
        ));
        let mut bad = sig;
        bad.value = "zz".into();
        assert_eq!(v.verify(b"payload", &bad), Err(VerifyError::BadEncoding));
    }

    #[test]
    fn no_configured_keys_accepts_nothing() {
        let s = pair("k1");
        let v = Ed25519Verifier::new(vec![]);
        assert!(!v.is_configured());
        assert_eq!(
            v.verify(b"x", &s.sign(b"x")),
            Err(VerifyError::NoTrustedKeys)
        );
    }

    #[test]
    fn key_rotation_accepts_old_and_new_keys() {
        let (a, b) = (pair("old"), pair("new"));
        let v = Ed25519Verifier::new(vec![a.trusted(), b.trusted()]);
        assert!(v.verify(b"m", &a.sign(b"m")).is_ok());
        assert!(v.verify(b"m", &b.sign(b"m")).is_ok());
    }

    #[test]
    fn key_list_parsing_ignores_garbage() {
        let s = pair("k1");
        let list = format!("k1:{}, bad:zz,nocolon", s.public_hex());
        let keys = parse_key_list(&list);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], s.trusted());
        assert!(parse_key_list("").is_empty());
    }
}
