//! Comandos do editor manual (Fase 3): pastas, formato, deliverables, grupos, transições, texto,
//! detach de áudio — semântica, erros estruturados, undo/redo e invariantes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{
    Actor, Command, CommandEnvelope, CommandError, CommitResult, Engine, NewClip, Transaction,
};
use capia_model::{
    Asset, ClipContent, ErrorCode, TextAlign, TextStyle, TrackKind, Transition, TransitionKind,
    validate_document,
};
use capia_time::{FrameRate, Rational, Ticks};

const FRAME: i64 = 23_520_000; // 30 fps
fn frames(n: i64) -> Ticks {
    Ticks(n * FRAME)
}

type R = Result<CommitResult, CommandError>;

struct W {
    e: Engine,
    n: u32,
}

impl W {
    fn new() -> Self {
        let mut w = Self {
            e: Engine::new(Default::default(), [7; 32]),
            n: 0,
        };
        w.run(vec![
            Command::CreateSequence {
                id: Some("s".into()),
                name: "main".into(),
                frame_rate: FrameRate::FPS_30,
                sample_rate: None,
                width: None,
                height: None,
                folder: None,
            },
            Command::AddTrack {
                sequence: "s".into(),
                id: Some("v".into()),
                kind: TrackKind::Visual,
                name: None,
                role: None,
                magnetic: false,
                index: None,
            },
            Command::AddTrack {
                sequence: "s".into(),
                id: Some("a".into()),
                kind: TrackKind::Audio,
                name: None,
                role: None,
                magnetic: false,
                index: None,
            },
            Command::RegisterAsset {
                asset: Asset {
                    id: "vid".into(),
                    name: "vid".into(),
                    duration: Some(frames(300)),
                    has_video: true,
                    has_audio: true,
                    offline: false,
                },
            },
        ])
        .unwrap();
        w
    }

    fn run(&mut self, cmds: Vec<Command>) -> R {
        let commands = cmds
            .into_iter()
            .map(|c| {
                self.n += 1;
                CommandEnvelope {
                    operation_id: format!("op-{}", self.n),
                    reference: None,
                    command: c,
                }
            })
            .collect();
        let r = self.e.execute(
            &Actor::user("t"),
            Transaction {
                transaction_id: None,
                label: "t".into(),
                base_revision: None,
                commands,
                max_ops: None,
            },
            0,
        );
        if r.is_ok() {
            let v = validate_document(self.e.document());
            assert!(v.is_empty(), "invariants broken: {v:?}");
        }
        r
    }

    fn cmd(&mut self, c: Command) -> R {
        self.run(vec![c])
    }

    fn insert(
        &mut self,
        track: &str,
        start: i64,
        dur: i64,
        id: &str,
        content: ClipContent,
        source_in: i64,
    ) -> R {
        self.cmd(Command::InsertClip {
            track: track.into(),
            start: frames(start),
            clip: NewClip {
                id: Some(id.into()),
                name: id.into(),
                duration: frames(dur),
                content,
                source_in: frames(source_in),
                speed: Rational::ONE,
                reversed: false,
                properties: Default::default(),
            },
            split_at_insert: false,
            split_new_id: None,
        })
    }

    fn media(&mut self, start: i64, dur: i64, id: &str, source_in: i64) -> R {
        self.insert(
            "v",
            start,
            dur,
            id,
            ClipContent::Media {
                asset: "vid".into(),
                has_video: true,
                has_audio: true,
            },
            source_in,
        )
    }

    fn solid(&mut self, start: i64, dur: i64, id: &str) -> R {
        self.insert(
            "v",
            start,
            dur,
            id,
            ClipContent::Solid {
                color: "#123456".into(),
            },
            0,
        )
    }
}

fn code(r: R) -> ErrorCode {
    r.expect_err("expected an error").code
}

