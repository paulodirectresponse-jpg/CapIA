//! Suíte de aceitação de comportamento da timeline (ADR-036) — critério da Fase 2.
//!
//! Carrega cada cenário de `tests/acceptance/timeline/*.json`, monta o `given` (por ops primitivas,
//! sem passar por validação de comandos), executa `when` pelo Command Engine real e confere
//! `then`. Unidade nos cenários: **frames** (inteiros ou `"n/d"`); o harness converte para ticks e
//! exige exatidão. Em erro, o documento deve permanecer **idêntico** (atomicidade).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use capia_commands::{
    Actor, Command, CommandEnvelope, Edge, Engine, GroupMoveRequest, GroupSnap, NewClip,
    PlacementStrategy, RippleScope, SnapRequest, Transaction, resolve_group_move,
    resolve_placement, resolve_snap, threshold_ticks,
};
use capia_model::{
    Animatable, Asset, Clip, ClipContent, ClipId, Document, Interp, Keyframe, PrimitiveOp,
    SequenceHeader, SequenceId, Track, TrackId, TrackKind, TrackSlot, validate_document,
};
use capia_time::{FrameRate, Rational, Ticks, TimeRange};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const SEQ: &str = "SEQ";

struct World {
    fr: FrameRate,
    engine: Engine,
    given_timing: BTreeMap<String, Vec<(String, i64, i64)>>,
    op_counter: u32,
}

fn rational(v: &Value) -> Rational {
    match v {
        Value::Number(n) => {
            let n = n.as_f64().expect("number");
            assert!(
                n.fract() == 0.0,
                "scenario numbers must be integers or strings, got {n}"
            );
            Rational::from_int(n as i64)
        }
        Value::String(s) => Rational::parse(s).expect("rational string"),
        other => panic!("not a rational: {other}"),
    }
}

impl World {
    fn ticks(&self, v: &Value) -> Ticks {
        self.fr
            .rational_frames_to_ticks(rational(v))
            .expect("frames to ticks")
    }

    fn frames_json(&self, t: Ticks) -> Value {
        let fd = self.fr.frame_duration().0;
        if t.0 % fd == 0 {
            json!(t.0 / fd)
        } else {
            let r = Rational::new(t.0, fd).unwrap();
            json!(r.to_string())
        }
    }

    fn doc(&self) -> &Document {
        self.engine.document()
    }
}

fn frame_rate(s: &str) -> FrameRate {
    FrameRate::new(Rational::parse(s).unwrap()).unwrap()
}

fn kind_of(v: &Value) -> TrackKind {
    match v.as_str().unwrap() {
        "visual" => TrackKind::Visual,
        "audio" => TrackKind::Audio,
        other => panic!("track kind {other}"),
    }
}

