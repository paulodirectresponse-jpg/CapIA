//! CLI mínima do EnginePort para o fluxo humano + CapIA (E2E): fala com a ponte HTTP loopback da
//! página viva do editor. Saída: uma linha de JSON por comando.
//!
//! `port_cli <host:port> inspect`
//! `port_cli <host:port> apply <label> '<comandos legados em JSON>'`
//! `port_cli <host:port> history`

use capia_actor::Actor;
use capia_engine_port::{AgentSurfaceEngine, HttpTransport};
use capia_intelligence::engine::Engine;
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(addr), Some(cmd)) = (args.first(), args.get(1)) else {
        eprintln!("usage: port_cli <host:port> inspect|history|apply <label> <json>");
        std::process::exit(2);
    };
    let transport = match HttpTransport::new(addr.clone()) {
        Ok(t) => t,
        Err(e) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": e.code, "message": e.message}})
            );
            std::process::exit(1);
        }
    };
    let engine = AgentSurfaceEngine::new(transport);
    let actor = Actor::agent("run:first-flow");
    let out: Result<Value, _> = match cmd.as_str() {
        "inspect" => engine
            .read("sequence.get", json!({}))
            .and_then(|seq| Ok(json!({"revision": engine.revision()?, "sequence": seq}))),
        "history" => engine.read("history.list", json!({})),
        "apply" => {
            let label = args.get(2).cloned().unwrap_or_default();
            let commands: Value =
                serde_json::from_str(args.get(3).map_or("[]", String::as_str)).unwrap_or(json!([]));
            engine.preview(&actor, &label, commands).and_then(|pv| {
                let token = pv["plan_token"].as_str().unwrap_or_default().to_owned();
                let reply = engine.apply(&actor, &token)?;
                Ok(json!({"op_count": pv["op_count"], "diff": pv["diff"], "apply": reply}))
            })
        }
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    };
    match out {
        Ok(v) => println!("{}", json!({"ok": true, "result": v})),
        Err(e) => {
            println!(
                "{}",
                json!({"ok": false, "error": {"code": e.code, "message": e.message}})
            );
            std::process::exit(1);
        }
    }
}