#[test]
fn folders_nest_move_and_refuse_cycles_or_non_empty_delete() {
    let mut w = W::new();
    w.run(vec![
        Command::CreateFolder {
            id: Some("f1".into()),
            name: "AD 1".into(),
            parent: None,
        },
        Command::CreateFolder {
            id: Some("f2".into()),
            name: "Hooks".into(),
            parent: Some("f1".into()),
        },
        Command::SetSequenceFolder {
            sequence: "s".into(),
            folder: Some("f2".into()),
        },
    ])
    .unwrap();
    assert_eq!(
        w.e.document().sequence(&"s".into()).unwrap().header.folder,
        Some("f2".into())
    );
    // ciclo: f1 dentro de f2 (descendente)
    assert_eq!(
        code(w.cmd(Command::MoveFolder {
            folder: "f1".into(),
            parent: Some("f2".into())
        })),
        ErrorCode::InvariantViolation
    );
    // pasta com conteúdo
    assert_eq!(
        code(w.cmd(Command::DeleteFolder {
            folder: "f2".into()
        })),
        ErrorCode::InUse
    );
    assert_eq!(
        code(w.cmd(Command::DeleteFolder {
            folder: "f1".into()
        })),
        ErrorCode::InUse
    );
    w.cmd(Command::SetSequenceFolder {
        sequence: "s".into(),
        folder: None,
    })
    .unwrap();
    w.cmd(Command::DeleteFolder {
        folder: "f2".into(),
    })
    .unwrap();
    w.cmd(Command::RenameFolder {
        folder: "f1".into(),
        name: "Masters".into(),
    })
    .unwrap();
    // undo restaura a pasta apagada? (last undo desfaz o rename)
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(w.e.document().folder(&"f1".into()).unwrap().name, "AD 1");
}

#[test]
fn sequence_format_and_creation_with_preset() {
    let mut w = W::new();
    w.cmd(Command::SetSequenceFormat {
        sequence: "s".into(),
        width: 1080,
        height: 1920,
    })
    .unwrap();
    let h = &w.e.document().sequence(&"s".into()).unwrap().header;
    assert_eq!((h.width, h.height), (1080, 1920));
    assert_eq!(
        code(w.cmd(Command::SetSequenceFormat {
            sequence: "s".into(),
            width: 4,
            height: 100
        })),
        ErrorCode::OutOfRange
    );
    w.cmd(Command::CreateSequence {
        id: Some("sq".into()),
        name: "square".into(),
        frame_rate: FrameRate::FPS_30,
        sample_rate: None,
        width: Some(1080),
        height: Some(1080),
        folder: None,
    })
    .unwrap();
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert!(w.e.document().sequence(&"sq".into()).is_none());
}

#[test]
fn deliverables_are_validated_and_removed_with_their_sequence() {
    let mut w = W::new();
    let create = |path: &str| Command::CreateDeliverable {
        id: Some("d1".into()),
        name: "Meta 9x16".into(),
        sequence: "s".into(),
        preset: "h264-mp4".into(),
        path: path.into(),
        width: Some(1080),
        height: Some(1920),
    };
    assert_eq!(code(w.cmd(create(" "))), ErrorCode::InvalidArgument);
    w.cmd(create("out/ad.mp4")).unwrap();
    assert!(w.e.document().deliverable(&"d1".into()).is_some());
    w.cmd(Command::DeleteSequence {
        sequence: "s".into(),
    })
    .unwrap();
    assert!(w.e.document().deliverable(&"d1".into()).is_none());
    // o undo da transação traz sequence e deliverable de volta juntos
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert!(w.e.document().deliverable(&"d1".into()).is_some());
    assert!(validate_document(w.e.document()).is_empty());
}

#[test]
fn group_and_ungroup_label_every_member() {
    let mut w = W::new();
    w.solid(0, 30, "c1").unwrap();
    w.solid(30, 30, "c2").unwrap();
    w.solid(60, 30, "c3").unwrap();
    assert_eq!(
        code(w.cmd(Command::GroupClips {
            clips: vec!["c1".into()],
            group: None
        })),
        ErrorCode::InvalidArgument
    );
    let r = w
        .cmd(Command::GroupClips {
            clips: vec!["c1".into(), "c2".into()],
            group: Some("g".into()),
        })
        .unwrap();
    assert!(!r.results.is_empty());
    let seq = w.e.document().sequence(&"s".into()).unwrap();
    assert_eq!(seq.clip(&"c1".into()).unwrap().group.as_deref(), Some("g"));
    assert_eq!(seq.clip(&"c2".into()).unwrap().group.as_deref(), Some("g"));
    assert_eq!(seq.clip(&"c3".into()).unwrap().group, None);
    // desagrupar por um membro limpa todos
    w.cmd(Command::Ungroup {
        clips: vec!["c2".into()],
    })
    .unwrap();
    let seq = w.e.document().sequence(&"s".into()).unwrap();
    assert!(seq.clips().all(|c| c.group.is_none()));
}

