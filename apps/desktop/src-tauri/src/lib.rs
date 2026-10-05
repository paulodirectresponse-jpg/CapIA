//! Shell desktop do CapIA. **Adaptador:** traduz IPC do Tauri para a fachada `capia-editor-api`.
//! Regra (ADR-002): nenhuma lógica de edição, IA ou persistência mora aqui; o Tauri é substituível.
//!
//! A superfície IPC é pequena e fixa (SECURITY.md §7): `editor_call` (JSON) e `editor_call_binary`
//! (quadros/miniaturas). Quais métodos existem é decidido por `capia_editor_api::Session::call` —
//! método desconhecido devolve `UNKNOWN_METHOD`; não há `eval`, shell nem acesso amplo a arquivos.

use capia_editor_api::{Outcome, Reply, Session, SessionConfig};
use capia_intelligence::{IntelligenceService, ServiceConfig, SessionEngine};
use capia_project::EngineInfo;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// Comando IPC legado do scaffold (contrato em `@capia/engine-bindings`).
#[tauri::command]
fn get_engine_info() -> EngineInfo {
    capia_project::engine_info()
}

/// Sessão do editor compartilhada pelos comandos IPC (uma por processo/janela).
#[derive(Debug)]
pub struct EditorState {
    session: Arc<Mutex<Session>>,
    /// Superfície P2 (SharedBuffer do WebView2), criada sob demanda pelo front.
    surface: Arc<Mutex<Option<Held>>>,
    seq: AtomicU32,
}

/// O que mantém a superfície viva: no Windows o SharedBuffer; nos demais sistemas nada (o front
/// usa o caminho por IPC binário).
#[cfg(windows)]
type Held = capia_webview_surface::SharedSurface;
#[cfg(not(windows))]
type Held = ();

#[cfg(windows)]
fn held_region(h: &Held) -> &capia_webview_surface::FrameRegion {
    h.region()
}

/// Serviço `ai.*` da Fase 4. Opcional por desenho: o editor funciona igual sem ele.
#[derive(Debug)]
pub struct AiState {
    svc: Arc<IntelligenceService>,
}