fn parse_interp(v: Option<&Value>) -> Interp {
    match v {
        None => Interp::Linear,
        Some(Value::String(s)) if s == "hold" => Interp::Hold,
        Some(Value::String(s)) if s == "linear" => Interp::Linear,
        Some(Value::Object(o)) => {
            let b: Vec<f64> = o["bezier"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            Interp::Bezier {
                x1: b[0],
                y1: b[1],
                x2: b[2],
                y2: b[3],
            }
        }
        other => panic!("interp {other:?}"),
    }
}

fn build_world(s: &Value) -> World {
    let fr = frame_rate(s["fps"].as_str().unwrap());
    let mut doc = Document::new();
    let mut world = World {
        fr,
        engine: Engine::new(Document::new(), [7; 32]),
        given_timing: BTreeMap::new(),
        op_counter: 0,
    };
    let seq_id = SequenceId::from(SEQ);
    doc.apply_op(&PrimitiveOp::Sequence {
        id: seq_id.clone(),
        old: None,
        new: Some(SequenceHeader {
            name: "seq".into(),
            frame_rate: fr,
            sample_rate: 48_000,
        }),
    })
    .unwrap();
    for (index, t) in s["given"]["tracks"].as_array().unwrap().iter().enumerate() {
        let tid = t["id"].as_str().unwrap();
        let kind = kind_of(&t["kind"]);
        let mut track = Track::new(tid, kind);
        track.magnetic = t["magnetic"].as_bool().unwrap_or(false);
        track.locked = t["locked"].as_bool().unwrap_or(false);
        track.sync_lock = t["sync_lock"].as_bool().unwrap_or(true);
        track.group = t["group"].as_str().map(str::to_owned);
        doc.apply_op(&PrimitiveOp::Track {
            sequence: seq_id.clone(),
            id: TrackId::from(tid),
            old: None,
            new: Some(TrackSlot { index, track }),
        })
        .unwrap();
        let mut timing = Vec::new();
        for c in t["clips"].as_array().unwrap() {
            let clip = given_clip(&world, tid, kind, c, &mut doc);
            timing.push((clip.id.0.clone(), clip.start.0, clip.duration.0));
            doc.apply_op(&PrimitiveOp::Clip {
                sequence: seq_id.clone(),
                id: clip.id.clone(),
                old: None,
                new: Some(clip),
            })
            .unwrap();
        }
        timing.sort_by_key(|(_, start, _)| *start);
        world.given_timing.insert(tid.to_owned(), timing);
    }
    let violations = validate_document(&doc);
    assert!(
        violations.is_empty(),
        "{}: invalid given state: {violations:?}",
        s["id"]
    );
    world.engine = Engine::new(doc, [7; 32]);
    world
}

fn content_for(c: &Value, track_kind: TrackKind, asset: &str) -> ClipContent {
    match c["content"].as_str() {
        Some("text") => ClipContent::Text {
            text: "text".into(),
        },
        Some("solid") => ClipContent::Solid {
            color: "#000000".into(),
        },
        Some(other) => panic!("content {other}"),
        None => {
            let audio = c["kind"]
                .as_str()
                .map_or(track_kind == TrackKind::Audio, |k| k == "audio");
            ClipContent::Media {
                asset: asset.into(),
                has_video: !audio,
                has_audio: audio,
            }
        }
    }
}

fn given_clip(w: &World, track: &str, kind: TrackKind, c: &Value, doc: &mut Document) -> Clip {
    let id = c["id"].as_str().unwrap();
    let asset_id = format!("asset_{id}");
    let content = content_for(c, kind, &asset_id);
    if matches!(content, ClipContent::Media { .. }) {
        let duration = c.get("src_dur").map(|v| w.ticks(v));
        doc.apply_op(&PrimitiveOp::Asset {
            id: asset_id.as_str().into(),
            old: None,
            new: Some(Asset {
                id: asset_id.as_str().into(),
                name: id.into(),
                duration,
                has_video: true,
                has_audio: true,
                offline: false,
            }),
        })
        .unwrap();
    }
    let mut properties = BTreeMap::new();
    if let Some(kf) = c.get("kf") {
        for (prop, list) in kf.as_object().unwrap() {
            let kfs = list
                .as_array()
                .unwrap()
                .iter()
                .map(|k| Keyframe {
                    time: w.ticks(&k[0]),
                    value: k[1].as_f64().unwrap(),
                    interp: parse_interp(k.get(2)),
                })
                .collect();
            properties.insert(prop.clone(), Animatable::Animated(kfs));
        }
    }
    Clip {
        id: id.into(),
        track: track.into(),
        start: w.ticks(&c["start"]),
        duration: w.ticks(&c["dur"]),
        name: id.into(),
        enabled: true,
        content,
        source_in: c.get("src_in").map_or(Ticks::ZERO, |v| w.ticks(v)),
        speed: c.get("speed").map_or(Rational::ONE, rational),
        reversed: c["reversed"].as_bool().unwrap_or(false),
        properties,
    }
}

fn scope_of(v: Option<&Value>) -> RippleScope {
    match v.and_then(Value::as_str) {
        None | Some("track") => RippleScope::Track,
        Some("sequence") => RippleScope::Sequence,
        Some("group") => RippleScope::Group,
        Some(other) => panic!("scope {other}"),
    }
}

fn scope_json(v: &Value) -> RippleScope {
    match v {
        Value::String(_) => scope_of(Some(v)),
        Value::Object(o) if o.contains_key("tracks") => RippleScope::Tracks {
            tracks: o["tracks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| TrackId::from(t.as_str().unwrap()))
                .collect(),
        },
        other => panic!("scope {other}"),
    }
}

/// `when` → comandos do engine (asset prévio incluído) — `None` para consultas puras.
fn to_commands(w: &World, cmd: &str, a: &Value) -> Option<Vec<Command>> {
    let clip = |k: &str| ClipId::from(a[k].as_str().unwrap());
    let scope = || a.get("scope").map_or(RippleScope::Track, scope_json);
    let ripple = || a.get("ripple").and_then(Value::as_bool);
    Some(match cmd {
        "insert_clip" => {
            let c = &a["clip"];
            let track_kind = {
                let tid = a["track"].as_str().unwrap();
                let (_, _, t) = w.doc().find_track(&TrackId::from(tid)).expect("track");
                t.kind
            };
            let id = c["id"].as_str().unwrap();
            let asset_id = format!("asset_{id}");
            let content = content_for(c, track_kind, &asset_id);
            let mut out = Vec::new();
            if matches!(content, ClipContent::Media { .. }) {
                out.push(Command::RegisterAsset {
                    asset: Asset {
                        id: asset_id.as_str().into(),
                        name: id.into(),
                        duration: c.get("src_dur").map(|v| w.ticks(v)),
                        has_video: true,
                        has_audio: true,
                        offline: false,
                    },
                });
            }
            out.push(Command::InsertClip {
                track: a["track"].as_str().unwrap().into(),
                start: w.ticks(&a["start"]),
                clip: NewClip {
                    id: Some(id.into()),
                    name: id.into(),
                    duration: w.ticks(&c["dur"]),
                    content,
                    source_in: c.get("src_in").map_or(Ticks::ZERO, |v| w.ticks(v)),
                    speed: c.get("speed").map_or(Rational::ONE, rational),
                    reversed: false,
                    properties: BTreeMap::new(),
                },
                split_at_insert: a["split_at_insert"].as_bool().unwrap_or(false),
                split_new_id: a["split_new_id"].as_str().map(ClipId::from),
            });
            out
        }
        "delete_clip" => vec![Command::DeleteClip {
            clip: clip("clip"),
            ripple: ripple(),
            scope: scope(),
        }],
        "trim_clip" => vec![Command::TrimClip {
            clip: clip("clip"),
            edge: if a["edge"] == "in" {
                Edge::In
            } else {
                Edge::Out
            },
            to: w.ticks(&a["to"]),
            ripple: ripple(),
            scope: scope(),
        }],
        "split_clip" => vec![Command::SplitClip {
            clip: clip("clip"),
            at: w.ticks(&a["at"]),
            new_id: a["new_id"].as_str().map(ClipId::from),
        }],
        "set_clip_speed" => vec![Command::SetClipSpeed {
            clip: clip("clip"),
            speed: rational(&a["speed"]),
            ripple: ripple(),
            scope: scope(),
        }],
        "add_keyframe" => vec![Command::AddKeyframe {
            clip: clip("clip"),
            prop: a["prop"].as_str().unwrap().into(),
            at: w.ticks(&a["at"]),
            value: a["value"].as_f64().unwrap(),
            interp: None,
        }],
        "move_keyframe" => vec![Command::MoveKeyframe {
            clip: clip("clip"),
            prop: a["prop"].as_str().unwrap().into(),
            from: w.ticks(&a["from"]),
            to: w.ticks(&a["to"]),
        }],
        _ => return None,
    })
}

#[derive(Default)]
struct Outcome {
    error: Option<String>,
    result: Option<Value>,
    values: Option<Vec<f64>>,
}

fn snap_threshold(w: &World, a: &Value) -> Ticks {
    w.ticks(&a["threshold_frames"])
}

fn ticks_list(w: &World, v: Option<&Value>) -> Vec<Ticks> {
    v.and_then(Value::as_array)
        .map(|l| l.iter().map(|x| w.ticks(x)).collect())
        .unwrap_or_default()
}

fn run_query(w: &World, cmd: &str, a: &Value) -> Outcome {
    let seq = || w.doc().sequence(&SequenceId::from(SEQ)).expect("sequence");
    let err = |e: capia_commands::CommandError| Outcome {
        error: Some(e.code.as_str().into()),
        ..Outcome::default()
    };
    match cmd {
        "resolve_placement" => {
            let strategy: PlacementStrategy =
                serde_json::from_value(a["strategy"].clone()).unwrap();
            let spans: Vec<TimeRange> = a["spans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| TimeRange::new(w.ticks(&s[0]), w.ticks(&s[1])))
                .collect();
            match resolve_placement(seq(), &strategy, kind_of(&a["kind"]), &spans) {
                Ok(p) => Outcome {
                    result: Some(serde_json::to_value(p).unwrap()),
                    ..Outcome::default()
                },
                Err(e) => err(e),
            }
        }
        "resolve_snap" => {
            let markers = ticks_list(w, a.get("markers"));
            let playhead = a
                .get("playhead")
                .filter(|v| !v.is_null())
                .map(|v| w.ticks(v));
            let clip = ClipId::from(a["clip"].as_str().unwrap());
            let req = SnapRequest {
                clip: &clip,
                proposed_start: w.ticks(&a["proposed_start"]),
                threshold: snap_threshold(w, a),
                markers: &markers,
                playhead,
                enabled: a["enabled"].as_bool().unwrap_or(true),
            };
            match resolve_snap(seq(), &req) {
                Ok(r) => Outcome {
                    result: Some(json!({
                        "start": w.frames_json(r.start),
                        "snapped_to": r.snapped_to.map(|t| json!({ "type": t.kind, "t": w.frames_json(t.t) })),
                    })),
                    ..Outcome::default()
                },
                Err(e) => err(e),
            }
        }
        "threshold_ticks" => {
            let t = threshold_ticks(rational(&a["px"]), rational(&a["pps"])).unwrap();
            Outcome {
                result: Some(json!({ "ticks": t.0 })),
                ..Outcome::default()
            }
        }
        "resolve_group_move" => {
            let members: Vec<ClipId> = a["members"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| ClipId::from(m.as_str().unwrap()))
                .collect();
            let markers = ticks_list(w, a.get("snap").and_then(|s| s.get("markers")));
            let snap = a.get("snap").map(|s| GroupSnap {
                threshold: snap_threshold(w, s),
                markers: &markers,
                playhead: s
                    .get("playhead")
                    .filter(|v| !v.is_null())
                    .map(|v| w.ticks(v)),
            });
            let req = GroupMoveRequest {
                members: &members,
                delta_time: w.ticks(&a["delta_time"]),
                delta_tracks: i32::try_from(a["delta_tracks"].as_i64().unwrap()).unwrap(),
                snap,
            };
            match resolve_group_move(seq(), &req) {
                Ok(r) => Outcome {
                    result: Some(
                        json!({ "delta_time": w.frames_json(r.delta_time), "delta_tracks": r.delta_tracks }),
                    ),
                    ..Outcome::default()
                },
                Err(e) => err(e),
            }
        }
        "eval_keyframes" => {
            let clip = ClipId::from(a["clip"].as_str().unwrap());
            let prop = a["prop"].as_str().unwrap();
            let values = a["at"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| capia_commands::eval_property(w.doc(), &clip, prop, w.ticks(t)).unwrap())
                .collect();
            Outcome {
                values: Some(values),
                ..Outcome::default()
            }
        }
        other => panic!("unknown command {other}"),
    }
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn run_scenario(s: &Value) -> Result<(), String> {
    let mut w = build_world(s);
    let when = &s["when"];
    let steps: Vec<&Value> = if when.is_array() {
        when.as_array().unwrap().iter().collect()
    } else {
        vec![when]
    };
    let mut last = Outcome::default();
    for step in steps {
        let cmd = step["cmd"].as_str().unwrap();
        let args = &step["args"];
        last = Outcome::default();
        match to_commands(&w, cmd, args) {
            Some(commands) => {
                w.op_counter += 1;
                let tx = Transaction {
                    transaction_id: None,
                    label: cmd.into(),
                    base_revision: None,
                    commands: commands
                        .into_iter()
                        .enumerate()
                        .map(|(i, command)| CommandEnvelope {
                            operation_id: format!("acc-{}-{i}", w.op_counter),
                            reference: None,
                            command,
                        })
                        .collect(),
                    max_ops: None,
                };
                let (rev, before) = (w.engine.revision(), w.engine.document().clone());
                if let Err(e) = w.engine.execute(&Actor::user("acceptance"), tx, 0) {
                    if w.engine.revision() != rev || w.engine.document() != &before {
                        return Err(format!(
                            "error {} left the document changed (atomicity)",
                            e.code
                        ));
                    }
                    last.error = Some(e.code.as_str().into());
                }
            }
            None => {
                let q = run_query(&w, cmd, args);
                if q.error.is_some() {
                    last.error = q.error;
                }
                last.result = q.result.or(last.result);
                if q.values.is_some() {
                    last.values = q.values;
                }
            }
        }
    }
    check_then(&w, s, &last)
}

fn check_then(w: &World, s: &Value, out: &Outcome) -> Result<(), String> {
    let then = &s["then"];
    if let Some(code) = then.get("error") {
        return match &out.error {
            Some(got) if got == code.as_str().unwrap() => Ok(()),
            got => Err(format!("expected error {code}, got {got:?}")),
        };
    }
    if let Some(got) = &out.error {
        return Err(format!("unexpected error {got}"));
    }
    let invalid = validate_document(w.doc());
    if !invalid.is_empty() {
        return Err(format!("document invariants violated: {invalid:?}"));
    }
    let seq = w.doc().sequence(&SequenceId::from(SEQ)).unwrap();
    // tracks: listadas conferem com o esperado; não listadas = timing inalterado
    let expected_tracks = then.get("tracks").and_then(Value::as_object);
    for track in seq.tracks() {
        let actual: Vec<(String, i64, i64)> = seq
            .track_clips(&track.id)
            .map(|c| (c.id.0.clone(), c.start.0, c.duration.0))
            .collect();
        let expected: Vec<(String, i64, i64)> =
            match expected_tracks.and_then(|m| m.get(track.id.as_str())) {
                Some(list) => list
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| {
                        (
                            t[0].as_str().unwrap().to_owned(),
                            w.ticks(&t[1]).0,
                            w.ticks(&t[2]).0,
                        )
                    })
                    .collect(),
                None => w.given_timing[track.id.as_str()].clone(),
            };
        if actual != expected {
            return Err(format!(
                "track {}: expected {expected:?}, got {actual:?}",
                track.id
            ));
        }
    }
    if let Some(ticks) = then.get("ticks").and_then(Value::as_object) {
        for (id, want) in ticks {
            let c = seq
                .clip(&ClipId::from(id.as_str()))
                .ok_or(format!("clip {id} missing"))?;
            let got = [c.start.0, c.duration.0];
            let want = [want[0].as_i64().unwrap(), want[1].as_i64().unwrap()];
            if got != want {
                return Err(format!("ticks of {id}: expected {want:?}, got {got:?}"));
            }
        }
    }
    if let Some(clips) = then.get("clips").and_then(Value::as_object) {
        for (id, want) in clips {
            let c = seq
                .clip(&ClipId::from(id.as_str()))
                .ok_or(format!("clip {id} missing"))?;
            if let Some(v) = want.get("src_in")
                && c.source_in != w.ticks(v)
            {
                return Err(format!(
                    "{id}.src_in: expected {v}, got {}",
                    w.frames_json(c.source_in)
                ));
            }
            if let Some(v) = want.get("speed")
                && c.speed != rational(v)
            {
                return Err(format!("{id}.speed: expected {v}, got {}", c.speed));
            }
            if let Some(v) = want.get("reversed")
                && c.reversed != v.as_bool().unwrap()
            {
                return Err(format!("{id}.reversed mismatch"));
            }
            if let Some(v) = want.get("kf_count") {
                let n: usize = c.properties.values().map(|a| a.keyframes().len()).sum();
                if n as u64 != v.as_u64().unwrap() {
                    return Err(format!("{id}.kf_count: expected {v}, got {n}"));
                }
            }
        }
    }
    if let Some(kfs) = then.get("kfs").and_then(Value::as_object) {
        for (id, props) in kfs {
            let c = seq
                .clip(&ClipId::from(id.as_str()))
                .ok_or(format!("clip {id} missing"))?;
            for (prop, list) in props.as_object().unwrap() {
                let want = list.as_array().unwrap();
                let got: Vec<Keyframe> = c
                    .properties
                    .get(prop)
                    .map(|a| a.keyframes().to_vec())
                    .unwrap_or_default();
                if got.len() != want.len() {
                    return Err(format!(
                        "{id}.{prop}: expected {} keyframes, got {got:?}",
                        want.len()
                    ));
                }
                for (g, k) in got.iter().zip(want) {
                    if g.time != w.ticks(&k[0]) || !approx(g.value, k[1].as_f64().unwrap()) {
                        return Err(format!("{id}.{prop}: expected {k}, got {g:?}"));
                    }
                }
            }
        }
    }
    if let Some(st) = then.get("static").and_then(Value::as_object) {
        for (id, props) in st {
            let c = seq
                .clip(&ClipId::from(id.as_str()))
                .ok_or(format!("clip {id} missing"))?;
            for (prop, v) in props.as_object().unwrap() {
                match c.properties.get(prop) {
                    Some(Animatable::Static(got)) if approx(*got, v.as_f64().unwrap()) => {}
                    other => {
                        return Err(format!("{id}.{prop}: expected static {v}, got {other:?}"));
                    }
                }
            }
        }
    }
    if let Some(want) = then.get("values") {
        let got = out.values.as_ref().ok_or("no eval_keyframes result")?;
        let want: Vec<f64> = want
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        if got.len() != want.len() || !got.iter().zip(&want).all(|(a, b)| approx(*a, *b)) {
            return Err(format!("values: expected {want:?}, got {got:?}"));
        }
    }
    if let Some(r) = then.get("all_within") {
        let (lo, hi) = (
            r["range"][0].as_f64().unwrap(),
            r["range"][1].as_f64().unwrap(),
        );
        let got = out.values.as_ref().ok_or("no eval_keyframes result")?;
        if let Some(bad) = got.iter().find(|v| **v < lo - 1e-12 || **v > hi + 1e-12) {
            return Err(format!("value {bad} outside [{lo}, {hi}]"));
        }
    }
    if let Some(want) = then.get("result") {
        let got = out.result.as_ref().ok_or("no query result")?;
        if got != want {
            return Err(format!("result: expected {want}, got {got}"));
        }
    }
    Ok(())
}

#[test]
fn every_acceptance_scenario_passes() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/acceptance/timeline"
    );
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let (mut total, mut failures, mut ids) = (0, Vec::new(), std::collections::BTreeSet::new());
    for file in files {
        let scenarios: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        for s in scenarios {
            total += 1;
            let id = s["id"].as_str().unwrap().to_owned();
            assert!(ids.insert(id.clone()), "duplicate scenario id {id}");
            if let Err(why) = run_scenario(&s) {
                failures.push(format!(
                    "{id} ({}): {why}",
                    s["title"].as_str().unwrap_or("")
                ));
            }
        }
    }
    assert!(
        total >= 108,
        "expected at least the 108 original scenarios, found {total}"
    );
    assert!(
        failures.is_empty(),
        "{} of {total} scenarios failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
