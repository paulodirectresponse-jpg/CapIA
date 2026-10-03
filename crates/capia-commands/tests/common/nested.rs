//! Gerador determinístico de comandos que exercitam nested sequences, `follow_length`, delete de
//! sequence e retarget — usado pelos testes de propriedade (em memória e com reabertura do `.capia`).

#![allow(dead_code, unreachable_pub, clippy::unwrap_used, clippy::expect_used)]

use super::{FRAME, Rng};
use capia_commands::{
    Actor, ClipMove, Command, CommandEnvelope, Edge, Engine, NewClip, Transaction,
};
use capia_model::{ClipContent, ClipId, SequenceId, TrackKind};
use capia_time::{FrameRate, Rational, Ticks};

pub const SEQS: usize = 5;

pub fn sid(i: usize) -> SequenceId {
    SequenceId::from(format!("s{i}").as_str())
}

/// Uma transação que cria `SEQS` sequences (fps alternados) com uma track livre `t{i}` e uma
/// magnética `m{i}` cada.
pub fn base_tx() -> Transaction {
    let mut commands = Vec::new();
    let mut n = 0;
    let mut push = |c: Command| {
        n += 1;
        commands.push(CommandEnvelope {
            operation_id: format!("nbase-{n}"),
            reference: None,
            command: c,
        });
    };
    for i in 0..SEQS {
        push(Command::CreateSequence {
            id: Some(sid(i)),
            name: format!("seq {i}"),
            frame_rate: if i % 2 == 0 {
                FrameRate::FPS_30
            } else {
                FrameRate::FPS_24
            },
            sample_rate: None,
        });
        for (prefix, magnetic) in [("t", false), ("m", true)] {
            push(Command::AddTrack {
                sequence: sid(i),
                id: Some(format!("{prefix}{i}").as_str().into()),
                kind: TrackKind::Visual,
                name: None,
                role: None,
                magnetic,
                index: None,
            });
        }
    }
    Transaction {
        transaction_id: None,
        label: "nested base".into(),
        base_revision: None,
        commands,
        max_ops: None,
    }
}

pub fn nested_base_engine() -> Engine {
    let mut e = Engine::new(Default::default(), [6; 32]);
    e.execute(&Actor::system(), base_tx(), 0).unwrap();
    e
}

fn all_clips(e: &Engine) -> Vec<(SequenceId, ClipId, bool)> {
    let mut out = Vec::new();
    for (sid, s) in e.document().sequences() {
        for c in s.clips() {
            out.push((
                sid.clone(),
                c.id.clone(),
                matches!(c.content, ClipContent::Nested { .. }),
            ));
        }
    }
    out
}

fn existing_seq(rng: &mut Rng, e: &Engine) -> SequenceId {
    let ids: Vec<SequenceId> = e.document().sequences().map(|(id, _)| id.clone()).collect();
    rng.pick(&ids).cloned().unwrap_or_else(|| sid(0))
}

fn near(rng: &mut Rng, e: &Engine, clip: &ClipId) -> Ticks {
    let c = e.document().find_clip(clip).map(|(_, _, c)| c.clone());
    match c {
        Some(c) if rng.chance(85) => {
            Ticks(c.start.0 + rng.below(c.duration.0 as u64 / FRAME as u64 + 1) as i64 * FRAME)
        }
        _ => Ticks(rng.below(80) as i64 * FRAME),
    }
}

/// Comandos de composição (ADR-050) sobre o estado atual: duplicar, tornar único, achatar,
/// agrupar a partir de seleção e gerar variantes. Muitos falham de propósito (ids ruins, tracks
/// magnéticas…): a propriedade é que falhas são atômicas e os acertos preservam as invariantes.
pub fn random_compose_command(rng: &mut Rng, e: &Engine, n: u32) -> Command {
    use capia_commands::{VariantSpec, VariantSwap};
    let nested: Vec<(SequenceId, ClipId)> = all_clips(e)
        .into_iter()
        .filter(|c| c.2)
        .map(|c| (c.0, c.1))
        .collect();
    let ghost: ClipId = "ghost".into();
    match rng.below(5) {
        0 => Command::DuplicateSequence {
            source: existing_seq(rng, e),
            new_sequence: Some(format!("d{n}").as_str().into()),
            name: None,
            deep: rng.chance(40),
        },
        1 => Command::MakeUnique {
            clip: rng.pick(&nested).map_or(ghost, |c| c.1.clone()),
            new_sequence: Some(format!("u{n}").as_str().into()),
            name: None,
            deep: rng.chance(40),
        },
        2 => Command::FlattenNested {
            clip: rng.pick(&nested).map_or(ghost, |c| c.1.clone()),
            prefix: if rng.chance(30) {
                Some(format!("f{n}"))
            } else {
                None
            },
        },
        3 => {
            let seq = existing_seq(rng, e);
            let mut ids: Vec<ClipId> = e
                .document()
                .sequence(&seq)
                .map(|s| s.clips().map(|c| c.id.clone()).collect())
                .unwrap_or_default();
            let take = 1 + usize::try_from(rng.below(3)).unwrap_or(0);
            let mut chosen = Vec::new();
            while chosen.len() < take && !ids.is_empty() {
                let i = usize::try_from(rng.below(ids.len() as u64)).unwrap_or(0);
                chosen.push(ids.remove(i));
            }
            if chosen.is_empty() {
                chosen.push(ghost);
            }
            Command::CreateNestedFromSelection {
                clips: chosen,
                new_sequence: Some(format!("g{n}").as_str().into()),
                name: None,
                clip_id: Some(format!("gn{n}").as_str().into()),
                track: None,
                follow_length: rng.chance(30),
            }
        }
        _ => {
            let template = existing_seq(rng, e);
            let tpl_nested: Vec<ClipId> = e
                .document()
                .sequence(&template)
                .map(|s| s.nested_refs().map(|(c, _)| c.clone()).collect())
                .unwrap_or_default();
            let swaps = match rng.pick(&tpl_nested) {
                Some(c) if rng.chance(70) => vec![VariantSwap::SetNested {
                    clip: c.clone(),
                    sequence: existing_seq(rng, e),
                }],
                _ => Vec::new(),
            };
            Command::GenerateVariants {
                template,
                variants: vec![VariantSpec {
                    sequence: Some(format!("v{n}").as_str().into()),
                    name: None,
                    deep: rng.chance(30),
                    swaps,
                }],
            }
        }
    }
}

