//! Fallback fonts: egui's built-in fonts lack some symbols used in the UI
//! (e.g. U+21B5, U+21E7, U+2191, U+25A0, U+25CF). System font files are
//! appended after the built-in fonts in every family, so the default look is
//! unchanged and only missing glyphs come from the fallback.
//!
//! egui has no bold font, and `RichText::strong()` only changes the colour.
//! Real bold text uses the font family `bold()`: a system bold font, followed
//! by the normal proportional fonts for glyphs it lacks (or only those, if no
//! bold font is installed, so the family always exists).

use std::path::Path;

use egui::{FontDefinitions, FontFamily};

/// Candidate system font files, most preferred first (per distro layout).
pub const FALLBACK_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",                // Arch
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",    // Debian/Ubuntu
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",  // Fedora
    "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf", // Arch
    "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf", // Debian
    "/usr/share/fonts/google-noto/NotoSansSymbols2-Regular.ttf", // Fedora
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];

/// Candidate bold font files, most preferred first.
pub const BOLD_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",          // Arch
    "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf", // Debian/Ubuntu
    "/usr/share/fonts/google-noto/NotoSans-Bold.ttf",   // Fedora
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",         // Arch
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", // Debian/Ubuntu
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf", // Fedora
    "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
];

/// Preferred text fonts for the UI, most preferred first: Inter-style
/// grotesques stay legible at the small sizes the egui default (a light
/// Ubuntu) is too thin for. Used only when installed, nothing is bundled.
pub const UI_FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/inter/InterVariable.ttf",
    "/usr/share/fonts/TTF/Inter-Regular.ttf",
    "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
    "/usr/share/fonts/opentype/inter/Inter-Regular.otf", // Debian/Ubuntu (fonts-inter)
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
];

/// Name of the bold font family.
const BOLD_FAMILY: &str = "bold";

/// The font family for bold text (always defined after `install_fallback_fonts`).
pub fn bold() -> FontFamily {
    FontFamily::Name(BOLD_FAMILY.into())
}

/// Files larger than this are not loaded as fallback fonts.
const MAX_FONT_FILE_BYTES: u64 = 30 * 1024 * 1024;

/// Returns up to `max` existing candidate paths, in candidate order.
pub fn pick_fallbacks(
    candidates: &[&str],
    exists: impl Fn(&str) -> bool,
    max: usize,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|p| exists(p))
        .take(max)
        .map(|p| (*p).to_owned())
        .collect()
}

/// Reads a font file, skipping unreadable files and files over the size cap.
fn read_font_file(path: &str) -> Option<Vec<u8>> {
    match crate::store::read_regular_capped(Path::new(path), MAX_FONT_FILE_BYTES) {
        crate::store::CappedRead::Bytes(b) => Some(b),
        _ => None,
    }
}

/// Default font definitions with the picked fallback fonts appended at the
/// end of both `Proportional` and `Monospace`, plus the `bold()` family.
pub fn fallback_font_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let paths = pick_fallbacks(FALLBACK_FONT_CANDIDATES, |p| Path::new(p).is_file(), 2);
    let mut index = 0;
    for path in &paths {
        let Some(bytes) = read_font_file(path) else {
            continue;
        };
        let name = format!("fallback-{index}");
        index += 1;
        defs.font_data
            .insert(name.clone(), egui::FontData::from_owned(bytes));
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .push(name.clone());
        defs.families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(name);
    }
    let mut bold_names = Vec::new();
    for path in pick_fallbacks(BOLD_FONT_CANDIDATES, |p| Path::new(p).is_file(), 1) {
        if let Some(bytes) = read_font_file(&path) {
            defs.font_data
                .insert("bold-0".into(), egui::FontData::from_owned(bytes));
            bold_names.push("bold-0".to_string());
        }
    }
    bold_names.extend(
        defs.families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    defs.families.insert(bold(), bold_names);
    defs
}

/// The fallback definitions with the preferred UI font in front of the
/// proportional text (and behind the real bold in the bold family). Without
/// an installed candidate this is exactly the fallback set.
pub fn ui_font_definitions() -> FontDefinitions {
    let mut defs = fallback_font_definitions();
    let picked = pick_fallbacks(UI_FONT_CANDIDATES, |p| Path::new(p).is_file(), 1);
    if let Some(bytes) = picked.first().and_then(|p| read_font_file(p)) {
        defs.font_data
            .insert("ui-0".into(), egui::FontData::from_owned(bytes));
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "ui-0".to_string());
        if let Some(bold_family) = defs.families.get_mut(&bold()) {
            let after_bold = usize::from(bold_family.first().is_some_and(|n| n == "bold-0"));
            bold_family.insert(after_bold, "ui-0".to_string());
        }
    }
    defs
}

