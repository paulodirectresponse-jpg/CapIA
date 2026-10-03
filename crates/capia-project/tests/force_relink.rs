//! Force relink: troca de conteúdo explícita, com validação dos clips dependentes pelo engine.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{Actor, Command, CommandEnvelope, NewClip, Transaction};
use capia_media::{FfprobeBackend, MediaConfig, MediaToolchain};
use capia_model::{AssetId, ClipContent, ErrorCode};
use capia_project::{Project, ProjectError};
use capia_store::{StoreOptions, Synchronous};
use capia_time::{FrameRate, Rational, TICKS_PER_SECOND, Ticks};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-force-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn fixture(&self, name: &str, to: &str) -> PathBuf {
        let dst = self.0.join(to);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/media")
                .join(name),
            &dst,
        )
        .unwrap();
        dst
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn probe() -> Option<FfprobeBackend> {
    match MediaToolchain::locate(&MediaConfig::default()) {
        Ok(t) => Some(FfprobeBackend::new(t)),
        Err(e) => {
            assert!(std::env::var_os("CAPIA_REQUIRE_FFMPEG").is_none(), "{e}");
            eprintln!("SKIP (no ffprobe)");
            None
        }
    }
}

macro_rules! need {
    () => {
        match probe() {
            Some(p) => p,
            None => return,
        }
    };
}

fn user() -> Actor {
    Actor::user("t")
}

fn project(t: &Tmp) -> Project {
    Project::create(
        &t.0.join("p.capia"),
        &StoreOptions {
            synchronous: Synchronous::Normal,
            ..StoreOptions::default()
        },
    )
    .unwrap()
}

fn env(op: &str, command: Command) -> CommandEnvelope {
    CommandEnvelope {
        operation_id: op.into(),
        reference: None,
        command,
    }
}

fn tx(commands: Vec<CommandEnvelope>) -> Transaction {
    Transaction {
        transaction_id: None,
        label: "t".into(),
        base_revision: None,
        commands,
        max_ops: None,
    }
}

fn place(p: &mut Project, asset: &AssetId, clip: &str, dur_ticks: i64, audio: bool) {
    if p.document().sequences().next().is_none() {
        p.execute(
            &user(),
            tx(vec![
                env(
                    "seq",
                    Command::CreateSequence {
                        id: Some("S".into()),
                        name: "S".into(),
                        frame_rate: FrameRate::FPS_30,
                        sample_rate: None,
                    },
                ),
                env(
                    "trk",
                    Command::AddTrack {
                        sequence: "S".into(),
                        id: Some("V1".into()),
                        name: None,
                        kind: capia_model::TrackKind::Visual,
                        role: None,
                        magnetic: false,
                        index: None,
                    },
                ),
            ]),
        )
        .unwrap();
    }
    p.execute(
        &user(),
        tx(vec![env(
            &format!("op-{clip}"),
            Command::InsertClip {
                track: "V1".into(),
                start: Ticks::ZERO,
                clip: NewClip {
                    id: Some(clip.into()),
                    name: clip.into(),
                    content: ClipContent::Media {
                        asset: asset.clone(),
                        has_video: true,
                        has_audio: audio,
                    },
                    duration: Ticks(dur_ticks),
                    source_in: Ticks::ZERO,
                    speed: Rational::ONE,
                    reversed: false,
                    properties: Default::default(),
                },
                split_at_insert: false,
                split_new_id: None,
            },
        )]),
    )
    .unwrap();
}

const HALF: i64 = TICKS_PER_SECOND / 2;

#[test]
fn a_clip_that_would_exceed_the_new_media_blocks_the_force_relink_without_trimming() {
    let pr = need!();
    let t = Tmp::new("exceeds");
    let mut p = project(&t);
    let long = t.fixture("cfr_gop.mp4", "m/long.mp4"); // 2 s, vídeo
    let short = t.fixture("video_only.mp4", "m/short.mp4"); // 1 s, vídeo
    let id = p.import_asset(&user(), &long, &pr).unwrap().asset_id;
    place(&mut p, &id, "C1", TICKS_PER_SECOND * 3 / 2, false); // 1,5 s > 1 s da mídia nova
    let rev = p.document().revision;
    let old_hash = p.asset(&id).unwrap().catalog.unwrap().content_hash;
    for dry in [true, false] {
        let e = p
            .force_relink_asset(&user(), &id, &short, &pr, dry)
            .unwrap_err();
        let ProjectError::Command(c) = &e else {
            panic!("{e}");
        };
        assert_eq!(c.code, ErrorCode::Conflict);
        let hint = c.hint.as_ref().unwrap();
        let conflicts = hint["conflicts"].as_array().unwrap();
        assert_eq!(conflicts[0]["clip"], "C1");
        assert_eq!(
            conflicts[0]["reasons"][0]["reason"],
            "SOURCE_RANGE_EXCEEDS_NEW_MEDIA"
        );
    }
    // nada mudou: documento, catálogo e clip intactos (sem trim silencioso)
    assert_eq!(p.document().revision, rev);
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().content_hash,
        old_hash
    );
    let clip = p
        .document()
        .sequences()
        .next()
        .unwrap()
        .1
        .clips()
        .next()
        .unwrap()
        .clone();
    assert_eq!(clip.duration, Ticks(TICKS_PER_SECOND * 3 / 2));
}

