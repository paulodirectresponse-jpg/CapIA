//! Manifesto de atualização assinado.
//!
//! O que é assinado: o JSON **canônico** do manifesto sem o campo `signature` (chaves ordenadas
//! lexicograficamente em todos os níveis, sem espaços, strings como `JSON.stringify`). O mesmo algoritmo
//! existe em `tools/release/make-update-manifest.mjs`; a verificação usa o JSON **recebido** (não uma
//! re-serialização), então nenhum campo desconhecido escapa da assinatura.

use crate::error::UpdateError;
use crate::verify::{Signature, Verifier};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MANIFEST_SCHEMA: u32 = 1;
pub const PRODUCT: &str = "capia";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(s: &str) -> Result<Self, UpdateError> {
        match s {
            "stable" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            other => Err(UpdateError::Invalid(format!("unknown channel `{other}`"))),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// Nome do arquivo (sem diretórios).
    pub name: String,
    pub url: String,
    /// SHA-256 em hexadecimal minúsculo (64 caracteres).
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateManifest {
    pub schema: u32,
    pub product: String,
    pub version: String,
    pub channel: Channel,
    pub artifact: Artifact,
    /// Versão mínima instalada para aplicar este update diretamente.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    pub notes: String,
    /// `true` só em manifesto de **rollback** deliberado (permite versão menor que a instalada).
    #[serde(default)]
    pub rollback: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
}

/// JSON canônico (chaves ordenadas, sem espaços).
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&m[*k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// Bytes assinados: JSON canônico do objeto sem `signature`.
pub fn signing_payload(mut obj: Value) -> Result<Vec<u8>, UpdateError> {
    let Some(map) = obj.as_object_mut() else {
        return Err(UpdateError::Invalid("manifest is not a JSON object".into()));
    };
    map.remove("signature");
    Ok(canonical_json(&obj).into_bytes())
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl UpdateManifest {
    /// Valida a forma (sem tocar em assinatura).
    pub fn validate_shape(&self) -> Result<(), UpdateError> {
        let bad = |m: &str| Err(UpdateError::Invalid(m.to_owned()));
        if self.schema != MANIFEST_SCHEMA {
            return bad("unsupported manifest schema");
        }
        if self.product != PRODUCT {
            return bad("manifest is for another product");
        }
        crate::version::parse(&self.version)?;
        if let Some(m) = &self.min_version {
            crate::version::parse(m)?;
        }
        let a = &self.artifact;
        if a.name.is_empty()
            || a.name.contains(['/', '\\'])
            || a.name == "."
            || a.name == ".."
            || a.name.contains('\0')
        {
            return bad("artifact name must be a plain file name");
        }
        if !is_hex64(&a.sha256) {
            return bad("artifact sha256 must be 64 lowercase hex characters");
        }
        if a.size == 0 {
            return bad("artifact size must be positive");
        }
        if !a.url.starts_with("https://") {
            return bad("artifact url must be https");
        }
        Ok(())
    }

    /// Analisa o JSON recebido, **verifica a assinatura** e só então devolve o manifesto.
    /// Sem assinatura, ou com assinatura inválida/chave desconhecida ⇒ erro; nada é instalado.
    pub fn parse_and_verify(bytes: &[u8], verifier: &dyn Verifier) -> Result<Self, UpdateError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|e| UpdateError::Invalid(format!("manifest is not valid JSON: {e}")))?;
        let manifest: Self = serde_json::from_value(value.clone())
            .map_err(|e| UpdateError::Invalid(format!("manifest shape: {e}")))?;
        let sig = manifest
            .signature
            .as_ref()
            .ok_or(UpdateError::SignatureMissing)?;
        let payload = signing_payload(value)?;
        verifier.verify(&payload, sig).map_err(UpdateError::from)?;
        manifest.validate_shape()?;
        Ok(manifest)
    }

    /// Assina (uso: ferramentas de release e testes). A chave real nunca fica no repositório.
    pub fn sign(mut self, signer: &crate::verify::Ed25519Signer) -> Result<Self, UpdateError> {
        self.signature = None;
        let value = serde_json::to_value(&self).map_err(|e| UpdateError::Invalid(e.to_string()))?;
        let payload = signing_payload(value)?;
        self.signature = Some(signer.sign(&payload));
        Ok(self)
    }

    pub fn to_json(&self) -> Result<String, UpdateError> {
        serde_json::to_string_pretty(self).map_err(|e| UpdateError::Invalid(e.to_string()))
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::verify::Ed25519Signer;

    pub(crate) fn signer() -> Ed25519Signer {
        let mut seed = [0_u8; 32];
        getrandom::fill(&mut seed).unwrap_or_else(|e| panic!("getrandom: {e}"));
        Ed25519Signer::from_seed("test-key-1", seed)
    }

    pub(crate) fn sample(version: &str, sha: &str, size: u64) -> UpdateManifest {
        UpdateManifest {
            schema: MANIFEST_SCHEMA,
            product: PRODUCT.to_owned(),
            version: version.to_owned(),
            channel: Channel::Stable,
            artifact: Artifact {
                name: format!("CapIA_{version}_x64-setup.exe"),
                url: format!("https://updates.example.invalid/{version}/setup.exe"),
                sha256: sha.to_owned(),
                size,
            },
            min_version: None,
            notes: "notas".to_owned(),
            rollback: false,
            signature: None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::fixtures::*;
    use super::*;
    use crate::verify::Ed25519Verifier;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let v: Value =
            serde_json::from_str(r#"{"b":1,"a":{"z":[{"y":2,"x":1}],"c":"é\n"}}"#).unwrap();
        assert_eq!(
            canonical_json(&v),
            r#"{"a":{"c":"é\n","z":[{"x":1,"y":2}]},"b":1}"#
        );
    }

    #[test]
    fn signed_manifest_round_trips_and_verifies() {
        let s = signer();
        let v = Ed25519Verifier::new(vec![s.trusted()]);
        let m = sample("0.7.0", SHA, 10).sign(&s).unwrap();
        let parsed = UpdateManifest::parse_and_verify(m.to_json().unwrap().as_bytes(), &v).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn any_tampering_is_rejected() {
        let s = signer();
        let v = Ed25519Verifier::new(vec![s.trusted()]);
        let json = sample("0.7.0", SHA, 10)
            .sign(&s)
            .unwrap()
            .to_json()
            .unwrap();
        for (from, to) in [
            ("0.7.0", "0.7.1"),
            ("\"size\": 10", "\"size\": 11"),
            ("notas", "outras"),
            ("\"rollback\": false", "\"rollback\": true"),
            (
                SHA,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            ("stable", "beta"),
        ] {
            let tampered = json.replacen(from, to, 1);
            assert_ne!(tampered, json, "test must actually change `{from}`");
            assert!(
                UpdateManifest::parse_and_verify(tampered.as_bytes(), &v).is_err(),
                "tampering {from}->{to} was accepted"
            );
        }
    }

    #[test]
    fn unsigned_unknown_key_and_unknown_fields_are_rejected() {
        let s = signer();
        let other = signer();
        let v = Ed25519Verifier::new(vec![s.trusted()]);
        let unsigned = sample("0.7.0", SHA, 10).to_json().unwrap();
        assert!(matches!(
            UpdateManifest::parse_and_verify(unsigned.as_bytes(), &v),
            Err(UpdateError::SignatureMissing)
        ));
        let by_other = sample("0.7.0", SHA, 10)
            .sign(&other)
            .unwrap()
            .to_json()
            .unwrap();
        assert!(UpdateManifest::parse_and_verify(by_other.as_bytes(), &v).is_err());
        let mut val: Value = serde_json::from_str(
            &sample("0.7.0", SHA, 10)
                .sign(&s)
                .unwrap()
                .to_json()
                .unwrap(),
        )
        .unwrap();
        val["extra"] = Value::from("x");
        assert!(UpdateManifest::parse_and_verify(val.to_string().as_bytes(), &v).is_err());
    }

    #[test]
    fn shape_rules() {
        let mut m = sample("0.7.0", SHA, 10);
        assert!(m.validate_shape().is_ok());
        m.artifact.name = "../evil.exe".into();
        assert!(m.validate_shape().is_err());
        let mut m = sample("0.7.0", SHA, 10);
        m.artifact.url = "http://insecure/x.exe".into();
        assert!(m.validate_shape().is_err());
        let mut m = sample("0.7.0", "ABC", 10);
        assert!(m.validate_shape().is_err());
        m = sample("not-semver", SHA, 10);
        assert!(m.validate_shape().is_err());
    }
}
