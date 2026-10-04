//! Fluxo de assets de ponta a ponta na fachada: import atômico, dedup, persistência, offline,
//! relink, verify e integração com clips — probe sintético (sem ffprobe).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_assets::testing::StaticProbe;
use capia_assets::{AssetErrorCode, Availability};
use capia_commands::{Actor, Command, CommandEnvelope, NewClip, Transaction};
use capia_media::{MediaError, MediaErrorCode, MediaInfo, MediaProbe};
use capia_model::{AssetId, ClipContent, ErrorCode};
use capia_project::{ImportOutcome, Project, ProjectError};
use capia_store::{StoreOptions, Synchronous};
use capia_time::{FrameRate, Rational, Ticks};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "capia-proj-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }
    fn project(&self) -> PathBuf {
        self.0.join("p.capia")
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn opts() -> StoreOptions {
    StoreOptions {
        synchronous: Synchronous::Normal,
        ..StoreOptions::default()
    }
}

fn user() -> Actor {
    Actor::user("test")
}

fn create(t: &Tmp) -> Project {
    Project::create(&t.project(), &opts()).unwrap()
}

fn reopen(t: &Tmp) -> Project {
    Project::open(&t.project(), &opts()).unwrap()
}

fn env(op: &str, command: Command) -> CommandEnvelope {
    CommandEnvelope {
        operation_id: op.into(),
        reference: None,
        command,
    }
}

fn tx(label: &str, commands: Vec<CommandEnvelope>) -> Transaction {
    Transaction {
        transaction_id: None,
        label: label.into(),
        base_revision: None,
        commands,
        max_ops: None,
    }
}

fn setup_sequence(p: &mut Project) {
    p.execute(
        &user(),
        tx(
            "setup",
            vec![
                env(
                    "seq",
                    Command::CreateSequence {
                        id: Some("S".into()),
                        name: "S".into(),
                        frame_rate: FrameRate::FPS_30,
                        sample_rate: None,
                        width: None,
                        height: None,
                        folder: None,
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
            ],
        ),
    )
    .unwrap();
}

fn place(
    p: &mut Project,
    asset: &AssetId,
    clip: &str,
    start: i64,
    dur: i64,
) -> Result<(), capia_commands::CommandError> {
    p.execute(
        &user(),
        tx(
            "place",
            vec![env(
                &format!("op-{clip}"),
                Command::InsertClip {
                    track: "V1".into(),
                    start: Ticks(start),
                    clip: NewClip {
                        id: Some(clip.into()),
                        name: clip.into(),
                        content: ClipContent::Media {
                            asset: asset.clone(),
                            has_video: true,
                            has_audio: true,
                        },
                        duration: Ticks(dur),
                        source_in: Ticks::ZERO,
                        speed: Rational::ONE,
                        reversed: false,
                        properties: Default::default(),
                    },
                    split_at_insert: false,
                    split_new_id: None,
                },
            )],
        ),
    )
    .map(|_| ())
}

fn import(p: &mut Project, path: &Path) -> Result<capia_project::ImportResult, ProjectError> {
    p.import_asset(&user(), path, &StaticProbe)
}

#[test]
fn import_video_audio_image_registers_document_and_catalog_together() {
    let t = Tmp::new("kinds");
    let mut p = create(&t);
    let v = import(&mut p, &t.file("m/a.mp4", b"video")).unwrap();
    let a = import(&mut p, &t.file("m/a.wav", b"audio")).unwrap();
    let i = import(&mut p, &t.file("m/a.png", b"image")).unwrap();
    for r in [&v, &a, &i] {
        assert_eq!(r.outcome, ImportOutcome::Created);
        assert!(r.commit.is_some());
        assert!(p.document().asset(&r.asset_id).is_some(), "document layer");
    }
    let views = p.assets().unwrap();
    assert_eq!(views.len(), 3);
    assert!(views.iter().all(|x| x.in_document && x.catalog.is_some()));
    // o asset lógico do documento reflete a mídia
    let doc_v = p.document().asset(&v.asset_id).unwrap();
    assert!(doc_v.has_video && doc_v.has_audio && doc_v.duration.is_some());
    let doc_a = p.document().asset(&a.asset_id).unwrap();
    assert!(!doc_a.has_video && doc_a.has_audio);
    let doc_i = p.document().asset(&i.asset_id).unwrap();
    assert_eq!(doc_i.duration, None, "image duration belongs to the clip");
}

#[test]
fn failed_imports_leave_nothing_behind() {
    let t = Tmp::new("fail");
    let mut p = create(&t);
    let revision = p.document().revision;
    let bad: Vec<(PathBuf, AssetErrorCode)> = vec![
        (t.0.join("missing.mp4"), AssetErrorCode::AssetFileNotFound),
        (t.file("empty.mp4", b""), AssetErrorCode::AssetEmptyFile),
        (
            t.file("notmedia.bad", b"junk"),
            AssetErrorCode::Media(MediaErrorCode::MediaProbeFailed),
        ),
        (t.0.clone(), AssetErrorCode::AssetNotRegularFile),
    ];
    for (path, want) in bad {
        match import(&mut p, &path).unwrap_err() {
            ProjectError::Asset(e) => assert_eq!(e.code, want, "{path:?}"),
            other => panic!("unexpected {other}"),
        }
    }
    // probe com falha de backend / saída malformada: erro estruturado, nada registrado
    struct Broken(MediaErrorCode);
    impl MediaProbe for Broken {
        fn probe(&self, _: &Path) -> Result<MediaInfo, MediaError> {
            Err(MediaError::new(self.0, "boom"))
        }
    }
    for code in [
        MediaErrorCode::MediaBackendNotFound,
        MediaErrorCode::MediaMetadataInvalid,
        MediaErrorCode::MediaProbeTimeout,
    ] {
        let f = t.file("ok.mp4", b"fine bytes");
        let e = p.import_asset(&user(), &f, &Broken(code)).unwrap_err();
        assert_eq!(e.code(), code.as_str());
    }
    assert_eq!(p.document().revision, revision, "document untouched");
    assert!(p.assets().unwrap().is_empty(), "catalog untouched");
    drop(p);
    let q = reopen(&t);
    assert_eq!(q.document().revision, revision);
    assert!(q.assets().unwrap().is_empty());
}

#[test]
fn identity_survives_close_and_reopen() {
    let t = Tmp::new("reopen");
    let (id, hash, media, name);
    {
        let mut p = create(&t);
        let r = import(&mut p, &t.file("clips/hero.mp4", b"HERO FOOTAGE")).unwrap();
        id = r.asset_id.clone();
        hash = r.record.content_hash.clone();
        media = r.record.media.clone();
        name = r.record.display_name.clone();
    }
    let p = reopen(&t);
    let v = p.asset(&id).unwrap();
    let rec = v.catalog.unwrap();
    assert_eq!(rec.asset_id, id);
    assert_eq!(rec.content_hash, hash);
    assert_eq!(rec.media, media);
    assert_eq!(rec.display_name, name);
    assert_eq!(rec.status, Availability::Online);
    assert!(v.in_document);
}

#[test]
fn dedup_by_content_across_names_and_paths_and_after_reopen() {
    let t = Tmp::new("dedup");
    let mut p = create(&t);
    let first = import(&mut p, &t.file("a/one.mp4", b"SAME BYTES")).unwrap();
    assert_eq!(first.outcome, ImportOutcome::Created);
    let revision = p.document().revision;
    // mesmo arquivo, mesmo path ⇒ nada muda
    let again = import(&mut p, &t.file("a/one.mp4", b"SAME BYTES")).unwrap();
    assert_eq!(
        (again.outcome, again.asset_id.clone()),
        (ImportOutcome::Existing, first.asset_id.clone())
    );
    assert!(again.commit.is_none());
    // mesmo conteúdo, nome e pasta diferentes ⇒ mesmo asset, alias registrado
    let copy = import(&mut p, &t.file("b/other-name.mp4", b"SAME BYTES")).unwrap();
    assert_eq!(
        (copy.outcome, copy.asset_id.clone()),
        (ImportOutcome::Aliased, first.asset_id.clone())
    );
    assert_eq!(
        p.document().revision,
        revision,
        "no document churn for duplicates"
    );
    assert_eq!(p.assets().unwrap().len(), 1);
    // conteúdo diferente, nome igual ⇒ outro asset
    let other = import(&mut p, &t.file("c/one.mp4", b"DIFFERENT")).unwrap();
    assert_eq!(other.outcome, ImportOutcome::Created);
    assert_ne!(other.asset_id, first.asset_id);
    assert_eq!(p.assets().unwrap().len(), 2);
    drop(p);
    // depois de reabrir, a política é a mesma
    let mut p = reopen(&t);
    let r1 = import(&mut p, &t.file("a/one.mp4", b"SAME BYTES")).unwrap();
    assert_eq!(
        (r1.outcome, r1.asset_id.clone()),
        (ImportOutcome::Existing, first.asset_id.clone())
    );
    let r2 = import(&mut p, &t.file("b/other-name.mp4", b"SAME BYTES")).unwrap();
    assert_eq!(r2.outcome, ImportOutcome::Existing, "alias already known");
    assert_eq!(p.assets().unwrap().len(), 2);
    let rec = p.asset(&first.asset_id).unwrap().catalog.unwrap();
    assert_eq!(rec.known_paths.len(), 1);
}

#[test]
fn offline_media_does_not_prevent_opening_and_keeps_the_clips() {
    let t = Tmp::new("offline");
    let file = t.file("media/v.mp4", b"video bytes");
    let (id, hash);
    {
        let mut p = create(&t);
        setup_sequence(&mut p);
        let r = import(&mut p, &file).unwrap();
        id = r.asset_id.clone();
        hash = r.record.content_hash.clone();
        place(&mut p, &id, "c1", 0, 705_600_000).unwrap();
    }
    // o arquivo some (movido para fora)
    let elsewhere = t.0.join("moved");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(&file, elsewhere.join("v.mp4")).unwrap();
    let p = reopen(&t);
    let v = p.asset(&id).unwrap();
    let rec = v.catalog.as_ref().unwrap();
    assert_eq!(rec.status, Availability::Offline);
    assert_eq!(rec.content_hash, hash, "known hash survives");
    // comparação por CAMINHO (componentes), não por texto: o produto guarda `std::path::absolute`
    // (no Windows `a/b` vira `a\b`)
    assert_eq!(
        PathBuf::from(&rec.location.path),
        file,
        "known path survives"
    );
    assert_eq!(v.resolved_path, None);
    assert!(
        p.document()
            .sequence(&"S".into())
            .unwrap()
            .clip(&"c1".into())
            .is_some()
    );
}

#[test]
fn relink_by_content_and_structured_rejection_of_other_content() {
    let t = Tmp::new("relink");
    let file = t.file("media/v.mp4", b"the original content");
    let mut p = create(&t);
    let id = import(&mut p, &file).unwrap().asset_id;
    std::fs::remove_file(&file).unwrap();
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Offline
    );

    // arquivo diferente ⇒ rejeição estruturada; nada muda
    let wrong = t.file("elsewhere/wrong.mp4", b"a different file");
    let e = p.relink_asset(&id, &wrong).unwrap_err();
    assert_eq!(e.code(), "ASSET_HASH_MISMATCH");
    let j = e.to_json();
    assert_eq!(j["details"]["asset_id"], id.as_str());
    assert_ne!(j["details"]["expected_hash"], j["details"]["found_hash"]);
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Offline
    );

    // mesmo conteúdo em outro caminho ⇒ relink funciona e sobrevive ao reopen
    let good = t.file("elsewhere/renamed.mp4", b"the original content");
    let r = p.relink_asset(&id, &good).unwrap();
    assert_eq!(PathBuf::from(&r.to), good);
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Online
    );
    drop(p);
    let p = reopen(&t);
    let v = p.asset(&id).unwrap();
    assert_eq!(v.catalog.as_ref().unwrap().status, Availability::Online);
    assert_eq!(v.resolved_path.as_deref().map(PathBuf::from), Some(good));
    let kinds: Vec<String> = p
        .asset_events(&id)
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        ["import", "relink"],
        "audit trail (the rejected attempt left no trace)"
    );
}

