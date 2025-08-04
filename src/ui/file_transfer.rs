use eframe::egui;
use super::ConnectionInfo;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileTransferItem {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_directory: bool,
    pub progress: f32, // 0.0 to 1.0
    pub status: TransferStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Completed,
    Failed,
    Cancelled,
}

pub struct FileTransfer {
    local_path: PathBuf,
    remote_path: String,
    local_files: Vec<FileTransferItem>,
    remote_files: Vec<FileTransferItem>,
    transfer_queue: Vec<FileTransferItem>,
    selected_local: Vec<usize>,
    selected_remote: Vec<usize>,
    show_hidden_files: bool,
}

impl FileTransfer {
    pub fn new() -> Self {
        let mut instance = Self {
            local_path: dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            remote_path: "/".to_string(),
            local_files: Vec::new(),
            remote_files: Vec::new(),
            transfer_queue: Vec::new(),
            selected_local: Vec::new(),
            selected_remote: Vec::new(),
            show_hidden_files: false,
        };
        
        instance.refresh_local_files();
        instance
    }
    
    pub fn draw(&mut self, ui: &mut egui::Ui, connection_info: &mut ConnectionInfo) {
        if !connection_info.is_connected {
            self.draw_not_connected(ui);
            return;
        }
        
        ui.vertical(|ui| {
            // Toolbar
            self.draw_toolbar(ui);
            
            ui.add_space(10.0);
            
            // File browser panels
            ui.horizontal(|ui| {
                // Local files panel
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width() * 0.45);
                    self.draw_local_panel(ui);
                });
                
                ui.add_space(10.0);
                
                // Transfer controls
                ui.vertical(|ui| {
                    ui.set_width(80.0);
                    ui.add_space(50.0);
                    
                    if ui.button("→").clicked() {
                        self.transfer_to_remote();
                    }
                    
                    ui.add_space(10.0);
                    
                    if ui.button("←").clicked() {
                        self.transfer_to_local();
                    }
                });
                
                ui.add_space(10.0);
                
                // Remote files panel
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width());
                    self.draw_remote_panel(ui);
                });
            });
            
            ui.add_space(10.0);
            
            // Transfer queue
            if !self.transfer_queue.is_empty() {
                ui.group(|ui| {
                    self.draw_transfer_queue(ui);
                });
            }
        });
    }
    
    fn draw_not_connected(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            
            ui.label("📁");
            ui.label(
                egui::RichText::new("File Transfer Not Available")
                    .size(24.0)
                    .color(egui::Color32::GRAY)
            );
            
            ui.add_space(20.0);
            
            ui.label("Connect to a partner from the Home screen to transfer files");
        });
    }
    
    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("🏠 Home").clicked() {
                self.local_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                self.refresh_local_files();
            }
            
            if ui.button("⬆ Up").clicked() {
                if let Some(parent) = self.local_path.parent() {
                    self.local_path = parent.to_path_buf();
                    self.refresh_local_files();
                }
            }
            
            if ui.button("🔄 Refresh").clicked() {
                self.refresh_local_files();
                self.refresh_remote_files();
            }
            
            ui.separator();
            
            ui.checkbox(&mut self.show_hidden_files, "Show hidden files");
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("📂 New Folder").clicked() {
                    self.create_folder();
                }
            });
        });
    }
    
    fn draw_local_panel(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("💻 Local Computer")
                        .size(16.0)
                        .strong()
                );
            });
            
            // Current path
            ui.horizontal(|ui| {
                ui.label("Path:");
                ui.label(
                    egui::RichText::new(self.local_path.to_string_lossy())
                        .color(egui::Color32::GRAY)
                );
            });
            
            ui.separator();
            
            // File list
            let mut navigate_to_path: Option<std::path::PathBuf> = None;
            let mut toggle_selection: Option<(usize, bool)> = None;
            
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    for (i, file) in self.local_files.iter().enumerate() {
                        let is_selected = self.selected_local.contains(&i);
                        
                        let size_str = if file.is_directory { 
                            String::new() 
                        } else { 
                            format_file_size(file.size) 
                        };
                        let response = ui.selectable_label(
                            is_selected,
                            format!(
                                "{} {} {}",
                                if file.is_directory { "📁" } else { "📄" },
                                file.name,
                                size_str
                            )
                        );
                        
                        if response.clicked() {
                            if file.is_directory {
                                // Navigate into directory
                                navigate_to_path = Some(file.path.clone());
                            } else {
                                // Select/deselect file
                                toggle_selection = Some((i, is_selected));
                            }
                        }
                        
                        // Context menu
                        response.context_menu(|ui| {
                            if ui.button("📋 Copy").clicked() {
                                ui.close_menu();
                            }
                            if ui.button("✂️ Cut").clicked() {
                                ui.close_menu();
                            }
                            if ui.button("🗑️ Delete").clicked() {
                                ui.close_menu();
                            }
                        });
                    }
                });
            
            // Handle navigation after the iterator is finished  
            if let Some(path) = navigate_to_path {
                self.local_path = path;
                self.refresh_local_files();
                self.selected_local.clear();
            }
            
            // Handle file selection changes
            if let Some((i, was_selected)) = toggle_selection {
                if was_selected {
                    self.selected_local.retain(|&x| x != i);
                } else {
                    self.selected_local.push(i);
                }
            }
        });
    }
    
    fn draw_remote_panel(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🌐 Remote Computer")
                        .size(16.0)
                        .strong()
                );
            });
            
            // Current path
            ui.horizontal(|ui| {
                ui.label("Path:");
                ui.label(
                    egui::RichText::new(&self.remote_path)
                        .color(egui::Color32::GRAY)
                );
            });
            
            ui.separator();
            
            // File list (demo data)
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    if self.remote_files.is_empty() {
                        // Show demo files
                        let demo_files = vec![
                            ("📁", "Documents", true, 0),
                            ("📁", "Pictures", true, 0),
                            ("📁", "Videos", true, 0),
                            ("📄", "report.pdf", false, 2_456_789),
                            ("📄", "presentation.pptx", false, 15_678_901),
                            ("📄", "data.xlsx", false, 987_654),
                        ];
                        
                        for (i, (icon, name, is_dir, size)) in demo_files.iter().enumerate() {
                            let is_selected = self.selected_remote.contains(&i);
                            
                            let size_str = if *is_dir { 
                                String::new() 
                            } else { 
                                format_file_size(*size) 
                            };
                            let response = ui.selectable_label(
                                is_selected,
                                format!(
                                    "{} {} {}",
                                    icon,
                                    name,
                                    size_str
                                )
                            );
                            
                            if response.clicked() {
                                if is_selected {
                                    self.selected_remote.retain(|&x| x != i);
                                } else {
                                    self.selected_remote.push(i);
                                }
                            }
                        }
                    }
                });
        });
    }
    
    fn draw_transfer_queue(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new("🔄 Transfer Queue")
                    .size(16.0)
                    .strong()
            );
            
            ui.separator();
            
            for item in &self.transfer_queue {
                ui.horizontal(|ui| {
                    // Status icon
                    let (icon, color) = match item.status {
                        TransferStatus::Pending => ("⏳", egui::Color32::GRAY),
                        TransferStatus::Transferring => ("🔄", egui::Color32::BLUE),
                        TransferStatus::Completed => ("✅", egui::Color32::GREEN),
                        TransferStatus::Failed => ("❌", egui::Color32::RED),
                        TransferStatus::Cancelled => ("🚫", egui::Color32::GRAY),
                    };
                    
                    ui.colored_label(color, icon);
                    
                    // File info
                    ui.label(&item.name);
                    ui.label(format_file_size(item.size));
                    
                    // Progress bar
                    if item.status == TransferStatus::Transferring {
                        ui.add(
                            egui::ProgressBar::new(item.progress)
                                .text(format!("{:.1}%", item.progress * 100.0))
                        );
                    }
                    
                    // Cancel button
                    if item.status == TransferStatus::Pending || item.status == TransferStatus::Transferring {
                        if ui.button("❌").clicked() {
                            // TODO: Cancel transfer
                        }
                    }
                });
            }
        });
    }
    
    fn refresh_local_files(&mut self) {
        self.local_files.clear();
        
        if let Ok(entries) = std::fs::read_dir(&self.local_path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                
                // Skip hidden files if not enabled
                if !self.show_hidden_files && name.starts_with('.') {
                    continue;
                }
                
                let is_directory = path.is_dir();
                let size = if is_directory {
                    0
                } else {
                    path.metadata().map(|m| m.len()).unwrap_or(0)
                };
                
                self.local_files.push(FileTransferItem {
                    name,
                    path,
                    size,
                    is_directory,
                    progress: 0.0,
                    status: TransferStatus::Pending,
                });
            }
        }
        
        // Sort: directories first, then by name
        self.local_files.sort_by(|a, b| {
            match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
    }
    
    fn refresh_remote_files(&mut self) {
        // TODO: Request remote file list
        tracing::info!("Refreshing remote files for path: {}", self.remote_path);
    }
    
    fn transfer_to_remote(&mut self) {
        for &i in &self.selected_local {
            if let Some(file) = self.local_files.get(i).cloned() {
                self.transfer_queue.push(file);
            }
        }
        self.selected_local.clear();
        
        // TODO: Start actual transfer
        tracing::info!("Starting transfer to remote");
    }
    
    fn transfer_to_local(&mut self) {
        // TODO: Transfer selected remote files to local
        tracing::info!("Starting transfer to local");
    }
    
    fn create_folder(&mut self) {
        // TODO: Show dialog to create new folder
        tracing::info!("Creating new folder");
    }
}

fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = size as f64;
    let mut unit_index = 0;
    
    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }
    
    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}