#[test]
fn text_style_and_content_are_validated() {
    let mut w = W::new();
    w.insert(
        "v",
        0,
        60,
        "t1",
        ClipContent::Text {
            text: "hi".into(),
            style: TextStyle::default(),
        },
        0,
    )
    .unwrap();
    let style = TextStyle {
        size_permille: 90,
        weight: 700,
        align: TextAlign::Left,
        color: "#FFCC00".into(),
        background: Some("#000000AA".into()),
        ..TextStyle::default()
    };
    w.cmd(Command::SetText {
        clip: "t1".into(),
        text: Some("Olá".into()),
        style: Some(style.clone()),
    })
    .unwrap();
    match &w
        .e
        .document()
        .sequence(&"s".into())
        .unwrap()
        .clip(&"t1".into())
        .unwrap()
        .content
    {
        ClipContent::Text { text, style: s } => {
            assert_eq!(text, "Olá");
            assert_eq!(s, &style);
        }
        other => panic!("{other:?}"),
    }
    let bad = TextStyle {
        color: "yellow".into(),
        ..TextStyle::default()
    };
    assert_eq!(
        code(w.cmd(Command::SetText {
            clip: "t1".into(),
            text: None,
            style: Some(bad)
        })),
        ErrorCode::OutOfRange
    );
    // não é um clip de texto
    w.solid(60, 30, "c").unwrap();
    assert_eq!(
        code(w.cmd(Command::SetText {
            clip: "c".into(),
            text: Some("x".into()),
            style: None
        })),
        ErrorCode::WrongTrackKind
    );
}

#[test]
fn transitions_need_adjacency_and_handles() {
    let mut w = W::new();
    w.media(0, 60, "a", 0).unwrap();
    w.media(60, 60, "b", 0).unwrap(); // sem handle antes do início
    let tr = |kind, d| Command::SetTransition {
        clip: "b".into(),
        transition: Some(Transition {
            kind,
            duration: frames(d),
        }),
    };
    // fade não precisa de handles
    w.cmd(tr(TransitionKind::Fade, 20)).unwrap();
    // dissolve: b não tem handle antes do início (source_in = 0)
    assert_eq!(
        code(w.cmd(tr(TransitionKind::Dissolve, 20))),
        ErrorCode::InsufficientHandles
    );
    // refaz b com handle (source_in = 30) e a termina com sobra de fonte
    w.cmd(Command::DeleteClip {
        clip: "b".into(),
        ripple: Some(false),
        scope: Default::default(),
    })
    .unwrap();
    w.media(60, 60, "b", 30).unwrap();
    w.cmd(tr(TransitionKind::Dissolve, 20)).unwrap();
    // maior que o permitido pelos vizinhos
    assert_eq!(
        code(w.cmd(tr(TransitionKind::Dissolve, 200))),
        ErrorCode::OutOfRange
    );
    // sem clip anterior adjacente
    w.solid(200, 30, "lonely").unwrap();
    assert_eq!(
        code(w.cmd(Command::SetTransition {
            clip: "lonely".into(),
            transition: Some(Transition {
                kind: TransitionKind::Fade,
                duration: frames(10)
            }),
        })),
        ErrorCode::NotOnBoundary
    );
    // slide_in dispensa vizinho, mas limita-se ao clip
    w.cmd(Command::SetTransition {
        clip: "lonely".into(),
        transition: Some(Transition {
            kind: TransitionKind::SlideIn,
            duration: frames(10),
        }),
    })
    .unwrap();
    // remover
    w.cmd(Command::SetTransition {
        clip: "lonely".into(),
        transition: None,
    })
    .unwrap();
    assert!(
        w.e.document()
            .sequence(&"s".into())
            .unwrap()
            .clip(&"lonely".into())
            .unwrap()
            .transition_in
            .is_none()
    );
}

