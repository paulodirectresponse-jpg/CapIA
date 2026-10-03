//! Grafo de nested sequences: caminho, profundidade, ciclo e o índice derivado.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_model::{
    Clip, ClipContent, Document, ErrorCode, MAX_NESTING_DEPTH, PrimitiveOp, SequenceHeader,
    SequenceId, Track, TrackKind, TrackSlot, check_nested_edge, nested_depth_above,
    nested_depth_below, nested_path, validate_nested_graph,
};
use capia_time::{FrameRate, Rational, Ticks};

fn sid(n: usize) -> SequenceId {
    SequenceId::from(format!("s{n}").as_str())
}

/// `n` sequences s0..s{n-1}, cada uma com uma track V.
fn doc_with(n: usize) -> Document {
    let mut d = Document::new();
    for i in 0..n {
        d.apply_op(&PrimitiveOp::Sequence {
            id: sid(i),
            old: None,
            new: Some(SequenceHeader {
                name: format!("s{i}"),
                frame_rate: FrameRate::FPS_30,
                sample_rate: 48_000,
            }),
        })
        .unwrap();
        d.apply_op(&PrimitiveOp::Track {
            sequence: sid(i),
            id: format!("t{i}").as_str().into(),
            old: None,
            new: Some(TrackSlot {
                index: 0,
                track: Track::new(format!("t{i}").as_str(), TrackKind::Visual),
            }),
        })
        .unwrap();
    }
    d
}

/// Insere em `parent` um nested apontando para `target`.
fn nest(d: &mut Document, parent: usize, target: usize, follow: bool) -> String {
    let id = format!("n{parent}_{target}");
    d.apply_op(&PrimitiveOp::Clip {
        sequence: sid(parent),
        id: id.as_str().into(),
        old: None,
        new: Some(Clip {
            id: id.as_str().into(),
            track: format!("t{parent}").as_str().into(),
            start: Ticks(0),
            duration: Ticks(23_520_000),
            name: String::new(),
            enabled: true,
            content: ClipContent::Nested {
                sequence: sid(target),
                follow_length: follow,
            },
            source_in: Ticks::ZERO,
            speed: Rational::ONE,
            reversed: false,
            properties: Default::default(),
        }),
    })
    .unwrap();
    id
}

#[test]
fn paths_and_depths_follow_the_nesting_chain() {
    let mut d = doc_with(4);
    // s0 -> s1 -> s2 ; s3 isolada
    nest(&mut d, 0, 1, false);
    nest(&mut d, 1, 2, false);
    assert_eq!(
        nested_path(&d, &sid(0), &sid(2)).unwrap(),
        [sid(0), sid(1), sid(2)]
    );
    assert_eq!(nested_path(&d, &sid(0), &sid(0)).unwrap(), [sid(0)]);
    assert!(nested_path(&d, &sid(2), &sid(0)).is_none());
    assert!(nested_path(&d, &sid(0), &sid(3)).is_none());
    assert_eq!(nested_depth_below(&d, &sid(0)), 2);
    assert_eq!(nested_depth_below(&d, &sid(2)), 0);
    assert_eq!(nested_depth_above(&d, &sid(2)), 2);
    assert_eq!(nested_depth_above(&d, &sid(0)), 0);
    assert_eq!(nested_depth_above(&d, &sid(3)), 0);
    assert!(validate_nested_graph(&d).is_empty());
}

#[test]
fn candidate_edges_are_checked_before_they_exist() {
    let mut d = doc_with(3);
    nest(&mut d, 0, 1, false);
    nest(&mut d, 1, 2, false);
    // A -> A
    assert_eq!(
        check_nested_edge(&d, &sid(0), &sid(0)).unwrap().code,
        ErrorCode::NestedCycle
    );
    // A -> B -> A
    let v = check_nested_edge(&d, &sid(1), &sid(0)).unwrap();
    assert_eq!(v.code, ErrorCode::NestedCycle);
    assert!(v.message.contains("s1 -> s0 -> s1"), "{}", v.message);
    // C -> A fecha a cadeia A -> B -> C -> A
    assert_eq!(
        check_nested_edge(&d, &sid(2), &sid(0)).unwrap().code,
        ErrorCode::NestedCycle
    );
    // arestas válidas (diamante)
    assert!(check_nested_edge(&d, &sid(0), &sid(2)).is_none());
}

