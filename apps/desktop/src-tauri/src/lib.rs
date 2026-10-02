//! Shell desktop do CapIA. **Adaptador:** traduz IPC do Tauri para a fachada `capia-project`.
//! Regra (ADR-002): nenhuma lógica de edição, IA ou persistência mora aqui; o Tauri é substituível.

use capia_project::EngineInfo;

/// Comando IPC: descreve o engine em execução (contrato em `@capia/engine-bindings`).
#[tauri::command]
fn get_engine_info() -> EngineInfo {
    capia_project::engine_info()
}

/// Inicia a aplicação desktop.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_engine_info])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o CapIA desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_command_delegates_to_the_project_facade() {
        assert_eq!(get_engine_info(), capia_project::engine_info());
    }
}
