//! Pipelines locais de ponta a ponta: projeto real + mídia real + provider Replay.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use capia_ai::CancelToken;
use capia_intelligence::captions::{self, CaptionOptions};
use capia_intelligence::silence::{self, SilenceParams};
use capia_intelligence::transcript::{TranscribeParams, transcribe_asset};
use common::*;
use serde_json::json;

fn no_progress(_: u32, _: u32) {}

#[tokio::test]
async fn transcription_runs_through_the_router_and_is_cached() {
    let Some(w) = world("tr", vec![transcript_response()]) else {
        return;
    };
    let t = task(&w, "t1");
    let p = TranscribeParams::new(&w.asset_id);
    let rec = transcribe_asset(&w.ctx, &t, &p, &no_progress)
        .await
        .unwrap();
    assert!(!rec.from_cache);
    assert!(
        rec.local,
        "o provider Replay é local: o áudio não saiu da máquina"
    );
    assert_eq!(rec.transcript.language.as_deref(), Some("pt"));
    assert_eq!(rec.transcript.segments.len(), 1);
    assert_eq!(w.stt.call_count(), 1);
    // segunda vez: do cache persistido no .capia, sem chamar o provider
    let again = transcribe_asset(&w.ctx, &t, &p, &no_progress)
        .await
        .unwrap();
    assert!(again.from_cache);
    assert_eq!(again.transcript, rec.transcript);
    assert_eq!(w.stt.call_count(), 1, "cache evitou a chamada");
    // `force` ignora o cache
    let mut forced = p.clone();
    forced.force = true;
    let w2 = transcribe_asset(&w.ctx, &t, &forced, &no_progress).await;
    assert!(w2.is_err() || w.stt.call_count() == 2);
}

#[tokio::test]
async fn cancelled_transcription_returns_cancelled_and_calls_nobody() {
    let Some(w) = world("trc", vec![transcript_response()]) else {
        return;
    };
    let t = task(&w, "t2");
    t.cancel.cancel();
    let e = transcribe_asset(
        &w.ctx,
        &t,
        &TranscribeParams::new(&w.asset_id),
        &no_progress,
    )
    .await
    .unwrap_err();
    assert!(e.is_cancelled(), "{e}");
    assert_eq!(w.stt.call_count(), 0);
}

#[tokio::test]
async fn captions_are_plain_editable_clips_written_only_through_preview_and_apply() {
    let Some(w) = world("cap", vec![transcript_response()]) else {
        return;
    };
    let t = task(&w, "t3");
    let rec = transcribe_asset(
        &w.ctx,
        &t,
        &TranscribeParams::new(&w.asset_id),
        &no_progress,
    )
    .await
    .unwrap();
    let before = call(&w.session, "history.list", json!({}));
    let plan =
        captions::plan_captions(&w.ctx, &rec, "s", &CaptionOptions::default(), "task-cap").unwrap();
    assert!(plan.cue_count >= 3, "{plan:?}");
    // o preview NÃO gravou nada
    let mid = call(&w.session, "history.list", json!({}));
    assert_eq!(before["entries"], mid["entries"], "preview não pode gravar");
    let token = plan.plan_token.clone().expect("plan token");
    let applied = captions::apply_plan(&w.ctx, &token).unwrap();
    assert!(applied["revision"].as_u64().is_some());

    let seq = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    let caps: Vec<_> = seq["clips"]
        .as_object()
        .unwrap()
        .values()
        .filter(|c| c["content"]["type"] == "text")
        .collect();
    assert_eq!(caps.len(), plan.cue_count);
    // 100% editáveis: texto comum numa track de legendas, alinhado ao frame
    assert!(
        caps.iter()
            .all(|c| c["start"].as_i64().unwrap() % FRAME == 0)
    );
    let track = seq["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["role"] == "captions")
        .expect("captions track");
    assert_eq!(track["name"], "Captions (auto)");
    // ator no histórico = agent
    let hist = call(&w.session, "history.list", json!({}));
    let last = hist["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(last["actor"]["kind"], "agent", "{last}");

    // reaplicar o mesmo token é idempotente (nada duplica)
    let again = captions::apply_plan(&w.ctx, &token).unwrap();
    assert_eq!(again["replayed"], true);
    let seq2 = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    assert_eq!(
        seq2["clips"].as_object().unwrap().len(),
        seq["clips"].as_object().unwrap().len()
    );

    // o usuário desfaz a IA como qualquer comando
    call(&w.session, "command.undo", json!({}));
    let seq3 = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    assert_eq!(seq3["clips"].as_object().unwrap().len(), 1);
}

#[test]
fn silence_removal_cuts_only_silence_and_keeps_speech() {
    let Some(w) = world("sil", vec![]) else {
        return;
    };
    let p = SilenceParams::default();
    let plan = silence::plan_silence_cut(
        &w.ctx,
        &w.asset_id,
        "s",
        &p,
        "track",
        "task-sil",
        &CancelToken::new(),
    )
    .unwrap();
    // o silêncio de 2,5 s (2–4,5 s) vira corte; o de 0,5 s (6,5–7 s) também passa do mínimo de 0,5 s
    // mas com o respiro de 120 ms de cada lado só sobra 0,26 s, então a faixa curta é preservada ou
    // cortada conforme o limiar — o essencial: nenhum corte invade a fala.
    assert!(plan.cut_count >= 1, "{plan:?}");
    for r in &plan.silences {
        let (s, e) = (r.start_us as f64 / 1e6, r.end_us as f64 / 1e6);
        let in_speech = |a: f64, b: f64| (s < b && e > a);
        assert!(
            !in_speech(0.0, 2.0) && !in_speech(4.5, 6.5) && !in_speech(7.0, 12.0),
            "{r:?}"
        );
    }
    let token = plan.plan_token.clone().unwrap();
    let before = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    let dur_before: i64 = before["clips"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| c["duration"].as_i64().unwrap())
        .sum();
    captions::apply_plan(&w.ctx, &token).unwrap();
    let after = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    let dur_after: i64 = after["clips"]
        .as_object()
        .unwrap()
        .values()
        .map(|c| c["duration"].as_i64().unwrap())
        .sum();
    let removed = dur_before - dur_after;
    let expect = plan.removed_us as f64 / 1e6 * 705_600_000.0;
    assert!(
        (removed as f64 - expect).abs() <= 2.0 * FRAME as f64 * plan.cut_count as f64,
        "removido {removed} ≠ planejado {expect}"
    );
    assert!(dur_after < dur_before);
    // continua um timeline comum: ainda dá para desfazer tudo
    call(&w.session, "command.undo", json!({}));
    let back = call(&w.session, "sequence.get", json!({ "sequence": "s" }));
    assert_eq!(back["clips"].as_object().unwrap().len(), 1);
}
