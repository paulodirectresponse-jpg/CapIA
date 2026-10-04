//! Núcleo de UX da timeline para a WebView (ADR-070). A UI mantém uma réplica da sequence no
//! módulo WASM (carregada por JSON e atualizada pelos **mesmos patches** que o engine devolve) e
//! calcula o *ghost* de arrastos/trim com `resolve_snap`, `resolve_point_snap`,
//! `resolve_group_move` e `resolve_placement` do `capia-commands` — nenhuma matemática de snap é
//! reimplementada em JS. A ABI é mínima: `capia_alloc/free` + `capia_call(op, ptr, len)` com JSON.
//!
//! O `unsafe` fica restrito à borda de ponteiros (alloc/free/leitura/escrita de buffers); toda a
//! lógica é Rust seguro em [`dispatch`], testável nativamente.

use capia_commands::{
    GroupMoveRequest, GroupSnap, Placement, PlacementStrategy, SnapRequest, resolve_group_move,
    resolve_placement, resolve_point_snap, resolve_snap, threshold_ticks,
};
use capia_model::{ClipId, Document, PrimitiveOp, SequenceId, TrackKind};
use capia_time::{Rational, Ticks, TimeRange};
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::RefCell;

/// Operações do `capia_call`.
pub mod op {
    pub const LOAD: u32 = 1;
    pub const PATCH: u32 = 2;
    pub const THRESHOLD: u32 = 3;
    pub const SNAP_CLIP: u32 = 4;
    pub const SNAP_POINT: u32 = 5;
    pub const GROUP_MOVE: u32 = 6;
    pub const PLACEMENT: u32 = 7;
    pub const FRAME_ALIGN: u32 = 8;
}

