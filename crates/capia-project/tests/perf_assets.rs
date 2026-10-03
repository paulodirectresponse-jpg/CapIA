//! Medições da fundação de mídia (ignoradas por padrão; `--release -- --ignored --nocapture`):
//! hash em streaming, probe real, import real e um projeto com 10.000 assets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use capia_assets::testing::{StaticProbe, synthetic_info};
use capia_assets::{AssetKind, AssetLocation, AssetRecord, Availability, ContentHash, hash_file};
use capia_commands::{Actor, Command, CommandEnvelope, Transaction};
use capia_media::{FfprobeBackend, MediaConfig, MediaKind, MediaProbe, MediaToolchain};
use capia_model::Asset;
use capia_project::Project;
use capia_store::{Catalog, CatalogEventKind, CatalogOp, StoreOptions, Synchronous};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

#[test]
#[ignore = "medição: cargo test --release -p capia-project --test perf_assets -- --ignored --nocapture"]
fn hash_probe_and_import_costs() {
    let dir = std::env::temp_dir().join(format!("capia-perf-media-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // ---- hash em streaming de 256 MiB (arquivo esparso: lê zeros do page cache) ----
    let big = dir.join("big.bin");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(256 * 1024 * 1024).unwrap();
    drop(f);
    let t = Instant::now();
    let d = hash_file(&big).unwrap();
    let el = t.elapsed();
    eprintln!(
        "PERF hash 256 MiB (streaming): {:.0} ms = {:.0} MiB/s ({})",
        ms(el),
        256.0 / el.as_secs_f64(),
        &d.hash.as_str()[..19]
    );
    assert!(el < Duration::from_secs(20), "hashing is unreasonably slow");
    let _ = std::fs::remove_file(&big);

    // ---- hash de uma fixture pequena (custo fixo de abrir/ler/fechar) ----
    let hashes: Vec<Duration> = (0..50)
        .map(|_| {
            let t = Instant::now();
            hash_file(&fixture("video_audio.mp4")).unwrap();
            t.elapsed()
        })
        .collect();
    eprintln!(
        "PERF hash of the 7.5 KiB fixture, median of 50: {:.3} ms",
        ms(median(hashes))
    );

    // ---- probe e import reais ----
    let Ok(tc) = MediaToolchain::locate(&MediaConfig::default()) else {
        eprintln!("PERF probe/import: SKIP (no ffprobe)");
        return;
    };
    eprintln!("PERF backend: {}", tc.version);
    let backend = FfprobeBackend::new(tc);
    let probes: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            backend.probe(&fixture("video_audio.mp4")).unwrap();
            t.elapsed()
        })
        .collect();
    eprintln!(
        "PERF probe (ffprobe, small mp4) median of 20: {:.1} ms",
        ms(median(probes))
    );

    let proj = dir.join("p.capia");
    let opts = StoreOptions::default(); // synchronous=FULL: o custo real de durabilidade
    let mut p = Project::create(&proj, &opts).unwrap();
    let user = Actor::user("perf");
    let imports: Vec<Duration> = (0..10)
        .map(|i| {
            let f = dir.join(format!("v{i}.mp4"));
            let mut bytes = std::fs::read(fixture("video_audio.mp4")).unwrap();
            bytes.extend_from_slice(format!("trailer-{i}").as_bytes()); // conteúdo distinto
            std::fs::write(&f, bytes).unwrap();
            let t = Instant::now();
            p.import_asset(&user, &f, &backend).unwrap();
            t.elapsed()
        })
        .collect();
    eprintln!(
        "PERF import (hash + ffprobe + atomic commit, sync=FULL) median of 10: {:.1} ms",
        ms(median(imports))
    );
    // ---- miniatura (ffmpeg real): cache frio × cache quente ----
    if let Some(ffmpeg_tc) = MediaToolchain::locate(&MediaConfig::default())
        .ok()
        .filter(|t| t.ffmpeg.is_some())
    {
        let any = p.assets().unwrap().into_iter().next().unwrap().asset_id;
        let cold: Vec<Duration> = (0..10)
            .map(|_| {
                p.cache_dir().clear().unwrap();
                let t = Instant::now();
                p.generate_thumbnail(
                    &any,
                    capia_time::Ticks(capia_time::TICKS_PER_SECOND / 2),
                    128,
                    &ffmpeg_tc,
                )
                .unwrap();
                t.elapsed()
            })
            .collect();
        eprintln!(
            "PERF thumbnail (cold cache: hash + ffmpeg + atomic write), median of 10: {:.1} ms",
            ms(median(cold))
        );
        let hot: Vec<Duration> = (0..20)
            .map(|_| {
                let t = Instant::now();
                p.generate_thumbnail(
                    &any,
                    capia_time::Ticks(capia_time::TICKS_PER_SECOND / 2),
                    128,
                    &ffmpeg_tc,
                )
                .unwrap();
                t.elapsed()
            })
            .collect();
        eprintln!(
            "PERF thumbnail (warm cache), median of 20: {:.2} ms",
            ms(median(hot))
        );
        let _ = p.cache_dir().clear();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn synthetic_record(i: usize, dir: &Path, with_file: bool) -> AssetRecord {
    let hash = format!("sha256:{:064x}", i as u128 + 1);
    let path = dir.join(format!("m{i}.mp4"));
    if with_file {
        std::fs::write(&path, format!("media {i}")).unwrap();
    }
    AssetRecord {
        asset_id: format!("ast_{:032x}", i as u128 + 1).as_str().into(),
        kind: AssetKind::Video,
        content_hash: ContentHash::parse(&hash).unwrap(),
        size_bytes: format!("media {i}").len() as u64,
        display_name: format!("m{i}.mp4"),
        location: AssetLocation::from_path(&path, Some(dir)),
        known_paths: Vec::new(),
        media: synthetic_info(MediaKind::Video, 10),
        status: Availability::Online,
        status_checked_ms: 1,
        imported_ms: 1,
    }
}

#[test]
#[ignore = "medição: cargo test --release -p capia-project --test perf_assets -- --ignored --nocapture"]
fn ten_thousand_assets_open_list_and_import() {
    const N: usize = 10_000;
    let dir = std::env::temp_dir().join(format!("capia-perf-10k-{}", std::process::id()));
    let media = dir.join("media");
    std::fs::create_dir_all(&media).unwrap();
    let proj = dir.join("p.capia");
    let opts = StoreOptions {
        synchronous: Synchronous::Full,
        ..StoreOptions::default()
    };
    let t = Instant::now();
    let records: Vec<AssetRecord> = (0..N).map(|i| synthetic_record(i, &media, true)).collect();
    eprintln!("PERF setup: {N} files written in {:.0} ms", ms(t.elapsed()));
    {
        let mut p = Project::create(&proj, &opts).unwrap();
        let t = Instant::now();
        // documento: 10.000 `register_asset` numa transação (um commit durável)
        let commands: Vec<CommandEnvelope> = records
            .iter()
            .map(|r| CommandEnvelope {
                operation_id: format!("reg-{}", r.asset_id),
                reference: None,
                command: Command::RegisterAsset {
                    asset: Asset {
                        id: r.asset_id.clone(),
                        name: r.display_name.clone(),
                        duration: r.media.duration,
                        has_video: true,
                        has_audio: true,
                        offline: false,
                    },
                },
            })
            .collect();
        p.execute(
            &Actor::user("perf"),
            Transaction {
                transaction_id: None,
                label: "10k".into(),
                base_revision: None,
                commands,
                max_ops: Some(20_000),
            },
        )
        .unwrap();
        eprintln!(
            "PERF document: 10k register_asset in one durable commit: {:.0} ms",
            ms(t.elapsed())
        );
        drop(p);
        let mut cat = Catalog::open(&proj, Duration::from_secs(5)).unwrap();
        let ops: Vec<CatalogOp> = records
            .iter()
            .map(|r| CatalogOp {
                record: r.clone(),
                event: CatalogEventKind::Import,
                detail: serde_json::json!({}),
                at_ms: 1,
            })
            .collect();
        let t = Instant::now();
        cat.apply_batch(&ops).unwrap();
        eprintln!(
            "PERF catalog: 10k rows + events in one transaction: {:.0} ms",
            ms(t.elapsed())
        );
    }
    let size = std::fs::metadata(&proj).unwrap().len();
    eprintln!("PERF file size with 10k assets: {} KiB", size / 1024);

    let t = Instant::now();
    let mut p = Project::open(&proj, &opts).unwrap();
    let open = t.elapsed();
    eprintln!("PERF open (10k assets, doc + catalog): {:.0} ms", ms(open));
    assert!(open < Duration::from_secs(2), "open must stay under 2 s");

    let t = Instant::now();
    let views = p.assets().unwrap();
    let list = t.elapsed();
    eprintln!(
        "PERF list 10k assets (catalog read + 10k stat calls): {:.0} ms",
        ms(list)
    );
    assert_eq!(views.len(), N);
    assert!(
        views
            .iter()
            .all(|v| v.catalog.as_ref().unwrap().status == Availability::Online)
    );
    assert!(list < Duration::from_secs(2), "listing must stay under 2 s");

    let t = Instant::now();
    for i in (0..N).step_by(N / 100) {
        p.asset(&records[i].asset_id).unwrap();
    }
    eprintln!(
        "PERF lookup by AssetId (O(1): row + metadata), mean of 100 in a 10k project: {:.3} ms",
        ms(t.elapsed()) / 100.0
    );
    let t = Instant::now();
    for i in (0..N).step_by(N / 100) {
        assert!(
            p.find_asset_by_hash(&records[i].content_hash)
                .unwrap()
                .is_some()
        );
    }
    eprintln!(
        "PERF lookup by content hash (unique index), mean of 100 in a 10k project: {:.3} ms",
        ms(t.elapsed()) / 100.0
    );
    // verify individual (recalcula o hash): o conteúdo sintético não bate com o hash fabricado ⇒
    // `modified`; mede-se o caminho completo (stat + hash + evento transacional)
    let t = Instant::now();
    let v = p.verify_asset(&records[N / 2].asset_id).unwrap();
    eprintln!(
        "PERF verify of one asset ({}): {:.2} ms",
        v.status.as_str(),
        ms(t.elapsed())
    );

    // import de um arquivo novo num projeto grande: custo de um commit comum
    let new = media.join("new.mp4");
    std::fs::write(&new, b"brand new media").unwrap();
    let t = Instant::now();
    p.import_asset(&Actor::user("perf"), &new, &StaticProbe)
        .unwrap();
    let imp = t.elapsed();
    eprintln!(
        "PERF import into a 10k-asset project (sync=FULL): {:.1} ms",
        ms(imp)
    );
    assert!(imp < Duration::from_millis(200));
    drop(p);
    let t = Instant::now();
    let p = Project::open(&proj, &opts).unwrap();
    eprintln!("PERF reopen after import: {:.0} ms", ms(t.elapsed()));
    assert_eq!(p.assets().unwrap().len(), N + 1);
    let _ = std::fs::remove_dir_all(&dir);
}
