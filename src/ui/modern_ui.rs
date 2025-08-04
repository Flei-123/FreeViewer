use eframe::egui::{self, *};
use std::time::{Duration, Instant};

/// Modern UI theme colors
#[derive(Clone)]
pub struct Theme {
    pub primary: Color32,
    pub secondary: Color32,
    pub accent: Color32,
    pub background: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub text: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub border: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            primary: Color32::from_rgb(24, 24, 27),      // Dark gray
            secondary: Color32::from_rgb(39, 39, 42),    // Lighter dark gray
            accent: Color32::from_rgb(99, 102, 241),     // Modern blue
            background: Color32::from_rgb(9, 9, 11),     // Very dark
            surface: Color32::from_rgb(24, 24, 27),      // Card background
            surface_hover: Color32::from_rgb(40, 40, 42), // Hover state
            text: Color32::from_rgb(250, 250, 250),      // Main text
            text_primary: Color32::from_rgb(250, 250, 250), // Almost white
            text_secondary: Color32::from_rgb(161, 161, 170), // Gray text
            text_muted: Color32::from_rgb(115, 115, 115), // Muted text
            border: Color32::from_rgb(60, 60, 60),       // Borders
            success: Color32::from_rgb(34, 197, 94),     // Green
            warning: Color32::from_rgb(251, 146, 60),    // Orange  
            error: Color32::from_rgb(239, 68, 68),       // Red
        }
    }

    pub fn light() -> Self {
        Self {
            primary: Color32::from_rgb(255, 255, 255),   // White
            secondary: Color32::from_rgb(248, 250, 252), // Light gray
            accent: Color32::from_rgb(99, 102, 241),     // Modern blue
            background: Color32::from_rgb(248, 250, 252), // Light background
            surface: Color32::from_rgb(255, 255, 255),   // White cards
            surface_hover: Color32::from_rgb(245, 245, 245), // Hover state
            text: Color32::from_rgb(15, 23, 42),         // Main text
            text_primary: Color32::from_rgb(15, 23, 42), // Dark text
            text_secondary: Color32::from_rgb(100, 116, 139), // Gray text
            text_muted: Color32::from_rgb(156, 163, 175), // Muted text
            border: Color32::from_rgb(200, 200, 200),    // Borders
            success: Color32::from_rgb(34, 197, 94),     // Green
            warning: Color32::from_rgb(251, 146, 60),    // Orange
            error: Color32::from_rgb(239, 68, 68),       // Red
        }
    }
    
    pub fn is_dark(&self) -> bool {
        // Simple heuristic: if background is dark, theme is dark
        let bg = self.background;
        (bg.r() as u16 + bg.g() as u16 + bg.b() as u16) < (128 * 3)
    }
}

/// Modern card component
pub struct Card {
    title: String,
}

impl Card {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
        }
    }

    pub fn show<R>(
        &self,
        ui: &mut Ui,
        theme: &Theme,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let frame = Frame::none()
            .fill(theme.surface)
            .rounding(Rounding::same(12.0))
            .stroke(Stroke::new(1.0, theme.secondary))
            .inner_margin(Margin::same(16.0))
            .shadow(epaint::Shadow::small_dark());

        frame.show(ui, |ui| {
            if !self.title.is_empty() {
                ui.label(egui::RichText::new(&self.title)
                    .size(18.0)
                    .color(theme.text_primary));
                ui.separator();
            }
            add_contents(ui)
        })
    }
}

/// Modern button styles
pub struct ModernButton;

impl ModernButton {
    pub fn primary(ui: &mut Ui, theme: &Theme, text: &str) -> Response {
        ui.add(
            Button::new(RichText::new(text).color(Color32::WHITE))
                .fill(theme.accent)
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::new(120.0, 40.0))
        )
    }

    pub fn secondary(ui: &mut Ui, theme: &Theme, text: &str) -> Response {
        ui.add(
            Button::new(RichText::new(text).color(theme.text))
                .fill(theme.secondary)
                .stroke(Stroke::new(1.0, theme.accent))
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::new(120.0, 40.0))
        )
    }

    pub fn danger(ui: &mut Ui, theme: &Theme, text: &str) -> Response {
        ui.add(
            Button::new(RichText::new(text).color(Color32::WHITE))
                .fill(theme.error)
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::new(120.0, 40.0))
        )
    }

    pub fn navigation(ui: &mut Ui, theme: &Theme, text: &str, icon: &str, is_selected: bool) -> Response {
        let text_color = if is_selected {
            theme.accent
        } else {
            Color32::LIGHT_GRAY
        };
        
        let bg_color = if is_selected {
            Color32::from_rgba_premultiplied(theme.accent.r(), theme.accent.g(), theme.accent.b(), 40)
        } else {
            Color32::TRANSPARENT
        };

        ui.add(
            Button::new(
                RichText::new(format!("{} {}", icon, text))
                    .color(text_color)
                    .strong()
            )
            .fill(bg_color)
            .stroke(if is_selected { Stroke::new(1.0, theme.accent) } else { Stroke::NONE })
            .rounding(Rounding::same(6.0))
            .min_size(Vec2::new(180.0, 36.0))
        )
    }
}

