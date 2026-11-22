//! Exo's Classic palette: Snow and Carbon, the same literals the original app draws with.
//!
//! The names here are the roles the original design system uses (canvas, right pane, selected
//! row, unread dot, …), so the UI code reads like the archived Exo: a white list on white, a
//! faint blue right pane, grey hairlines, and periwinkle for selection.

use egui::Color32;
use exo_ui::Theme;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// The window and the list's canvas.
    pub window: Color32,
    /// Raised panels: the compose window.
    pub panel: Color32,
    /// The left rail and the tab strip.
    pub chrome: Color32,
    /// The thread list.
    pub list: Color32,
    /// The right contact pane, `hsl(216, 100%, 99%)`.
    pub right: Color32,
    /// Primary text: a row's sender and subject.
    pub ink: Color32,
    /// Secondary text: snippets, dates, the contact's email.
    pub muted: Color32,
    /// Tertiary text: section separators and placeholders.
    pub faint: Color32,
    /// Hairlines: the rail, the right pane, the tab strip.
    pub line: Color32,
    /// The periwinkle accent: selection bars, primary buttons.
    pub accent: Color32,
    /// Text on the accent.
    pub accent_ink: Color32,
    /// The list cursor's row.
    pub selected: Color32,
    pub hover: Color32,
    pub card: Color32,
    pub danger: Color32,
    /// The unread dot.
    pub unread: Color32,
    /// The star's dot.
    pub star: Color32,
    /// The account dot in the tab strip.
    pub account_dot: Color32,
    /// The light grey disc behind a contact section's icon.
    pub disc: Color32,
    /// Icons at rest and hovered in the chrome.
    pub icon: Color32,
    pub icon_hover: Color32,
    pub shadow: Color32,
    pub scrim: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            window: Color32::WHITE,
            panel: Color32::WHITE,
            chrome: Color32::WHITE,
            list: Color32::WHITE,
            right: Color32::from_rgb(0xfa, 0xfc, 0xff),
            ink: Color32::from_black_alpha(230),
            muted: Color32::from_black_alpha(128),
            faint: Color32::from_black_alpha(89),
            line: Color32::from_black_alpha(26),
            accent: Color32::from_rgb(0xae, 0xb1, 0xdd),
            accent_ink: Color32::from_white_alpha(250),
            selected: Color32::from_rgb(0xf4, 0xf6, 0xfb),
            hover: Color32::from_rgb(0xf4, 0xf6, 0xfb),
            card: Color32::WHITE,
            danger: Color32::from_rgb(0xef, 0x8e, 0x8e),
            unread: Color32::from_rgb(0x72, 0xc0, 0xea),
            star: Color32::from_rgb(0xff, 0xcb, 0x33),
            account_dot: Color32::from_rgb(0xbf, 0xc8, 0xd2),
            disc: Color32::from_rgb(0xbf, 0xc8, 0xd2),
            icon: Color32::from_black_alpha(77),
            icon_hover: Color32::from_black_alpha(128),
            shadow: Color32::from_black_alpha(38),
            scrim: Color32::from_black_alpha(20),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            window: Color32::from_rgb(0x27, 0x29, 0x2d),
            panel: Color32::from_rgb(0x27, 0x29, 0x2d),
            chrome: Color32::from_rgb(0x27, 0x29, 0x2d),
            list: Color32::from_rgb(0x27, 0x29, 0x2d),
            right: Color32::from_rgb(0x18, 0x19, 0x1a),
            ink: Color32::from_white_alpha(230),
            muted: Color32::from_white_alpha(128),
            faint: Color32::from_white_alpha(89),
            line: Color32::from_white_alpha(26),
            accent: Color32::from_rgb(0xae, 0xb1, 0xdd),
            accent_ink: Color32::from_white_alpha(250),
            selected: Color32::from_rgb(0x46, 0x4b, 0x53),
            hover: Color32::from_rgb(0x46, 0x4b, 0x53),
            card: Color32::from_rgb(0x36, 0x3a, 0x40),
            danger: Color32::from_rgb(0xef, 0x8e, 0x8e),
            unread: Color32::from_rgb(0x72, 0xc0, 0xea),
            star: Color32::from_rgb(0xff, 0xcb, 0x33),
            account_dot: Color32::from_rgb(0x8d, 0x93, 0x9c),
            disc: Color32::from_rgb(0x55, 0x5b, 0x64),
            icon: Color32::from_white_alpha(77),
            icon_hover: Color32::from_white_alpha(128),
            shadow: Color32::from_black_alpha(90),
            scrim: Color32::from_black_alpha(60),
        }
    }

    /// The design system's theme for the same colours, so its components (label chips, icons,
    /// status dots, toasts) match.
    pub fn exo_theme(self) -> Theme {
        if self.dark { Theme::Carbon } else { Theme::Snow }
    }

    pub fn install(self, ctx: &egui::Context) {
        exo_ui::set_theme(ctx, self.exo_theme());
        ctx.set_theme(if self.dark { egui::Theme::Dark } else { egui::Theme::Light });
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = self.window;
        visuals.window_fill = self.panel;
        visuals.extreme_bg_color = if self.dark {
            Color32::from_rgb(0x20, 0x23, 0x28)
        } else {
            Color32::from_rgb(0xfa, 0xfa, 0xfa)
        };
        visuals.faint_bg_color = self.hover;
        visuals.override_text_color = Some(self.ink);
        visuals.hyperlink_color = Color32::from_rgb(0x54, 0xac, 0xdc);
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.45);
        visuals.selection.stroke = egui::Stroke::new(1.0, self.ink);
        visuals.window_stroke = egui::Stroke::new(1.0, self.line);
        visuals.window_corner_radius = egui::CornerRadius::same(8);
        visuals.menu_corner_radius = egui::CornerRadius::same(6);
        visuals.widgets.noninteractive.bg_fill = self.list;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.line);
        visuals.widgets.inactive.bg_fill = self.card;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.line);
        visuals.widgets.inactive.weak_bg_fill = self.card;
        visuals.widgets.hovered.bg_fill = self.hover;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, self.line);
        visuals.widgets.active.bg_fill = self.selected;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, self.accent);
        visuals.widgets.inactive.fg_stroke.color = self.ink;
        visuals.widgets.hovered.fg_stroke.color = self.ink;
        visuals.widgets.active.fg_stroke.color = self.ink;
        visuals.widgets.noninteractive.fg_stroke.color = self.ink;
        ctx.set_visuals(visuals);
        ctx.all_styles_mut(|style| {
            for widget in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widget.corner_radius = egui::CornerRadius::same(4);
            }
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.interact_size.y = 24.0;
            style.visuals.text_cursor.stroke.color = self.accent;
        });
    }
}