#[test]
fn detach_audio_splits_and_undo_restores_exactly() {
    let mut w = W::new();
    w.media(0, 90, "m", 15).unwrap();
    let before = w.e.document().clone();
    let r = w
        .cmd(Command::DetachAudio {
            clip: "m".into(),
            audio_track: None,
            audio_clip_id: Some("m.a".into()),
        })
        .unwrap();
    assert!(!r.affected.is_empty());
    let seq = w.e.document().sequence(&"s".into()).unwrap();
    let video = seq.clip(&"m".into()).unwrap();
    let audio = seq.clip(&"m.a".into()).unwrap();
    assert!(matches!(
        video.content,
        ClipContent::Media {
            has_video: true,
            has_audio: false,
            ..
        }
    ));
    assert!(matches!(
        audio.content,
        ClipContent::Media {
            has_video: false,
            has_audio: true,
            ..
        }
    ));
    assert_eq!(
        (audio.start, audio.duration, audio.source_in),
        (video.start, video.duration, video.source_in)
    );
    assert_eq!(audio.track.as_str(), "a");
    // não há mais áudio para separar
    assert_eq!(
        code(w.cmd(Command::DetachAudio {
            clip: "m".into(),
            audio_track: None,
            audio_clip_id: None
        })),
        ErrorCode::InvalidArgument
    );
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(
        w.e.document().sequence(&"s".into()),
        before.sequence(&"s".into())
    );
}

#[test]
fn detach_audio_without_a_free_audio_track_asks_for_one() {
    let mut w = W::new();
    w.media(0, 90, "m", 0).unwrap();
    w.cmd(Command::SetTrackFlags {
        track: "a".into(),
        locked: Some(true),
        hidden: None,
        muted: None,
        solo: None,
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    })
    .unwrap();
    assert_eq!(
        code(w.cmd(Command::DetachAudio {
            clip: "m".into(),
            audio_track: None,
            audio_clip_id: None
        })),
        ErrorCode::NotFound
    );
}

#[test]
fn rename_and_enable_clip() {
    let mut w = W::new();
    w.solid(0, 30, "c").unwrap();
    w.cmd(Command::RenameClip {
        clip: "c".into(),
        name: "Hook".into(),
    })
    .unwrap();
    w.cmd(Command::SetClipEnabled {
        clip: "c".into(),
        enabled: false,
    })
    .unwrap();
    let c =
        w.e.document()
            .sequence(&"s".into())
            .unwrap()
            .clip(&"c".into())
            .unwrap()
            .clone();
    assert_eq!((c.name.as_str(), c.enabled), ("Hook", false));
}

fn starts(w: &W, track: &str) -> Vec<(String, i64)> {
    w.e.document()
        .sequence(&"s".into())
        .unwrap()
        .track_clips(&track.into())
        .map(|c| (c.id.to_string(), c.start.0 / FRAME))
        .collect()
}

fn with_main() -> W {
    let mut w = W::new();
    w.cmd(Command::AddTrack {
        sequence: "s".into(),
        id: Some("m".into()),
        kind: TrackKind::Visual,
        name: None,
        role: None,
        magnetic: true,
        index: None,
    })
    .unwrap();
    for (i, id) in ["a", "b", "c"].iter().enumerate() {
        w.insert(
            "m",
            i as i64 * 30,
            30,
            id,
            ClipContent::Solid {
                color: "#112233".into(),
            },
            0,
        )
        .unwrap();
    }
    w
}

#[test]
fn reorder_within_a_magnetic_track_keeps_it_gapless() {
    let mut w = with_main();
    w.cmd(Command::ReorderClip {
        clip: "c".into(),
        track: None,
        before: Some("a".into()),
        start: None,
    })
    .unwrap();
    assert_eq!(
        starts(&w, "m"),
        [("c".into(), 0), ("a".into(), 30), ("b".into(), 60)]
    );
    w.cmd(Command::ReorderClip {
        clip: "c".into(),
        track: None,
        before: None,
        start: None,
    })
    .unwrap();
    assert_eq!(
        starts(&w, "m"),
        [("a".into(), 0), ("b".into(), 30), ("c".into(), 60)]
    );
    // undo devolve exatamente a ordem anterior
    w.e.undo(&Actor::user("t"), 0).unwrap();
    assert_eq!(
        starts(&w, "m"),
        [("c".into(), 0), ("a".into(), 30), ("b".into(), 60)]
    );
}

