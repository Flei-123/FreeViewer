use eframe::egui;
use super::ConnectionInfo;

pub struct ConnectionPanel {
    partner_id_input: String,
    password_input: String,
    is_connecting: bool,
}

impl ConnectionPanel {
    pub fn new() -> Self {
        Self {
            partner_id_input: String::new(),
            password_input: String::new(),
            is_connecting: false,
        }
    }
    
    pub fn draw(&mut self, ui: &mut egui::Ui, connection_info: &mut ConnectionInfo) {
        ui.group(|ui| {
            ui.set_min_width(400.0);
            
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("Connect to Partner")
                        .size(18.0)
                        .strong()
                );
                
                ui.add_space(15.0);
                
                // Partner ID input
                ui.horizontal(|ui| {
                    ui.label("Partner ID:");
                    ui.add_space(10.0);
                    let response = ui.add_sized(
                        [200.0, 25.0],
                        egui::TextEdit::singleline(&mut self.partner_id_input)
                            .hint_text("123 456 789")
                            .font(egui::TextStyle::Monospace)
                    );
                    
                    // Format the ID as user types
                    if response.changed() {
                        self.partner_id_input = format_partner_id(&self.partner_id_input);
                    }
                });
                
                ui.add_space(10.0);
                
                // Password input
                ui.horizontal(|ui| {
                    ui.label("Password:");
                    ui.add_space(10.0);
                    ui.add_sized(
                        [200.0, 25.0],
                        egui::TextEdit::singleline(&mut self.password_input)
                            .password(true)
                            .hint_text("Enter password")
                    );
                });
                
                ui.add_space(20.0);
                
                // Connection status
                if connection_info.is_connected {
                    ui.horizontal(|ui| {
                        ui.label("🟢");
                        ui.label(
                            egui::RichText::new("Connected")
                                .color(egui::Color32::from_rgb(0, 150, 0))
                        );
                        
                        // Connection quality indicator
                        let quality_text = match connection_info.connection_quality {
                            q if q > 0.8 => "Excellent",
                            q if q > 0.6 => "Good", 
                            q if q > 0.4 => "Fair",
                            q if q > 0.2 => "Poor",
                            _ => "Very Poor",
                        };
                        
                        ui.label(format!("Quality: {}", quality_text));
                    });
                    
                    ui.add_space(10.0);
                    
                    if ui.button("🔌 Disconnect").clicked() {
                        self.disconnect(connection_info);
                    }
                } else if self.is_connecting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Connecting...");
                    });
                    
                    if ui.button("Cancel").clicked() {
                        self.is_connecting = false;
                    }
                } else {
                    // Connect button
                    let can_connect = !self.partner_id_input.is_empty() && 
                                    !self.password_input.is_empty();
                    
                    ui.add_enabled_ui(can_connect, |ui| {
                        if ui.add_sized([120.0, 35.0], egui::Button::new("🔗 Connect")).clicked() {
                            self.start_connection(connection_info);
                        }
                    });
                    
                    if !can_connect {
                        ui.label(
                            egui::RichText::new("Enter Partner ID and Password to connect")
                                .size(12.0)
                                .color(egui::Color32::GRAY)
                        );
                    }
                }
            });
        });
    }
    
    fn start_connection(&mut self, connection_info: &mut ConnectionInfo) {
        self.is_connecting = true;
        connection_info.partner_id = self.partner_id_input.clone();
        connection_info.password = self.password_input.clone();
        
        // TODO: Start actual connection process
        // For now, simulate a connection after a delay
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            // Update connection status
        });
        
        // Simulate successful connection for demo
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // In a real app, this would be handled by the networking code
        });
    }
    
    fn disconnect(&mut self, connection_info: &mut ConnectionInfo) {
        connection_info.is_connected = false;
        connection_info.connection_quality = 0.0;
        self.is_connecting = false;
        
        // TODO: Close actual connection
        tracing::info!("Disconnected from partner");
    }
}

fn format_partner_id(input: &str) -> String {
    // Remove all non-digits
    let digits: String = input.chars().filter(|c| c.is_ascii_digit()).collect();
    
    // Limit to 9 digits
    let digits = if digits.len() > 9 {
        &digits[..9]
    } else {
        &digits
    };
    
    // Format as "XXX XXX XXX"
    match digits.len() {
        0..=3 => digits.to_string(),
        4..=6 => format!("{} {}", &digits[..3], &digits[3..]),
        7..=9 => format!("{} {} {}", &digits[..3], &digits[3..6], &digits[6..]),
        _ => digits.to_string(),
    }
}
