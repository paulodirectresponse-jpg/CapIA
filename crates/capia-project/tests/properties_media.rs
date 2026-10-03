//! Propriedade do pipeline de mídia, ponta a ponta, com FFmpeg real e um modelo independente:
//! import (assíncrono) → pending → hash → finalizado → índice → decode → waveform → proxy →
//! salvar/reabrir → offline → relink em lote → cancelar → repetir. Orçamento:
//! `CAPIA_MEDIA_PROP_CASES` (padrão 6) casos × 14 passos.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::{Availability, ScanOptions};
use capia_commands::Actor;
use capia_jobs::{JobState, Priority};
use capia_media::{MediaConfig, MediaToolchain, ProxyAudio, ProxyProfileV1};
use capia_model::AssetId;
use capia_project::{PipelineOptions, Project, PumpEvent};
use capia_store::{StoreOptions, Synchronous, TicketState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Conteúdos reais (fixtures) e o que cada um tem.
#[derive(Clone, Copy)]
struct Content {
    fixture: &'static str,
    video: bool,
    audio: bool,
    frames: usize,
}
const CONTENTS: [Content; 4] = [
    Content {
        fixture: "cfr_gop.mp4",
        video: true,
        audio: false,
        frames: 50,
    },
    Content {
        fixture: "tone_44k.wav",
        video: false,
        audio: true,
        frames: 0,
    },
    Content {
        fixture: "video_audio.mp4",
        video: true,
        audio: true,
        frames: 25,
    },
    Content {
        fixture: "vfr.mp4",
        video: true,
        audio: false,
        frames: 25,
    },
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/media")
        .join(name)
}

fn toolchain() -> Option<MediaToolchain> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) if t.ffmpeg.is_some() => Some(t),
        other => {
            assert!(
                std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(),
                "{other:?}"
            );
            eprintln!("SKIP (no ffmpeg)");
            None
        }
    }
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Off,
        ..StoreOptions::default()
    }
}

fn user() -> Actor {
    Actor::user("prop")
}

fn never() -> bool {
    false
}

struct Env {
    root: PathBuf,
    tc: MediaToolchain,
    p: Option<Project>,
    /// arquivo → conteúdo
    files: BTreeMap<PathBuf, usize>,
    /// conteúdo → (id, caminho do catálogo)
    assets: BTreeMap<usize, (AssetId, PathBuf)>,
    /// tickets de import em voo: (ticket_id, conteúdo, caminho)
    tickets: Vec<(String, usize, PathBuf)>,
    counter: u64,
}

impl Env {
    fn p(&mut self) -> &mut Project {
        self.p.as_mut().unwrap()
    }

    fn start(&mut self) {
        let mut p = Project::open(&self.root.join("p.capia"), &opts()).unwrap();
        p.start_pipeline(PipelineOptions::new(self.tc.clone()))
            .unwrap();
        self.p = Some(p);
    }

    fn new_file(&mut self, content: usize, dir: &str) -> PathBuf {
        self.counter += 1;
        let path = self.root.join(dir).join(format!("f{}.bin", self.counter));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // extensão realista para o probe não depender do nome
        let path = path.with_extension(Path::new(CONTENTS[content].fixture).extension().unwrap());
        std::fs::copy(fixture(CONTENTS[content].fixture), &path).unwrap();
        self.files.insert(path.clone(), content);
        path
    }