#[test]
fn reimporting_identical_content_from_a_new_place_relinks_an_offline_asset() {
    let t = Tmp::new("autorelink");
    let file = t.file("a/v.mp4", b"payload");
    let mut p = create(&t);
    let id = import(&mut p, &file).unwrap().asset_id;
    std::fs::remove_file(&file).unwrap();
    let r = import(&mut p, &t.file("b/v2.mp4", b"payload")).unwrap();
    assert_eq!(
        (r.outcome, r.asset_id.clone()),
        (ImportOutcome::Relinked, id.clone())
    );
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Online
    );
}

#[test]
fn verify_detects_modified_content_not_just_missing_files() {
    let t = Tmp::new("verify");
    let file = t.file("v.mp4", b"original bytes");
    let mut p = create(&t);
    let id = import(&mut p, &file).unwrap().asset_id;
    assert_eq!(p.verify_asset(&id).unwrap().status, Availability::Online);
    // mesmo tamanho, outro conteúdo: só o hash enxerga
    std::fs::write(&file, b"ORIGINAL BYTES").unwrap();
    assert_eq!(
        p.asset(&id).unwrap().catalog.unwrap().status,
        Availability::Online,
        "cheap check cannot know"
    );
    let r = p.verify_asset(&id).unwrap();
    assert_eq!(r.status, Availability::Modified);
    assert_ne!(r.actual_hash.unwrap(), r.expected_hash);
    drop(p);
    let p = reopen(&t);
    // o estado verificado ficou registrado no catálogo
    let rec = p.asset(&id).unwrap().catalog.unwrap();
    assert_eq!(rec.status, Availability::Online); // reavaliado agora pelo status barato
    std::fs::remove_file(&file).unwrap();
    let mut p = p;
    assert_eq!(p.verify_asset(&id).unwrap().status, Availability::Offline);
    // asset lógico (sem arquivo) e id inexistente
    assert_eq!(
        p.verify_asset(&"ghost".into()).unwrap_err().code(),
        "ASSET_NOT_FOUND"
    );
}

