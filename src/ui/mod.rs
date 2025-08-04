use eframe::egui;
use std::sync::Arc;
use tokio::sync::Mutex;

mod connection_panel;
mod settings_panel;
mod remote_desktop;
mod file_transfer;

pub use connection_panel::ConnectionPanel;
pub use settings_panel::SettingsPanel;
pub use remote_desktop::RemoteDesktop;
pub use file_transfer::FileTransfer;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Home,
    RemoteControl,
    FileTransfer,
    Settings,
}

#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub partner_id: String,
    pub password: String,
    pub is_connected: bool,
    pub connection_quality: f32, // 0.0 - 1.0
}

impl Default for ConnectionInfo {
    fn default() -> Self {
        Self {
            partner_id: String::new(),
            password: String::new(),
            is_connected: false,
            connection_quality: 0.0,
        }
    }
}

pub struct FreeViewerApp {
    mode: AppMode,
    connection_info: ConnectionInfo,
    my_id: String,
    connection_panel: ConnectionPanel,
    settings_panel: SettingsPanel,
    remote_desktop: RemoteDesktop,
    file_transfer: FileTransfer,
    show_about: bool,
}

impl FreeViewerApp {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Home,
            connection_info: ConnectionInfo::default(),
            my_id: generate_partner_id(),
            connection_panel: ConnectionPanel::new(),
            settings_panel: SettingsPanel::new(),
            remote_desktop: RemoteDesktop::new(),
            file_transfer: FileTransfer::new(),
            show_about: false,
        }
    }
}

impl eframe::App for FreeViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw_top_bar(ctx);
        self.draw_main_content(ctx);
        self.draw_about_dialog(ctx);
        
        // Request repaint for smooth animations
        ctx.request_repaint();
    }
}

impl FreeViewerApp {
    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // Logo and title
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label("🖥️");
                    ui.label(
                        egui::RichText::new("FreeViewer")
                            .size(18.0)
                            .strong()
                            .color(egui::Color32::from_rgb(70, 130, 180))
                    );
                });
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Navigation buttons
                    if ui.button("⚙ Settings").clicked() {
                        self.mode = AppMode::Settings;
                    }
                    
                    if ui.button("📁 Files").clicked() {
                        self.mode = AppMode::FileTransfer;
                    }
                    
                    if ui.button("🖥️ Remote").clicked() {
                        self.mode = AppMode::RemoteControl;
                    }
                    
                    if ui.button("🏠 Home").clicked() {
                        self.mode = AppMode::Home;
                    }
                    
                    ui.separator();
                    
                    if ui.button("ℹ About").clicked() {
                        self.show_about = true;
                    }
                });
            });
        });
    }
    
    fn draw_main_content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.mode {
                AppMode::Home => {
                    self.draw_home_screen(ui);
                }
                AppMode::RemoteControl => {
                    self.remote_desktop.draw(ui, &mut self.connection_info);
                }
                AppMode::FileTransfer => {
                    self.file_transfer.draw(ui, &mut self.connection_info);
                }
                AppMode::Settings => {
                    self.settings_panel.draw(ui);
                }
            }
        });
    }
    
    fn draw_home_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            
            // Welcome header
            ui.label(
                egui::RichText::new("Welcome to FreeViewer")
                    .size(32.0)
                    .strong()
                    .color(egui::Color32::from_rgb(70, 130, 180))
            );
            
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Open-source remote desktop software")
                    .size(16.0)
                    .color(egui::Color32::GRAY)
            );
            
            ui.add_space(40.0);
            
            // My ID section
            ui.group(|ui| {
                ui.set_min_width(400.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Your ID")
                            .size(18.0)
                            .strong()
                    );
                    
                    ui.add_space(10.0);
                    
                    // ID display with copy button
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [300.0, 30.0],
                            egui::TextEdit::singleline(&mut self.my_id.clone())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(300.0)
                        );
                        
                        if ui.button("📋 Copy").clicked() {
                            ui.output_mut(|o| o.copied_text = self.my_id.clone());
                        }
                    });
                    
                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new("Share this ID to allow remote connections")
                            .size(12.0)
                            .color(egui::Color32::GRAY)
                    );
                });
            });
            
            ui.add_space(30.0);
            
            // Connection section
            self.connection_panel.draw(ui, &mut self.connection_info);
            
            ui.add_space(30.0);
            
            // Quick actions
            ui.horizontal(|ui| {
                if ui.add_sized([200.0, 40.0], egui::Button::new("🖥️ Start Remote Control")).clicked() {
                    self.mode = AppMode::RemoteControl;
                }
                
                ui.add_space(20.0);
                
                if ui.add_sized([200.0, 40.0], egui::Button::new("📁 Transfer Files")).clicked() {
                    self.mode = AppMode::FileTransfer;
                }
            });
        });
    }
    
    fn draw_about_dialog(&mut self, ctx: &egui::Context) {
        if self.show_about {
            egui::Window::new("About FreeViewer")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label("🖥️");
                        ui.label(
                            egui::RichText::new("FreeViewer")
                                .size(24.0)
                                .strong()
                        );
                        
                        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                        
                        ui.add_space(10.0);
                        
                        ui.label("Open-source remote desktop software");
                        ui.label("Built with Rust 🦀");
                        
                        ui.add_space(10.0);
                        
                        ui.hyperlink_to("GitHub Repository", "https://github.com/yourusername/freeviewer");
                        
                        ui.add_space(20.0);
                        
                        if ui.button("Close").clicked() {
                            self.show_about = false;
                        }
                    });
                });
        }
    }
}

fn generate_partner_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    
    // Generate a 9-digit ID like TeamViewer
    format!("{:03} {:03} {:03}", 
        rng.gen_range(100..=999),
        rng.gen_range(100..=999),
        rng.gen_range(100..=999)
    )
}