#[derive(Default)]
struct State {
    doc: Option<Document>,
    sequence: Option<SequenceId>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn err(code: &str, message: impl Into<String>) -> Value {
    json!({ "error": { "code": code, "message": message.into() } })
}

fn parse<T: for<'de> Deserialize<'de>>(v: &Value) -> Result<T, Value> {
    serde_json::from_value(v.clone()).map_err(|e| err("INVALID_ARGUMENT", e.to_string()))
}

#[derive(Deserialize)]
struct Load {
    id: SequenceId,
    sequence: Value,
}

#[derive(Deserialize)]
struct Threshold {
    px_milli: i64,
    pps_milli: i64,
}

#[derive(Deserialize)]
struct SnapClip {
    clip: ClipId,
    proposed_start: Ticks,
    threshold: Ticks,
    #[serde(default)]
    playhead: Option<Ticks>,
    #[serde(default = "yes")]
    enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct SnapPoint {
    t: Ticks,
    threshold: Ticks,
    #[serde(default)]
    exclude: Vec<ClipId>,
    #[serde(default)]
    playhead: Option<Ticks>,
}

#[derive(Deserialize)]
struct GroupMoveReq {
    members: Vec<ClipId>,
    delta_time: Ticks,
    delta_tracks: i32,
    #[serde(default)]
    snap: Option<GroupSnapReq>,
}

#[derive(Deserialize)]
struct GroupSnapReq {
    threshold: Ticks,
    #[serde(default)]
    playhead: Option<Ticks>,
}

#[derive(Deserialize)]
struct PlacementReq {
    strategy: PlacementStrategy,
    kind: TrackKind,
    spans: Vec<(Ticks, Ticks)>,
}

#[derive(Deserialize)]
struct FrameAlign {
    t: Ticks,
}

fn with_seq<R>(f: impl FnOnce(&capia_model::Sequence) -> Result<R, Value>) -> Result<R, Value> {
    STATE.with(|s| {
        let s = s.borrow();
        let (Some(doc), Some(id)) = (&s.doc, &s.sequence) else {
            return Err(err("NOT_LOADED", "no sequence is loaded"));
        };
        let seq = doc
            .sequence(id)
            .ok_or_else(|| err("NOT_LOADED", "the loaded sequence disappeared"))?;
        f(seq)
    })
}

/// Despacho puro (sem ponteiros): entrada e saída em JSON.
pub fn dispatch(op_id: u32, input: &Value) -> Value {
    match run(op_id, input) {
        Ok(v) | Err(v) => v,
    }
}

fn run(op_id: u32, input: &Value) -> Result<Value, Value> {
    match op_id {
        op::LOAD => {
            let Load { id, sequence } = parse(input)?;
            let doc: Document = serde_json::from_value(json!({
                "schema_version": capia_model::DOCUMENT_SCHEMA_VERSION,
                "revision": 0,
                "sequences": { id.as_str(): sequence },
                "assets": {},
            }))
            .map_err(|e| err("INVALID_SEQUENCE", e.to_string()))?;
            STATE.with(|s| {
                *s.borrow_mut() = State {
                    doc: Some(doc),
                    sequence: Some(id),
                };
            });
            Ok(json!({ "ok": true }))
        }
        op::PATCH => {
            let ops: Vec<PrimitiveOp> = parse(&input["patches"])?;
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                let Some(doc) = s.doc.as_mut() else {
                    return Err(err("NOT_LOADED", "no sequence is loaded"));
                };
                for op in &ops {
                    // só ops de entidades da sequence carregada (as demais não interessam à réplica)
                    let relevant = matches!(
                        op,
                        PrimitiveOp::Clip { .. }
                            | PrimitiveOp::Track { .. }
                            | PrimitiveOp::Marker { .. }
                    ) && op
                        .sequence_id()
                        .is_some_and(|sid| doc.sequence(sid).is_some());
                    if relevant {
                        doc.apply_op(op)
                            .map_err(|e| err("PATCH_MISMATCH", e.to_string()))?;
                    }
                }
                Ok(json!({ "ok": true }))
            })
        }
        op::THRESHOLD => {
            let t: Threshold = parse(input)?;
            let px = Rational::new(t.px_milli, 1000)
                .map_err(|e| err("INVALID_ARGUMENT", e.to_string()))?;
            let pps = Rational::new(t.pps_milli, 1000)
                .map_err(|e| err("INVALID_ARGUMENT", e.to_string()))?;
            let ticks =
                threshold_ticks(px, pps).map_err(|e| err(&e.code.to_string(), e.message))?;
            Ok(json!({ "ticks": ticks }))
        }
        op::SNAP_CLIP => {
            let r: SnapClip = parse(input)?;
            with_seq(|seq| {
                let res = resolve_snap(
                    seq,
                    &SnapRequest {
                        clip: &r.clip,
                        proposed_start: r.proposed_start,
                        threshold: r.threshold,
                        markers: &[],
                        playhead: r.playhead,
                        enabled: r.enabled,
                    },
                )
                .map_err(|e| err(&e.code.to_string(), e.message))?;
                serde_json::to_value(res).map_err(|e| err("INTERNAL", e.to_string()))
            })
        }
        op::SNAP_POINT => {
            let r: SnapPoint = parse(input)?;
            with_seq(|seq| {
                let hit = resolve_point_snap(seq, &r.exclude, r.t, r.threshold, &[], r.playhead);
                Ok(json!({ "target": hit }))
            })
        }
        op::GROUP_MOVE => {
            let r: GroupMoveReq = parse(input)?;
            with_seq(|seq| {
                let snap = r.snap.as_ref().map(|s| GroupSnap {
                    threshold: s.threshold,
                    markers: &[],
                    playhead: s.playhead,
                });
                let res = resolve_group_move(
                    seq,
                    &GroupMoveRequest {
                        members: &r.members,
                        delta_time: r.delta_time,
                        delta_tracks: r.delta_tracks,
                        snap,
                    },
                )
                .map_err(|e| {
                    let mut v = err(&e.code.to_string(), e.message);
                    if let Some(h) = e.hint {
                        v["error"]["hint"] = h;
                    }
                    v
                })?;
                serde_json::to_value(res).map_err(|e| err("INTERNAL", e.to_string()))
            })
        }
        op::PLACEMENT => {
            let r: PlacementReq = parse(input)?;
            let spans: Vec<TimeRange> = r
                .spans
                .iter()
                .map(|(s, d)| TimeRange::new(*s, *d))
                .collect();
            with_seq(|seq| {
                let p: Placement = resolve_placement(seq, &r.strategy, r.kind, &spans)
                    .map_err(|e| err(&e.code.to_string(), e.message))?;
                serde_json::to_value(p).map_err(|e| err("INTERNAL", e.to_string()))
            })
        }
        op::FRAME_ALIGN => {
            let r: FrameAlign = parse(input)?;
            with_seq(|seq| {
                let t = seq
                    .frame_rate()
                    .align_half_up(r.t)
                    .map_err(|e| err("OUT_OF_RANGE", e.to_string()))?;
                Ok(json!({ "t": t }))
            })
        }
        other => Err(err("UNKNOWN_OP", format!("unknown op {other}"))),
    }
}

