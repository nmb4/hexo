//! Exo's font families, matched the way a stylesheet's `@font-face` rules declare them, drawn in
//! Exo's fonts (`exo-fonts`).
//!
//! The rules don't always map a weight to the obvious file: [`Family::Sans`] (the UI face) at
//! weight 700 is the *Semibold* cut, while [`Family::SansEmail`] (message bodies, the editor,
//! signatures) gets the true Bold. [`Font::face`] reproduces those rules and CSS's weight-matching
//! algorithm, so a CSS weight resolves to the face Chromium would pick.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use egui::{FontData, FontDefinitions, FontFamily, epaint::text::VariationCoords};
use skrifa::{FontRef, MetadataProvider as _, raw::TableProvider as _};

use crate::AssetError;

/// A CSS `font-family`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// The UI face: Exo Sans.
    Sans,
    /// Message bodies, the compose editor, signatures: Exo Sans, with its true Bold.
    SansEmail,
    /// The UI's monospaced face: Exo Mono.
    Mono,
    /// Monospaced text in messages: Exo Mono Email.
    MonoEmail,
    /// The macOS system font, used by the tab bar.
    System,
    /// The other style's UI face, which Classic draws only inside a component forced into that
    /// style's light theme (the Out of Office Reply dialog's): Exo Sans Alt.
    SansAlt,
    /// The `monospace` generic family: Courier, Chromium's fixed-width font on macOS (the raw
    /// source viewer's `12px monospace`).
    Monospace,
    /// Arial, the font Chromium gives an unstyled `<button>` (the editor's image resizer's `Small`
    /// and `Best Fit`, `13.333px Arial` bold). Only its bold is loaded.
    Arial,
    /// The `sans-serif` generic family: Helvetica, Chromium's on macOS (the compose form's `…` that
    /// shows the quoted text).
    SansSerif,
}

/// A font: one of Exo's (`exo-fonts`), or one of macOS's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs, reason = "each variant is the face its name says")]
pub enum Face {
    SansLight,
    SansRegular,
    SansItalic,
    SansSemibold,
    SansSemiboldItalic,
    SansBold,
    SansBoldItalic,
    Mono,
    MonoEmail,
    SystemUi,
    SansAltLight,
    SansAltRegular,
    SansAltItalic,
    SansAltSemibold,
    SansAltSemiboldItalic,
    Courier,
    ArialBold,
    Helvetica,
}

/// Where a face's file comes from.
enum Source {
    /// Compiled in.
    Exo(&'static exo_fonts::Font),
    /// macOS's: its PostScript name and file.
    System(&'static str, &'static str),
}

impl Face {
    const ALL: [Self; 18] = [
        Self::SansLight,
        Self::SansRegular,
        Self::SansItalic,
        Self::SansSemibold,
        Self::SansSemiboldItalic,
        Self::SansBold,
        Self::SansBoldItalic,
        Self::Mono,
        Self::MonoEmail,
        Self::SystemUi,
        Self::SansAltLight,
        Self::SansAltRegular,
        Self::SansAltItalic,
        Self::SansAltSemibold,
        Self::SansAltSemiboldItalic,
        Self::Courier,
        Self::ArialBold,
        Self::Helvetica,
    ];

    fn source(self) -> Source {
        use exo_fonts::*;
        match self {
            Self::SansLight => Source::Exo(&SANS_LIGHT),
            Self::SansRegular => Source::Exo(&SANS_REGULAR),
            Self::SansItalic => Source::Exo(&SANS_ITALIC),
            Self::SansSemibold => Source::Exo(&SANS_SEMIBOLD),
            Self::SansSemiboldItalic => Source::Exo(&SANS_SEMIBOLD_ITALIC),
            Self::SansBold => Source::Exo(&SANS_BOLD),
            Self::SansBoldItalic => Source::Exo(&SANS_BOLD_ITALIC),
            Self::Mono => Source::Exo(&MONO),
            Self::MonoEmail => Source::Exo(&MONO_EMAIL),
            Self::SystemUi => Source::System(".SFNS-Regular", "/System/Library/Fonts/SFNS.ttf"),
            Self::SansAltLight => Source::Exo(&SANS_ALT_LIGHT),
            Self::SansAltRegular => Source::Exo(&SANS_ALT_REGULAR),
            Self::SansAltItalic => Source::Exo(&SANS_ALT_ITALIC),
            Self::SansAltSemibold => Source::Exo(&SANS_ALT_SEMIBOLD),
            Self::SansAltSemiboldItalic => Source::Exo(&SANS_ALT_SEMIBOLD_ITALIC),
            Self::Courier => Source::System("Courier", "/System/Library/Fonts/Courier.ttc"),
            Self::ArialBold => Source::System(
                "Arial-BoldMT",
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            ),
            Self::Helvetica => Source::System("Helvetica", HELVETICA),
        }
    }