#[test]
fn reorder_into_and_out_of_a_magnetic_track() {
    let mut w = with_main();
    w.solid(200, 15, "o").unwrap(); // overlay livre em 200..215
    // entra na track magnética antes de `b`: abre espaço e reempurra b, c
    w.cmd(Command::ReorderClip {
        clip: "o".into(),
        track: Some("m".into()),
        before: Some("b".into()),
        start: None,
    })
    .unwrap();
    assert_eq!(
        starts(&w, "m"),
        [
            ("a".into(), 0),
            ("o".into(), 30),
            ("b".into(), 45),
            ("c".into(), 75)
        ]
    );
    assert!(starts(&w, "v").is_empty());
    // sai da magnética para a track livre em 100: fecha o gap
    w.cmd(Command::ReorderClip {
        clip: "o".into(),
        track: Some("v".into()),
        before: None,
        start: Some(frames(100)),
    })
    .unwrap();
    assert_eq!(
        starts(&w, "m"),
        [("a".into(), 0), ("b".into(), 30), ("c".into(), 60)]
    );
    assert_eq!(starts(&w, "v"), [("o".into(), 100)]);
}

#[test]
fn reorder_errors_are_structured() {
    let mut w = with_main();
    w.solid(200, 15, "o").unwrap();
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "a".into(),
            track: None,
            before: Some("a".into()),
            start: None
        })),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "o".into(),
            track: Some("m".into()),
            before: Some("ghost".into()),
            start: None
        })),
        ErrorCode::InvalidArgument
    );
    // livre → livre não é reorder
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "o".into(),
            track: None,
            before: None,
            start: Some(frames(10))
        })),
        ErrorCode::InvalidArgument
    );
    // destino livre sem start
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "a".into(),
            track: Some("v".into()),
            before: None,
            start: None
        })),
        ErrorCode::InvalidArgument
    );
    // destino ocupado
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "a".into(),
            track: Some("v".into()),
            before: None,
            start: Some(frames(205))
        })),
        ErrorCode::Overlap
    );
    // nenhuma das falhas alterou o documento
    assert_eq!(
        starts(&w, "m"),
        [("a".into(), 0), ("b".into(), 30), ("c".into(), 60)]
    );
    // track travada
    w.cmd(Command::SetTrackFlags {
        track: "m".into(),
        locked: Some(true),
        hidden: None,
        muted: None,
        solo: None,
        magnetic: None,
        sync_lock: None,
        group: None,
        clear_group: false,
        compact: false,
    })
    .unwrap();
    assert_eq!(
        code(w.cmd(Command::ReorderClip {
            clip: "a".into(),
            track: None,
            before: None,
            start: None
        })),
        ErrorCode::TrackLocked
    );
}

#[test]
fn point_snap_prefers_the_playhead_then_markers_then_edges_and_respects_exclusion() {
    use capia_commands::{SnapTargetKind, resolve_point_snap};
    let mut w = W::new();
    w.solid(10, 10, "a").unwrap(); // bordas em 10F e 20F
    w.cmd(Command::AddMarker {
        sequence: "s".into(),
        id: None,
        time: frames(30),
        label: String::new(),
    })
    .unwrap();
    let seq = w.e.document().sequence(&"s".into()).unwrap();
    let th = Ticks(FRAME * 2);
    // borda de clip
    let hit = resolve_point_snap(seq, &[], frames(11), th, &[], None).unwrap();
    assert_eq!((hit.kind, hit.t), (SnapTargetKind::ClipStart, frames(10)));
    // o clip excluído deixa de ser alvo (sobra o início da sequence, fora do limiar)
    assert!(resolve_point_snap(seq, &["a".into()], frames(11), th, &[], None).is_none());
    // empate de distância: playhead vence marcador e borda
    let hit = resolve_point_snap(
        seq,
        &[],
        frames(29),
        Ticks(FRAME * 5),
        &[],
        Some(frames(28)),
    )
    .unwrap();
    assert_eq!(hit.kind, SnapTargetKind::Playhead);
    // marcador vence borda quando equidistantes
    let hit = resolve_point_snap(seq, &[], frames(25), Ticks(FRAME * 6), &[], None).unwrap();
    // fim do clip (20F) e marcador (30F) a 5F: o marcador tem prioridade maior
    assert_eq!((hit.kind, hit.t), (SnapTargetKind::Marker, frames(30)));
}
