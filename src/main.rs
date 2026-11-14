// Pas de console noire derrière la fenêtre en version release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(debug_assertions)]
mod demo;
mod format;
mod history;
mod icon;
mod icons;
mod pdf;
mod report;
mod scanner;
mod theme;
mod tree;
mod treemap;
mod views;
mod widgets;
mod win;

use eframe::egui;

/// Usage : `DiskScan.exe [dossier] [--start]`
/// ex. `DiskScan.exe "C:\Users" --start` pré-remplit la cible et lance l'analyse.
fn main() -> eframe::Result<()> {
    let mut cli_target = None;
    let mut autostart = false;
    for arg in std::env::args().skip(1) {
        match arg.to_lowercase().as_str() {
            "--start" | "-s" | "/start" => autostart = true,
            _ => cli_target = Some(arg),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DiskScan")
            .with_inner_size([1240.0, 728.0])
            .with_min_inner_size([960.0, 560.0])
            // Pas de barre de titre Windows : l'app dessine ses propres boutons de fenêtre.
            .with_decorations(false)
            .with_icon(app::app_icon()),
        // L'app mémorise elle-même taille, position et état « agrandi » (voir app.rs).
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "DiskScan",
        options,
        Box::new(move |cc| Ok(Box::new(app::DiskScanApp::new(cc, cli_target, autostart)))),
    )
}
