//! Hash em job (cancelável, com progresso, detecta mudança) e impressão rápida (nunca identidade).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::*;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

struct Tmp(PathBuf);
impl Tmp {
    fn new(n: &str) -> Self {
        let p = std::env::temp_dir().join(format!("capia-hash-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn big(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

#[test]
fn job_hash_equals_sync_hash_and_reports_monotonic_progress() {
    let t = Tmp::new("eq");
    let p = t.file("a.bin", &big(5 * 1024 * 1024 + 17, 1));
    let mut seen = Vec::new();
    let d = hash_file_job(&p, &|| false, &mut |done, total| seen.push((done, total))).unwrap();
    assert_eq!(d, hash_file(&p).unwrap());
    assert!(seen.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(seen.last().unwrap().0, d.size);
    assert!(seen.iter().all(|(_, total)| *total == d.size));
}

#[test]
fn cancelling_stops_hashing_between_blocks() {
    let t = Tmp::new("cancel");
    let p = t.file("a.bin", &big(8 * 1024 * 1024, 2));
    let stop = AtomicBool::new(false);
    let mut blocks = 0u32;
    let e = hash_file_job(&p, &|| stop.load(Ordering::SeqCst), &mut |_, _| {
        blocks += 1;
        if blocks == 2 {
            stop.store(true, Ordering::SeqCst);
        }
    })
    .unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetCancelled);
    assert_eq!(blocks, 2, "no block was read after the cancellation");
}

#[test]
fn growing_or_rewriting_the_file_during_hashing_is_detected() {
    let t = Tmp::new("changed");
    // 1) crescimento
    let p = t.file("grow.bin", &big(4 * 1024 * 1024, 3));
    let mut first = true;
    let e = hash_file_job(&p, &|| false, &mut |_, _| {
        if first {
            first = false;
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(b"more bytes").unwrap();
        }
    })
    .unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetChangedDuringProcessing);
    assert_eq!(e.code.as_str(), "ASSET_CHANGED_DURING_PROCESSING");
    // 2) regravação no lugar com o MESMO tamanho (mtime muda)
    let p = t.file("rewrite.bin", &big(4 * 1024 * 1024, 4));
    let mut first = true;
    let e = hash_file_job(&p, &|| false, &mut |_, _| {
        if first {
            first = false;
            std::thread::sleep(std::time::Duration::from_millis(60));
            std::fs::write(&p, big(4 * 1024 * 1024, 5)).unwrap();
        }
    })
    .unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetChangedDuringProcessing);
    // 3) arquivo trocado por outro (rename por cima)
    let p = t.file("swap.bin", &big(4 * 1024 * 1024, 6));
    let other = t.file("other.bin", &big(4 * 1024 * 1024, 7));
    let mut first = true;
    let e = hash_file_job(&p, &|| false, &mut |_, _| {
        if first {
            first = false;
            std::thread::sleep(std::time::Duration::from_millis(60));
            std::fs::rename(&other, &p).unwrap();
        }
    });
    // em Windows o rename por cima de arquivo aberto pode falhar no próprio teste; só valida se rodou
    if let Err(e) = e {
        assert_eq!(e.code, AssetErrorCode::AssetChangedDuringProcessing);
    }
    // arquivo estável continua funcionando
    let ok = t.file("stable.bin", &big(1024, 8));
    assert!(hash_file_job(&ok, &|| false, &mut |_, _| {}).is_ok());
}

#[test]
fn fingerprint_is_deterministic_and_sensitive_to_size_and_sampled_content() {
    let t = Tmp::new("fp");
    let a = t.file("a.bin", &big(3 * 1024 * 1024, 1));
    let f1 = fingerprint_file(&a).unwrap();
    assert_eq!(f1, fingerprint_file(&a).unwrap());
    assert_eq!(f1.version, FINGERPRINT_VERSION);
    assert_eq!(f1.size, 3 * 1024 * 1024);
    assert_eq!(Fingerprint::parse(&f1.to_text()).unwrap(), f1);
    // tamanho diferente
    let b = t.file("b.bin", &big(3 * 1024 * 1024 + 1, 1));
    assert_ne!(f1, fingerprint_file(&b).unwrap());
    // diferença na região amostrada (início)
    let mut v = big(3 * 1024 * 1024, 1);
    v[10] ^= 1;
    let c = t.file("c.bin", &v);
    assert_ne!(f1, fingerprint_file(&c).unwrap());
    // pequeno arquivo, vazio, inexistente
    assert!(fingerprint_file(&t.file("e.bin", b"")).is_ok());
    assert!(fingerprint_file(&t.0.join("nope")).is_err());
}

/// Colisão **deliberada**: arquivos diferentes fora das regiões amostradas ⇒ mesma impressão, mas o
/// SHA-256 completo os separa. É por isso que a impressão nunca é identidade.
#[test]
fn a_deliberate_collision_is_separated_by_sha256() {
    let t = Tmp::new("collision");
    let size = 32 * 1024 * 1024;
    let base = big(size, 9);
    let offs = sample_offsets(size as u64);
    // acha um byte fora de TODAS as regiões amostradas
    let free = (0..size as u64)
        .step_by(4096)
        .find(|o| !offs.iter().any(|(s, l)| o >= s && *o < s + l))
        .expect("an unsampled byte exists") as usize;
    let mut other = base.clone();
    other[free] ^= 0xFF;
    let a = t.file("a.bin", &base);
    let b = t.file("b.bin", &other);
    let (fa, fb) = (fingerprint_file(&a).unwrap(), fingerprint_file(&b).unwrap());
    assert_eq!(
        fa, fb,
        "the fingerprint cannot see the unsampled byte (by design)"
    );
    let (ha, hb) = (hash_file(&a).unwrap(), hash_file(&b).unwrap());
    assert_ne!(ha.hash, hb.hash, "SHA-256 distinguishes them");
    assert_ne!(asset_id_for(&ha.hash), asset_id_for(&hb.hash));
}
