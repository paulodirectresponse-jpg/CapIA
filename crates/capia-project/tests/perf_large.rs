//! Benchmarks do projeto grande (Fase 6, Track D-1): 36 sequences, 6.000 clips, nested, legendas,
//! áudio, 2.500 assets no catálogo e 60 AI Runs — construído pelo `capia-fixtures` (arquivo REAL,
//! só pelo Command Engine). `--release --ignored`; grava `target/perf/phase6-large-project.json`
//! com as **amostras brutas**, p50/p95/máx e os dados da máquina. O gate estatístico é
//! `tools/phase6-acceptance/performance/run.mjs` (lê o JSON; limites em `thresholds.json`).
//!
//! `cargo test --release -p capia-project --test perf_large -- --ignored --nocapture --test-threads=1`
//!
//! Variáveis: `CAPIA_PERF_REPS` (repetições dos itens pesados, padrão 9), `CAPIA_PERF_OUT` (diretório
//! do relatório). Este teste só afirma o que é SANIDADE (a fixture é real, as respostas têm o
//! conteúdo esperado) e os tetos absolutos já documentados com folga larga (abrir/recarregar < 2 s,
//! transação durável < 200 ms, undo/redo < 50 ms, scrub < 100 ms); os limites estatísticos finos
//! vivem no gate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]

use capia_commands::{Actor, Command, CommandEnvelope, Transaction, document_digest};
use capia_fixtures::api::ApiProbe;
use capia_fixtures::bench::{Report, Stat, ms, ms_reps, perf_out_path, time_reps};
use capia_fixtures::query::clips_in_range;
use capia_fixtures::{FRAME, LargeSpec, build_large_project, offline_toolchain, proc};
use capia_model::{ClipContent, SequenceId};
use capia_project::{Project, RenderServices, RenderSettings};
use capia_store::{AutonomyStore, ProjectStore, StoreOptions, Synchronous};
use capia_time::Ticks;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn heavy_reps() -> usize {
    std::env::var("CAPIA_PERF_REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n >= 3)
        .unwrap_or(9)
}

const LIGHT: usize = 40;

fn full() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Full, // o custo real de um commit durável
        ..StoreOptions::default()
    }
}

fn set_opacity(op: &str, clip: &str, value: f64) -> Transaction {
    Transaction {
        transaction_id: None,
        label: "perf".into(),
        base_revision: None,
        commands: vec![CommandEnvelope {
            operation_id: op.into(),
            reference: None,
            command: Command::SetProperty {
                clip: clip.into(),
                prop: "opacity".into(),
                value,
            },
        }],
        max_ops: None,
    }
}