    /// The face's PostScript name, which is also its egui family name.
    pub fn postscript_name(self) -> &'static str {
        match self.source() {
            Source::Exo(font) => font.name,
            Source::System(name, _) => name,
        }
    }

    /// Axis values for variable faces at `size` px.
    pub(crate) fn variations(self, size: f32) -> VariationCoords {
        match self {
            // `font-optical-sizing: auto` sets `opsz` to the font size (the font clamps it to 17).
            // The system font only appears in the shell's tab bar, which leaves font smoothing on,
            // so CoreText thickens its stems; drawing at 485 matches that ink at 12.8px.
            Self::SystemUi => VariationCoords::new([(b"wght", 485.0), (b"opsz", size)]),
            _ => VariationCoords::default(),
        }
    }

    pub(crate) fn egui_family(self) -> FontFamily {
        FontFamily::Name(self.postscript_name().into())
    }

    /// The face as `font-variant-ligatures: none` draws it, a copy whose ligature and contextual
    /// alternate features are switched off (see [`without_ligatures`]).
    fn egui_family_without_ligatures(self) -> FontFamily {
        FontFamily::Name(format!("{}{NO_LIGATURES}", self.postscript_name()).into())
    }

    /// Whether the face is a bold cut, which takes the bold cut of each system fallback.
    fn is_bold(self) -> bool {
        use Face::*;
        matches!(
            self,
            SansSemibold
                | SansSemiboldItalic
                | SansBold
                | SansBoldItalic
                | SansAltSemibold
                | SansAltSemiboldItalic
                | ArialBold
        )
    }
}

impl Family {
    /// This family's `@font-face` rules: `(weight, italic, face)`.
    fn rules(self) -> &'static [(u16, bool, Face)] {
        use Face::*;
        match self {
            Self::Sans => &[
                (100, false, SansLight),
                (400, false, SansRegular),
                (400, true, SansItalic),
                (700, false, SansSemibold),
                (700, true, SansSemiboldItalic),
            ],
            Self::SansEmail => &[
                (400, false, SansRegular),
                (400, true, SansItalic),
                (700, false, SansBold),
                (700, true, SansBoldItalic),
            ],
            Self::Mono => &[(400, false, Mono)],
            Self::MonoEmail => &[(400, false, MonoEmail)],
            Self::System => &[(400, false, SystemUi)],
            Self::Monospace => &[(400, false, Courier)],
            Self::Arial => &[(700, false, ArialBold)],
            Self::SansSerif => &[(400, false, Helvetica)],
            // Its 700 is the SemiBold cut, as `Sans`'s is.
            Self::SansAlt => &[
                (100, false, SansAltLight),
                (400, false, SansAltRegular),
                (400, true, SansAltItalic),
                (700, false, SansAltSemibold),
                (700, true, SansAltSemiboldItalic),
            ],
        }
    }
}

/// A CSS font request: `font-family`, `font-weight`, `font-style: italic` and
/// `font-variant-ligatures`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Font {
    /// The `font-family`.
    pub family: Family,
    /// The `font-weight` (`normal` is 400, `bold` is 700).
    pub weight: u16,
    /// Whether `font-style: italic` is set.
    pub italic: bool,
    /// Whether ligatures and contextual alternates apply (`font-variant-ligatures: none` turns
    /// them off).
    pub ligatures: bool,
}

