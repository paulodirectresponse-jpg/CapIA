//! Impressão digital **rápida** (ADR-053): tamanho + regiões amostradas (início, fim e janelas
//! espaçadas) + versão do algoritmo. Serve só para **triagem** (candidatos de relink, "talvez já
//! importado", import que retorna antes do hash completo). **Nunca** é identidade: dois arquivos
//! diferentes podem colidir; só o SHA-256 completo decide (ver teste de colisão deliberada).

use crate::error::{AssetError, AssetErrorCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Versão do algoritmo; muda ⇒ impressões antigas deixam de ser comparáveis.
pub const FINGERPRINT_VERSION: u32 = 1;
/// Tamanho de cada região amostrada.
pub const SAMPLE_LEN: u64 = 64 * 1024;
/// Nº de janelas internas (além de início e fim).
pub const INNER_SAMPLES: u64 = 14;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint {
    pub version: u32,
    pub size: u64,
    /// SHA-256 (hex) de `versão ‖ tamanho ‖ regiões amostradas`.
    pub digest: String,
}

impl Fingerprint {
    /// `fp1:<tamanho>:<digest>`.
    pub fn to_text(&self) -> String {
        format!("fp{}:{}:{}", self.version, self.size, self.digest)
    }

    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("fp")?;
        let mut it = rest.splitn(3, ':');
        let version = it.next()?.parse().ok()?;
        let size = it.next()?.parse().ok()?;
        let digest = it.next()?.to_owned();
        (digest.len() == 64
            && digest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .then_some(Self {
            version,
            size,
            digest,
        })
    }

    /// Só compara impressões da MESMA versão (senão são incomparáveis, nunca "iguais").
    pub fn comparable(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

/// Offsets das regiões amostradas para um arquivo de `size` bytes (determinístico, ordenado).
pub fn sample_offsets(size: u64) -> Vec<(u64, u64)> {
    if size <= SAMPLE_LEN * (INNER_SAMPLES + 2) {
        return vec![(0, size)]; // arquivo pequeno: lê tudo (continua sendo só uma impressão)
    }
    let mut v = vec![(0, SAMPLE_LEN)];
    let span = size - SAMPLE_LEN * 2;
    for i in 1..=INNER_SAMPLES {
        let off =
            SAMPLE_LEN + (u128::from(span) * u128::from(i) / u128::from(INNER_SAMPLES + 1)) as u64;
        v.push((off.min(size - SAMPLE_LEN), SAMPLE_LEN));
    }
    v.push((size - SAMPLE_LEN, SAMPLE_LEN));
    v
}

pub fn fingerprint_file(path: &Path) -> Result<Fingerprint, AssetError> {
    let io = |e: std::io::Error| {
        AssetError::new(
            AssetErrorCode::AssetIo,
            format!("cannot read `{}`: {e}", path.display()),
        )
    };
    let mut f = std::fs::File::open(path).map_err(io)?;
    let meta = f.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(AssetError::new(
            AssetErrorCode::AssetNotRegularFile,
            format!("`{}` is not a regular file", path.display()),
        ));
    }
    let size = meta.len();
    let mut h = Sha256::new();
    h.update(FINGERPRINT_VERSION.to_le_bytes());
    h.update(size.to_le_bytes());
    let mut buf = vec![0u8; SAMPLE_LEN as usize];
    for (off, len) in sample_offsets(size) {
        f.seek(SeekFrom::Start(off)).map_err(io)?;
        let mut remaining = len as usize;
        while remaining > 0 {
            let take = remaining.min(buf.len());
            let n = f.read(&mut buf[..take]).map_err(io)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            remaining -= n;
        }
        h.update(off.to_le_bytes());
    }
    let digest = h.finalize().iter().fold(String::new(), |mut s, b| {
        use core::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    });
    Ok(Fingerprint {
        version: FINGERPRINT_VERSION,
        size,
        digest,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn offsets_are_in_bounds_ordered_and_cover_the_edges() {
        for size in [
            0u64,
            1,
            1000,
            SAMPLE_LEN * 16,
            SAMPLE_LEN * 16 + 1,
            10 << 30,
            u64::MAX / 2,
        ] {
            let v = sample_offsets(size);
            assert!(!v.is_empty() || size == 0);
            let mut last = 0;
            for (off, len) in &v {
                assert!(off.checked_add(*len).unwrap() <= size, "{size}");
                assert!(*off >= last);
                last = *off;
            }
            if size > SAMPLE_LEN * 16 {
                assert_eq!(v.first().unwrap().0, 0);
                assert_eq!(v.last().unwrap().0 + SAMPLE_LEN, size);
            }
        }
    }

    #[test]
    fn text_round_trip_and_validation() {
        let fp = Fingerprint {
            version: 1,
            size: 42,
            digest: "a".repeat(64),
        };
        assert_eq!(Fingerprint::parse(&fp.to_text()).unwrap(), fp);
        for bad in [
            "",
            "fp",
            "fp1:x:y",
            "fp1:1:abc",
            "xx1:1:aaaa",
            &format!("fp1:1:{}", "G".repeat(64)),
        ] {
            assert!(Fingerprint::parse(bad).is_none(), "{bad}");
        }
    }
}
