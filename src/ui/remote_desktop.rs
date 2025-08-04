use eframe::egui;
use super::ConnectionInfo;

pub struct RemoteDesktop {
    screen_texture: Option<egui::TextureHandle>,
    mouse_pos: egui::Pos2,
    is_fullscreen: bool,
    show_toolbar: bool,
    zoom_level: f32,
    toolbar_timer: std::time::Instant,
}

impl RemoteDesktop {
    pub fn new() -> Self {
        Self {
            screen_texture: None,
            mouse_pos: egui::Pos2::ZERO,
            is_fullscreen: false,
            show_toolbar: true,
            zoom_level: 1.0,
            toolbar_timer: std::time::Instant::now(),
        }
    }
    
    pub fn draw(&mut self, ui: &mut egui::Ui, connection_info: &mut ConnectionInfo) {
        if !connection_info.is_connected {
            self.draw_not_connected(ui);
            return;
        }
        
        // Auto-hide toolbar after 3 seconds of inactivity
        if self.toolbar_timer.elapsed().as_secs() > 3 {
            self.show_toolbar = false;
        }
        
        // Show toolbar on mouse movement
        if ui.ctx().input(|i| i.pointer.velocity().length() > 0.0) {
            self.show_toolbar = true;
            self.toolbar_timer = std::time::Instant::now();
        }
        
        ui.vertical(|ui| {
            // Toolbar
            if self.show_toolbar {
                self.draw_toolbar(ui, connection_info);
            }
            
            // Remote screen area
            let available_rect = ui.available_rect_before_wrap();
            self.draw_remote_screen(ui, available_rect, connection_info);
        });
    }
    