impl Font {
    /// The upright face of `family` at `weight`.
    pub const fn new(family: Family, weight: u16) -> Self {
        Self {
            family,
            weight,
            italic: false,
            ligatures: true,
        }
    }

    /// The same request with `font-style: italic`.
    pub const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    /// The same request with `font-variant-ligatures: none`, as inputs, textareas and the editor
    /// have it, with no `liga`, `clig`, `dlig` or `calt`, so Exo Sans's `fi` and `fl` don't join.
    pub const fn without_ligatures(self) -> Self {
        Self {
            ligatures: false,
            ..self
        }
    }

    /// The egui family that draws `face` (this request's [`Font::face`]) for this request.
    pub(crate) fn egui_family(self, face: Face) -> FontFamily {
        if self.ligatures {
            face.egui_family()
        } else {
            face.egui_family_without_ligatures()
        }
    }

    /// The face Chromium would use, and whether it has to slant an upright face to fake italics
    /// (when the family has no italic file).
    pub fn face(self) -> (Face, bool) {
        let rules = self.family.rules();
        let styled: Vec<_> = rules
            .iter()
            .filter(|(_, italic, _)| *italic == self.italic)
            .collect();
        let (candidates, synthetic_italic) = if styled.is_empty() {
            (rules.iter().collect(), self.italic)
        } else {
            (styled, false)
        };
        let (_, _, face) = candidates
            .into_iter()
            .min_by_key(|(weight, _, _)| weight_preference(self.weight, *weight))
            .expect("every family declares at least one face");
        (*face, synthetic_italic)
    }
}

/// CSS Fonts 4 weight matching, as a sort key where lower is better.
fn weight_preference(desired: u16, available: u16) -> (u8, u16) {
    let distance = desired.abs_diff(available);
    let tier = if (400..=500).contains(&desired) {
        // Up to 500 first, then lighter, then heavier.
        if (desired..=500).contains(&available) {
            0
        } else if available < desired {
            1
        } else {
            2
        }
    } else if desired < 400 {
        u8::from(available > desired)
    } else {
        u8::from(available < desired)
    };
    (tier, distance)
}

/// A face's vertical metrics from its `hhea` table, in ems. CoreText (and so Chromium on macOS)
/// lays out lines with these, ignoring the OS/2 typo metrics some of these files also carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VerticalMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
}

/// Every face, loaded and measured, ready to hand to egui.
pub(crate) struct FontBook {
    pub(crate) definitions: FontDefinitions,
    pub(crate) metrics: HashMap<Face, VerticalMetrics>,
    /// The system emoji font's cell, where there is one.
    pub(crate) emoji: Option<crate::emoji::Cell>,
}

