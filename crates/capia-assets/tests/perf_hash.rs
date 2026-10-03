//! Medições de fingerprint e SHA-256 (ignoradas por padrão):
//! `cargo test --release -p capia-assets --test perf_hash -- --ignored --nocapture`

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::{fingerprint_file, hash_file, hash_file_job};
use std::io::Write;
use std::time::Instant;

#[test]
#[ignore = "medição"]
fn fingerprint_and_sha256_of_256_mib() {
    let p = std::env::temp_dir().join(format!("capia-perf-hash-{}.bin", std::process::id()));
    {
        let mut f = std::fs::File::create(&p).unwrap();
        let block: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
        for _ in 0..256 {
            f.write_all(&block).unwrap();
        }
    }
    let t = Instant::now();
    let fp = fingerprint_file(&p).unwrap();
    let t_fp = t.elapsed();
    let t = Instant::now();
    let d = hash_file(&p).unwrap();
    let t_sha = t.elapsed();
    let t = Instant::now();
    let mut ticks = 0u32;
    let dj = hash_file_job(&p, &|| false, &mut |_, _| ticks += 1).unwrap();
    let t_job = t.elapsed();
    assert_eq!(d, dj);
    println!(
        "PERF assets.fingerprint_256MiB: {t_fp:?} ({})",
        fp.to_text()
    );
    println!(
        "PERF assets.sha256_256MiB: {t_sha:?} ({:.0} MiB/s)",
        256.0 / t_sha.as_secs_f64()
    );
    println!(
        "PERF assets.sha256_job_256MiB: {t_job:?} (cancel/progress overhead {:+.1}%, {ticks} progress ticks)",
        (t_job.as_secs_f64() / t_sha.as_secs_f64() - 1.0) * 100.0
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
#[ignore = "medição"]
fn cache_lookup_and_concurrent_hits() {
    use capia_assets::{CacheDir, CacheKey, hash_reader};
    use std::sync::Arc;
    let root = std::env::temp_dir().join(format!("capia-perf-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let cache = Arc::new(CacheDir::new(&root));
    let content = hash_reader(&b"perf"[..]).unwrap().hash;
    let key = Arc::new(CacheKey::new(&content, "index", &[("s", "0")], "p/1").unwrap());
    let valid = |p: &std::path::Path| {
        std::fs::metadata(p).map(|_| ()).map_err(|e| {
            capia_assets::AssetError::new(capia_assets::AssetErrorCode::AssetIo, e.to_string())
        })
    };
    cache
        .produce(&key, "bin", &|| false, &valid, &|p| {
            std::fs::write(p, vec![7u8; 1 << 20]).map_err(|e| {
                capia_assets::AssetError::new(capia_assets::AssetErrorCode::AssetIo, e.to_string())
            })
        })
        .unwrap();
    // hit sequencial (get + validação por metadata)
    let t = Instant::now();
    for _ in 0..20_000 {
        assert!(cache.get_valid(&key, "bin", &valid).is_some());
    }
    println!(
        "PERF cache.get_valid_hit: {:?}/lookup",
        t.elapsed() / 20_000
    );
    // hits concorrentes via `produce` (lock por chave + lock do SO + validação)
    let threads = 8usize;
    let per = 500usize;
    let t = Instant::now();
    let hs: Vec<_> = (0..threads)
        .map(|_| {
            let (c, k) = (cache.clone(), key.clone());
            std::thread::spawn(move || {
                for _ in 0..per {
                    let r = c
                        .produce(&k, "bin", &|| false, &|_| Ok(()), &|_| {
                            unreachable!("must be a hit")
                        })
                        .unwrap();
                    assert!(r.hit);
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let total = t.elapsed();
    println!(
        "PERF cache.concurrent_hits: {threads} threads × {per} = {} hits in {total:?} ({:?}/hit)",
        threads * per,
        total / (threads * per) as u32
    );
    let _ = std::fs::remove_dir_all(&root);
}
