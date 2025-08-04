use eframe::egui;

#[derive(Debug, Clone)]
pub struct Settings {
    pub video_quality: VideoQuality,
    pub enable_clipboard_sync: bool,
    pub enable_file_transfer: bool,
    pub enable_sound: bool,
    pub auto_start_with_system: bool,
    pub relay_server: String,
    pub custom_relay_server: String,
    pub encryption_enabled: bool,
    pub log_level: LogLevel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoQuality {
    Low,
    Medium,
    High,
    Adaptive,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            video_quality: VideoQuality::Adaptive,
            enable_clipboard_sync: true,
            enable_file_transfer: true,
            enable_sound: false, // Disabled by default for now
            auto_start_with_system: false,
            relay_server: "Official".to_string(),
            custom_relay_server: String::new(),
            encryption_enabled: true,
            log_level: LogLevel::Info,
        }
    }
}

pub struct SettingsPanel {
    settings: Settings,
    show_advanced: bool,
}

impl SettingsPanel {
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
            show_advanced: false,
        }
    }
    
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Settings");
            ui.add_space(20.0);
            
            // Video Settings
            ui.group(|ui| {
                ui.label(
                    egui::RichText::new("🖥️ Video Settings")
                        .size(16.0)
                        .strong()
                );
                
                ui.add_space(10.0);
                
                ui.horizontal(|ui| {
                    ui.label("Quality:");
                    egui::ComboBox::from_id_source("video_quality")
                        .selected_text(format!("{:?}", self.settings.video_quality))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.video_quality, VideoQuality::Low, "Low (Fast)");
                            ui.selectable_value(&mut self.settings.video_quality, VideoQuality::Medium, "Medium");
                            ui.selectable_value(&mut self.settings.video_quality, VideoQuality::High, "High (Slow)");
                            ui.selectable_value(&mut self.settings.video_quality, VideoQuality::Adaptive, "Adaptive (Recommended)");
                        });
                });
                
                ui.add_space(5.0);
                
                match self.settings.video_quality {
                    VideoQuality::Low => ui.label("⚡ Fastest connection, lower image quality"),
                    VideoQuality::Medium => ui.label("⚖️ Balanced speed and quality"),
                    VideoQuality::High => ui.label("🎯 Best image quality, slower connection"),
                    VideoQuality::Adaptive => ui.label("🤖 Automatically adjusts to network conditions"),
                };
            });
            
            ui.add_space(15.0);
            
            // Feature Settings
            ui.group(|ui| {
                ui.label(
                    egui::RichText::new("⚙️ Features")
                        .size(16.0)
                        .strong()
                );
                
                ui.add_space(10.0);
                
                ui.checkbox(&mut self.settings.enable_clipboard_sync, "📋 Sync clipboard between computers");
                ui.checkbox(&mut self.settings.enable_file_transfer, "📁 Enable file transfer");
                ui.checkbox(&mut self.settings.enable_sound, "🔊 Transfer sound (experimental)");
            });
            
            ui.add_space(15.0);
            
            // System Settings
            ui.group(|ui| {
                ui.label(
                    egui::RichText::new("🖥️ System")
                        .size(16.0)
                        .strong()
                );
                
                ui.add_space(10.0);
                
                ui.checkbox(&mut self.settings.auto_start_with_system, "🚀 Start with Windows");
            });
            
            ui.add_space(15.0);
            
            // Advanced Settings Toggle
            ui.horizontal(|ui| {
                if ui.button(if self.show_advanced { "Hide Advanced" } else { "Show Advanced" }).clicked() {
                    self.show_advanced = !self.show_advanced;
                }
            });
            
            if self.show_advanced {
                ui.add_space(10.0);
                
                // Network Settings
                ui.group(|ui| {
                    ui.label(
                        egui::RichText::new("🌐 Network")
                            .size(16.0)
                            .strong()
                    );
                    
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Relay Server:");
                        egui::ComboBox::from_id_source("relay_server")
                            .selected_text(&self.settings.relay_server)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.settings.relay_server, "Official".to_string(), "Official FreeViewer Relay");
                                ui.selectable_value(&mut self.settings.relay_server, "Custom".to_string(), "Custom Server");
                            });
                    });
                    
                    if self.settings.relay_server == "Custom" {
                        ui.horizontal(|ui| {
                            ui.label("Server URL:");
                            ui.text_edit_singleline(&mut self.settings.custom_relay_server);
                        });
                    }
                });
                
                ui.add_space(15.0);
                
                // Security Settings
                ui.group(|ui| {
                    ui.label(
                        egui::RichText::new("🔒 Security")
                            .size(16.0)
                            .strong()
                    );
                    
                    ui.add_space(10.0);
                    
                    ui.checkbox(&mut self.settings.encryption_enabled, "🔐 Enable end-to-end encryption");
                    
                    if !self.settings.encryption_enabled {
                        ui.label(
                            egui::RichText::new("⚠️ Warning: Disabling encryption is not recommended")
                                .color(egui::Color32::from_rgb(255, 165, 0))
                        );
                    }
                });
                
                ui.add_space(15.0);
                
                // Logging Settings
                ui.group(|ui| {
                    ui.label(
                        egui::RichText::new("📝 Logging")
                            .size(16.0)
                            .strong()
                    );
                    
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Log Level:");
                        egui::ComboBox::from_id_source("log_level")
                            .selected_text(format!("{:?}", self.settings.log_level))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.settings.log_level, LogLevel::Error, "Error");
                                ui.selectable_value(&mut self.settings.log_level, LogLevel::Warn, "Warning");
                                ui.selectable_value(&mut self.settings.log_level, LogLevel::Info, "Info");
                                ui.selectable_value(&mut self.settings.log_level, LogLevel::Debug, "Debug");
                                ui.selectable_value(&mut self.settings.log_level, LogLevel::Trace, "Trace");
                            });
                    });
                    
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        if ui.button("📂 Open Log Folder").clicked() {
                            self.open_log_folder();
                        }
                        
                        if ui.button("🗑️ Clear Logs").clicked() {
                            self.clear_logs();
                        }
                    });
                });
            }
            
            ui.add_space(30.0);
            
            // Action buttons
            ui.horizontal(|ui| {
                if ui.button("💾 Save Settings").clicked() {
                    self.save_settings();
                }
                
                ui.add_space(10.0);
                
                if ui.button("🔄 Reset to Default").clicked() {
                    self.settings = Settings::default();
                }
            });
        });
    }
    
    fn save_settings(&self) {
        // TODO: Implement settings persistence
        tracing::info!("Settings saved: {:?}", self.settings);
    }
    
    fn open_log_folder(&self) {
        // TODO: Open log folder in file explorer
        #[cfg(windows)]
        {
            if let Some(data_dir) = dirs::data_dir() {
                let log_path = data_dir.join("FreeViewer").join("logs");
                let _ = std::process::Command::new("explorer")
                    .arg(log_path)
                    .spawn();
            }
        }
        
        #[cfg(target_os = "linux")]
        {
            if let Some(data_dir) = dirs::data_dir() {
                let log_path = data_dir.join("freeviewer").join("logs");
                let _ = std::process::Command::new("xdg-open")
                    .arg(log_path)
                    .spawn();
            }
        }
        
        #[cfg(target_os = "macos")]
        {
            if let Some(data_dir) = dirs::data_dir() {
                let log_path = data_dir.join("FreeViewer").join("logs");
                let _ = std::process::Command::new("open")
                    .arg(log_path)
                    .spawn();
            }
        }
    }
    
    fn clear_logs(&self) {
        // TODO: Clear log files
        tracing::info!("Clearing log files");
    }
}