impl FontBook {
    /// Loads every face: Exo's compiled in, the system UI font, Courier, Arial Bold and Helvetica
    /// from macOS.
    pub(crate) fn load() -> Result<Self, AssetError> {
        let mut definitions = FontDefinitions::empty();
        let mut metrics = HashMap::new();
        for face in Face::ALL {
            let name = face.postscript_name();
            let data = match face.source() {
                Source::Exo(font) => FontData::from_static(font.data),
                Source::System(_, path) => match std::fs::read(path) {
                    Ok(bytes) => FontData::from_owned(bytes),
                    // macOS system faces are absent elsewhere: fall back to a compiled-in Exo
                    // face so fonts, icons and image loaders still get installed.
                    Err(_) => FontData::from_static(match face {
                        Face::Courier => exo_fonts::MONO.data,
                        Face::ArialBold => exo_fonts::SANS_BOLD.data,
                        Face::SystemUi | Face::Helvetica => exo_fonts::SANS_REGULAR.data,
                        _ => exo_fonts::SANS_REGULAR.data,
                    }),
                },
            };
            let bytes: &[u8] = &data.font;
            // A collection's face is the one with the face's name (Courier.ttc holds four).
            let index = (0..)
                .map_while(|index| {
                    FontRef::from_index(bytes, index)
                        .ok()
                        .map(|font| (index, font))
                })
                .find_map(|(index, font)| instance_named(&font, name).map(|_| index))
                .unwrap_or(0);
            metrics.insert(face, measure(name, bytes, index)?);

            for (key, mut data) in [
                (
                    format!("{name}{NO_LIGATURES}"),
                    FontData::from_owned(without_ligatures(bytes)),
                ),
                (name.to_owned(), data),
            ] {
                data.index = index;
                // CoreText doesn't hint, and egui's hinting snaps stems and visibly changes the
                // shapes.
                data.tweak.hinting = Some(false);
                definitions.font_data.insert(key, Arc::new(data));
            }
        }
        let before_emoji = add_system_fonts(&mut definitions, &BEFORE_EMOJI);
        let after_emoji = add_system_fonts(&mut definitions, &AFTER_EMOJI);
        let emoji = crate::emoji::load().map(|(data, cell)| {
            definitions
                .font_data
                .insert(crate::emoji::FONT_NAME.into(), Arc::new(data));
            cell
        });
        let emoji_font = emoji.map(|_| crate::emoji::FONT_NAME.to_owned());
        for face in Face::ALL {
            let pick = |fonts: &[(String, String)]| -> Vec<String> {
                fonts
                    .iter()
                    .map(|(regular, bold)| {
                        if face.is_bold() {
                            bold.clone()
                        } else {
                            regular.clone()
                        }
                    })
                    .collect()
            };
            let mut fallbacks = pick(&before_emoji);
            fallbacks.extend(emoji_font.clone());
            fallbacks.extend(pick(&after_emoji));
            let name = face.postscript_name();
            definitions.families.insert(
                face.egui_family(),
                std::iter::once(name.to_owned())
                    .chain(fallbacks.clone())
                    .collect(),
            );
            definitions.families.insert(
                face.egui_family_without_ligatures(),
                std::iter::once(format!("{name}{NO_LIGATURES}"))
                    .chain(fallbacks)
                    .collect(),
            );
        }
        // egui's own widgets (and anything unstyled) default to the UI face.
        let regular = Face::SansRegular.postscript_name().to_owned();
        let with_emoji = |fonts: Vec<String>| fonts.into_iter().chain(emoji_font.clone()).collect();
        definitions
            .families
            .insert(FontFamily::Proportional, with_emoji(vec![regular.clone()]));
        definitions.families.insert(
            FontFamily::Monospace,
            with_emoji(vec![Face::Mono.postscript_name().into(), regular]),
        );
        Ok(Self {
            definitions,
            metrics,
            emoji,
        })
    }
}

/// What a face's ligature-free copy adds to its name.
const NO_LIGATURES: &str = "+noliga";

/// The GSUB features `font-variant-ligatures: none` turns off.
const LIGATURE_FEATURES: [&[u8; 4]; 4] = [b"liga", b"clig", b"dlig", b"calt"];

/// A copy of the font file `bytes` whose ligature features (`LIGATURE_FEATURES`) are renamed in
/// its GSUB feature list, so a shaper no longer applies them. The glyphs and every other feature
/// (kerning included) stay. A file without GSUB is copied as is.
fn without_ligatures(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let Some(gsub) = FontRef::new(bytes)
        .ok()
        .and_then(|font| font.table_data(skrifa::Tag::new(b"GSUB")))
    else {
        return out;
    };
    let gsub = gsub.as_bytes();
    // The table's place in the file, then its header: versions, ScriptList and FeatureList
    // offsets. The FeatureList is a count and 6-byte records whose first 4 bytes are the tag.
    let start = gsub.as_ptr() as usize - bytes.as_ptr() as usize;
    let read_u16 = |at: usize| {
        out.get(at..at + 2)
            .map(|pair| usize::from(u16::from_be_bytes([pair[0], pair[1]])))
    };
    let Some(list) = read_u16(start + 6).map(|offset| start + offset) else {
        return out;
    };
    let count = read_u16(list).unwrap_or(0);
    for record in (0..count).map(|i| list + 2 + 6 * i) {
        if let Some(tag) = out.get_mut(record..record + 4)
            && LIGATURE_FEATURES.iter().any(|feature| tag == &feature[..])
        {
            // A tag no shaper turns on by default.
            tag[0] = b'_';
        }
    }
    out
}

