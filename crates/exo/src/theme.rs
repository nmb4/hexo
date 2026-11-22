//! Exo for HEY's palette. HEY's warm yellow is the accent; the rest is quiet neutral so the
//! message is what you look at.

use egui::Color32;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub window: Color32,
    pub panel: Color32,
    pub list: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    pub selected: Color32,
    pub hover: Color32,
    pub card: Color32,
    pub danger: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            window: Color32::from_rgb(244, 243, 240),
            panel: Color32::from_rgb(255, 255, 255),
            list: Color32::from_rgb(250, 249, 247),
            ink: Color32::from_rgb(28, 28, 30),
            muted: Color32::from_rgb(112, 112, 118),
            faint: Color32::from_rgb(154, 154, 160),
            line: Color32::from_rgb(224, 223, 219),
            accent: Color32::from_rgb(245, 196, 0),
            accent_ink: Color32::from_rgb(28, 28, 30),
            selected: Color32::from_rgb(255, 246, 209),
            hover: Color32::from_rgb(240, 239, 235),
            card: Color32::from_rgb(255, 255, 255),
            danger: Color32::from_rgb(196, 58, 58),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            window: Color32::from_rgb(24, 24, 26),
            panel: Color32::from_rgb(31, 31, 34),
            list: Color32::from_rgb(28, 28, 31),
            ink: Color32::from_rgb(235, 235, 238),
            muted: Color32::from_rgb(160, 160, 168),
            faint: Color32::from_rgb(118, 118, 126),
            line: Color32::from_rgb(54, 54, 58),
            accent: Color32::from_rgb(245, 196, 0),
            accent_ink: Color32::from_rgb(24, 24, 26),
            selected: Color32::from_rgb(62, 54, 28),
            hover: Color32::from_rgb(42, 42, 46),
            card: Color32::from_rgb(35, 35, 39),
            danger: Color32::from_rgb(226, 96, 96),
        }
    }

    pub fn install(self, ctx: &egui::Context) {
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.window_fill = self.panel;
        visuals.panel_fill = self.panel;
        visuals.extreme_bg_color = self.list;
        visuals.faint_bg_color = self.hover;
        visuals.hyperlink_color = if self.dark {
            Color32::from_rgb(240, 190, 70)
        } else {
            Color32::from_rgb(120, 88, 0)
        };
        visuals.widgets.noninteractive.bg_fill = self.list;
        visuals.widgets.inactive.bg_fill = self.card;
        visuals.widgets.hovered.bg_fill = self.hover;
        visuals.widgets.active.bg_fill = self.selected;
        visuals.selection.bg_fill = self.accent;
        visuals.selection.stroke = egui::Stroke::new(1.0, self.accent_ink);
        visuals.widgets.inactive.fg_stroke.color = self.ink;
        visuals.widgets.hovered.fg_stroke.color = self.ink;
        visuals.widgets.active.fg_stroke.color = self.ink;
        ctx.set_visuals(visuals);
        ctx.all_styles_mut(|style| {
            style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
            style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        });
    }
}
