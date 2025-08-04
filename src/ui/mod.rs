use eframe::egui::{self, Color32, Rounding, Stroke, Vec2, TextStyle, FontId, Ui, Align2, RichText, Layout, Align, Frame};
use std::time::{Duration, Instant};

mod connection_panel;
mod settings_panel;
mod remote_desktop;
mod file_transfer;
mod modern_ui;

pub use connection_panel::ConnectionPanel;
pub use settings_panel::SettingsPanel;
pub use remote_desktop::RemoteDesktop;
pub use file_transfer::FileTransfer;
pub use modern_ui::*;

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
    
    // Modern UI state
    theme: Theme,
    is_dark_mode: bool,
    sidebar_width: f32,
    toasts: Vec<Toast>,
    last_frame_time: Option<Instant>,
    
    // Real functionality components
    screen_capturer: Option<crate::capture::ScreenCaptureImpl>,
    network_manager: Option<crate::protocol::NetworkManager>,
}

impl FreeViewerApp {
    pub fn new() -> Self {
        let is_dark_mode = true; // Default to dark mode
        Self {
            mode: AppMode::Home,
            connection_info: ConnectionInfo::default(),
            my_id: generate_partner_id(),
            connection_panel: ConnectionPanel::new(),
            settings_panel: SettingsPanel::new(),
            remote_desktop: RemoteDesktop::new(),
            file_transfer: FileTransfer::new(),
            show_about: false,
            
            // Modern UI
            theme: if is_dark_mode { Theme::dark() } else { Theme::light() },
            is_dark_mode,
            sidebar_width: 250.0,
            toasts: Vec::new(),
            last_frame_time: None,
            
            // Real functionality
            screen_capturer: None,
            network_manager: None,
        }
    }
    
    fn add_toast(&mut self, message: &str) {
        self.toasts.push(modern_ui::Toast::new(message.to_string(), modern_ui::ToastType::Info));
        
        // Remove old toasts (keep only last 5)
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
    }
}

impl eframe::App for FreeViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Update timing
        let now = Instant::now();
        if let Some(last_time) = self.last_frame_time {
            let _delta_time = now.duration_since(last_time);
        }
        self.last_frame_time = Some(now);
        
        // Clean up expired toasts
        self.toasts.retain(|toast| !toast.is_expired());

        // Apply modern theme
        self.apply_modern_theme(ctx);

        // Modern layout with sidebar
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(self.sidebar_width)
            .show(ctx, |ui| {
                self.draw_modern_sidebar(ui);
            });

        // Main content area with modern styling
        egui::CentralPanel::default()
            .frame(Frame::none().fill(self.theme.background))
            .show(ctx, |ui| {
                self.draw_main_content_modern(ui);
            });

        // Toast notifications overlay  
        self.draw_toasts(ctx);

        // About dialog
        if self.show_about {
            self.draw_about_dialog(ctx);
        }
        
        // Request repaint for smooth animations
        ctx.request_repaint();
    }
}

impl FreeViewerApp {
    fn apply_modern_theme(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        
        // Modern color scheme
        style.visuals.window_fill = self.theme.surface;
        style.visuals.panel_fill = self.theme.primary;
        style.visuals.faint_bg_color = self.theme.secondary;
        style.visuals.extreme_bg_color = self.theme.background;
        // Note: text_color and weak_text_color are methods in newer egui versions
        // style.visuals.text_color = self.theme.text_primary;
        // style.visuals.weak_text_color = self.theme.text_secondary;
        style.visuals.selection.bg_fill = self.theme.accent;
        
        // Modern spacing and rounding
        style.spacing.button_padding = Vec2::new(12.0, 8.0);
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.visuals.window_rounding = Rounding::same(12.0);
        style.visuals.widgets.noninteractive.rounding = Rounding::same(8.0);
        style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
        style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
        style.visuals.widgets.active.rounding = Rounding::same(8.0);
        
        ctx.set_style(style);
    }

