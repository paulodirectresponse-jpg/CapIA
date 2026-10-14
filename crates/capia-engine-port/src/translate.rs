//! Tradução do vocabulário de comandos legado (JSON do Editor Agent: `insert_clip`, `add_track`…) para
//! operações da AgentSurface v1. Explícita e **fechada**: o que não tem equivalente falha com
//! `UNSUPPORTED_COMMAND` (nunca é descartado em silêncio).

use crate::PortError;
use crate::units::to_editor_ticks;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct AssetFacts {
    /// Duração total da fonte, em ticks do editor (se conhecida).
    pub duration_editor_ticks: Option<i64>,
}

/// Estado atual de um clip da timeline (ticks do editor), lido por `inspect_tracks`.
#[derive(Clone, Debug, Default)]
pub struct ClipFacts {
    pub start: i64,
    pub duration: i64,
    pub trim_start: i64,
    pub trim_end: i64,
}

#[derive(Clone, Debug, Default)]
pub struct TranslateCtx {
    /// Clips já existentes na timeline (edição de clips por id do editor).
    pub clips: HashMap<String, ClipFacts>,
    /// Id da track principal da sequence ativa (colocação explícita dos beats `Main`).
    pub main_track_id: Option<String>,
    pub assets: HashMap<String, AssetFacts>,
}

#[derive(Clone, Debug, Default)]
pub struct Translated {
    pub operations: Vec<Value>,
    /// Comandos que só descrevem estrutura que a AgentSurface resolve sozinha (ex.: `add_track`).
    pub bookkeeping: Vec<String>,
}

fn unsupported(kind: &str, why: &str) -> PortError {
    PortError::new("UNSUPPORTED_COMMAND", format!("`{kind}`: {why}"))
}

fn int(v: &Value, what: &str) -> Result<i64, PortError> {
    v.as_i64()
        .ok_or_else(|| PortError::new("INVALID_COMMAND", format!("`{what}` must be an integer")))
}

/// Propriedade legada (`capia-model::property`) → parâmetros do elemento no editor OpenCut.
/// Posição: pixels do canvas a partir do centro, Y para baixo (igual nos dois lados).
fn map_property(prop: &str, value: &Value) -> Result<Vec<(&'static str, Value)>, PortError> {
    let v = value.as_f64().ok_or_else(|| {
        PortError::new("INVALID_COMMAND", "set_property `value` must be a number")
    })?;
    Ok(match prop {
        "opacity" => vec![("opacity", json!(v))],
        "scale" => vec![
            ("transform.scaleX", json!(v)),
            ("transform.scaleY", json!(v)),
        ],
        "position_x" => vec![("transform.positionX", json!(v))],
        "position_y" => vec![("transform.positionY", json!(v))],
        "rotation" => vec![("transform.rotate", json!(v))],
        "volume_db" => vec![("volume", json!(v))],
        other => {
            return Err(unsupported(
                "set_property",
                &format!(
                    "property `{other}` has no editor parameter (fade_in/fade_out need keyframes — AgentSurface v2)"
                ),
            ));
        }
    })
}

/// Estilo de texto legado → parâmetros do elemento de texto. Tamanho: o editor usa
/// `fontSize * altura_canvas / 90`, logo `size_permille/1000 * 90`.
fn map_text_style(style: &Value) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(size) = style["size_permille"].as_f64() {
        params.insert(
            "fontSize".into(),
            json!((size * 90.0 / 1000.0 * 100.0).round() / 100.0),
        );
    }
    if let Some(weight) = style["weight"].as_i64() {
        params.insert(
            "fontWeight".into(),
            json!(if weight >= 600 { "bold" } else { "normal" }),
        );
    }
    if let Some(color) = style["color"].as_str() {
        params.insert("color".into(), json!(color));
    }
    if let Some(background) = style["background"].as_str() {
        params.insert("background.enabled".into(), json!(true));
        params.insert("background.color".into(), json!(background));
    }
    Value::Object(params)
}

