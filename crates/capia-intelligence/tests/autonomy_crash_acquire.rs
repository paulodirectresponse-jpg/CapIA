//! Crash/resume em ACQUIRE (gateway e geração): morrer depois do download / depois do submit nunca
//! baixa ou gera (e paga) duas vezes; o asset importado é único e a Run termina.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_intelligence::autonomy::failpoint;
use capia_intelligence::autonomy::gateway::{
    Candidate, CatalogEntry, LicenseStatus, ReplayCatalogAdapter,
};
use capia_intelligence::autonomy::generation::{GenKind, ReplayGenerationProvider};
use capia_intelligence::autonomy::machine::RunStatus;
use capia_intelligence::autonomy::model::RunPolicy;
use capia_intelligence::autonomy::plan::AssetKind;
use common::auto::*;
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn candidate(id: &str, license: LicenseStatus, price: Option<u64>) -> Candidate {
    Candidate {
        id: id.into(),
        adapter: "cat".into(),
        title: "pessoa usando cafe na cozinha".into(),
        description: "b-roll produto cozinha".into(),
        kind: AssetKind::Video,
        duration_ms: Some(4000),
        width: None,
        height: None,
        license,
        license_text: Some("test license".into()),
        price_micros: price,
        source_uri: Some(format!("https://stock.example/{id}")),
        score: 0.0,
        score_components: vec![],
    }
}

fn need(id: &str, required: bool, priority: &[&str]) -> Value {
    json!({"id": id, "kind": "video", "purpose": "b-roll produto", "description": "pessoa usando cafe na cozinha",
           "required": required, "source_priority": priority, "target_duration_ms": 4000})
}

fn beats_with_need(asset: &str, need_id: &str) -> Value {
    json!({"format": {"width": 1080, "height": 1920, "fps": 30}, "estimated_duration_ms": 7000, "beats": [
        {"id": "hook", "role": "hook", "duration_ms": 3000, "asset": {"asset_id": asset, "source_in_ms": 0},
         "overlays": [{"text": "Compre agora", "start_offset_ms": 0, "duration_ms": 1500}]},
        {"id": "broll", "role": "broll", "duration_ms": 3000, "asset": {"need_id": need_id, "source_in_ms": 0}},
        {"id": "cta", "role": "cta", "duration_ms": 1000, "asset": {"asset_id": asset, "source_in_ms": 7000}}]})
}

async fn crash_then_finish(a: &AutoWorld, run_id: &str, point: &str) {
    failpoint::arm(point);
    a.orch.start(run_id).unwrap();
    let t0 = std::time::Instant::now();
    while a.orch.is_driving(run_id) || a.orch.load(run_id).unwrap().status == RunStatus::Pending {
        assert!(t0.elapsed().as_secs() < 90, "no crash at {point}");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    failpoint::disarm_all();
    assert_eq!(
        a.orch.load(run_id).unwrap().status,
        RunStatus::Running,
        "died mid-run at {point}"
    );
    let o2 = a.restart();
    o2.recover().unwrap();
    assert_eq!(o2.load(run_id).unwrap().status, RunStatus::Paused);
    o2.resume(run_id).unwrap();
    let t0 = std::time::Instant::now();
    loop {
        let r = o2.load(run_id).unwrap();
        if r.status == RunStatus::Completed && !o2.is_driving(run_id) {
            break;
        }
        assert!(
            !matches!(r.status, RunStatus::Failed | RunStatus::Cancelled),
            "{point}: {:?}",
            r.error
        );
        assert!(
            t0.elapsed().as_secs() < 90,
            "{point}: {:?} {:?}",
            r.status,
            r.pending
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn scripted(name: &str, src: &str) -> Option<AutoWorld> {
    auto_world(name, |asset| {
        let src = src.to_owned();
        Script::new(
            || demand_json(false),
            move || {
                producer_json(
                    json!([{"key": "main", "sequence_strategy": "standalone"}]),
                    json!([need("n1", true, &[src.as_str()])]),
                )
            },
            move |_d, _n| beats_with_need(&asset.lock().unwrap(), "n1"),
            |_n| json!({"findings": []}),
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_after_the_download_never_downloads_twice_and_imports_one_asset() {
    let _g = SERIAL.lock().await;
    let Some(tc) = ffmpeg() else { return };
    let Some(a) = scripted("crash-dl", "gateway") else {
        return;
    };
    let payload = broll_bytes(&tc, &a.w.dir, "stock.mp4", 4);
    let cat = Arc::new(ReplayCatalogAdapter::new(
        "cat",
        vec![CatalogEntry {
            candidate: candidate("ok", LicenseStatus::KnownAllowed, Some(0)),
            payload,
        }],
    ));
    a.gateways.register_shared(cat.clone());
    let run = a.create(a.inputs(), auto_policy());
    let before =
        a.w.ctx
            .engine
            .read("assets.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len();
    crash_then_finish(&a, &run.id, "autonomy_acquire_after_download").await;
    assert_eq!(
        cat.fetch_count.load(Ordering::SeqCst),
        1,
        "the verified staged file was reused"
    );
    let after =
        a.w.ctx
            .engine
            .read("assets.list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len();
    assert_eq!(after, before + 1, "exactly one imported asset");
    let done = a.orch.load(&run.id).unwrap();
    let asset = done.production_plan.unwrap().asset_needs[0]
        .resolved_asset_id
        .clone()
        .unwrap();
    assert!(a.orch.store().get_provenance(&asset).unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_after_the_generation_submit_never_submits_or_pays_twice() {
    let _g = SERIAL.lock().await;
    let Some(tc) = ffmpeg() else { return };
    let Some(a) = scripted("crash-gen", "generate") else {
        return;
    };
    let payload = broll_bytes(&tc, &a.w.dir, "gen.mp4", 4);
    let prov = Arc::new(
        ReplayGenerationProvider::new("gen", vec![GenKind::Video], Some(500_000), payload)
            .with_running_polls(2),
    );
    a.generators.register(prov.clone());
    let policy = RunPolicy {
        generation_requires_approval: false,
        ..auto_policy()
    };
    let run = a.create(a.inputs(), policy);
    crash_then_finish(&a, &run.id, "autonomy_generation_after_submit").await;
    assert_eq!(
        prov.submit_count(),
        1,
        "the provider job was found by key, not resubmitted"
    );
    let done = a.orch.load(&run.id).unwrap();
    assert_eq!(done.usage.generations, 1);
    assert_eq!(done.usage.cost_micros, 500_000, "paid exactly once");
}
