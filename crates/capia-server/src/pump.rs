//! Bomba de eventos: **única** consumidora de `events.poll` do servidor. Finaliza imports prontos
//! (o `pump` do engine roda aí), converte os eventos de export em linhas de `exports` + eventos do
//! servidor, e deduz as transições das Runs por diferença de estado. Cada evento tem um `event_id`
//! determinístico: reprocessar (ou reiniciar) nunca cria um evento duplicado.

use crate::auth::now_ms;
use crate::core::Core;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Debug)]
pub struct Pump {
    last_status: HashMap<String, String>,
    seeded: bool,
}

impl Pump {
    pub fn new() -> Self {
        Self {
            last_status: HashMap::new(),
            seeded: false,
        }
    }
}

impl Default for Pump {
    fn default() -> Self {
        Self::new()
    }
}

fn terminal(s: &str) -> bool {
    matches!(s, "completed" | "failed" | "cancelled")
}

impl Core {
    /// Um ciclo da bomba (público para os testes chamarem de forma determinística).
    pub fn pump_once(&self, st: &mut Pump) {
        self.pump_session_events();
        // o serviço de IA acumula eventos de progresso/streaming: ninguém os lê aqui, então são
        // descartados (o estado da Run é a fonte de verdade) em vez de crescer sem limite
        let _ = self.ai.poll_events();
        self.pump_runs(st);
    }

    fn pump_session_events(&self) {
        let events = match self.session_call("events.poll", json!({})) {
            Ok(v) => v["events"].as_array().cloned().unwrap_or_default(),
            Err(_) => return,
        };
        let project = self.open_project_id();
        for ev in events {
            match ev["kind"].as_str().unwrap_or_default() {
                "export_item_started" => {
                    if let Some(id) = ev["id"].as_str() {
                        let _ = self.db.export_update(id, "running", None, None, now_ms());
                    }
                }
                "export_item_finished" => self.on_export_finished(&ev, project.as_deref()),
                "import_finalized" => {
                    let ticket = ev["ticket_id"].as_str().unwrap_or_default();
                    let asset = ev["result"]["asset_id"]
                        .as_str()
                        .or(ev["asset_id"].as_str());
                    self.emit(
                        &format!("asset.imported.{ticket}"),
                        "asset.imported",
                        project.as_deref(),
                        None,
                        None,
                        &json!({"ticket_id": ticket, "asset_id": asset, "outcome": ev["result"]["outcome"]}),
                    );
                }
                _ => {}
            }
        }
    }

    fn on_export_finished(&self, ev: &Value, project: Option<&str>) {
        let Some(id) = ev["id"].as_str() else { return };
        let ok = ev["ok"].as_bool().unwrap_or(false);
        let cancelled = ev["cancelled"].as_bool().unwrap_or(false);
        let state = if ok {
            "completed"
        } else if cancelled {
            "cancelled"
        } else {
            "failed"
        };
        let report = ok.then(|| ev["report"].clone());
        let error = (!ok).then(|| ev["error"].clone()).filter(|e| !e.is_null());
        let _ = self
            .db
            .export_update(id, state, report.as_ref(), error.as_ref(), now_ms());
        let pid = project
            .map(str::to_owned)
            .or_else(|| self.db.export_get(id).ok().flatten().map(|e| e.project_id));
        let kind = if ok {
            "export.completed"
        } else {
            "export.failed"
        };
        let mut data = json!({"export_id": id, "state": state});
        if let Some(r) = report {
            data["report"] = r;
        }
        if let Some(e) = error {
            data["error"] = e;
        }
        if cancelled {
            data["cancelled"] = json!(true);
        }
        self.emit(
            &format!("{kind}.{id}"),
            kind,
            pid.as_deref(),
            None,
            Some(id),
            &data,
        );
    }

    fn pump_runs(&self, st: &mut Pump) {
        let Some(project) = self.open_project_id() else {
            st.seeded = false;
            st.last_status.clear();
            return;
        };
        let Ok(v) = self.ai_call("ai.run.list", json!({"limit": 200})) else {
            return;
        };
        let runs = v["runs"].as_array().cloned().unwrap_or_default();
        for r in &runs {
            let (Some(id), Some(status)) = (r["id"].as_str(), r["status"].as_str()) else {
                continue;
            };
            let prev = st.last_status.get(id).cloned();
            if prev.as_deref() == Some(status) {
                continue;
            }
            st.last_status.insert(id.to_owned(), status.to_owned());
            // a primeira observação de uma Run já terminada (reinício) não é um evento novo,
            // mas o `event_id` determinístico tornaria o replay inofensivo de qualquer forma
            if prev.is_none() && !st.seeded && terminal(status) {
                continue;
            }
            let rev = r["revision"].as_u64().unwrap_or(0);
            let data = json!({"run": r});
            if prev.is_none() && !terminal(status) && status != "waiting_user" {
                self.emit(
                    &format!("run.started.{id}"),
                    "run.started",
                    Some(&project),
                    Some(id),
                    None,
                    &data,
                );
            }
            let kind = match status {
                "waiting_user" => Some("run.waiting_user"),
                "completed" => Some("run.completed"),
                "failed" => Some("run.failed"),
                "cancelled" => Some("run.cancelled"),
                _ => None,
            };
            if let Some(kind) = kind {
                self.emit(
                    &format!("{kind}.{id}.{rev}"),
                    kind,
                    Some(&project),
                    Some(id),
                    None,
                    &data,
                );
            }
        }
        st.seeded = true;
    }
}

/// Laço da bomba até o shutdown.
pub fn run_pump(core: &Arc<Core>) {
    let mut st = Pump::new();
    while !core.shutting_down.load(Ordering::SeqCst) {
        core.pump_once(&mut st);
        std::thread::sleep(Duration::from_millis(150));
    }
    // último ciclo: não perder o evento de uma Run que acabou de terminar
    core.pump_once(&mut st);
}
