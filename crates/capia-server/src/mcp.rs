//! Servidor MCP (Trilha B): adaptador JSON-RPC 2.0 sobre `Core::call` — **não** é um segundo
//! backend. Tools = operações do catálogo (`runs.create` → `runs_create`), resources = leituras
//! `capia://…`. **Stub do contrato**: a implementação preenche `handle_rpc`/`serve_stdio`.

use crate::auth::Principal;
use crate::core::Core;
use serde_json::{Value, json};

/// Trata UMA mensagem JSON-RPC; `None` para notificações (sem resposta).
pub fn handle_rpc(_core: &Core, _principal: Option<&Principal>, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    Some(
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "MCP is not implemented yet"}}),
    )
}