// ----------------------------------------------------------------------------- borda FFI (WASM)

/// Núcleo da chamada: bytes JSON de entrada → bytes JSON de saída (testável nativamente).
pub fn call_bytes(op_id: u32, input: &[u8]) -> Vec<u8> {
    let v: Value = if input.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(input).unwrap_or(Value::Null)
    };
    dispatch(op_id, &v).to_string().into_bytes()
}

/// ABI C **só para wasm32** (ponteiros de 32 bits cabem no empacotamento `ptr << 32 | len`).
#[cfg(target_arch = "wasm32")]
mod abi {
    use super::call_bytes;

    /// Aloca `len` bytes para o chamador escrever a entrada. Devolve o ponteiro (0 se `len == 0`).
    #[unsafe(no_mangle)]
    pub(crate) extern "C" fn capia_alloc(len: usize) -> *mut u8 {
        if len == 0 {
            return core::ptr::null_mut();
        }
        let mut v = Vec::<u8>::with_capacity(len);
        let p = v.as_mut_ptr();
        core::mem::forget(v);
        p
    }

    /// Libera um buffer obtido por `capia_alloc` ou devolvido por `capia_call`.
    ///
    /// # Safety
    /// `ptr`/`len` devem ser exatamente os de uma alocação ainda viva deste módulo.
    #[unsafe(no_mangle)]
    pub(crate) unsafe extern "C" fn capia_free(ptr: *mut u8, len: usize) {
        if !ptr.is_null() && len > 0 {
            // SAFETY: contrato acima — reconstrói o Vec com a mesma capacidade para liberar.
            drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
        }
    }