#[test]
fn clips_reference_assets_and_in_use_assets_cannot_be_deleted() {
    let t = Tmp::new("inuse");
    let mut p = create(&t);
    setup_sequence(&mut p);
    let id = import(&mut p, &t.file("v.mp4", b"vid")).unwrap().asset_id;
    // referência a asset inexistente é rejeitada
    let e = place(
        &mut p,
        &"ast_does_not_exist".into(),
        "bad",
        0,
        23_520_000 * 10,
    )
    .unwrap_err();
    assert!(
        matches!(e.code, ErrorCode::DanglingReference | ErrorCode::NotFound),
        "{e}"
    );
    place(&mut p, &id, "c1", 0, 705_600_000).unwrap();
    // em uso ⇒ IN_USE com a lista de clips
    let del = |p: &mut Project| {
        p.execute(
            &user(),
            tx(
                "rm",
                vec![env("rm-asset", Command::DeleteAsset { asset: id.clone() })],
            ),
        )
    };
    let e = del(&mut p).unwrap_err();
    assert_eq!(e.code, ErrorCode::InUse);
    assert_eq!(e.entities.len(), 1);
    assert!(p.document().asset(&id).is_some());
    // sem uso ⇒ remove; o catálogo (biblioteca) permanece e o undo devolve o asset lógico
    p.execute(
        &user(),
        tx(
            "rm clip",
            vec![env(
                "rm-clip",
                Command::DeleteClip {
                    clip: "c1".into(),
                    ripple: None,
                    scope: Default::default(),
                },
            )],
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let rev = p.document().revision;
    // novo operation_id para a nova tentativa
    p.execute(
        &user(),
        tx(
            "rm",
            vec![env(
                "rm-asset-2",
                Command::DeleteAsset { asset: id.clone() },
            )],
        ),
    )
    .unwrap();
    assert!(p.document().asset(&id).is_none());
    assert!(p.document().revision > rev);
    let v = p.asset(&id).unwrap();
    assert!(!v.in_document && v.catalog.is_some(), "catalog row stays");
    p.undo(&user()).unwrap();
    assert!(p.document().asset(&id).is_some());
    p.redo(&user()).unwrap();
    // reimportar o mesmo conteúdo reaproveita o AssetId e registra de novo
    let again = import(&mut p, &t.0.join("v.mp4")).unwrap();
    assert_eq!(
        (again.outcome, again.asset_id),
        (ImportOutcome::Reregistered, id.clone())
    );
    assert!(p.document().asset(&id).is_some());
    drop(p);
    // e tudo isso persiste
    let q = reopen(&t);
    assert!(q.document().asset(&id).is_some());
}

#[test]
fn nested_clips_do_not_depend_on_assets() {
    let t = Tmp::new("nested");
    let mut p = create(&t);
    setup_sequence(&mut p);
    // uma sequence com um clip nested não precisa de nenhum asset
    p.execute(
        &user(),
        tx(
            "body",
            vec![env(
                "body-seq",
                Command::CreateSequence {
                    id: Some("BODY".into()),
                    name: "BODY".into(),
                    frame_rate: FrameRate::FPS_30,
                    sample_rate: None,
                    width: None,
                    height: None,
                    folder: None,
                },
            )],
        ),
    )
    .unwrap();
    assert!(p.assets().unwrap().is_empty());
    assert!(p.document().sequence(&"BODY".into()).is_some());
}

#[test]
fn the_same_import_in_two_projects_yields_the_same_asset_id() {
    let t1 = Tmp::new("twin1");
    let t2 = Tmp::new("twin2");
    let mut p1 = create(&t1);
    let mut p2 = create(&t2);
    let a = import(&mut p1, &t1.file("x.mp4", b"identical")).unwrap();
    let b = import(&mut p2, &t2.file("deep/y.mp4", b"identical")).unwrap();
    assert_eq!(a.asset_id, b.asset_id);
    assert_eq!(a.record.content_hash, b.record.content_hash);
    assert_eq!(a.record.media, b.record.media);
}

/// Import atômico: se o commit do documento falha (escritor obsoleto), a linha do catálogo
/// que viajava na mesma transação também NÃO existe.
#[test]
fn a_stale_writers_import_leaves_no_trace_in_the_catalog() {
    let t = Tmp::new("stale");
    let mut first = create(&t);
    let mut second = reopen(&t); // mesma base; `first` vai avançar na frente
    import(&mut first, &t.file("one.mp4", b"first writer")).unwrap();
    let before = first.document().revision;
    let err = import(&mut second, &t.file("two.mp4", b"second writer")).unwrap_err();
    assert!(
        matches!(&err, ProjectError::Command(e) if e.code == ErrorCode::PersistenceFailed),
        "{err}"
    );
    drop(first);
    drop(second);
    let p = reopen(&t);
    assert_eq!(p.document().revision, before);
    let views = p.assets().unwrap();
    assert_eq!(
        views.len(),
        1,
        "the stale import left neither a document asset nor a catalog row"
    );
    assert_eq!(views[0].catalog.as_ref().unwrap().display_name, "one.mp4");
}
