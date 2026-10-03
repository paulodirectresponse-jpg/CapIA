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
