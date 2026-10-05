//! Cache derivado v2: produção atômica, validação, lock por chave, GC (ADR-057).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

struct Tmp(PathBuf);
impl Tmp {
    fn new(n: &str) -> Self {
        let p = std::env::temp_dir().join(format!("capia-cache-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hash_of(bytes: &[u8]) -> ContentHash {
    hash_reader(bytes).unwrap().hash
}

fn key(content: &ContentHash, op: &str) -> CacheKey {
    CacheKey::new(content, op, &[("p", "1")], "t/1").unwrap()
}

fn ok(_: &Path) -> Result<(), AssetError> {
    Ok(())
}

fn magic_valid(p: &Path) -> Result<(), AssetError> {
    let b =
        std::fs::read(p).map_err(|e| AssetError::new(AssetErrorCode::AssetIo, e.to_string()))?;
    if b.starts_with(b"GOOD") && b.ends_with(b"END") {
        Ok(())
    } else {
        Err(AssetError::new(AssetErrorCode::AssetIo, "invalid"))
    }
}

fn write_good(p: &Path) -> Result<(), AssetError> {
    std::fs::write(p, b"GOOD-payload-END")
        .map_err(|e| AssetError::new(AssetErrorCode::AssetIo, e.to_string()))
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |rd| rd.flatten().count())
}

#[test]
fn produce_publishes_atomically_and_hits_the_second_time() {
    let t = Tmp::new("atomic");
    let c = CacheDir::new(t.0.join("c"));
    let k = key(&hash_of(b"a"), "waveform");
    let calls = AtomicU32::new(0);
    let w = |p: &Path| {
        calls.fetch_add(1, Ordering::SeqCst);
        write_good(p)
    };
    let a = c.produce(&k, "bin", &|| false, &magic_valid, &w).unwrap();
    assert!(!a.hit);
    let b = c.produce(&k, "bin", &|| false, &magic_valid, &w).unwrap();
    assert!(b.hit);
    assert_eq!(a.path, b.path);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // sem temporários nem locks sobrando
    assert_eq!(count_files(&c.root().join(".tmp")), 0);
}

#[test]
fn an_invalid_result_is_never_published_and_a_previous_good_file_survives() {
    let t = Tmp::new("partial");
    let c = CacheDir::new(t.0.join("c"));
    let k = key(&hash_of(b"a"), "proxy");
    // escrita parcial (truncada): validação recusa ⇒ nada publicado, temporário removido
    let e = c
        .produce(&k, "bin", &|| false, &magic_valid, &|p| {
            std::fs::write(p, b"GOOD-trunc")
                .map_err(|e| AssetError::new(AssetErrorCode::AssetIo, e.to_string()))
        })
        .unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetIo);
    assert!(c.get(&k, "bin").is_none());
    assert_eq!(count_files(&c.root().join(".tmp")), 0);
    // falha do writer
    assert!(
        c.produce(&k, "bin", &|| false, &magic_valid, &|_| Err(
            AssetError::new(AssetErrorCode::AssetIo, "boom")
        ))
        .is_err()
    );
    assert!(c.get(&k, "bin").is_none());
    // um arquivo bom já publicado sobrevive a uma regeneração que falha: o `hit` o preserva
    let first = c
        .produce(&k, "bin", &|| false, &magic_valid, &write_good)
        .unwrap();
    let again = c.produce(&k, "bin", &|| false, &magic_valid, &|_| {
        Err(AssetError::new(AssetErrorCode::AssetIo, "must not run"))
    });
    assert!(again.unwrap().hit);
    assert_eq!(std::fs::read(&first.path).unwrap(), b"GOOD-payload-END");
}

#[test]
fn cancellation_discards_the_temp_and_publishes_nothing() {
    let t = Tmp::new("cancel");
    let c = CacheDir::new(t.0.join("c"));
    let k = key(&hash_of(b"a"), "proxy");
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let e = c
        .produce(
            &k,
            "bin",
            &|| cancelled.load(Ordering::SeqCst),
            &magic_valid,
            &|p| {
                write_good(p)?;
                cancelled.store(true, Ordering::SeqCst); // cancela DEPOIS de escrever, antes de publicar
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(e.code, AssetErrorCode::AssetCancelled);
    assert!(c.get(&k, "bin").is_none());
    assert_eq!(count_files(&c.root().join(".tmp")), 0);
}

#[test]
fn a_corrupt_entry_is_a_miss_and_is_regenerated() {
    let t = Tmp::new("corrupt");
    let c = CacheDir::new(t.0.join("c"));
    let k = key(&hash_of(b"a"), "waveform");
    let p = c
        .produce(&k, "bin", &|| false, &magic_valid, &write_good)
        .unwrap()
        .path;
    std::fs::write(&p, b"GOOD-bit-rot").unwrap();
    assert!(c.get_valid(&k, "bin", &magic_valid).is_none());
    assert!(!p.exists(), "invalid entries are deleted");
    let again = c
        .produce(&k, "bin", &|| false, &magic_valid, &write_good)
        .unwrap();
    assert!(!again.hit);
}

#[test]
fn concurrent_producers_of_one_key_generate_exactly_once() {
    let t = Tmp::new("race");
    let c = Arc::new(CacheDir::new(t.0.join("c")));
    let k = Arc::new(key(&hash_of(b"race"), "index"));
    let calls = Arc::new(AtomicU32::new(0));
    let handles: Vec<_> = (0..12)
        .map(|_| {
            let (c, k, calls) = (c.clone(), k.clone(), calls.clone());
            std::thread::spawn(move || {
                c.produce(&k, "bin", &|| false, &magic_valid, &|p| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    write_good(p)
                })
                .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "work was duplicated");
    assert_eq!(results.iter().filter(|r| !r.hit).count(), 1);
    assert!(results.iter().all(|r| r.path == results[0].path));
}

#[test]
fn different_keys_do_not_block_each_other() {
    let t = Tmp::new("parallel");
    let c = Arc::new(CacheDir::new(t.0.join("c")));
    let started = std::time::Instant::now();
    let hs: Vec<_> = (0..4)
        .map(|i| {
            let c = c.clone();
            std::thread::spawn(move || {
                let k = key(&hash_of(format!("k{i}").as_bytes()), "index");
                c.produce(&k, "bin", &|| false, &ok, &|p| {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    write_good(p)
                })
                .unwrap();
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    // em paralelo ~400 ms; serializado >= 1600 ms. O limite fica no meio, com folga
    // para runners carregados (Windows) sem perder o poder de detectar a serialização.
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1200),
        "keys were serialized"
    );
}

#[test]
fn gc_usage_invalidate_and_clear_keep_the_project_valid() {
    let t = Tmp::new("gc");
    let c = CacheDir::new(t.0.join("c"));
    let (ha, hb) = (hash_of(b"asset-a"), hash_of(b"asset-b"));
    for h in [&ha, &hb] {
        for op in ["index", "waveform"] {
            c.produce(&key(h, op), "bin", &|| false, &magic_valid, &write_good)
                .unwrap();
        }
    }
    let u = c.usage().unwrap();
    assert_eq!(u.files, 4);
    assert_eq!(u.by_op["index"].0, 2);
    assert!(u.bytes > 0);
    // GC: só `a` está vivo ⇒ remove as entradas de `b`
    let live: HashSet<String> = [ha.hex()[..16].to_owned()].into();
    let r = c.remove_unused(&live).unwrap();
    assert_eq!(r.removed_files, 2);
    assert!(c.get(&key(&ha, "index"), "bin").is_some());
    assert!(c.get(&key(&hb, "index"), "bin").is_none());
    // invalidar um conteúdo remove tudo dele
    let r = c.invalidate_content(&ha).unwrap();
    assert_eq!(r.removed_files, 2);
    assert_eq!(c.usage().unwrap().files, 0);
    // limpar tudo é idempotente e o cache volta a funcionar
    c.clear().unwrap();
    c.clear().unwrap();
    assert_eq!(c.usage().unwrap(), CacheUsage::default());
    c.produce(
        &key(&ha, "index"),
        "bin",
        &|| false,
        &magic_valid,
        &write_good,
    )
    .unwrap();
}

#[test]
fn stale_temp_files_are_swept_and_reported() {
    let t = Tmp::new("sweep");
    let c = CacheDir::new(t.0.join("c"));
    let p = c.temp_path("bin").unwrap();
    std::fs::write(&p, b"leftover of a crashed job").unwrap();
    let u = c.usage().unwrap();
    assert_eq!((u.temp_files, u.files), (1, 0));
    assert_eq!(
        c.sweep_temp(std::time::Duration::from_secs(3600)),
        0,
        "recent temp is kept"
    );
    assert_eq!(c.sweep_temp(std::time::Duration::ZERO), 1);
    assert!(!p.exists());
}