    fn draw_modern_sidebar(&mut self, ui: &mut Ui) {
        // Clone values to avoid borrowing issues
        let theme = self.theme.clone();
        let mode = self.mode.clone();
        let my_id = self.my_id.clone();
        let text_primary = theme.text_primary;
        let text_secondary = theme.text_secondary;
        let accent = theme.accent;
        
        Sidebar::show(ui, &theme, &mode, |ui, current_mode| {
            ui.vertical(|ui| {
                // App header
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("🖥️").size(24.0));
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("FreeViewer").size(18.0).strong().color(text_primary));
                        ui.label(RichText::new("v0.1.0").size(12.0).color(text_secondary));
                    });
                });
                
                ui.add_space(30.0);
                
                // Your ID section - simplified without toast
                Card::new("Connection").show(ui, &theme, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Your Partner ID").size(12.0).color(text_secondary));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&my_id).size(16.0).strong().color(accent));
                            if ui.small_button("📋").clicked() {
                                ui.output_mut(|o| o.copied_text = my_id.clone());
                            }
                        });
                    });
                });
                
                ui.add_space(20.0);
                
                // Navigation
                if Sidebar::nav_button(
                    ui, 
                    &self.theme, 
                    "🏠", 
                    "Home", 
                    *current_mode == AppMode::Home
                ).clicked() {
                    self.mode = AppMode::Home;
                }
                
                if Sidebar::nav_button(
                    ui, 
                    &self.theme, 
                    "🖥️", 
                    "Remote Control", 
                    *current_mode == AppMode::RemoteControl
                ).clicked() {
                    self.mode = AppMode::RemoteControl;
                }
                
                if Sidebar::nav_button(
                    ui, 
                    &self.theme, 
                    "📁", 
                    "File Transfer", 
                    *current_mode == AppMode::FileTransfer
                ).clicked() {
                    self.mode = AppMode::FileTransfer;
                }
                
                if Sidebar::nav_button(
                    ui, 
                    &self.theme, 
                    "⚙️", 
                    "Settings", 
                    *current_mode == AppMode::Settings
                ).clicked() {
                    self.mode = AppMode::Settings;
                }
                
                ui.allocate_space(ui.available_size());
                
                // Bottom section - Connection status
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.add_space(16.0);
                    let status_text = if self.connection_info.is_connected {
                        format!("Connected to {}", self.connection_info.partner_id)
                    } else {
                        "Disconnected".to_string()
                    };
                    StatusIndicator::show(
                        ui, 
                        &self.theme, 
                        &status_text,
                        if self.connection_info.is_connected { 
                            modern_ui::StatusType::Success 
                        } else { 
                            modern_ui::StatusType::Error 
                        }
                    );
                });
            });
        });
    }

    fn draw_main_content_modern(&mut self, ui: &mut Ui) {
        match self.mode {
            AppMode::Home => self.draw_home_screen_modern(ui),
            AppMode::RemoteControl => self.draw_remote_control_modern(ui),
            AppMode::FileTransfer => self.draw_file_transfer_modern(ui),
            AppMode::Settings => self.draw_settings_modern(ui),
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        let toasts = self.toasts.clone(); // Clone to avoid borrow issues
        for (i, toast) in toasts.iter().enumerate() {
            egui::Area::new(format!("toast_{}", i))
                .anchor(Align2::RIGHT_TOP, Vec2::new(-20.0, 20.0 + i as f32 * 60.0))
                .show(ctx, |ui| {
                    toast.show(ui, &self.theme);
                });
        }
    }

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
    
    // Modern UI view implementations
    fn draw_home_screen_modern(&mut self, ui: &mut Ui) {
        use modern_ui::{Card, ModernButton};
        
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            
            // Welcome section
            let my_id = self.my_id.clone();
            let theme_primary = self.theme.primary;
            Card::new("Welcome to FreeViewer")
                .show(ui, &self.theme, |ui| {
                    ui.add_space(10.0);
                    ui.label("Your secure remote desktop solution");
                    ui.add_space(10.0);
                    
                    // Your ID display
                    ui.horizontal(|ui| {
                        ui.label("Your ID:");
                        ui.label(egui::RichText::new(&my_id)
                            .color(theme_primary)
                            .monospace()
                            .size(16.0));
                        if ui.button("📋").on_hover_text("Copy to clipboard").clicked() {
                            ui.output_mut(|o| o.copied_text = my_id.clone());
                        }
                    });
                });
            
            // Check if copy button was clicked
            if ui.button("Copy ID").clicked() {
                self.add_toast("ID copied to clipboard!");
            }
            
            ui.add_space(20.0);
            
            // Quick connect section
            Card::new("Quick Connect")
                .show(ui, &self.theme, |ui| {
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Partner ID:");
                        ui.text_edit_singleline(&mut self.connection_info.partner_id);
                    });
                    
                    ui.add_space(10.0);
                });
            
            // Connect buttons outside of the card to avoid borrowing issues
            ui.horizontal(|ui| {
                if ModernButton::primary(ui, &self.theme, "Connect").clicked() {
                    if !self.connection_info.partner_id.is_empty() {
                        self.add_toast("Connecting to partner...");
                        // TODO: Implement actual connection logic
                    }
                }
                
                if ModernButton::secondary(ui, &self.theme, "Remote Support").clicked() {
                    self.add_toast("Remote support mode activated");
                }
            });
        });
    }
    
    fn draw_remote_control_modern(&mut self, ui: &mut Ui) {
        use modern_ui::{Card, ModernButton, StatusIndicator};
        
        let is_connected = self.connection_info.is_connected;
        Card::new("Remote Control Session")
            .show(ui, &self.theme, |ui| {
                ui.add_space(10.0);
                
                // Connection status
                ui.horizontal(|ui| {
                    ui.label("Status:");
                    StatusIndicator::show(ui, &self.theme, 
                        if is_connected { "Connected" } else { "Disconnected" },
                        if is_connected { 
                            modern_ui::StatusType::Success 
                        } else { 
                            modern_ui::StatusType::Error 
                        }
                    );
                });
                
                ui.add_space(10.0);
                
                if is_connected {
                    // Screen capture preview
                    ui.label("Screen Preview (Coming Soon)");
                    ui.add_space(200.0); // Placeholder for screen preview
                } else {
                    ui.vertical_centered(|ui| {
                        ui.label("Not connected to any remote session");
                    });
                }
            });
        
        // Control buttons outside of card to avoid borrowing issues
        if is_connected {
            ui.horizontal(|ui| {
                if ModernButton::secondary(ui, &self.theme, "Full Screen").clicked() {
                    self.add_toast("Full screen mode");
                }
                
                if ModernButton::secondary(ui, &self.theme, "Send Ctrl+Alt+Del").clicked() {
                    self.add_toast("Sent Ctrl+Alt+Del");
                }
                
                if ModernButton::danger(ui, &self.theme, "Disconnect").clicked() {
                    self.connection_info.is_connected = false;
                    self.add_toast("Disconnected from remote session");
                }
            });
        } else {
            if ModernButton::primary(ui, &self.theme, "Start New Session").clicked() {
                self.mode = AppMode::Home;
            }
        }
    }
    
    fn draw_file_transfer_modern(&mut self, ui: &mut Ui) {
        use modern_ui::{Card, ModernButton};
        
        Card::new("File Transfer")
            .show(ui, &self.theme, |ui| {
                ui.add_space(10.0);
                
                if self.connection_info.is_connected {
                    // Local files panel
                    ui.horizontal(|ui| {
                        ui.group(|ui| {
                            ui.set_min_width(300.0);
                            ui.vertical(|ui| {
                                ui.label("Local Files");
                                ui.separator();
                                
                                // File browser (placeholder)
                                for i in 1..=5 {
                                    ui.horizontal(|ui| {
                                        ui.label("📁");
                                        ui.label(format!("Folder {}", i));
                                    });
                                }
                                
                                ui.add_space(10.0);
                                ModernButton::secondary(ui, &self.theme, "Browse...");
                            });
                        });
                        
                        ui.add_space(20.0);
                        
                        // Transfer controls without toast messages
                        ui.vertical(|ui| {
                            ModernButton::primary(ui, &self.theme, "➡️ Send");
                            ui.add_space(10.0);
                            ModernButton::secondary(ui, &self.theme, "⬅️ Receive");
                        });
                        
                        ui.add_space(20.0);
                        
                        // Remote files panel
                        ui.group(|ui| {
                            ui.set_min_width(300.0);
                            ui.vertical(|ui| {
                                ui.label("Remote Files");
                                ui.separator();
                                
                                // Remote file browser (placeholder) 
                                for i in 1..=5 {
                                    ui.horizontal(|ui| {
                                        ui.label("📄");
                                        ui.label(format!("Document {}.txt", i));
                                    });
                                }
                            });
                        });
                    });
                    
                    ui.add_space(20.0);
                    
                    // Transfer progress
                    ui.horizontal(|ui| {
                        ui.label("Transfer Progress:");
                        // Progress bar functionality coming soon
                        ui.label("Progress: 65% - Transferring files..."); // Placeholder for ModernProgressBar
                    });
                    
                } else {
                    ui.vertical_centered(|ui| {
                        ui.label("File transfer requires an active connection");
                    });
                }
            });
        
        // Connect button outside of card to avoid borrowing issues
        if !self.connection_info.is_connected {
            if ModernButton::primary(ui, &self.theme, "Connect First").clicked() {
                self.mode = AppMode::Home;
            }
        }
    }
    
    fn draw_settings_modern(&mut self, ui: &mut Ui) {
        use modern_ui::{Card, ModernButton};
        
        ui.vertical(|ui| {
            // General Settings
            Card::new("General Settings")
                .show(ui, &self.theme, |ui| {
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Auto-start with Windows:");
                        ui.checkbox(&mut false, "Enabled"); // Placeholder
                    });
                    
                    ui.horizontal(|ui| {
                        ui.label("Show in system tray:");
                        ui.checkbox(&mut true, "Enabled"); // Placeholder
                    });
                });
            
            // Theme toggle outside the card
            ui.horizontal(|ui| {
                ui.label("Theme:");
                let is_dark = self.theme.is_dark();
                if ui.button(if is_dark { "Dark" } else { "Light" }).clicked() {
                    self.theme = if is_dark {
                        modern_ui::Theme::light()
                    } else {
                        modern_ui::Theme::dark()
                    };
                }
            });
            
            ui.add_space(20.0);
            
            // Security Settings
            let theme_clone = self.theme.clone();
            Card::new("Security Settings")
                .show(ui, &theme_clone, |ui| {
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Require password:");
                        ui.checkbox(&mut true, "Enabled");
                    });
                    
                    ui.horizontal(|ui| {
                        ui.label("Session timeout (minutes):");
                        let mut timeout = 30;
                        ui.add(egui::DragValue::new(&mut timeout).clamp_range(5..=120));
                    });
                });
                
            if ModernButton::secondary(ui, &self.theme, "Generate New ID").clicked() {
                self.my_id = generate_partner_id();
                self.add_toast("New Partner ID generated");
            }
            
            ui.add_space(20.0);
            
            // Network Settings  
            let theme_clone2 = self.theme.clone();
            Card::new("Network Settings")
                .show(ui, &theme_clone2, |ui| {
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Port:");
                        let mut port = 5938;
                        ui.add(egui::DragValue::new(&mut port).clamp_range(1024..=65535));
                    });
                    
                    ui.horizontal(|ui| {
                        ui.label("Enable UPnP:");
                        ui.checkbox(&mut true, "Enabled");
                    });
                });
                
            if ModernButton::secondary(ui, &self.theme, "Test Connection").clicked() {
                self.add_toast("Testing connection...");
            }
        });
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