impl AiState {
    /// Credenciais: Credential Manager do Windows; sem cofre seguro, memória (não persiste — o
    /// usuário recadastra a chave; a UI mostra o backend em uso). Registry: AppDb do app.
    pub fn new(session: Arc<Mutex<Session>>, appdb: Option<PathBuf>) -> Result<Self, String> {
        let secrets: Arc<dyn capia_secrets::SecretStore> = capia_secrets::platform_store()
            .unwrap_or_else(|_| Arc::new(capia_secrets::MemoryStore::new()));
        let engine = Arc::new(SessionEngine::new(session));
        let svc = IntelligenceService::new(
            engine,
            ServiceConfig {
                appdb_path: appdb,
                secrets,
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { svc: Arc::new(svc) })
    }
}

impl EditorState {
    pub fn new(cfg: SessionConfig) -> Self {
        Self {
            session: Arc::new(Mutex::new(Session::new(cfg))),
            surface: Arc::new(Mutex::new(None)),
            seq: AtomicU32::new(0),
        }
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new(SessionConfig::default())
    }
}

fn run_call(session: &Mutex<Session>, method: &str, params: Value) -> Result<Reply, Value> {
    // fase 1 sob o lock; quadros (compositor/decode) rodam fora dele
    let begun = match session.lock() {
        Ok(mut s) => s.begin(method, params),
        Err(_) => {
            return Err(json!({"code": "POISONED", "message": "editor session lock poisoned"}));
        }
    };
    match begun.map_err(|e| e.to_json())? {
        Outcome::Done(r) => Ok(r),
        Outcome::Later(job) => job.run().map_err(|e| e.to_json()),
    }
}

/// Como [`call_json`], roteando `ai.*` ao serviço de inteligência e acrescentando os eventos de IA
/// ao `events.poll` (a UI mantém um único laço de poll). Sem serviço, `ai.*` é `UNKNOWN_METHOD`.
pub fn call_json_ai(
    session: &Mutex<Session>,
    ai: Option<&IntelligenceService>,
    method: &str,
    params: Value,
) -> Result<Value, Value> {
    if IntelligenceService::handles(method) {
        return match ai {
            Some(svc) => svc.call_json(method, params),
            None => {
                Err(json!({"code": "UNKNOWN_METHOD", "message": "AI service is not available"}))
            }
        };
    }
    let mut v = call_json(session, method, params)?;
    if method == "events.poll"
        && let Some(svc) = ai
    {
        svc.merge_events(&mut v);
    }
    Ok(v)
}

/// Resposta JSON; um método que devolve bytes deve usar `editor_call_binary`.
pub fn call_json(session: &Mutex<Session>, method: &str, params: Value) -> Result<Value, Value> {
    match run_call(session, method, params)? {
        Reply::Json(v) => Ok(v),
        Reply::Binary { .. } => Err(json!({
            "code": "BINARY_REPLY",
            "message": format!("`{method}` returns bytes: use editor_call_binary"),
        })),
    }
}

/// Empacota bytes + metadados num único buffer: `u32 LE tamanho do JSON` · JSON · bytes. O front
/// separa sem cópia extra (`DataView` + `subarray`).
pub fn pack_binary(mime: &str, meta: &Value, bytes: &[u8]) -> Vec<u8> {
    let header = json!({ "mime": mime, "meta": meta })
        .to_string()
        .into_bytes();
    let len = u32::try_from(header.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(4 + header.len() + bytes.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(bytes);
    out
}

pub fn call_binary(
    session: &Mutex<Session>,
    method: &str,
    params: Value,
) -> Result<Vec<u8>, Value> {
    match run_call(session, method, params)? {
        Reply::Binary { mime, bytes, meta } => Ok(pack_binary(mime, &meta, &bytes)),
        Reply::Json(_) => Err(json!({
            "code": "JSON_REPLY",
            "message": format!("`{method}` returns JSON: use editor_call"),
        })),
    }
}

#[tauri::command]
async fn editor_call(
    state: tauri::State<'_, EditorState>,
    ai: tauri::State<'_, AiState>,
    method: String,
    params: Option<Value>,
) -> Result<Value, Value> {
    let session = Arc::clone(&state.session);
    let svc = Arc::clone(&ai.svc);
    tauri::async_runtime::spawn_blocking(move || {
        call_json_ai(
            &session,
            Some(&svc),
            &method,
            params.unwrap_or_else(|| json!({})),
        )
    })
    .await
    .map_err(|e| json!({"code": "JOIN", "message": e.to_string()}))?
}

#[tauri::command]
async fn editor_call_binary(
    state: tauri::State<'_, EditorState>,
    method: String,
    params: Option<Value>,
) -> Result<tauri::ipc::Response, Value> {
    let session = Arc::clone(&state.session);
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        call_binary(&session, &method, params.unwrap_or_else(|| json!({})))
    })
    .await
    .map_err(|e| json!({"code": "JOIN", "message": e.to_string()}))??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Maior quadro do preview (720p em vertical/horizontal cabe em 1280×1280).
#[cfg_attr(not(windows), allow(dead_code))]
const SURFACE_MAX_SIDE: u32 = 1280;

/// Cria o SharedBuffer P2 e o posta à página. O front confirma o recebimento
/// (`sharedbufferreceived`) antes de usar; qualquer falha devolve `UNSUPPORTED`/`SURFACE` e o
/// front segue pelo IPC binário (nunca fica sem preview).
#[tauri::command]
async fn preview_surface_init(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, EditorState>,
) -> Result<Value, Value> {
    #[cfg(windows)]
    {
        let size = capia_webview_surface::HEADER_BYTES
            + (SURFACE_MAX_SIDE as usize) * (SURFACE_MAX_SIDE as usize) * 4;
        let meta = json!({"kind": "capia-preview", "max_side": SURFACE_MAX_SIDE}).to_string();
        let surface = capia_webview_surface::SharedSurface::create(&window, size, &meta)
            .map_err(|e| json!({"code": "SURFACE", "message": e.to_string()}))?;
        if let Ok(mut slot) = state.surface.lock() {
            *slot = Some(surface);
        }
        Ok(json!({"bytes": size, "header": capia_webview_surface::HEADER_BYTES}))
    }
    #[cfg(not(windows))]
    {
        let _ = (&window, &state);
        Err(
            json!({"code": "UNSUPPORTED", "message": "SharedBuffer is a WebView2 (Windows) feature"}),
        )
    }
}

/// Renderiza o quadro **dentro** do SharedBuffer (sem bytes no IPC): o front só recebe o cabeçalho.
#[tauri::command]
async fn preview_render_shared(
    state: tauri::State<'_, EditorState>,
    sequence: String,
    at: i64,
    width: u32,
    height: u32,
) -> Result<Value, Value> {
    let session = Arc::clone(&state.session);
    let surface = Arc::clone(&state.surface);
    let seq = state.seq.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    tauri::async_runtime::spawn_blocking(move || {
        render_into_surface(&session, &surface, seq, &sequence, at, width, height)
    })
    .await
    .map_err(|e| json!({"code": "JOIN", "message": e.to_string()}))?
}

fn render_into_surface(
    session: &Mutex<Session>,
    surface: &Mutex<Option<Held>>,
    seq: u32,
    sequence: &str,
    at: i64,
    width: u32,
    height: u32,
) -> Result<Value, Value> {
    let reply = run_call(
        session,
        "render.frame",
        json!({"sequence": sequence, "at": at, "width": width, "height": height}),
    )?;
    let Reply::Binary { bytes, meta, .. } = reply else {
        return Err(json!({"code": "JSON_REPLY", "message": "render.frame must return bytes"}));
    };
    let guard = surface
        .lock()
        .map_err(|_| json!({"code": "POISONED", "message": "surface lock poisoned"}))?;
    #[cfg(windows)]
    {
        let held = guard
            .as_ref()
            .ok_or_else(|| json!({"code": "SURFACE", "message": "surface not initialised"}))?;
        let (w, h) = (
            meta["width"].as_u64().unwrap_or(0) as u32,
            meta["height"].as_u64().unwrap_or(0) as u32,
        );
        held_region(held)
            .write_frame(seq, w, h, &bytes)
            .map_err(|e| json!({"code": "SURFACE", "message": e.to_string()}))?;
        Ok(json!({"seq": seq, "width": w, "height": h, "warnings": meta["warnings"]}))
    }
    #[cfg(not(windows))]
    {
        let _ = (&guard, &bytes, &meta, seq);
        Err(
            json!({"code": "UNSUPPORTED", "message": "SharedBuffer is a WebView2 (Windows) feature"}),
        )
    }
}

/// Inicia a aplicação desktop.
pub fn run() {
    // saída de crash sem segredo (SECURITY.md): a mensagem de pânico é redigida
    capia_secrets::install_redacting_panic_hook();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(EditorState::default())
        .setup(|app| {
            // dados do app (registry/perfis; nunca segredos) ficam no diretório de dados do usuário
            let appdb = app
                .path()
                .app_data_dir()
                .ok()
                .map(|d| d.join("capia-app.db"));
            let session = Arc::clone(&app.state::<EditorState>().session);
            let state = AiState::new(Arc::clone(&session), appdb)
                .or_else(|_| AiState::new(session, None))
                .map_err(|e| format!("could not start the AI service: {e}"))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_info,
            editor_call,
            editor_call_binary,
            preview_surface_init,
            preview_render_shared
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o CapIA desktop");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn ipc_command_delegates_to_the_project_facade() {
        assert_eq!(get_engine_info(), capia_project::engine_info());
    }

    fn session() -> Mutex<Session> {
        Mutex::new(Session::new(SessionConfig::default()))
    }

    #[test]
    fn json_calls_reach_the_editor_api_and_unknown_methods_are_structured_errors() {
        let s = session();
        let v = call_json(&s, "engine.info", json!({})).unwrap();
        assert!(v.get("media_available").is_some());
        let e = call_json(&s, "shell.exec", json!({"cmd": "calc"})).unwrap_err();
        assert_eq!(e["code"], "UNKNOWN_METHOD");
    }

    #[test]
    fn ai_methods_are_routed_to_the_service_and_absent_without_it() {
        let s = Arc::new(session());
        // sem serviço: a IA não existe, o editor segue igual
        let e = call_json_ai(&s, None, "ai.status", json!({})).unwrap_err();
        assert_eq!(e["code"], "UNKNOWN_METHOD");
        assert!(call_json_ai(&s, None, "engine.info", json!({})).is_ok());
        // com serviço (cofre em memória): `ai.status` responde e nunca traz segredo
        let ai = AiState::new(Arc::clone(&s), None).unwrap();
        let st = call_json_ai(&s, Some(&ai.svc), "ai.status", json!({})).unwrap();
        assert_eq!(st["any_usable_model"], false);
        assert!(st["providers"].as_array().unwrap().is_empty());
        // eventos de IA entram no `events.poll` do editor (um único laço)
        let ev = call_json_ai(&s, Some(&ai.svc), "events.poll", json!({})).unwrap();
        assert!(ev["events"].is_array());
        // método inexistente do namespace ai.* é erro estruturado
        let e = call_json_ai(&s, Some(&ai.svc), "ai.shell", json!({})).unwrap_err();
        assert_eq!(e["code"], "UNKNOWN_METHOD");
    }

    #[test]
    fn binary_packing_round_trips() {
        let packed = pack_binary("application/x-rgba", &json!({"width": 2}), &[1, 2, 3]);
        let n = u32::from_le_bytes(packed[..4].try_into().unwrap()) as usize;
        let header: Value = serde_json::from_slice(&packed[4..4 + n]).unwrap();
        assert_eq!(header["mime"], "application/x-rgba");
        assert_eq!(header["meta"]["width"], 2);
        assert_eq!(&packed[4 + n..], &[1, 2, 3]);
    }

    #[test]
    fn json_method_through_the_binary_path_is_refused() {
        let s = session();
        let e = call_binary(&s, "engine.info", json!({})).unwrap_err();
        assert_eq!(e["code"], "JSON_REPLY");
    }
}
