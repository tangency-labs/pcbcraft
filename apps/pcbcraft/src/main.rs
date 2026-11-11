//! PCBCraft on the desktop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pcbcraft_app::PcbApp;

fn main() -> eframe::Result {
    let integrated_titlebar = cfg!(target_os = "linux");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(pcbcraft_app::APP_TITLE)
            .with_decorations(!integrated_titlebar)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("PCBCraft", options, Box::new(move |cc| Ok(Box::new(PcbApp::new(cc).with_integrated_titlebar(integrated_titlebar)))))
}
