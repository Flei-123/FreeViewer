use eframe::egui::{self, Color32, Rounding, Stroke, Vec2, TextStyle, FontId, Ui, Align2, RichText, Layout, Align, Frame};
use std::time::{Duration, Instant};
use rand::Rng;

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
    pub my_id: String,
    pub my_password: String,
    pub easy_access: bool,
    pub permanent_password: String,
    pub audio_enabled: bool,
    pub quality: String,
    pub port: String,
}

impl Default for ConnectionInfo {
    fn default() -> Self {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        
        // Generate random ID and password like TeamViewer
        let my_id = format!("{:03} {:03} {:03}", 
            rng.gen_range(100..=999),
            rng.gen_range(100..=999),
            rng.gen_range(100..=999)
        );
        let my_password = format!("{:04}", rng.gen_range(1000..=9999));
        
        Self {
            partner_id: String::new(),
            password: String::new(),
            is_connected: false,
            connection_quality: 0.0,
            my_id,
            my_password,
            easy_access: false,
            permanent_password: String::new(),
            audio_enabled: true,
            quality: "Medium".to_string(),
            port: "5900".to_string(),
        }
    }
}

impl ConnectionInfo {
    pub fn generate_new_password(&mut self) {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        self.my_password = format!("{:04}", rng.gen_range(1000..=9999));
    }
}

pub struct FreeViewerApp {
    mode: AppMode,
    connection_info: ConnectionInfo,
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

        // Update theme based on egui's style
        let is_dark = ctx.style().visuals.dark_mode;
        if is_dark != self.is_dark_mode {
            self.is_dark_mode = is_dark;
            self.theme = if is_dark { Theme::dark() } else { Theme::light() };
        }

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
            // About dialog temporarily disabled
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
                