/// A macOS font Chromium falls back to, as `(PostScript name, file)` for its regular and its bold
/// (which bold faces take, as Chromium matches the weight).
type SystemFont = [(&'static str, &'static str); 2];

/// Where Chromium finds what Exo's fonts lack, in the order it looks (measured with
/// `CSS.getPlatformFontsForNode` on the UI face's stack, which falls back to `sans-serif`, then
/// `"Apple Color Emoji"`): `sans-serif`, which is Helvetica (Greek, Georgian, rarer Latin), then
/// CoreText's cascade, script by script. Lucida Grande (Hebrew, `⌘`, `✓`) draws no emoji, so it can
/// come before the emoji font.
const BEFORE_EMOJI: [SystemFont; 2] = [
    [("Helvetica", HELVETICA), ("Helvetica-Bold", HELVETICA)],
    [
        ("LucidaGrande", LUCIDA_GRANDE),
        ("LucidaGrande-Bold", LUCIDA_GRANDE),
    ],
];

/// The rest of CoreText's cascade, after the emoji font because some of these draw emoji
/// (`☕`, `⚪`, `🈚`) that Chromium takes from Apple Color Emoji. A script's font comes before any
/// other here that also covers it: Chinese takes Hiragino Sans GB over Hiragino Sans, `★`
/// Hiragino over STIX.
const AFTER_EMOJI: [SystemFont; 11] = [
    // Chinese (and kana, in the same design as Hiragino Sans's). Chromium draws Chinese with
    // PingFang SC, whose outlines are in Apple's `hvgl` table, which egui can't draw. This is
    // the system's other Simplified Chinese sans.
    [
        ("HiraginoSansGB-W3", HIRAGINO_GB),
        ("HiraginoSansGB-W6", HIRAGINO_GB),
    ],
    [
        (
            "HiraginoSans-W3",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        ),
        (
            "HiraginoSans-W6",
            "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc",
        ),
    ],
    [
        ("AppleSDGothicNeo-Regular", SD_GOTHIC_NEO),
        ("AppleSDGothicNeo-Bold", SD_GOTHIC_NEO),
    ],
    // Mathematical alphanumerics (`𝓇𝒶𝓌`). There is no bold, which Chromium fakes and egui can't.
    [
        ("STIXTwoMath-Regular", STIX_TWO_MATH),
        ("STIXTwoMath-Regular", STIX_TWO_MATH),
    ],
    [("GeezaPro", GEEZA_PRO), ("GeezaPro-Bold", GEEZA_PRO)],
    [
        ("KohinoorDevanagari-Regular", KOHINOOR),
        ("KohinoorDevanagari-Bold", KOHINOOR),
    ],
    [("Thonburi", THONBURI), ("Thonburi-Bold", THONBURI)],
    [
        ("KohinoorBangla-Regular", KOHINOOR_BANGLA),
        ("KohinoorBangla-Bold", KOHINOOR_BANGLA),
    ],
    [
        ("TamilSangamMN", TAMIL_SANGAM),
        ("TamilSangamMN-Bold", TAMIL_SANGAM),
    ],
    [
        ("NotoSansArmenian-Regular", NOTO_ARMENIAN),
        ("NotoSansArmenian-Bold", NOTO_ARMENIAN),
    ],
    [("KefaIII-Regular", KEFA), ("KefaIII-Bold", KEFA)],
];

