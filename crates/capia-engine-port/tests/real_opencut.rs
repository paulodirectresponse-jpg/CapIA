//! EnginePort contra o domínio **real** do OpenCut (sem mocks): o Editor Agent compila um `EditPlan`
//! sintético, a `AgentSurfaceEngine` traduz e fala JSON-RPC com um sidecar Bun que roda a
//! `AgentSurface` sobre o `EditorCore` do editor (headless — os mesmos Commands da UI).
//!
//! Requer `bun` no PATH. `CAPIA_REQUIRE_BUN=1` (CI) transforma a ausência em falha.

#![allow(clippy::unwrap_used)]

use capia_actor::Actor;
use capia_engine_port::{AgentSurfaceEngine, StdioTransport, Transport};
use capia_intelligence::autonomy::plan::{CompileEnv, EditPlan, Inventory, compile_edit_plan};
use capia_intelligence::engine::Engine;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn editor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/editor")
}

fn bun_available() -> bool {
    Command::new("bun")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn open_host() -> Option<AgentSurfaceEngine<StdioTransport>> {
    if !bun_available() {
        assert!(
            std::env::var("CAPIA_REQUIRE_BUN").is_err(),
            "CAPIA_REQUIRE_BUN is set but `bun` is not on PATH"
        );
        eprintln!("SKIP: bun not available");
        return None;
    }
    let root = editor_root();
    let mut cmd = Command::new("bun");
    cmd.current_dir(&root).args([
        "--preload",
        "./apps/web/test-setup/wasm-esm-shim.ts",
        "--preload",
        "./apps/web/test-setup/canvas-shim.ts",
        "apps/web/src/agent-surface/bridge/stdio-host.ts",
    ]);
    let transport = StdioTransport::spawn(&mut cmd).expect("spawn bun host");
    transport
        .call(
            "host.open",
            json!({"assets": [
                {"id": "asset-a", "name": "a.webm", "type": "video", "durationSeconds": 10},
                {"id": "asset-b", "name": "b.webm", "type": "video", "durationSeconds": 8},
            ]}),
        )
        .expect("host.open");
    Some(AgentSurfaceEngine::new(transport))
}

fn edit_plan(target_sequence: &str) -> EditPlan {
    serde_json::from_value(json!({
        "schema_version": 1, "id": "ep1", "version": 1, "deliverable_key": "AD1",
        "target_sequence": target_sequence,
        "format": {"width": 1280, "height": 720, "fps": 30},
        "beats": [
            {"id": "hook", "role": "hook", "duration_ms": 2000,
             "asset": {"asset_id": "asset-a", "source_in_ms": 1000}},
            {"id": "body", "role": "body", "duration_ms": 3000,
             "asset": {"asset_id": "asset-b", "source_in_ms": 0}},
            {"id": "cta", "role": "cta", "duration_ms": 1500,
             "asset": {"asset_id": "asset-a", "source_in_ms": 5000}}
        ],
        "estimated_duration_ms": 6500
    }))
    .expect("edit plan")
}

fn compiled_commands(engine: &AgentSurfaceEngine<StdioTransport>) -> Vec<Value> {
    let inventory = Inventory::from_assets_list(&engine.read("assets.list", json!({})).unwrap());
    assert_eq!(inventory.assets.len(), 2);
    let snapshot = engine.read("project.snapshot", json!({})).unwrap();
    let sequence = snapshot["project"]["activeSequenceId"]
        .as_str()
        .unwrap()
        .to_owned();
    let resolutions = BTreeMap::new();
    let env = CompileEnv {
        run_id: "run1",
        ns: "p1",
        inventory: &inventory,
        resolutions: &resolutions,
    };
    compile_edit_plan(&env, &edit_plan(&sequence))
        .expect("compile")
        .commands
}

fn clips(engine: &AgentSurfaceEngine<StdioTransport>) -> Vec<Value> {
    let tracks = engine.read("sequence.get", json!({})).unwrap();
    tracks["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["clips"].as_array().unwrap().clone())
        .collect()
}

#[test]
fn editor_agent_plan_reaches_the_real_timeline_through_the_engine_port() {
    let Some(engine) = open_host() else { return };
    let commands = compiled_commands(&engine);
    let actor = Actor::agent("run:run1");

    // preview: nada é gravado
    let preview = engine
        .preview(&actor, "AI Run", Value::Array(commands))
        .expect("preview");
    assert!(
        preview["op_count"].as_u64().unwrap() >= 5,
        "3 inserts + trims"
    );
    assert_eq!(preview["diff"]["added"].as_array().unwrap().len(), 3);
    assert!(clips(&engine).is_empty(), "preview must not write");

    // apply: UMA entrada de histórico, clips na timeline do editor
    let revision_before = engine.revision().unwrap();
    let token = preview["plan_token"].as_str().unwrap();
    let reply = engine.apply(&actor, token).expect("apply");
    assert!(reply["revision"].as_u64().unwrap() > revision_before);
    let mut placed = clips(&engine);
    placed.sort_by_key(|c| c["startTicks"].as_i64().unwrap());
    let starts: Vec<i64> = placed
        .iter()
        .map(|c| c["startTicks"].as_i64().unwrap())
        .collect();
    let durations: Vec<i64> = placed
        .iter()
        .map(|c| c["durationTicks"].as_i64().unwrap())
        .collect();
    assert_eq!(starts, vec![0, 240_000, 600_000], "0s, 2s, 5s @120000 t/s");
    assert_eq!(
        durations,
        vec![240_000, 360_000, 180_000],
        "2.0s, 3.0s, 1.5s"
    );
    assert_eq!(placed[0]["trimStartTicks"], 120_000, "source_in 1s");
    let history = engine.read("history.list", json!({})).unwrap();
    assert_eq!(
        history.as_array().unwrap().len(),
        1,
        "one plan = one history entry"
    );
    assert_eq!(history[0]["actor"], "agent:run:run1");
}

#[test]
fn token_is_actor_bound_and_stale_plans_are_rejected() {
    let Some(engine) = open_host() else { return };
    let commands = compiled_commands(&engine);
    let actor = Actor::agent("run:run1");
    let preview = engine
        .preview(&actor, "AI Run", Value::Array(commands.clone()))
        .unwrap();
    let token = preview["plan_token"].as_str().unwrap().to_owned();
    let other = engine
        .apply(&Actor::agent("run:other"), &token)
        .unwrap_err();
    assert_eq!(other.code, "NOT_ALLOWED");

    // o documento muda entre preview e apply (outro ator): CAS de revisão derruba o plano
    let preview = engine
        .preview(&actor, "AI Run", Value::Array(commands))
        .unwrap();
    let token = preview["plan_token"].as_str().unwrap().to_owned();
    // outro ator escreve entre o preview e o apply (muda a revisão do documento)
    let other_plan = vec![
        json!({"type":"add_track","id":"t","role":"main","operation_id":"x0"}),
        json!({"type":"insert_clip","track":"t","start":0,"operation_id":"x1",
               "clip":{"id":"cx","duration":705_600_000,"source_in":0,"content":{"type":"media","asset":"asset-b"}}}),
    ];
    let p2 = engine
        .preview(
            &Actor::agent("run:other"),
            "other",
            Value::Array(other_plan),
        )
        .unwrap();
    engine
        .apply(
            &Actor::agent("run:other"),
            p2["plan_token"].as_str().unwrap(),
        )
        .unwrap();
    let stale = engine.apply(&actor, &token).unwrap_err();
    assert_eq!(stale.code, "STALE_REVISION");
    assert_eq!(
        clips(&engine).len(),
        1,
        "only the other actor's clip is there"
    );
}

#[test]
fn unsupported_legacy_commands_fail_loudly_and_write_nothing() {
    let Some(engine) = open_host() else { return };
    let actor = Actor::agent("run:run1");
    let e = engine
        .preview(
            &actor,
            "x",
            json!([{"type": "set_property", "clip": "c", "prop": "scale", "value": 1.2}]),
        )
        .unwrap_err();
    assert_eq!(e.code, "UNSUPPORTED_COMMAND");
    assert!(clips(&engine).is_empty());
    let e = engine.read("project.close", json!({})).unwrap_err();
    assert_eq!(
        e.code, "NOT_ALLOWED",
        "writes are never reachable through read()"
    );
}
