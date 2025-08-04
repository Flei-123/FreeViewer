use eframe::egui;
use std::sync::Arc;
use tokio::sync::Mutex;

mod client;
mod host;
mod protocol;
mod security;
mod capture;
mod ui;

use ui::FreeViewerApp;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    tracing::info!("Starting FreeViewer v{}", env!("CARGO_PKG_VERSION"));

    // Create the egui app
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_icon(Arc::new(load_icon()))
            .with_title("FreeViewer - Open Source Remote Desktop"),
        ..Default::default()
    };

    eframe::run_native(
        "FreeViewer",
        options,
        Box::new(|cc| {
            // Setup custom fonts
            setup_custom_fonts(&cc.egui_ctx);
            
            Box::new(FreeViewerApp::new())
        }),
    )
    .map_err(|e| format!("Failed to run native app: {}", e).into())
}

fn load_icon() -> egui::IconData {
    // Create a simple icon (in a real app, you'd load from a file)
    let size = 64;
    let mut pixels = vec![0; size * size * 4]; // RGBA
    
    // Draw a simple monitor icon
    for y in 0..size {
        for x in 0..size {
            let idx = (y * size + x) * 4;
            
            // Draw monitor outline
            if (x >= 8 && x < 56 && y >= 8 && y < 40) ||  // Screen
               (x >= 24 && x < 40 && y >= 40 && y < 48) || // Stand
               (x >= 16 && x < 48 && y >= 48 && y < 52)    // Base
            {
                pixels[idx] = 70;      // R
                pixels[idx + 1] = 130; // G
                pixels[idx + 2] = 180; // B
                pixels[idx + 3] = 255; // A
            }
        }
    }
    
    egui::IconData {
        rgba: pixels,
        width: size as u32,
        height: size as u32,
    }
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    
    // Add custom fonts if needed
    fonts.font_data.insert(
        "fira_code".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/FiraCode-Regular.ttf")),
    );
    
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "fira_code".to_owned());
    
    ctx.set_fonts(fonts);
}