#[test]
fn depth_limit_is_exact() {
    // s0 -> s1 -> ... -> s16 : 16 saltos = permitido
    let n = MAX_NESTING_DEPTH + 2;
    let mut d = doc_with(n + 1);
    for i in 0..MAX_NESTING_DEPTH {
        assert!(
            check_nested_edge(&d, &sid(i), &sid(i + 1)).is_none(),
            "edge {i}"
        );
        nest(&mut d, i, i + 1, false);
    }
    assert!(validate_nested_graph(&d).is_empty());
    assert_eq!(nested_depth_below(&d, &sid(0)), MAX_NESTING_DEPTH);
    // o 17º salto estoura, tanto pela ponta de baixo quanto pela de cima
    let below =
        check_nested_edge(&d, &sid(MAX_NESTING_DEPTH), &sid(MAX_NESTING_DEPTH + 1)).unwrap();
    assert_eq!(below.code, ErrorCode::NestedDepth);
    let above = check_nested_edge(&d, &sid(MAX_NESTING_DEPTH + 2), &sid(0)).unwrap();
    assert_eq!(above.code, ErrorCode::NestedDepth);
    // forçando a aresta, a validação global também acusa
    nest(&mut d, MAX_NESTING_DEPTH, MAX_NESTING_DEPTH + 1, false);
    assert!(
        validate_nested_graph(&d)
            .iter()
            .any(|v| v.code == ErrorCode::NestedDepth)
    );
}

#[test]
fn the_nested_index_tracks_inserts_removes_and_replaces() {
    let mut d = doc_with(3);
    let id = nest(&mut d, 0, 1, true);
    let seq = d.sequence(&sid(0)).unwrap();
    let refs: Vec<_> = seq.nested_refs().collect();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].1.target, sid(1));
    assert!(refs[0].1.follow_length);
    assert!(seq.index_is_consistent());
    // retarget por replace
    let old = seq.clip(&id.as_str().into()).unwrap().clone();
    let mut new = old.clone();
    new.content = ClipContent::Nested {
        sequence: sid(2),
        follow_length: false,
    };
    d.apply_op(&PrimitiveOp::Clip {
        sequence: sid(0),
        id: id.as_str().into(),
        old: Some(old.clone()),
        new: Some(new),
    })
    .unwrap();
    let seq = d.sequence(&sid(0)).unwrap();
    assert_eq!(seq.nested_refs().next().unwrap().1.target, sid(2));
    assert!(seq.index_is_consistent());
    // remoção
    let cur = seq.clip(&id.as_str().into()).unwrap().clone();
    d.apply_op(&PrimitiveOp::Clip {
        sequence: sid(0),
        id: id.as_str().into(),
        old: Some(cur),
        new: None,
    })
    .unwrap();
    assert_eq!(d.sequence(&sid(0)).unwrap().nested_refs().count(), 0);
    assert!(d.sequence(&sid(0)).unwrap().index_is_consistent());
}

#[test]
fn follow_length_serializes_only_when_set_and_round_trips() {
    let off = ClipContent::Nested {
        sequence: sid(1),
        follow_length: false,
    };
    let on = ClipContent::Nested {
        sequence: sid(1),
        follow_length: true,
    };
    assert_eq!(
        serde_json::to_string(&off).unwrap(),
        r#"{"type":"nested","sequence":"s1"}"#
    );
    assert_eq!(
        serde_json::to_string(&on).unwrap(),
        r#"{"type":"nested","sequence":"s1","follow_length":true}"#
    );
    assert_eq!(
        serde_json::from_str::<ClipContent>(r#"{"type":"nested","sequence":"s1"}"#).unwrap(),
        off
    );
    assert_eq!(
        serde_json::from_str::<ClipContent>(&serde_json::to_string(&on).unwrap()).unwrap(),
        on
    );
}
