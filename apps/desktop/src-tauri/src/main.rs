// Evita a janela de console extra no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--version` / `--self-test` são headless (sem janela, sem rede, sem IA) — usados pelo instalador
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = capia_desktop_lib::cli::handle(&args) {
        std::process::exit(code);
    }
    capia_desktop_lib::run();
}