    /// Executa uma operação. Entrada: JSON UTF-8 em `ptr[..len]`. Saída: `(ptr << 32) | len` do
    /// JSON de resposta (libere com `capia_free`). O buffer de entrada **não** é liberado aqui.
    ///
    /// # Safety
    /// `ptr[..len]` deve ser memória válida deste módulo.
    #[unsafe(no_mangle)]
    pub(crate) unsafe extern "C" fn capia_call(op_id: u32, ptr: *const u8, len: usize) -> u64 {
        let input: &[u8] = if ptr.is_null() || len == 0 {
            &[]
        } else {
            // SAFETY: contrato acima.
            unsafe { core::slice::from_raw_parts(ptr, len) }
        };
        let mut out = call_bytes(op_id, input);
        out.shrink_to_fit();
        let n = out.len();
        let p = out.as_mut_ptr();
        core::mem::forget(out);
        ((p as usize as u64) << 32) | (n as u64)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const F: i64 = 23_520_000;

    fn sequence_json() -> Value {
        json!({
            "header": {"name": "s", "frame_rate": "30", "sample_rate": 48000},
            "tracks": [
                {"id":"v","name":"","kind":"visual","role":"overlay","magnetic":false,"locked":false,"hidden":false,"muted":false,"solo":false,"sync_lock":true,"group":null},
                {"id":"a","name":"","kind":"audio","role":"music","magnetic":false,"locked":false,"hidden":false,"muted":false,"solo":false,"sync_lock":true,"group":null}
            ],
            "clips": {
                "c1": {"id":"c1","track":"v","start":0,"duration":F*10,"name":"","enabled":true,
                       "content":{"type":"solid","color":"#fff"},"source_in":0,"speed":"1","reversed":false,"properties":{}},
                "c2": {"id":"c2","track":"v","start":F*30,"duration":F*10,"name":"","enabled":true,
                       "content":{"type":"solid","color":"#fff"},"source_in":0,"speed":"1","reversed":false,"properties":{}}
            },
            "markers": {}
        })
    }

    fn load() {
        let r = dispatch(op::LOAD, &json!({"id":"s","sequence":sequence_json()}));
        assert_eq!(r["ok"], true, "{r}");
    }

    #[test]
    fn operations_fail_cleanly_before_a_sequence_is_loaded() {
        STATE.with(|s| *s.borrow_mut() = State::default());
        let r = dispatch(op::SNAP_POINT, &json!({"t":0,"threshold":1}));
        assert_eq!(r["error"]["code"], "NOT_LOADED");
        assert_eq!(dispatch(999, &Value::Null)["error"]["code"], "UNKNOWN_OP");
    }

    #[test]
    fn threshold_matches_the_engine_exactly() {
        let r = dispatch(
            op::THRESHOLD,
            &json!({"px_milli": 10_000, "pps_milli": 50_000}),
        );
        assert_eq!(r["ticks"], 141_120_000);
        assert!(
            dispatch(op::THRESHOLD, &json!({"px_milli": 10_000, "pps_milli": 0}))["error"]["code"]
                .is_string()
        );
    }

    #[test]
    fn snap_clip_uses_the_engine_rules_on_the_loaded_replica() {
        load();
        // arrastar c1 para começar em 29F: a ponta final (39F) encosta no início de c2 (30F)? não;
        // mas o início de c1 a 29F está a 1F do início de c2 (30F) → cola em 30F? c2 ocupa 30..40 ⇒ bloqueado
        let r = dispatch(
            op::SNAP_CLIP,
            &json!({"clip":"c1","proposed_start":F*19,"threshold":F*2}),
        );
        // a ponta final de c1 (29F) fica a 1F do início de c2 → cola em 20F (fim de c1 em 30F)
        assert_eq!(r["start"], F * 20, "{r}");
        assert_eq!(r["snapped_to"]["type"], "clip_start");
        let off = dispatch(
            op::SNAP_CLIP,
            &json!({"clip":"c1","proposed_start":F*19,"threshold":F*2,"enabled":false}),
        );
        assert_eq!(off["start"], F * 19);
        assert!(off["snapped_to"].is_null());
    }

    #[test]
    fn patches_keep_the_replica_in_sync_with_the_engine() {
        load();
        let moved = {
            let mut c = sequence_json()["clips"]["c1"].clone();
            c["start"] = json!(F * 5);
            c
        };
        let r = dispatch(
            op::PATCH,
            &json!({"patches":[{"op":"clip","sequence":"s","id":"c1",
                "old": sequence_json()["clips"]["c1"], "new": moved}]}),
        );
        assert_eq!(r["ok"], true, "{r}");
        // agora o início de c1 (5F) é alvo de snap para um ponto a 5F+1 tick
        let hit = dispatch(
            op::SNAP_POINT,
            &json!({"t": F*5 + 1, "threshold": 100, "exclude": ["c2"]}),
        );
        assert_eq!(hit["target"]["t"], F * 5);
        // patch com `old` errado é recusado (réplica divergente → a UI recarrega)
        let bad = dispatch(
            op::PATCH,
            &json!({"patches":[{"op":"clip","sequence":"s","id":"c1",
                "old": sequence_json()["clips"]["c1"], "new": null}]}),
        );
        assert_eq!(bad["error"]["code"], "PATCH_MISMATCH");
    }

    #[test]
    fn group_move_and_placement_and_alignment() {
        load();
        let g = dispatch(
            op::GROUP_MOVE,
            &json!({"members":["c1"],"delta_time": -F*3,"delta_tracks":0}),
        );
        assert_eq!(g["delta_time"], 0, "clamped at the sequence start: {g}");
        // faixa [0,10F) ocupada em v ⇒ nova track visual acima
        let p = dispatch(
            op::PLACEMENT,
            &json!({"strategy":{"type":"first_available"},"kind":"visual","spans":[[0, F*10]]}),
        );
        assert_eq!(p["kind"], "new_track");
        let free = dispatch(
            op::PLACEMENT,
            &json!({"strategy":{"type":"first_available"},"kind":"visual","spans":[[F*12, F*5]]}),
        );
        assert_eq!(free["kind"], "existing");
        // alinhamento ao quadro (half-up) pelo frame rate da sequence
        let a = dispatch(op::FRAME_ALIGN, &json!({"t": F*3 + F/2}));
        assert_eq!(a["t"], F * 4);
    }

    #[test]
    fn the_byte_boundary_round_trips_json_and_survives_garbage() {
        let input = json!({"id":"s","sequence":sequence_json()}).to_string();
        let out: Value = serde_json::from_slice(&call_bytes(op::LOAD, input.as_bytes())).unwrap();
        assert_eq!(out["ok"], true);
        // entrada inválida nunca derruba: vira erro estruturado
        let bad: Value = serde_json::from_slice(&call_bytes(op::LOAD, b"{not json")).unwrap();
        assert_eq!(bad["error"]["code"], "INVALID_ARGUMENT");
        let empty: Value = serde_json::from_slice(&call_bytes(op::SNAP_POINT, b"")).unwrap();
        assert!(empty["error"].is_object());
    }
}