const HELVETICA: &str = "/System/Library/Fonts/Helvetica.ttc";
const LUCIDA_GRANDE: &str = "/System/Library/Fonts/LucidaGrande.ttc";
const HIRAGINO_GB: &str = "/System/Library/Fonts/Hiragino Sans GB.ttc";
const SD_GOTHIC_NEO: &str = "/System/Library/Fonts/AppleSDGothicNeo.ttc";
const STIX_TWO_MATH: &str = "/System/Library/Fonts/Supplemental/STIXTwoMath.otf";
const GEEZA_PRO: &str = "/System/Library/Fonts/GeezaPro.ttc";
const KOHINOOR: &str = "/System/Library/Fonts/Kohinoor.ttc";
const THONBURI: &str = "/System/Library/Fonts/Supplemental/Thonburi.ttc";
const KOHINOOR_BANGLA: &str = "/System/Library/Fonts/KohinoorBangla.ttc";
const TAMIL_SANGAM: &str = "/System/Library/Fonts/Supplemental/Tamil Sangam MN.ttc";
const NOTO_ARMENIAN: &str = "/System/Library/Fonts/NotoSansArmenian.ttc";
const KEFA: &str = "/System/Library/Fonts/Supplemental/KefaIII.ttf";

/// Registers the faces of `fonts` the system has, and returns the egui names of each one's
/// regular and bold. Missing files or faces are left out (off macOS, all of them), and those
/// characters render as egui's replacement glyph.
fn add_system_fonts(
    definitions: &mut FontDefinitions,
    fonts: &[SystemFont],
) -> Vec<(String, String)> {
    let mut register = |(name, path): (&str, &'static str)| -> Option<String> {
        let bytes = mapped(path)?;
        let (index, coords) = (0..)
            .map_while(|index| {
                FontRef::from_index(bytes, index)
                    .ok()
                    .map(|font| (index, font))
            })
            .find_map(|(index, font)| Some((index, instance_named(&font, name)?)))?;
        let mut data = FontData::from_static(bytes);
        data.index = index;
        data.tweak.hinting = Some(false);
        data.tweak.coords = coords;
        definitions.font_data.insert(name.into(), Arc::new(data));
        Some(name.to_owned())
    };
    fonts
        .iter()
        .filter_map(|&[regular, bold]| {
            let regular = register(regular)?;
            Some((regular.clone(), register(bold).unwrap_or(regular)))
        })
        .collect()
}

/// The variation coordinates that make `font` the face PostScript-named `name`: none if that's
/// the font's own name, a named instance's if it's one of its instances (Kefa ships its weights
/// as instances of one variable face).
fn instance_named(font: &FontRef<'_>, name: &str) -> Option<VariationCoords> {
    let is_name = |id| {
        font.localized_strings(id)
            .english_or_first()
            .is_some_and(|found| found.chars().eq(name.chars()))
    };
    if is_name(skrifa::string::StringId::POSTSCRIPT_NAME) {
        return Some(VariationCoords::default());
    }
    let instance = font
        .named_instances()
        .iter()
        .find(|instance| instance.postscript_name_id().is_some_and(is_name))?;
    let tags = font
        .axes()
        .iter()
        .map(|axis| u32::from_be_bytes(axis.tag().to_be_bytes()));
    Some(VariationCoords::new(tags.zip(instance.user_coords())))
}