/// Status indicators
/// Status types for indicators
#[derive(Clone, Copy)]
pub enum StatusType {
    Success,
    Warning,
    Error,
    Info,
}

pub struct StatusIndicator;

impl StatusIndicator {
    pub fn show(ui: &mut Ui, theme: &Theme, status: &str, status_type: StatusType) {
        ui.horizontal(|ui| {
            let color = match status_type {
                StatusType::Success => theme.success,
                StatusType::Warning => theme.warning,
                StatusType::Error => theme.error,
                StatusType::Info => theme.accent,
            };
            
            ui.label(RichText::new("●").color(color).size(16.0));
            ui.label(RichText::new(status).color(theme.text_secondary));
        });
    }
}

/// Modern progress bar
pub struct ModernProgressBar;

impl ModernProgressBar {
    pub fn show(ui: &mut Ui, theme: &Theme, progress: f32, label: &str) {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).color(theme.text_secondary).size(12.0));
            
            let desired_size = Vec2::new(ui.available_width(), 8.0);
            let (rect, _) = ui.allocate_exact_size(desired_size, Sense::hover());
            
            // Background
            ui.painter().rect_filled(
                rect,
                Rounding::same(4.0),
                theme.secondary,
            );
            
            // Progress
            let progress_width = rect.width() * progress.clamp(0.0, 1.0);
            let progress_rect = Rect::from_min_size(
                rect.min,
                Vec2::new(progress_width, rect.height()),
            );
            
            ui.painter().rect_filled(
                progress_rect,
                Rounding::same(4.0),
                theme.accent,
            );
        });
    }
}

/// Modern sidebar navigation
pub struct Sidebar;

impl Sidebar {
    pub fn show<R>(
        ui: &mut Ui,
        theme: &Theme,
        current_mode: &crate::ui::AppMode,
        add_contents: impl FnOnce(&mut Ui, &crate::ui::AppMode) -> R,
    ) -> InnerResponse<R> {
        let frame = Frame::none()
            .fill(theme.primary)
            .inner_margin(Margin::symmetric(8.0, 16.0));

        frame.show(ui, |ui| {
            add_contents(ui, current_mode)
        })
    }

    pub fn nav_button(
        ui: &mut Ui,
        theme: &Theme,
        icon: &str,
        label: &str,
        is_active: bool,
    ) -> Response {
        let button_height = 48.0;
        let desired_size = Vec2::new(ui.available_width(), button_height);
        let response = ui.allocate_response(desired_size, Sense::click());
        let rect = response.rect;

        let fill_color = if is_active {
            theme.accent
        } else if response.hovered() {
            theme.secondary
        } else {
            Color32::TRANSPARENT
        };

        let text_color = if is_active {
            Color32::WHITE
        } else {
            theme.text_primary
        };

        ui.painter().rect_filled(rect, Rounding::same(8.0), fill_color);

        // Icon and text
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(icon).size(18.0).color(text_color));
                ui.add_space(8.0);
                ui.label(RichText::new(label).size(14.0).color(text_color));
            });
        });

        response
    }
}

/// Toast notification system
#[derive(Clone)]
pub struct Toast {
    message: String,
    toast_type: ToastType,
    created_at: Instant,
    duration: Duration,
}

#[derive(Debug, Clone)]
pub enum ToastType {
    Info,
    Success,
    Warning,
    Error,
}

impl Toast {
    pub fn new(message: String, toast_type: ToastType) -> Self {
        Self {
            message,
            toast_type,
            created_at: Instant::now(),
            duration: Duration::from_secs(4),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.duration
    }

    pub fn show(&self, ui: &mut Ui, theme: &Theme) {
        let alpha = self.alpha();
        if alpha <= 0.0 {
            return;
        }

        let color = match self.toast_type {
            ToastType::Info => theme.accent,
            ToastType::Success => theme.success,
            ToastType::Warning => theme.warning,
            ToastType::Error => theme.error,
        };

        let frame = Frame::popup(&ui.style())
            .fill(color.gamma_multiply(alpha))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::symmetric(16.0, 12.0))
            .shadow(epaint::Shadow::small_dark());

        frame.show(ui, |ui| {
            ui.label(RichText::new(&self.message).color(Color32::WHITE));
        });
    }

    fn alpha(&self) -> f32 {
        let elapsed = self.created_at.elapsed().as_secs_f32();
        let total = self.duration.as_secs_f32();
        
        if elapsed < total * 0.8 {
            1.0
        } else {
            ((total - elapsed) / (total * 0.2)).max(0.0)
        }
    }
}