pub fn random_nested_command(rng: &mut Rng, e: &Engine, counter: &mut u32) -> Command {
    *counter += 1;
    let n = *counter;
    if rng.chance(22) {
        return random_compose_command(rng, e, n);
    }
    let clips = all_clips(e);
    let nested: Vec<_> = clips.iter().filter(|c| c.2).cloned().collect();
    let pick_clip = |rng: &mut Rng| {
        clips
            .get(rng.below(clips.len() as u64) as usize)
            .map(|c| c.1.clone())
    };
    let any_seq_index = |rng: &mut Rng| usize::try_from(rng.below(SEQS as u64)).unwrap();
    let track_in =
        |rng: &mut Rng, i: usize| format!("{}{i}", if rng.chance(30) { "m" } else { "t" });
    match rng.below(100) {
        0..=24 => {
            let i = any_seq_index(rng);
            Command::InsertClip {
                track: track_in(rng, i).as_str().into(),
                start: Ticks(rng.below(60) as i64 * FRAME),
                clip: NewClip {
                    id: Some(format!("c{n}").as_str().into()),
                    name: String::new(),
                    duration: Ticks((1 + rng.below(40) as i64) * FRAME),
                    content: ClipContent::Solid {
                        color: "#000".into(),
                    },
                    source_in: Ticks::ZERO,
                    speed: Rational::ONE,
                    reversed: false,
                    properties: Default::default(),
                },
                split_at_insert: rng.chance(50),
                split_new_id: Some(format!("sp{n}").as_str().into()),
            }
        }
        25..=49 => {
            let parent = any_seq_index(rng);
            Command::InsertNested {
                track: track_in(rng, parent).as_str().into(),
                start: Ticks(rng.below(60) as i64 * FRAME),
                sequence: sid(any_seq_index(rng)),
                id: Some(format!("n{n}").as_str().into()),
                name: String::new(),
                duration: if rng.chance(30) {
                    Some(Ticks((1 + rng.below(30) as i64) * FRAME))
                } else {
                    None
                },
                source_in: if rng.chance(20) {
                    Ticks(rng.below(10) as i64 * FRAME)
                } else {
                    Ticks::ZERO
                },
                follow_length: rng.chance(60),
                split_at_insert: rng.chance(50),
                split_new_id: Some(format!("nsp{n}").as_str().into()),
            }
        }
        50..=57 => match rng.pick(&nested) {
            Some((_, clip, _)) => Command::SetNestedTarget {
                clip: clip.clone(),
                sequence: sid(any_seq_index(rng)),
            },
            None => Command::RenameSequence {
                sequence: existing_seq(rng, e),
                name: format!("r{n}"),
            },
        },
        58..=65 => match rng.pick(&nested) {
            Some((_, clip, _)) => Command::SetFollowLength {
                clip: clip.clone(),
                follow_length: rng.chance(50),
            },
            None => Command::RenameSequence {
                sequence: existing_seq(rng, e),
                name: format!("r{n}"),
            },
        },
        66..=70 => Command::DeleteSequence {
            sequence: sid(any_seq_index(rng)),
        },
        71..=74 => Command::RenameSequence {
            sequence: sid(any_seq_index(rng)),
            name: format!("renamed {n}"),
        },
        75..=82 => Command::DeleteClip {
            clip: pick_clip(rng).unwrap_or_else(|| "ghost".into()),
            ripple: None,
            scope: capia_commands::RippleScope::Track,
        },
        83..=90 => {
            let clip = pick_clip(rng).unwrap_or_else(|| "ghost".into());
            let to = near(rng, e, &clip);
            Command::TrimClip {
                clip,
                edge: if rng.chance(50) { Edge::Out } else { Edge::In },
                to,
                ripple: None,
                scope: capia_commands::RippleScope::Track,
            }
        }
        91..=95 => {
            let clip = pick_clip(rng).unwrap_or_else(|| "ghost".into());
            let at = near(rng, e, &clip);
            Command::SplitClip {
                clip,
                at,
                new_id: Some(format!("sc{n}").as_str().into()),
            }
        }
        _ => {
            let clip = pick_clip(rng).unwrap_or_else(|| "ghost".into());
            let start = near(rng, e, &clip);
            Command::MoveClips {
                moves: vec![ClipMove {
                    clip,
                    track: None,
                    start,
                }],
            }
        }
    }
}
