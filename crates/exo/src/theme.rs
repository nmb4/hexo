//! Exo for HEY's palette: quiet neutral surfaces, crisp typography, a blue selection and
//! HEY's yellow for primary actions.

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
    pub shadow: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            window: Color32::from_rgb(246, 247, 249),
            panel: Color32::from_rgb(255, 255, 255),
            list: Color32::from_rgb(255, 255, 255),
            ink: Color32::from_rgb(28, 32, 38),
            muted: Color32::from_rgb(112, 119, 130),
            faint: Color32::from_rgb(168, 175, 185),
            line: Color32::from_rgb(232, 235, 239),
            accent: Color32::from_rgb(245, 196, 0),
            accent_ink: Color32::from_rgb(28, 32, 38),
            selected: Color32::from_rgb(233, 242, 255),
            hover: Color32::from_rgb(245, 248, 252),
            card: Color32::from_rgb(255, 255, 255),
            danger: Color32::from_rgb(190, 62, 62),
            shadow: Color32::from_rgba_premultiplied(20, 30, 50, 12),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            window: Color32::from_rgb(20, 22, 25),
            panel: Color32::from_rgb(28, 30, 34),
            list: Color32::from_rgb(25, 27, 31),
            ink: Color32::from_rgb(234, 237, 241),
            muted: Color32::from_rgb(156, 164, 174),
            faint: Color32::from_rgb(112, 120, 130),
            line: Color32::from_rgb(48, 52, 58),
            accent: Color32::from_rgb(245, 196, 0),
            accent_ink: Color32::from_rgb(24, 26, 30),
            selected: Color32::from_rgb(34, 52, 78),
            hover: Color32::from_rgb(35, 38, 44),
            card: Color32::from_rgb(32, 35, 40),
            danger: Color32::from_rgb(226, 102, 102),
            shadow: Color32::from_rgba_premultiplied(0, 0, 0, 40),
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
            Color32::from_rgb(48, 104, 184)
        };
        visuals.widgets.noninteractive.bg_fill = self.list;
        visuals.widgets.inactive.bg_fill = self.card;
        visuals.widgets.hovered.bg_fill = self.hover;
        visuals.widgets.active.bg_fill = self.selected;
        visuals.selection.bg_fill = self.selected;
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