/// A system font file, mapped once per process. Mapping instead of reading keeps the large ones
/// (Apple SD Gothic Neo is 55 MB, Hiragino Sans GB 23 MB) out of memory, since only the tables and
/// glyphs in use are paged in.
pub(crate) fn mapped(path: &'static str) -> Option<&'static [u8]> {
    /// Each file's bytes, or `None` where it's missing.
    type Files = HashMap<&'static str, Option<&'static [u8]>>;
    static FILES: LazyLock<Mutex<Files>> = LazyLock::new(Mutex::default);
    let mut files = FILES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *files.entry(path).or_insert_with(|| {
        let file = std::fs::File::open(path).ok()?;
        // SAFETY: system fonts are on the sealed, read-only system volume, so nothing modifies
        // them while they're mapped.
        let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
        Some(&**Box::leak(Box::new(map)))
    })
}

fn measure(name: &'static str, bytes: &[u8], index: u32) -> Result<VerticalMetrics, AssetError> {
    let parse_error = |e: skrifa::raw::ReadError| AssetError::Parse {
        name,
        message: e.to_string(),
    };
    let font = FontRef::from_index(bytes, index).map_err(parse_error)?;
    let units_per_em = f32::from(font.head().map_err(parse_error)?.units_per_em());
    let hhea = font.hhea().map_err(parse_error)?;
    let ascent = f32::from(hhea.ascender().to_i16()) / units_per_em;
    let descent = -f32::from(hhea.descender().to_i16()) / units_per_em;
    // Blink on macOS gives Courier and Helvetica (as Times) 15% more ascent, to match the
    // metrics of their Windows counterparts (`SimpleFontData::PlatformInit`), so Courier's 12px
    // line is 14px, with the baseline 11px down.
    let ascent = if [Face::Courier, Face::Helvetica]
        .iter()
        .any(|face| face.postscript_name() == name)
    {
        ascent + 0.15 * (ascent + descent)
    } else {
        ascent
    };
    Ok(VerticalMetrics { ascent, descent })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_bold_is_the_semibold_file() {
        assert_eq!(
            Font::new(Family::Sans, 700).face(),
            (Face::SansSemibold, false)
        );
        assert_eq!(
            Font::new(Family::SansEmail, 700).face(),
            (Face::SansBold, false)
        );
    }

    #[test]
    fn weights_resolve_like_css() {
        let sans = |weight| Font::new(Family::Sans, weight).face().0;
        assert_eq!(sans(400), Face::SansRegular);
        assert_eq!(
            sans(500),
            Face::SansRegular,
            "400–500 prefers lighter faces up to 500"
        );
        assert_eq!(
            sans(600),
            Face::SansSemibold,
            "above 500 prefers heavier faces"
        );
        assert_eq!(
            sans(300),
            Face::SansLight,
            "below 400 prefers lighter faces"
        );
        assert_eq!(
            sans(900),
            Face::SansSemibold,
            "falls back to the heaviest available"
        );
        assert_eq!(
            Font::new(Family::SansEmail, 300).face().0,
            Face::SansRegular,
            "the email family has no Light"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn every_system_fallback_is_on_this_mac() {
        for fonts in [&BEFORE_EMOJI[..], &AFTER_EMOJI[..]] {
            let found = add_system_fonts(&mut FontDefinitions::empty(), fonts);
            let listed: Vec<_> = fonts
                .iter()
                .map(|[regular, bold]| (regular.0.to_owned(), bold.0.to_owned()))
                .collect();
            assert_eq!(found, listed);
        }
    }

    #[test]
    fn inputs_draw_without_ligatures() {
        let tags = |list: Result<
            skrifa::raw::tables::layout::FeatureList<'_>,
            skrifa::raw::ReadError,
        >|
         -> Vec<String> {
            list.expect("a feature list")
                .feature_records()
                .iter()
                .map(|record| record.feature_tag().to_string())
                .collect()
        };
        let bytes = exo_fonts::SANS_REGULAR.data;
        let plain = without_ligatures(bytes);
        let (font, plain) = (
            FontRef::new(bytes).expect("Exo Sans"),
            FontRef::new(&plain).expect("its copy"),
        );
        let before = tags(font.gsub().expect("GSUB").feature_list());
        assert!(before.iter().any(|tag| tag == "liga"), "{before:?}");
        // The ligature features lose their tags; every other feature keeps its own.
        let renamed: Vec<String> = before
            .iter()
            .map(|tag| {
                if LIGATURE_FEATURES
                    .iter()
                    .any(|feature| tag.as_bytes() == &feature[..])
                {
                    format!("_{}", &tag[1..])
                } else {
                    tag.clone()
                }
            })
            .collect();
        assert_eq!(tags(plain.gsub().expect("GSUB").feature_list()), renamed);
        assert!(
            tags(plain.gpos().expect("GPOS").feature_list())
                .iter()
                .any(|tag| tag == "kern"),
            "kerning stays"
        );
    }

    #[test]
    fn italics_are_synthesized_only_when_the_family_has_none() {
        assert_eq!(
            Font::new(Family::Sans, 400).italic().face(),
            (Face::SansItalic, false)
        );
        assert_eq!(
            Font::new(Family::Mono, 400).italic().face(),
            (Face::Mono, true)
        );
    }
}