/// Conteúdo do documento sem o contador `revision` (undo/redo são revisões novas; o resto volta igual).
fn content(doc: &capia_model::Document) -> serde_json::Value {
    let mut v = serde_json::to_value(doc).unwrap();
    v.as_object_mut().unwrap().remove("revision");
    v
}

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        if std::env::var_os("CAPIA_KEEP_PERF").is_none() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
#[ignore = "benchmark: cargo test --release -p capia-project --test perf_large -- --ignored --nocapture --test-threads=1"]
#[allow(clippy::too_many_lines)]
fn phase6_large_project_benchmarks() {
    if cfg!(debug_assertions) {
        panic!("medição só vale em --release (o debug não representa o produto)");
    }
    let dir = Dir(std::env::temp_dir().join(format!("capia-perf-large-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("large.capia");
    let spec = LargeSpec::default();
    let stats = build_large_project(&path, &spec).unwrap();
    eprintln!(
        "PERF fixture: {} sequences, {} clips ({} nested, {} captions, {} audio), {} assets, {} runs, {} history entries, {:.1} MiB, built in {} ms",
        stats.sequences,
        stats.clips,
        stats.nested_clips,
        stats.caption_clips,
        stats.audio_clips,
        stats.catalog_assets,
        stats.runs,
        stats.history_entries,
        stats.file_bytes as f64 / 1_048_576.0,
        stats.build_ms
    );
    assert!(
        stats.sequences >= 30 && stats.clips >= 5_000,
        "fixture below the Phase 6 minimums"
    );
    let mut report = Report::new("phase6-large-project");
    report.scalar("fixture", serde_json::to_value(&stats).unwrap());
    report.scalar("spec", serde_json::to_value(&spec).unwrap());
    let reps = heavy_reps();
    let main: SequenceId = stats.biggest_sequence.as_str().into();
    let rss_start = proc::rss_kib();

    // ---- abrir / recarregar ------------------------------------------------------------------
    let open = time_reps(reps, |_| {
        let p = Project::open(&path, &full()).unwrap();
        assert_eq!(p.document().sequences().count(), stats.sequences);
        p
    });
    let st = report.time(
        "open_project",
        &open,
        "Project::open: doc + catálogo + jobs (cache de SO quente)",
    );
    assert!(st.p95 < 2_000.0, "open must stay under 2 s: {st:?}");

    let (store, _state) = ProjectStore::open(&path, &full()).unwrap();
    let reload = time_reps(reps, |_| {
        let s = store.load_state().unwrap();
        assert_eq!(document_digest(&s.doc), stats.document_digest);
        s
    });
    let st = report.time(
        "reload_state",
        &reload,
        "ProjectStore::load_state: snapshot + replay + validação",
    );
    assert!(st.p95 < 2_000.0, "reload must stay under 2 s: {st:?}");
    drop(store);

    // ---- Engine API (leituras) ---------------------------------------------------------------
    let mut api = ApiProbe::new();
    let api_open = ms_reps(reps.min(5), |_| {
        let t = api.open(&path).unwrap();
        api.close().unwrap();
        t.elapsed.as_secs_f64() * 1000.0
    });
    report.time(
        "api_project_open",
        &api_open,
        "Session project.open (inclui start_project)",
    );
    api.open(&path).unwrap();
    let mut snap_bytes = 0;
    let snapshot = time_reps(reps, |_| {
        let t = api.call("project.snapshot", json!({})).unwrap();
        assert_eq!(
            t.value["sequences"].as_array().unwrap().len(),
            stats.sequences
        );
        assert_eq!(
            t.value["assets"].as_array().unwrap().len(),
            stats.document_assets
        );
        snap_bytes = t.reply_bytes;
        ms(t.elapsed)
    });
    report.time(
        "api_project_snapshot",
        &snapshot,
        "Session project.snapshot (36 seq + 2.500 assets, com stat de arquivo por asset)",
    );
    report.scalar("project_snapshot_reply_bytes", json!(snap_bytes));
    let mut seq_bytes = 0;
    let seq_get = time_reps(reps, |_| {
        let t = api
            .call(
                "sequence.get",
                json!({ "sequence": stats.biggest_sequence }),
            )
            .unwrap();
        assert_eq!(
            t.value["clips"].as_object().map_or_else(
                || t.value["clips"].as_array().map_or(0, Vec::len),
                serde_json::Map::len
            ),
            stats.biggest_sequence_clips
        );
        seq_bytes = t.reply_bytes;
        ms(t.elapsed)
    });
    report.time(
        "api_sequence_get_biggest",
        &seq_get,
        "Session sequence.get da maior sequence (1.800 clips)",
    );
    report.scalar("sequence_get_reply_bytes", json!(seq_bytes));
    let mut hist_bytes = 0;
    let history = time_reps(reps, |_| {
        let t = api.call("history.list", json!({})).unwrap();
        assert_eq!(
            t.value["entries"].as_array().unwrap().len() as u64,
            stats.history_entries
        );
        hist_bytes = t.reply_bytes;
        ms(t.elapsed)
    });
    report.time(
        "api_history_list",
        &history,
        "Session history.list (≈350 entradas)",
    );
    report.scalar("history_list_reply_bytes", json!(hist_bytes));
    let assets_list = time_reps(reps, |_| {
        let t = api.call("assets.list", json!({})).unwrap();
        ms(t.elapsed)
    });
    report.time(
        "api_assets_list",
        &assets_list,
        "Session assets.list (2.500 assets offline)",
    );

    // ---- fila de export (sem encoder): abre instância própria, valida o lote, cancela ----------
    if api.call("engine.info", json!({})).unwrap().value["media_available"] == json!(true) {
        let mut start_ms = Vec::new();
        let mut drain_ms = Vec::new();
        for r in 0..reps.min(5) {
            let items: Vec<_> = (0..8)
                .map(|i| {
                    json!({
                        "id": format!("it{i}"),
                        "sequence": stats.biggest_sequence,
                        "preset": "intermediate",
                        "path": dir.0.join(format!("out-{r}-{i}")).display().to_string(),
                    })
                })
                .collect();
            let t0 = Instant::now();
            let started = api.call("export.start", json!({ "items": items })).unwrap();
            start_ms.push(ms(started.elapsed));
            let batch = started.value["batch"].as_str().unwrap().to_owned();
            api.call("export.cancel", json!({ "id": batch })).unwrap();
            let mut finished = false;
            while t0.elapsed() < Duration::from_secs(60) {
                let ev = api.call("events.poll", json!({})).unwrap();
                if ev
                    .value
                    .to_string()
                    .contains(&format!("\"export_batch_finished\",\"batch\":\"{batch}\""))
                    || ev.value.to_string().contains("export_batch_finished")
                {
                    finished = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(finished, "export batch never finished");
            drain_ms.push(ms(t0.elapsed()));
        }
        report.time(
            "export_queue_start",
            &start_ms,
            "export.start com 8 itens (retorno da chamada)",
        );
        report.time(
            "export_queue_start_to_cancelled",
            &drain_ms,
            "start → cancel → export_batch_finished (abre instância própria do projeto grande)",
        );
    } else {
        report.skip(
            "export_queue_start",
            "no ffmpeg/ffprobe: Session has no render services",
        );
        report.skip(
            "export_queue_start_to_cancelled",
            "no ffmpeg/ffprobe: Session has no render services",
        );
    }
    api.close().unwrap();
    drop(api);

    // ---- consulta de intervalo da timeline (virtualização) -----------------------------------
    {
        let p = Project::open(&path, &full()).unwrap();
        let seq = p.document().sequence(&main).unwrap();
        let total = seq.duration().0;
        let window = 60 * capia_time::TICKS_PER_SECOND; // viewport de 60 s
        let mut visible = 0usize;
        let q = time_reps(200, |i| {
            let start = Ticks((total - window) / 200 * i as i64);
            let v = clips_in_range(seq, start, Ticks(start.0 + window));
            visible += v.len();
            v.len()
        });
        let avg = visible as f64 / 200.0;
        assert!(avg > 20.0, "viewport must contain real clips (avg {avg})");
        report.scalar("range_query_avg_visible_clips", json!(avg));
        report.time(
            "timeline_range_query_60s",
            &q,
            "clips_in_range (todas as tracks) em viewport de 60 s da maior sequence",
        );
        // exigência de fluidez: a consulta cabe com folga num quadro de 16 ms
        assert!(Stat::of(&q).p95 < 16.0);
        let all = time_reps(30, |_| {
            clips_in_range(seq, Ticks::ZERO, Ticks(total + 1)).len()
        });
        report.time(
            "timeline_range_query_all",
            &all,
            "clips_in_range da sequence inteira (zoom-to-fit, 1.800 clips)",
        );
    }

    // ---- grafo de render / quadro de preview ---------------------------------------------------
    {
        let services = Arc::new(RenderServices::new(offline_toolchain()));
        let mut settings = RenderSettings::new(404, 720);
        settings.strict_sources = false;
        settings.design_size = Some((1920, 1080));
        let p = Project::open(&path, &full()).unwrap();
        let graph = time_reps(reps, |_| p.render_graph(&main).unwrap());
        report.time(
            "render_graph_compile_biggest",
            &graph,
            "compilar o grafo da maior sequence (também o 1º passo do export)",
        );

        let total = p.document().sequence(&main).unwrap().duration().0;
        let times: Vec<Ticks> = (1..=LIGHT as i64)
            .map(|k| Ticks((total / (LIGHT as i64 + 2) * k) / FRAME * FRAME))
            .collect();
        let mut warnings = 0usize;
        let mut layers = 0usize;
        // frio: projeto recém-aberto (grafo compilado no 1º quadro)
        let cold = ms_reps(reps.min(5), |_| {
            let fresh = Project::open(&path, &full()).unwrap();
            let t = Instant::now();
            let f = fresh
                .render_frame(&services, &main, times[LIGHT / 2], &settings)
                .unwrap();
            let el = ms(t.elapsed());
            std::hint::black_box(f.image.data.len());
            el
        });
        report.time(
            "preview_frame_cold",
            &cold,
            "1º quadro após abrir (compila o grafo; 404×720, compositor CPU)",
        );
        let warm = time_reps(LIGHT, |i| {
            let f = p
                .render_frame(&services, &main, times[i], &settings)
                .unwrap();
            warnings += f.warnings.len();
            layers += clips_in_range(
                p.document().sequence(&main).unwrap(),
                times[i],
                Ticks(times[i].0 + 1),
            )
            .len();
            assert_eq!(f.image.width, 404);
            f
        });
        let st = report.time("preview_frame_warm", &warm, "quadros distintos com grafo em cache (404×720, compositor CPU; fontes offline viram aviso)");
        report.scalar(
            "preview_frame_avg_active_layers",
            json!(layers as f64 / LIGHT as f64),
        );
        report.scalar("preview_frame_warnings_total", json!(warnings));
        assert!(st.p50 < 100.0, "scrub p50 must stay under 100 ms: {st:?}");
        // quadro dentro de um clip nested (render recursivo)
        let seq = p.document().sequence(&main).unwrap();
        let nested = seq
            .clips()
            .filter(|c| matches!(c.content, ClipContent::Nested { .. }))
            .min_by_key(|c| (c.start, c.id.clone()))
            .unwrap();
        let t_nested = Ticks(nested.start.0 + 5 * FRAME);
        let nested_ms = time_reps(LIGHT / 2, |_| {
            p.render_frame(&services, &main, t_nested, &settings)
                .unwrap()
        });
        report.time(
            "preview_frame_nested",
            &nested_ms,
            "mesmo quadro dentro de um clip nested (render recursivo)",
        );
    }

    // ---- histórico de AI Runs ------------------------------------------------------------------
    {
        let runs = AutonomyStore::open(&path, Duration::from_secs(5)).unwrap();
        let list = time_reps(reps, |_| runs.list_runs(None, 100).unwrap().len());
        report.time("ai_runs_list", &list, "AutonomyStore::list_runs(100)");
        let mut total_events = 0;
        let full_read = time_reps(reps, |_| {
            let mut n = 0;
            for r in runs.list_runs(None, 100).unwrap() {
                n += runs.list_stages(&r.run_id).unwrap().len();
                n += runs.events_after(&r.run_id, 0, 5_000).unwrap().len();
                n += runs.effects_for_run(&r.run_id).unwrap().len();
                n += runs.ledger_rows(&r.run_id).unwrap().len();
            }
            total_events = n;
            n
        });
        assert!(
            total_events >= stats.run_stages + stats.run_events,
            "read everything: {total_events}"
        );
        report.time(
            "ai_runs_full_history_read",
            &full_read,
            "60 runs: stages + eventos + efeitos + orçamento (carga do painel de histórico)",
        );
    }

    // ---- commit / histórico / undo-redo (escrita durável; por último: altera o arquivo) -----------
    {
        let mut p = Project::open(&path, &full()).unwrap();
        let clips: Vec<String> = p
            .document()
            .sequence(&main)
            .unwrap()
            .clips()
            .filter(|c| c.id.as_str().contains("_v1_"))
            .map(|c| c.id.to_string())
            .take(400)
            .collect();
        let user = Actor::user("perf");
        let commit = time_reps(LIGHT, |i| {
            p.execute(&user, set_opacity(&format!("pf-c-{i}"), &clips[i], 0.4))
                .unwrap()
        });
        let st = report.time(
            "commit_small",
            &commit,
            "Project::execute 1 comando (commit durável FULL; engine + journal + history append)",
        );
        assert!(
            st.p95 < 200.0,
            "durable transaction must stay under 200 ms: {st:?}"
        );

        let bulk = time_reps(reps, |r| {
            let commands: Vec<CommandEnvelope> = (0..50)
                .map(|k| CommandEnvelope {
                    operation_id: format!("pf-b-{r}-{k}"),
                    reference: None,
                    command: Command::SetProperty {
                        clip: clips[100 + k].as_str().into(),
                        prop: "opacity".into(),
                        value: 0.3 + (r % 5) as f64 / 10.0,
                    },
                })
                .collect();
            p.execute(
                &user,
                Transaction {
                    transaction_id: None,
                    label: "perf bulk".into(),
                    base_revision: None,
                    commands,
                    max_ops: None,
                },
            )
            .unwrap()
        });
        report.time(
            "commit_bulk_50",
            &bulk,
            "Project::execute com 50 comandos numa transação",
        );
        let rev_before = p.engine().revision();
        let content_before = content(p.document());

        let undo = time_reps(LIGHT, |_| p.undo(&user).unwrap());
        let redo = time_reps(LIGHT, |_| p.redo(&user).unwrap());
        assert_eq!(
            p.engine().revision(),
            rev_before + 2 * LIGHT as u64,
            "undo/redo are new revisions"
        );
        assert!(
            content(p.document()) == content_before,
            "undo×N then redo×N restores the document content"
        );
        // as metas de 50 ms (undo/redo) e 100 ms (scrub) são impostas pelo GATE sobre a mediana de N
        // execuções (um único p95 aqui falharia por ruído de runner compartilhado)
        report.time(
            "undo",
            &undo,
            "Project::undo (patches inversos; commit durável do evento)",
        );
        report.time("redo", &redo, "Project::redo");
        drop(p);

        // pior caso: todo commit grava um snapshot do documento inteiro (6.000 clips)
        let snap_opts = StoreOptions {
            snapshot_every: 1,
            ..full()
        };
        let mut p = Project::open(&path, &snap_opts).unwrap();
        let snap = time_reps(reps, |i| {
            p.execute(
                &user,
                set_opacity(&format!("pf-s-{i}"), &clips[200 + i], 0.7),
            )
            .unwrap()
        });
        report.time(
            "commit_with_snapshot",
            &snap,
            "commit que também grava o snapshot do documento (snapshot_every=1; pior caso)",
        );
    }

    // ---- recursos --------------------------------------------------------------------------------
    report.scalar("rss_kib_start", json!(rss_start));
    report.scalar("rss_kib_end", json!(proc::rss_kib()));
    report.scalar("peak_rss_kib", json!(proc::peak_rss_kib()));
    let out = report.write("phase6-large-project.json").unwrap();
    eprintln!("PERF report: {}", out.display());
    assert_eq!(out, perf_out_path("phase6-large-project.json"));
}