/// Installs the UI fonts on the context.
pub fn install_fallback_fonts(ctx: &egui::Context) {
    ctx.set_fonts(ui_font_definitions());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_font_goes_first_and_keeps_the_fallbacks_behind_it() {
        let ui = ui_font_definitions();
        let base = fallback_font_definitions();
        let names = &ui.families[&FontFamily::Proportional];
        if pick_fallbacks(UI_FONT_CANDIDATES, |p| Path::new(p).is_file(), 1).is_empty() {
            assert_eq!(names, &base.families[&FontFamily::Proportional]);
        } else {
            assert_eq!(names[0], "ui-0");
            assert_eq!(
                &names[1..],
                base.families[&FontFamily::Proportional].as_slice()
            );
        }
    }

    #[test]
    fn pick_fallbacks_keeps_order_and_respects_max() {
        let candidates = ["a", "b", "c", "d"];
        let existing = ["a", "b", "c", "d"];
        let picked = pick_fallbacks(&candidates, |p| existing.contains(&p), 2);
        assert_eq!(picked, vec!["a".to_string(), "b".to_string()]);
        let picked = pick_fallbacks(&candidates, |p| existing.contains(&p), 10);
        assert_eq!(picked.len(), 4);
        assert_eq!(picked[2], "c");
        assert_eq!(picked[3], "d");
    }

    #[test]
    fn pick_fallbacks_skips_missing_entries() {
        let picked = pick_fallbacks(&["a", "b", "c"], |p| p != "b", 10);
        assert_eq!(picked, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn pick_fallbacks_returns_empty_when_nothing_exists() {
        let picked = pick_fallbacks(&["a", "b"], |_| false, 2);
        assert!(picked.is_empty());
    }

    #[test]
    fn fallback_font_definitions_keeps_defaults_first_and_appends_fallbacks() {
        let defs = fallback_font_definitions();
        let defaults = FontDefinitions::default();
        let expected = pick_fallbacks(FALLBACK_FONT_CANDIDATES, |p| Path::new(p).is_file(), 2);
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let default_names = &defaults.families[&family];
            let names = &defs.families[&family];
            // The egui default fonts stay first, in their original order.
            assert!(names.len() >= default_names.len());
            assert_eq!(&names[..default_names.len()], default_names.as_slice());
            // Each found fallback file is appended at the end as fallback-N.
            let appended = &names[default_names.len()..];
            assert_eq!(appended.len(), expected.len());
            for (i, name) in appended.iter().enumerate() {
                assert_eq!(name, &format!("fallback-{i}"));
                assert!(defs.font_data.contains_key(name.as_str()));
            }
        }
    }

    #[test]
    fn bold_family_always_exists_and_falls_back_to_proportional() {
        let defs = fallback_font_definitions();
        let names = &defs.families[&bold()];
        let prop = &defs.families[&FontFamily::Proportional];
        let found = pick_fallbacks(BOLD_FONT_CANDIDATES, |p| Path::new(p).is_file(), 1);
        if found.is_empty() {
            assert_eq!(names, prop);
        } else {
            assert_eq!(names[0], "bold-0");
            assert!(defs.font_data.contains_key("bold-0"));
            assert_eq!(&names[1..], prop.as_slice());
        }
        // usable in a context: laying out bold text must not panic
        let ctx = egui::Context::default();
        install_fallback_fonts(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let w = ctx.fonts(|f| f.glyph_width(&egui::FontId::new(15.0, bold()), 'W'));
        assert!(w > 0.0);
    }

    #[test]
    fn installed_fallbacks_provide_the_missing_glyphs() {
        let found = pick_fallbacks(FALLBACK_FONT_CANDIDATES, |p| Path::new(p).is_file(), 2);
        if found.is_empty() {
            return;
        }
        let ctx = egui::Context::default();
        install_fallback_fonts(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        for c in "↵⇧↑■●".chars() {
            let has = ctx.fonts(|f| f.has_glyph(&egui::FontId::proportional(14.0), c));
            assert!(has, "missing glyph U+{:04X}", c as u32);
        }
    }
}