/// Como cada comando do vocabulário legado é tratado pela porta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Partial(&'static str),
    Unsupported(&'static str),
}

/// Matriz de suporte (única fonte da verdade; testada contra o que o Editor Agent emite).
pub fn command_support(kind: &str) -> Support {
    match kind {
        "add_track" | "split_clip" | "move_clips" | "delete_clip" | "trim_clip"
        | "set_clip_enabled" => Support::Supported,
        "insert_clip" => {
            Support::Partial("media/image/text; solid, generated e nested não têm equivalente")
        }
        "set_property" => Support::Partial(
            "opacity, scale, position_x/y, rotation, volume_db; fade_in/fade_out exigem keyframes",
        ),
        "set_transition" => Support::Unsupported("o editor OpenCut não tem transições de clip"),
        "create_sequence" | "insert_nested" => {
            Support::Unsupported("multi-sequence (cenas) fora da AgentSurface v1.x")
        }
        _ => Support::Unsupported("comando desconhecido"),
    }
}

pub fn translate(commands: &[Value], ctx: &TranslateCtx) -> Result<Translated, PortError> {
    let mut out = Translated::default();
    let mut track_roles: HashMap<String, String> = HashMap::new();
    // clips criados por este mesmo plano (alvo por `clipRef`, não por id do editor)
    let mut created: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (index, cmd) in commands.iter().enumerate() {
        let kind = cmd["type"].as_str().unwrap_or("");
        let op_id = cmd["operation_id"]
            .as_str()
            .map_or_else(|| format!("op{index}"), str::to_owned);
        match kind {
            "add_track" => {
                let id = cmd["id"].as_str().unwrap_or("").to_owned();
                let role = cmd["role"]
                    .as_str()
                    .unwrap_or(cmd["kind"].as_str().unwrap_or(""))
                    .to_owned();
                track_roles.insert(id, role);
                out.bookkeeping.push(op_id);
            }
            "insert_clip" => {
                let track = cmd["track"].as_str().unwrap_or("");
                let role = track_roles.get(track).map_or("main", String::as_str);
                let clip = &cmd["clip"];
                let content = &clip["content"];
                if content["type"].as_str() == Some("text") {
                    let clip_ref = clip["id"].as_str().unwrap_or(&op_id).to_owned();
                    let insert = json!({
                        "type": "insert_text", "operationId": op_id,
                        "text": content["text"].as_str().unwrap_or(""),
                        "startTicks": to_editor_ticks(int(&cmd["start"], "start")?)?,
                        "durationTicks": to_editor_ticks(int(&clip["duration"], "clip.duration")?)?,
                        "ref": clip_ref,
                        "params": map_text_style(&content["style"]),
                    });
                    created.insert(clip_ref);
                    out.operations.push(insert);
                    continue;
                }
                let asset = match content["type"].as_str() {
                    Some("media" | "image") => content["asset"].as_str().ok_or_else(|| {
                        PortError::new("INVALID_COMMAND", "insert_clip content needs `asset`")
                    })?,
                    other => {
                        return Err(unsupported(
                            "insert_clip",
                            &format!(
                                "content type {other:?} has no AgentSurface v1 equivalent (solid/text/generated)"
                            ),
                        ));
                    }
                };
                let clip_ref = clip["id"].as_str().unwrap_or(&op_id).to_owned();
                let start = to_editor_ticks(int(&cmd["start"], "start")?)?;
                let duration = to_editor_ticks(int(&clip["duration"], "clip.duration")?)?;
                let source_in = to_editor_ticks(clip["source_in"].as_i64().unwrap_or(0))?;
                let mut insert = json!({
                    "type": "insert_clip", "operationId": op_id, "mediaId": asset,
                    "startTicks": start, "ref": clip_ref,
                });
                match role {
                    "main" => {
                        let main = ctx.main_track_id.as_ref().ok_or_else(|| {
                            PortError::new(
                                "INVALID_STATE",
                                "main track id unknown (inspect_tracks)",
                            )
                        })?;
                        insert["trackId"] = json!(main);
                    }
                    "voice" | "audio" | "music" => {}
                    other => {
                        return Err(unsupported(
                            "insert_clip",
                            &format!(
                                "track role `{other}` needs explicit track targeting (AgentSurface v2)"
                            ),
                        ));
                    }
                }
                created.insert(clip_ref.clone());
                out.operations.push(insert);
                let full = ctx.assets.get(asset).and_then(|a| a.duration_editor_ticks);
                let mut trim = json!({
                    "type": "trim_clip", "operationId": format!("{op_id}:trim"),
                    "clipRef": clip_ref, "durationTicks": duration,
                });
                if let Some(full) = full {
                    trim["trimStartTicks"] = json!(source_in);
                    trim["trimEndTicks"] = json!((full - source_in - duration).max(0));
                }
                if full != Some(duration) || source_in != 0 {
                    out.operations.push(trim);
                }
            }
            "split_clip" => {
                let clip = cmd["clip"].as_str().unwrap_or("");
                let mut op = json!({
                    "type": "split_clip", "operationId": op_id, "clipId": clip,
                    "atTicks": to_editor_ticks(int(&cmd["at"], "at")?)?,
                });
                if let Some(new_id) = cmd["new_id"].as_str() {
                    op["rightRef"] = json!(new_id);
                }
                out.operations.push(op);
            }
            "move_clips" => {
                for (i, m) in cmd["moves"].as_array().into_iter().flatten().enumerate() {
                    let mut op = json!({
                        "type": "move_clip", "operationId": format!("{op_id}:{i}"),
                        "clipId": m["clip"].as_str().unwrap_or(""),
                        "newStartTicks": to_editor_ticks(int(&m["start"], "moves[].start")?)?,
                    });
                    if let Some(track) = m["track"].as_str() {
                        op["targetTrackId"] = json!(track);
                    }
                    out.operations.push(op);
                }
            }
            "delete_clip" => {
                out.operations.push(json!({
                    "type": "delete_clip", "operationId": op_id,
                    "clipId": cmd["clip"].as_str().unwrap_or(""),
                }));
            }
            "trim_clip" => {
                let clip = cmd["clip"].as_str().unwrap_or("");
                let facts = ctx.clips.get(clip).ok_or_else(|| {
                    PortError::new("NOT_FOUND", format!("clip `{clip}` is not on the timeline"))
                })?;
                let to = to_editor_ticks(int(&cmd["to"], "to")?)?;
                let end = facts.start + facts.duration;
                let mut op = json!({"type": "trim_clip", "operationId": op_id, "clipId": clip});
                match cmd["edge"].as_str() {
                    Some("in") => {
                        let delta = to - facts.start;
                        op["startTicks"] = json!(to);
                        op["durationTicks"] = json!(end - to);
                        op["trimStartTicks"] = json!((facts.trim_start + delta).max(0));
                    }
                    Some("out") => {
                        op["durationTicks"] = json!(to - facts.start);
                        op["trimEndTicks"] = json!((facts.trim_end + (end - to)).max(0));
                    }
                    other => {
                        return Err(PortError::new(
                            "INVALID_COMMAND",
                            format!("trim_clip edge must be `in` or `out`, got {other:?}"),
                        ));
                    }
                }
                out.operations.push(op);
            }
            "set_property" => {
                let clip = cmd["clip"].as_str().unwrap_or("");
                let params: serde_json::Map<String, Value> =
                    map_property(cmd["prop"].as_str().unwrap_or(""), &cmd["value"])?
                        .into_iter()
                        .map(|(k, v)| (k.to_owned(), v))
                        .collect();
                let mut op = json!({
                    "type": "set_clip_params", "operationId": op_id, "params": params,
                });
                if created.contains(clip) {
                    op["clipRef"] = json!(clip);
                } else {
                    op["clipId"] = json!(clip);
                }
                out.operations.push(op);
            }
            "set_clip_enabled" => {
                let clip = cmd["clip"].as_str().unwrap_or("");
                let mut op = json!({
                    "type": "set_clip_enabled", "operationId": op_id,
                    "enabled": cmd["enabled"].as_bool().unwrap_or(true),
                });
                if created.contains(clip) {
                    op["clipRef"] = json!(clip);
                } else {
                    op["clipId"] = json!(clip);
                }
                out.operations.push(op);
            }
            other => {
                let why = match command_support(other) {
                    Support::Unsupported(why) | Support::Partial(why) => why,
                    Support::Supported => "suportado, mas sem tradução (bug da porta)",
                };
                return Err(unsupported(other, why));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn ctx() -> TranslateCtx {
        let mut assets = HashMap::new();
        assets.insert(
            "a1".to_owned(),
            AssetFacts {
                duration_editor_ticks: Some(10 * 120_000),
            },
        );
        TranslateCtx {
            clips: HashMap::new(),
            main_track_id: Some("main-id".into()),
            assets,
        }
    }

    #[test]
    fn translates_track_and_clip_with_trim_in_editor_ticks() {
        let cmds = vec![
            json!({"type":"add_track","id":"t1","role":"main","operation_id":"o0"}),
            json!({"type":"insert_clip","track":"t1","start":705_600_000,
                   "clip":{"id":"c1","duration":2_i64*705_600_000,"source_in":705_600_000,
                           "content":{"type":"media","asset":"a1"}},"operation_id":"o1"}),
        ];
        let t = translate(&cmds, &ctx()).unwrap();
        assert_eq!(t.bookkeeping, vec!["o0"]);
        assert_eq!(t.operations[0]["startTicks"], 120_000);
        assert_eq!(t.operations[0]["trackId"], "main-id");
        assert_eq!(t.operations[1]["trimStartTicks"], 120_000);
        assert_eq!(t.operations[1]["durationTicks"], 240_000);
        assert_eq!(
            t.operations[1]["trimEndTicks"],
            10 * 120_000 - 120_000 - 240_000
        );
    }

    #[test]
    fn edits_existing_clips_by_editor_id() {
        let mut c = ctx();
        c.clips.insert(
            "k1".into(),
            ClipFacts {
                start: 120_000,
                duration: 480_000,
                trim_start: 0,
                trim_end: 0,
            },
        );
        let cmds = vec![
            json!({"type":"split_clip","clip":"k1","at":2_i64*705_600_000,"operation_id":"s"}),
            json!({"type":"move_clips","moves":[{"clip":"k1","start":3_i64*705_600_000}],"operation_id":"m"}),
            json!({"type":"trim_clip","clip":"k1","edge":"out","to":4_i64*705_600_000,"operation_id":"t"}),
            json!({"type":"trim_clip","clip":"k1","edge":"in","to":2_i64*705_600_000,"operation_id":"ti"}),
            json!({"type":"delete_clip","clip":"k1","operation_id":"d"}),
        ];
        let t = translate(&cmds, &c).unwrap();
        assert_eq!(t.operations[0]["atTicks"], 240_000);
        assert_eq!(t.operations[1]["newStartTicks"], 360_000);
        // out: start 1s, end -> 4s => duration 3s, trimEnd grows by (old end 5s - 4s)
        assert_eq!(t.operations[2]["durationTicks"], 360_000);
        assert_eq!(t.operations[2]["trimEndTicks"], 120_000);
        // in: new start 2s => duration (5s-2s), trimStart += 1s
        assert_eq!(t.operations[3]["startTicks"], 240_000);
        assert_eq!(t.operations[3]["durationTicks"], 360_000);
        assert_eq!(t.operations[3]["trimStartTicks"], 120_000);
        assert_eq!(t.operations[4]["type"], "delete_clip");
        let missing = translate(
            &[json!({"type":"trim_clip","clip":"zz","edge":"out","to":0})],
            &c,
        )
        .unwrap_err();
        assert_eq!(missing.code, "NOT_FOUND");
    }

    #[test]
    fn unknown_commands_fail_loudly() {
        let e = translate(
            &[json!({"type":"set_transition","clip":"c","transition":{"kind":"fade","duration":1}})],
            &ctx(),
        )
        .unwrap_err();
        assert_eq!(e.code, "UNSUPPORTED_COMMAND");
        let e = translate(
            &[json!({"type":"set_property","clip":"c","prop":"fade_in","value":0.5})],
            &ctx(),
        )
        .unwrap_err();
        assert_eq!(e.code, "UNSUPPORTED_COMMAND");
        let e = translate(&[json!({"type":"insert_clip","track":"t","start":0,"clip":{"id":"c","duration":1,"content":{"type":"solid"}}})], &ctx()).unwrap_err();
        assert_eq!(e.code, "UNSUPPORTED_COMMAND");
    }

    #[test]
    fn sub_tick_values_are_rejected_not_rounded() {
        let e = translate(&[json!({"type":"insert_clip","track":"t","start":1,"clip":{"id":"c","duration":5880,"content":{"type":"media","asset":"a1"}}})], &ctx()).unwrap_err();
        assert_eq!(e.code, "TIMEBASE_INEXACT");
    }

    #[test]
    fn text_clips_and_properties_translate_with_refs_and_units() {
        let cmds = vec![
            json!({"type":"add_track","id":"tx","role":"text","operation_id":"o0"}),
            json!({"type":"insert_clip","track":"tx","start":705_600_000,"operation_id":"o1",
                   "clip":{"id":"txt1","duration":2_i64*705_600_000,
                           "content":{"type":"text","text":"OFERTA",
                                      "style":{"size_permille":55,"weight":800,"color":"#FFFFFF",
                                               "background":"#000000B3"}}}}),
            json!({"type":"set_property","clip":"txt1","prop":"position_y","value":315.0,"operation_id":"o2"}),
            json!({"type":"set_property","clip":"existing","prop":"scale","value":1.2,"operation_id":"o3"}),
            json!({"type":"set_clip_enabled","clip":"existing","enabled":false,"operation_id":"o4"}),
        ];
        let t = translate(&cmds, &ctx()).unwrap();
        let ops = &t.operations;
        assert_eq!(ops[0]["type"], "insert_text");
        assert_eq!(ops[0]["text"], "OFERTA");
        assert_eq!(ops[0]["startTicks"], 120_000);
        assert_eq!(ops[0]["durationTicks"], 240_000);
        assert_eq!(ops[0]["params"]["fontSize"], 4.95);
        assert_eq!(ops[0]["params"]["fontWeight"], "bold");
        assert_eq!(ops[0]["params"]["background.enabled"], true);
        // clip criado no mesmo plano ⇒ clipRef; clip existente ⇒ clipId
        assert_eq!(ops[1]["type"], "set_clip_params");
        assert_eq!(ops[1]["clipRef"], "txt1");
        assert_eq!(ops[1]["params"]["transform.positionY"], 315.0);
        assert_eq!(ops[2]["clipId"], "existing");
        assert_eq!(ops[2]["params"]["transform.scaleX"], 1.2);
        assert_eq!(ops[2]["params"]["transform.scaleY"], 1.2);
        assert_eq!(ops[3]["type"], "set_clip_enabled");
        assert_eq!(ops[3]["enabled"], false);
    }

    #[test]
    fn every_command_the_editor_agent_emits_is_classified() {
        // Vocabulário emitido por `compile_edit_plan` (capia-intelligence/autonomy/plan.rs).
        for kind in [
            "create_sequence",
            "add_track",
            "insert_clip",
            "set_property",
            "set_transition",
            "insert_nested",
            "split_clip",
            "delete_clip",
            "trim_clip",
            "move_clips",
            "set_clip_enabled",
        ] {
            assert_ne!(
                command_support(kind),
                Support::Unsupported("comando desconhecido"),
                "`{kind}` não está classificado"
            );
        }
        assert_eq!(
            command_support("set_transition"),
            Support::Unsupported("o editor OpenCut não tem transições de clip")
        );
    }
}