    fn draw_not_connected(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            
            ui.label("🔌");
            ui.label(
                egui::RichText::new("Not Connected")
                    .size(24.0)
                    .color(egui::Color32::GRAY)
            );
            
            ui.add_space(20.0);
            
            ui.label("Connect to a partner from the Home screen to start remote control");
        });
    }
    
    fn draw_toolbar(&mut self, ui: &mut egui::Ui, connection_info: &mut ConnectionInfo) {
        egui::TopBottomPanel::top("remote_toolbar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                // Connection status
                ui.label("🟢");
                ui.label(format!("Connected to {}", connection_info.partner_id));
                
                ui.separator();
                
                // Quality indicator
                let quality_color = match connection_info.connection_quality {
                    q if q > 0.8 => egui::Color32::from_rgb(0, 150, 0),
                    q if q > 0.6 => egui::Color32::from_rgb(150, 150, 0),
                    _ => egui::Color32::from_rgb(150, 0, 0),
                };
                
                ui.colored_label(quality_color, "●");
                ui.label(format!("Quality: {:.0}%", connection_info.connection_quality * 100.0));
                
                ui.separator();
                
                // Zoom controls
                ui.label("Zoom:");
                if ui.button("−").clicked() && self.zoom_level > 0.5 {
                    self.zoom_level -= 0.1;
                }
                ui.label(format!("{:.0}%", self.zoom_level * 100.0));
                if ui.button("+").clicked() && self.zoom_level < 3.0 {
                    self.zoom_level += 0.1;
                }
                
                if ui.button("100%").clicked() {
                    self.zoom_level = 1.0;
                }
                
                ui.separator();
                
                // Screen controls
                if ui.button("📷 Screenshot").clicked() {
                    self.take_screenshot();
                }
                
                if ui.button(if self.is_fullscreen { "🗗 Exit Fullscreen" } else { "🗖 Fullscreen" }).clicked() {
                    self.is_fullscreen = !self.is_fullscreen;
                }
                
                ui.separator();
                
                // Special keys
                if ui.button("Ctrl+Alt+Del").clicked() {
                    self.send_ctrl_alt_del();
                }
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🔌 Disconnect").clicked() {
                        connection_info.is_connected = false;
                    }
                });
            });
        });
    }
    
    fn draw_remote_screen(&mut self, ui: &mut egui::Ui, rect: egui::Rect, _connection_info: &ConnectionInfo) {
        // Create a demo screen texture if none exists
        if self.screen_texture.is_none() {
            self.screen_texture = Some(self.create_demo_texture(ui.ctx()));
        }
        
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        
        if let Some(texture) = &self.screen_texture {
            // Calculate the scaled size
            let texture_size = texture.size_vec2();
            let scaled_size = texture_size * self.zoom_level;
            
            // Center the image in the available space
            let offset = (rect.size() - scaled_size) * 0.5;
            let image_rect = egui::Rect::from_min_size(
                rect.min + offset.max(egui::Vec2::ZERO),
                scaled_size.min(rect.size())
            );
            
            // Draw the remote screen
            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            
            // Handle mouse input
            if response.clicked() || response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if image_rect.contains(pos) {
                        // Convert screen coordinates to remote screen coordinates
                        let relative_pos = (pos - image_rect.min) / scaled_size;
                        self.send_mouse_event(relative_pos, response.clicked());
                        self.mouse_pos = pos;
                    }
                }
            }
            
            // Draw mouse cursor
            if image_rect.contains(self.mouse_pos) {
                ui.painter().circle_filled(
                    self.mouse_pos,
                    3.0,
                    egui::Color32::from_rgba_premultiplied(255, 0, 0, 100),
                );
            }
        }
        
        // Handle keyboard input
        ui.ctx().input(|i| {
            for event in &i.events {
                if let egui::Event::Key { key, pressed: true, .. } = event {
                    self.send_key_event(*key);
                }
            }
        });
    }
    
    fn create_demo_texture(&self, ctx: &egui::Context) -> egui::TextureHandle {
        // Create a demo desktop image
        let width = 1920;
        let height = 1080;
        let mut pixels = vec![0u8; width * height * 4]; // RGBA
        
        // Create a gradient background
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                let r = ((x as f32 / width as f32) * 255.0) as u8;
                let g = ((y as f32 / height as f32) * 255.0) as u8;
                let b = 100;
                
                pixels[idx] = r;
                pixels[idx + 1] = g;
                pixels[idx + 2] = b;
                pixels[idx + 3] = 255; // Alpha
            }
        }
        
        // Add some demo windows
        self.draw_demo_window(&mut pixels, width, height, 100, 100, 400, 300, [240, 240, 240, 255]);
        self.draw_demo_window(&mut pixels, width, height, 600, 200, 500, 400, [200, 220, 255, 255]);
        
        let color_image = egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels);
        ctx.load_texture("remote_screen", color_image, egui::TextureOptions::LINEAR)
    }
    
    fn draw_demo_window(&self, pixels: &mut [u8], width: usize, height: usize, 
                       x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
        for dy in 0..h {
            for dx in 0..w {
                let px = x + dx;
                let py = y + dy;
                
                if px < width && py < height {
                    let idx = (py * width + px) * 4;
                    
                    // Window border
                    if dx == 0 || dy == 0 || dx == w - 1 || dy == h - 1 {
                        pixels[idx] = 100;
                        pixels[idx + 1] = 100;
                        pixels[idx + 2] = 100;
                        pixels[idx + 3] = 255;
                    }
                    // Title bar
                    else if dy < 30 {
                        pixels[idx] = 70;
                        pixels[idx + 1] = 130;
                        pixels[idx + 2] = 180;
                        pixels[idx + 3] = 255;
                    }
                    // Window content
                    else {
                        pixels[idx] = color[0];
                        pixels[idx + 1] = color[1];
                        pixels[idx + 2] = color[2];
                        pixels[idx + 3] = color[3];
                    }
                }
            }
        }
    }
    
    fn send_mouse_event(&self, relative_pos: egui::Vec2, clicked: bool) {
        // TODO: Send mouse event to remote computer
        tracing::debug!("Mouse event: pos={:?}, clicked={}", relative_pos, clicked);
    }
    
    fn send_key_event(&self, key: egui::Key) {
        // TODO: Send keyboard event to remote computer
        tracing::debug!("Key event: {:?}", key);
    }
    
    fn take_screenshot(&self) {
        // TODO: Take screenshot of remote screen
        tracing::info!("Taking screenshot");
    }
    
    fn send_ctrl_alt_del(&self) {
        // TODO: Send Ctrl+Alt+Del to remote computer
        tracing::info!("Sending Ctrl+Alt+Del");
    }
}
