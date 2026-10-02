// Evita a janela de console extra no Windows em release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    capia_desktop_lib::run();
}