#[test]
fn a_missing_audio_stream_is_a_structured_conflict() {
    let pr = need!();
    let t = Tmp::new("noaudio");
    let mut p = project(&t);
    let with_audio = t.fixture("video_audio.mp4", "m/va.mp4");
    let silent = t.fixture("video_only.mp4", "m/vo.mp4");
    let id = p.import_asset(&user(), &with_audio, &pr).unwrap().asset_id;
    place(&mut p, &id, "C1", HALF, true);
    let e = p
        .force_relink_asset(&user(), &id, &silent, &pr, false)
        .unwrap_err();
    let ProjectError::Command(c) = &e else {
        panic!("{e}");
    };
    let reasons = &c.hint.as_ref().unwrap()["conflicts"][0]["reasons"];
    assert_eq!(reasons[0]["reason"], "MISSING_AUDIO_STREAM");
}

#[test]
fn a_compatible_force_relink_keeps_the_id_updates_both_layers_and_drops_stale_caches() {
    let pr = need!();
    let t = Tmp::new("ok");
    let mut p = project(&t);
    let long = t.fixture("cfr_gop.mp4", "m/long.mp4"); // 2 s
    let short = t.fixture("video_only.mp4", "m/short.mp4"); // 1 s
    let id = p.import_asset(&user(), &long, &pr).unwrap().asset_id;
    place(&mut p, &id, "C1", HALF, false);
    // um derivado do conteúdo ANTIGO no cache
    let tc = MediaToolchain::locate(&MediaConfig::default()).unwrap();
    if tc.ffmpeg.is_some() {
        p.frame_source(&id, &tc, &|| false).unwrap();
        assert_eq!(p.cache_usage().unwrap().files, 1);
    }
    let old = p.asset(&id).unwrap().catalog.unwrap();
    let dry = p
        .force_relink_asset(&user(), &id, &short, &pr, true)
        .unwrap();
    assert!(dry.dry_run && !dry.same_content && dry.commit.is_none());
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().content_hash,
        old.content_hash,
        "dry run writes nothing"
    );
    let r = p
        .force_relink_asset(&user(), &id, &short, &pr, false)
        .unwrap();
    assert_eq!(r.asset_id, id, "the id is stable");
    assert_ne!(r.old_hash, r.new_hash);
    assert_eq!(r.dependents.len(), 1);
    assert!(r.commit.is_some());
    let now = p.asset(&id).unwrap();
    let cat = now.catalog.unwrap();
    assert_eq!(cat.content_hash.to_string(), r.new_hash);
    assert!(cat.location.path.ends_with("short.mp4"));
    assert!(
        cat.known_paths.is_empty(),
        "aliases of the old content are dropped"
    );
    assert_eq!(now.document.unwrap().duration, cat.media.duration);
    assert_eq!(
        p.cache_usage().unwrap().files,
        0,
        "derived data of the old content is gone"
    );
    // trilha de auditoria
    let kinds: Vec<_> = p
        .asset_events(&id)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(kinds.last().map(String::as_str), Some("force_relink"));
    // sobrevive a fechar/reabrir
    drop(p);
    let p = Project::open(&t.0.join("p.capia"), &StoreOptions::default()).unwrap();
    assert_eq!(
        p.asset(&id)
            .unwrap()
            .catalog
            .unwrap()
            .content_hash
            .to_string(),
        r.new_hash
    );
}

#[test]
fn forcing_into_content_that_is_already_another_asset_is_refused() {
    let pr = need!();
    let t = Tmp::new("dup");
    let mut p = project(&t);
    let a = t.fixture("video_audio.mp4", "m/a.mp4");
    let b = t.fixture("video_only.mp4", "m/b.mp4");
    let ia = p.import_asset(&user(), &a, &pr).unwrap().asset_id;
    let ib = p.import_asset(&user(), &b, &pr).unwrap().asset_id;
    assert_ne!(ia, ib);
    let e = p
        .force_relink_asset(&user(), &ia, &b, &pr, false)
        .unwrap_err();
    let ProjectError::Asset(ae) = &e else {
        panic!("{e}");
    };
    assert_eq!(
        ae.details.as_ref().unwrap()["code"],
        "FORCE_RELINK_DUPLICATE_CONTENT"
    );
}

#[test]
fn the_same_content_is_just_a_regular_relink() {
    let pr = need!();
    let t = Tmp::new("same");
    let mut p = project(&t);
    let a = t.fixture("video_audio.mp4", "m/a.mp4");
    let id = p.import_asset(&user(), &a, &pr).unwrap().asset_id;
    let moved = t.fixture("video_audio.mp4", "other/moved.mp4");
    std::fs::remove_file(&a).unwrap();
    let r = p
        .force_relink_asset(&user(), &id, &moved, &pr, false)
        .unwrap();
    assert!(r.same_content && r.commit.is_none());
    assert!(
        p.asset(&id)
            .unwrap()
            .catalog
            .unwrap()
            .location
            .path
            .ends_with("moved.mp4")
    );
}
