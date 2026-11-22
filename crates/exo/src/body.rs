//! Message bodies, laid out and painted by Blitz through `exo-mail`, then uploaded as an
//! egui texture. The pipeline does the sanitizing, linkifying, date marking, quote splitting
//! and remote-image decisions; Blitz turns the resulting HTML into pixels.

use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};

/// A cache of painted bodies. A thread can be repainted every frame; the pixels are only
/// re-laid-out when the width, theme or message changes.
pub struct BodyRenderer {
    fonts: exo_mail::Fonts,
    cache: HashMap<(u64, u32, bool), (TextureHandle, egui::Vec2)>,
}

impl Default for BodyRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyRenderer {
    pub fn new() -> Self {
        Self {
            fonts: exo_mail::Fonts::load(),
            cache: HashMap::new(),
        }
    }

    /// Paints `html` and returns the texture plus its size in logical points.
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        html: &str,
        width: f32,
        dark: bool,
    ) -> Option<(TextureHandle, egui::Vec2)> {
        if html.trim().is_empty() {
            return None;
        }
        let width_bucket = (width / 8.0).round() as u32 * 8;
        let key = (id, width_bucket, dark);
        if let Some(hit) = self.cache.get(&key) {
            return Some(hit.clone());
        }

        let message = exo_mail::Message {
            html: html.to_string(),
            ..exo_mail::Message::default()
        };
        let rendered = exo_mail::render(&message, None, &[]);
        let theme = if dark {
            exo_mail::Theme::Dark
        } else {
            exo_mail::Theme::Light
        };
        let mut view = exo_mail::BodyView::new(
            &rendered,
            exo_mail::ViewOptions {
                theme,
                ..exo_mail::ViewOptions::default()
            },
            &self.fonts,
            None,
        );
        let css_height = view.layout(width_bucket as f32, 2.0).min(8000.0);
        let image = view.paint(
            exo_mail::Region {
                top: 0.0,
                height: css_height,
                left: 0.0,
                width: view.content_width(),
                phase: [0.0; 2],
            },
            if dark {
                [32, 35, 40, 255]
            } else {
                [255, 255, 255, 255]
            },
        );
        if image.rgba.is_empty() || image.width == 0 || image.height == 0 {
            return None;
        }
        let pixels = ColorImage::from_rgba_premultiplied(
            [image.width as usize, image.height as usize],
            &image.rgba,
        );
        let texture = ctx.load_texture(
            format!("exo-body-{id}-{width_bucket}-{dark}"),
            pixels,
            TextureOptions::LINEAR,
        );
        let size = egui::vec2(image.width as f32 / 2.0, image.height as f32 / 2.0);
        let result = (texture, size);
        self.cache.insert(key, result.clone());
        Some(result)
    }
}