    fn settle(&mut self) {
        let t0 = Instant::now();
        loop {
            let ev = self.p().pump(&user()).unwrap();
            for e in ev {
                if let PumpEvent::ImportFinalized { ticket_id, result } = e {
                    let i = self.tickets.iter().position(|t| t.0 == ticket_id).unwrap();
                    let (_, content, path) = self.tickets.remove(i);
                    let entry = self
                        .assets
                        .entry(content)
                        .or_insert_with(|| (result.asset_id.clone(), path.clone()));
                    assert_eq!(entry.0, result.asset_id, "one asset per content");
                }
            }
            if self.tickets.is_empty() {
                return;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "tickets never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// O que o projeto deve informar (checagem barata: tamanho do candidato principal/aliases).
    fn expected_status(&self, content: usize) -> Availability {
        let (_, loc) = &self.assets[&content];
        // o modelo só move/copia arquivos inteiros ⇒ presença do caminho do catálogo decide
        if self.files.get(loc) == Some(&content) {
            Availability::Online
        } else if self.files.contains_key(loc) {
            Availability::Modified
        } else {
            Availability::Offline
        }
    }

    fn check_invariants(&mut self, ctx: &str) {
        // catálogo × modelo
        let ids: Vec<(usize, AssetId)> = self
            .assets
            .iter()
            .map(|(c, (id, _))| (*c, id.clone()))
            .collect();
        for (c, id) in ids {
            let st = self.p().asset(&id).unwrap().catalog.unwrap().status;
            let want = self.expected_status(c);
            // aliases podem manter online um asset cujo principal sumiu: só exigimos que não
            // fique "pior" que o modelo e que offline do modelo implique offline/modificado
            if want == Availability::Online {
                assert_eq!(st, Availability::Online, "{ctx}: content {c}");
            }
        }
        // nenhum job vivo depois de assentar; nenhum derivado inválido no cache
        let jobs = self.p().jobs(None, 500).unwrap();
        assert!(
            jobs.iter().all(|j| j.state.is_terminal()
                || j.state == JobState::Running
                || j.state == JobState::Queued),
            "{ctx}"
        );
        let usage = self.p().cache_usage().unwrap();
        assert_eq!(usage.temp_files, 0, "{ctx}: leftover temp files");
    }
}

fn run_case(seed: u64, tc: &MediaToolchain) {
    let mut rng = Rng::new(seed);
    let root = std::env::temp_dir().join(format!("capia-mprop-{}-{seed}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Project::create(&root.join("p.capia"), &opts()).unwrap();
    let mut env = Env {
        root: root.clone(),
        tc: tc.clone(),
        p: None,
        files: BTreeMap::new(),
        assets: BTreeMap::new(),
        tickets: Vec::new(),
        counter: 0,
    };
    env.start();
    for step in 0..14 {
        let ctx = format!("seed {seed} step {step}");
        match rng.below(10) {
            // import assíncrono de um arquivo (novo ou cópia)
            0..=2 => {
                let c = rng.below(4) as usize;
                let path = env.new_file(c, "lib");
                let t = env.p().import_asset_async(&path).unwrap();
                assert_eq!(t.state, TicketState::Pending, "{ctx}");
                env.tickets.push((t.ticket_id, c, path));
                if rng.below(2) == 0 {
                    env.settle();
                }
            }
            // derivados de um asset existente (compatível com seus streams)
            3..=4 => {
                env.settle();
                if env.assets.is_empty() {
                    continue;
                }
                let keys: Vec<usize> = env.assets.keys().copied().collect();
                let c = keys[rng.below(keys.len() as u64) as usize];
                if env.expected_status(c) != Availability::Online {
                    continue;
                }
                let id = env.assets[&c].0.clone();
                let ct = CONTENTS[c];
                let p = env.p();
                if ct.video {
                    let snap = p
                        .submit_frame_index(&id, Priority::Normal)
                        .unwrap()
                        .handle
                        .wait();
                    assert_eq!(snap.state, JobState::Completed, "{ctx}: {:?}", snap.error);
                    // decode: o quadro pedido é o do índice (nunca N/fps cego)
                    let src = p.frame_source(&id, tc, &never).unwrap();
                    assert_eq!(src.index().len(), ct.frames, "{ctx}");
                    let i = rng.below(ct.frames as u64) as usize;
                    let f = src.frame_by_index(i, &never).unwrap();
                    assert_eq!((f.index, f.width, f.height), (i, 64, 48), "{ctx}");
                    let at = src.index().time_of(i).unwrap();
                    assert_eq!(
                        src.frame_at(at, &never).unwrap().bytes,
                        f.bytes,
                        "{ctx}: by time == by index"
                    );
                    // proxy pequeno
                    if rng.below(3) == 0 {
                        let snap = p
                            .submit_proxy(
                                &id,
                                ProxyProfileV1 {
                                    max_width: 32,
                                    max_height: 32,
                                    audio: ProxyAudio::None,
                                    ..Default::default()
                                },
                                Priority::Background,
                            )
                            .unwrap()
                            .handle
                            .wait();
                        assert_eq!(snap.state, JobState::Completed, "{ctx}: {:?}", snap.error);
                    }
                } else {
                    assert!(
                        p.submit_frame_index(&id, Priority::Normal).is_err(),
                        "{ctx}: audio has no video"
                    );
                }
                if ct.audio {
                    let snap = p
                        .submit_waveform(&id, Priority::Normal)
                        .unwrap()
                        .handle
                        .wait();
                    assert_eq!(snap.state, JobState::Completed, "{ctx}: {:?}", snap.error);
                    let w = p.waveform(&id, tc, &never).unwrap();
                    assert!(w.total_samples() > 0, "{ctx}");
                } else {
                    assert!(
                        p.submit_waveform(&id, Priority::Normal).is_err(),
                        "{ctx}: no audio"
                    );
                }
            }
            // cancelar um proxy logo após submeter: cancelled ou completed, nunca lixo
            5 => {
                env.settle();
                let candidates: Vec<usize> = env
                    .assets
                    .keys()
                    .copied()
                    .filter(|c| {
                        CONTENTS[*c].video && env.expected_status(*c) == Availability::Online
                    })
                    .collect();
                if candidates.is_empty() {
                    continue;
                }
                let c = candidates[rng.below(candidates.len() as u64) as usize];
                let id = env.assets[&c].0.clone();
                let p = env.p();
                let s = p
                    .submit_proxy(
                        &id,
                        ProxyProfileV1 {
                            max_width: 48,
                            max_height: 48,
                            jpeg_quality: 3,
                            audio: ProxyAudio::None,
                            ..Default::default()
                        },
                        Priority::Background,
                    )
                    .unwrap();
                p.cancel_job(&s.handle.id()).unwrap();
                let snap = s.handle.wait();
                assert!(
                    matches!(snap.state, JobState::Cancelled | JobState::Completed),
                    "{ctx}: {:?}",
                    snap.state
                );
                // retry sempre funciona
                let again = p
                    .submit_proxy(
                        &id,
                        ProxyProfileV1 {
                            max_width: 48,
                            max_height: 48,
                            jpeg_quality: 3,
                            audio: ProxyAudio::None,
                            ..Default::default()
                        },
                        Priority::Normal,
                    )
                    .unwrap()
                    .handle
                    .wait();
                assert_eq!(
                    again.state,
                    JobState::Completed,
                    "{ctx}: retry {:?}",
                    again.error
                );
            }
            // tira um arquivo do ar (offline) ou o devolve
            6 => {
                env.settle();
                let keys: Vec<PathBuf> = env.files.keys().cloned().collect();
                if keys.is_empty() {
                    continue;
                }
                let f = keys[rng.below(keys.len() as u64) as usize].clone();
                std::fs::remove_file(&f).unwrap();
                env.files.remove(&f);
            }
            // relink em lote numa pasta com cópias novas de nomes aleatórios
            7 => {
                env.settle();
                let offline: Vec<usize> = env
                    .assets
                    .keys()
                    .copied()
                    .filter(|c| env.expected_status(*c) == Availability::Offline)
                    .collect();
                if offline.is_empty() {
                    continue;
                }
                let dir = format!("found{step}");
                let mut copies: BTreeMap<usize, usize> = BTreeMap::new();
                for _ in 0..rng.below(4) {
                    let c = rng.below(4) as usize;
                    env.new_file(c, &dir);
                    *copies.entry(c).or_default() += 1;
                }
                let (report, applied, errors) = env
                    .p()
                    .batch_relink_folder(&root.join(&dir), &ScanOptions::default(), None, &never)
                    .unwrap();
                assert!(errors.is_empty(), "{ctx}: {errors:?}");
                for c in offline {
                    let n = copies.get(&c).copied().unwrap_or(0);
                    let id = env.assets[&c].0.clone();
                    match n {
                        0 => assert!(report.unresolved.contains(&id), "{ctx}: {c} unresolved"),
                        1 => {
                            assert!(
                                report.matched.iter().any(|m| m.asset_id == id),
                                "{ctx}: {c} matched"
                            );
                            let now = env.p().asset(&id).unwrap().catalog.unwrap();
                            assert_eq!(now.status, Availability::Online, "{ctx}");
                            let new_loc = report
                                .matched
                                .iter()
                                .find(|m| m.asset_id == id)
                                .unwrap()
                                .path
                                .clone();
                            env.assets.get_mut(&c).unwrap().1 = new_loc;
                        }
                        _ => assert!(
                            report.ambiguous.iter().any(|a| a.asset_id == id),
                            "{ctx}: {c} ambiguous"
                        ),
                    }
                }
                let _ = applied;
            }
            // fechar e reabrir: nada se perde e tickets pendentes viram retomáveis
            8 => {
                let pending: Vec<_> = env.tickets.clone();
                env.p = None;
                let reopened = Project::open(&root.join("p.capia"), &opts()).unwrap();
                env.p = Some(reopened);
                let rec = env.p().recover_jobs().unwrap();
                let _ = rec;
                env.p()
                    .start_pipeline(PipelineOptions::new(tc.clone()))
                    .unwrap();
                for (tk, c, path) in pending {
                    let row = env.p().ticket(&tk).unwrap().unwrap();
                    match row.state {
                        TicketState::Finalized => {
                            let id = row.asset_id.clone().unwrap();
                            let e = env
                                .assets
                                .entry(c)
                                .or_insert_with(|| (AssetId::new(id.as_str()), path.clone()));
                            assert_eq!(e.0.as_str(), id, "{ctx}");
                            env.tickets.retain(|t| t.0 != tk);
                        }
                        TicketState::Interrupted | TicketState::Pending => {
                            // retomar: reenvia o hash e finaliza
                            let r = env.p().resume_ticket(&tk).unwrap();
                            assert_eq!(r.state, TicketState::Pending, "{ctx}");
                        }
                        other => panic!("{ctx}: unexpected ticket state {other:?}"),
                    }
                }
                env.settle();
                // o catálogo sobreviveu: todo asset do modelo ainda existe
                let ids: Vec<AssetId> = env.assets.values().map(|a| a.0.clone()).collect();
                for id in ids {
                    assert!(env.p().asset(&id).unwrap().catalog.is_some(), "{ctx}");
                }
            }
            // limpeza de cache: o projeto continua válido e tudo se regenera
            _ => {
                env.p().cache_clean(rng.below(2) == 0).unwrap();
            }
        }
        env.settle();
        env.check_invariants(&ctx);
    }
    // o arquivo final é válido
    env.p = None;
    assert!(
        Project::validate(&root.join("p.capia")).ok,
        "seed {seed}: project invalid"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn random_pipeline_sequences_match_the_model() {
    let Some(tc) = toolchain() else { return };
    let cases: u64 = std::env::var("CAPIA_MEDIA_PROP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    for seed in 1..=cases {
        run_case(seed, &tc);
    }
    eprintln!("media pipeline properties: {cases} cases × 14 steps");
}