                // Your ID section - improved visibility
                Card::new("Your ID").show(ui, &theme, |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Partner ID").size(12.0).color(text_secondary));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let id_color = if ui.ctx().style().visuals.dark_mode { 
                                egui::Color32::WHITE 
                            } else { 
                                egui::Color32::BLACK 
                            };
                            ui.label(
                                RichText::new(&self.connection_info.my_id)
                                    .size(16.0)
                                    .strong()
                                    .color(id_color)
                                    .background_color(theme.secondary)
                            );
                            if ui.small_button("📋").clicked() {
                                ui.output_mut(|o| o.copied_text = self.connection_info.my_id.clone());
                                self.add_toast("ID copied to clipboard!");
                            }
                        });
                        
                        ui.add_space(8.0);
                        ui.label(RichText::new("Password").size(12.0).color(text_secondary));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let password = if self.connection_info.easy_access && !self.connection_info.permanent_password.is_empty() {
                                &self.connection_info.permanent_password
                            } else {
                                &self.connection_info.my_password
                            };
                            
                            let password_color = if ui.ctx().style().visuals.dark_mode { 
                                egui::Color32::WHITE 
                            } else { 
                                egui::Color32::BLACK 
                            };
                            ui.label(
                                RichText::new(password)
                                    .size(16.0)
                                    .strong()
                                    .color(password_color)
                                    .background_color(theme.secondary)
                            );
                            if ui.small_button("📋").clicked() {
                                ui.output_mut(|o| o.copied_text = password.clone());
                                self.add_toast("Password copied to clipboard!");
                            }
                        });
                        
                        if self.connection_info.easy_access {
                            ui.add_space(4.0);
                            ui.label(RichText::new("🟢 Easy Access").size(11.0).color(Color32::GREEN));
                        }
                    });
                });
                
                ui.add_space(20.0);
                
                // Navigation  
                if ModernButton::navigation(ui, &self.theme, "Home", "🏠", *current_mode == AppMode::Home).clicked() {
                    self.mode = AppMode::Home;
                }
                
                if ModernButton::navigation(ui, &self.theme, "Remote Control", "🖥️", *current_mode == AppMode::RemoteControl).clicked() {
                    self.mode = AppMode::RemoteControl;
                }
                
                if ModernButton::navigation(ui, &self.theme, "File Transfer", "📁", *current_mode == AppMode::FileTransfer).clicked() {
                    self.mode = AppMode::FileTransfer;
                }
                
                if ModernButton::navigation(ui, &self.theme, "Settings", "⚙️", *current_mode == AppMode::Settings).clicked() {
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
            AppMode::Home => self.draw_home_modern(ui),
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
                    self.draw_home_modern(ui);
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
    
    fn draw_home_modern(&mut self, ui: &mut Ui) {
        use modern_ui::Card;
        
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            
            // Header
            ui.heading("🖥️ FreeViewer");
            ui.label("Professional Remote Desktop Solution");
            ui.add_space(20.0);
            
            // Connection Info Panel - showing your ID and password
            let theme = &self.theme;
            let my_id = self.connection_info.my_id.clone();
            let my_password = self.connection_info.my_password.clone();
            
            let mut copy_id_clicked = false;
            let mut copy_password_clicked = false;
            let mut generate_new_clicked = false;
            let mut set_permanent_clicked = false;
            
            Card::new("Your Connection Details")
                .show(ui, theme, |ui| {
                    ui.add_space(10.0);
                    
                    // Your ID section with improved visibility
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Your ID:")
                            .color(theme.text)
                            .size(14.0)
                            .strong());
                        
                        ui.add_space(10.0);
                        
                        // ID display with proper contrast
                        let id_color = if ui.ctx().style().visuals.dark_mode { 
                            egui::Color32::WHITE 
                        } else { 
                            egui::Color32::BLACK 
                        };
                        let id_text = egui::RichText::new(&my_id)
                            .size(16.0)
                            .color(id_color)
                            .monospace()
                            .background_color(theme.secondary);
                            
                        ui.label(id_text);
                        
                        ui.add_space(10.0);
                        
                        if ui.button("📋 Copy ID").clicked() {
                            ui.output_mut(|o| o.copied_text = my_id.clone());
                            copy_id_clicked = true;
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // Password section
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Password:")
                            .color(theme.text)
                            .size(14.0)
                            .strong());
                        
                        ui.add_space(10.0);
                        
                        let password_color = if ui.ctx().style().visuals.dark_mode { 
                            egui::Color32::WHITE 
                        } else { 
                            egui::Color32::BLACK 
                        };
                        let password_text = egui::RichText::new(&my_password)
                            .size(16.0)
                            .color(password_color)
                            .monospace()
                            .background_color(theme.secondary);
                            
                        ui.label(password_text);
                        
                        ui.add_space(10.0);
                        
                        if ui.button("📋 Copy Password").clicked() {
                            ui.output_mut(|o| o.copied_text = my_password.clone());
                            copy_password_clicked = true;
                        }
                        
                        if ui.button("🔄 Generate New").clicked() {
                            generate_new_clicked = true;
                        }
                    });
                    
                    ui.add_space(10.0);
                    
                    // Easy Access toggle
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.connection_info.easy_access, "Easy Access Mode");
                        ui.label("(No password required for trusted partners)");
                    });
                    
                    ui.add_space(10.0);
                    
                    // Permanent password option
                    ui.horizontal(|ui| {
                        ui.label("Permanent Password:");
                        ui.text_edit_singleline(&mut self.connection_info.permanent_password);
                        if ui.button("Set").clicked() && !self.connection_info.permanent_password.is_empty() {
                            set_permanent_clicked = true;
                        }
                    });
                });
            
            // Handle button clicks after the card
            if copy_id_clicked {
                self.add_toast("ID copied to clipboard!");
            }
            if copy_password_clicked {
                self.add_toast("Password copied to clipboard!");
            }
            if generate_new_clicked {
                self.connection_info.generate_new_password();
                self.add_toast("New password generated!");
            }
            if set_permanent_clicked {
                self.connection_info.my_password = self.connection_info.permanent_password.clone();
                self.add_toast("Permanent password set!");
            }
            
            ui.add_space(20.0);
            
            // Quick Connect Panel
            let theme = &self.theme;
            let mut remote_control_clicked = false;
            let mut file_transfer_clicked = false;
            
            Card::new("Connect to Partner")
                .show(ui, theme, |ui| {
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Partner ID:");
                        ui.text_edit_singleline(&mut self.connection_info.partner_id);
                    });
                    
                    ui.add_space(5.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Password:");
                        ui.text_edit_singleline(&mut self.connection_info.password);
                    });
                    
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        if ui.add_sized([120.0, 35.0], egui::Button::new("🖥️ Remote Control")).clicked() {
                            remote_control_clicked = true;
                        }
                        
                        ui.add_space(10.0);
                        
                        if ui.add_sized([120.0, 35.0], egui::Button::new("📁 File Transfer")).clicked() {
                            file_transfer_clicked = true;
                        }
                    });
                });
            
            // Handle button clicks
            if remote_control_clicked {
                self.mode = AppMode::RemoteControl;
                self.add_toast("Starting remote control session...");
            }
            if file_transfer_clicked {
                self.mode = AppMode::FileTransfer;
                self.add_toast("Starting file transfer...");
            }
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
                // Generate new ID
                let mut rng = rand::thread_rng();
                self.connection_info.my_id = format!("{:03} {:03} {:03}", 
                    rng.gen_range(100..=999),
                    rng.gen_range(100..=999),
                    rng.gen_range(100..=999)
                );
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


