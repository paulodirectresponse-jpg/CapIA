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

#[derive(Clone, Debug, Default)]
pub struct TranslateCtx {
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

pub fn translate(commands: &[Value], ctx: &TranslateCtx) -> Result<Translated, PortError> {
    let mut out = Translated::default();
    let mut track_roles: HashMap<String, String> = HashMap::new();
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
            "split_clip" | "move_clip" | "trim_clip" | "delete_clip" => {
                return Err(unsupported(
                    kind,
                    "direct clip edits use legacy clip ids; map them with clipRef in a Phase 2B translator",
                ));
            }
            other => {
                return Err(unsupported(
                    other,
                    "no AgentSurface v1 equivalent (set_property, set_transition, text, captions, create_sequence…)",
                ));
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
            main_track_id: Some("main-id".into()),
            assets,
        }
    }

    #[test]
    fn translates_track_and_clip_with_trim_in_editor_ticks() {
        let cmds = vec![
            json!({"type":"add_track","id":"t1","role":"main","operation_id":"o0"}),
            json!({"type":"insert_clip","track":"t1","start":705_600_000,
                   "clip":{"id":"c1","duration":2*705_600_000,"source_in":705_600_000,
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
    fn unknown_commands_fail_loudly() {
        let e = translate(
            &[json!({"type":"set_property","clip":"c","prop":"scale","value":1.2})],
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
}
