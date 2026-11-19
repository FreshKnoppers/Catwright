//! UI translation. The UI text in the code is English; other languages come
//! from gettext catalogs under `app/i18n/` (`de.po`, `fr.po`, …), compiled
//! into the binary. A missing translation falls back to the English text, so an
//! incomplete catalog never shows a key or an empty label.
//!
//! The current language is process-wide (`set_lang`) and read on every
//! call, so switching it in the settings takes effect on the next frame.
//! Placeholders are named, `{name}`, and filled by `trf`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// A UI language. English is the source language of the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
    Fr,
    Es,
    It,
    Pt,
}

impl Lang {
    /// Every language, in the order the pickers list them.
    pub const ALL: [Lang; 6] = [Lang::En, Lang::De, Lang::Fr, Lang::Es, Lang::It, Lang::Pt];

    /// The code stored in the settings (`"en"`, `"de"`, `"fr"`, …).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::It => "it",
            Lang::Pt => "pt",
        }
    }

    /// A stored code back to a language; unknown codes are `None`.
    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The language's own name, as pickers show it (never translated).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Fr => "Français",
            Lang::Es => "Español",
            Lang::It => "Italiano",
            Lang::Pt => "Português",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Switch the UI language for the whole process.
pub fn set_lang(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// The current UI language (English until `set_lang` is called).
pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::De,
        2 => Lang::Fr,
        3 => Lang::Es,
        4 => Lang::It,
        5 => Lang::Pt,
        _ => Lang::En,
    }
}

const DE_PO: &str = include_str!("../i18n/de.po");
const FR_PO: &str = include_str!("../i18n/fr.po");
const ES_PO: &str = include_str!("../i18n/es.po");
const IT_PO: &str = include_str!("../i18n/it.po");
const PT_PO: &str = include_str!("../i18n/pt.po");

/// Every compiled catalog as `(file name, PO text)`, for the tests.
#[cfg(test)]
fn all_pos() -> [(&'static str, &'static str); 5] {
    [
        ("de.po", DE_PO),
        ("fr.po", FR_PO),
        ("es.po", ES_PO),
        ("it.po", IT_PO),
        ("pt.po", PT_PO),
    ]
}

/// Parse `po` on first use, then return the cached catalog.
fn po(
    po: &'static str,
    cell: &'static OnceLock<HashMap<String, String>>,
) -> &'static HashMap<String, String> {
    cell.get_or_init(|| parse_po(po))
}

fn catalog(lang: Lang) -> Option<&'static HashMap<String, String>> {
    static DE: OnceLock<HashMap<String, String>> = OnceLock::new();
    static FR: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ES: OnceLock<HashMap<String, String>> = OnceLock::new();
    static IT: OnceLock<HashMap<String, String>> = OnceLock::new();
    static PT: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Lang::En => None,
        Lang::De => Some(po(DE_PO, &DE)),
        Lang::Fr => Some(po(FR_PO, &FR)),
        Lang::Es => Some(po(ES_PO, &ES)),
        Lang::It => Some(po(IT_PO, &IT)),
        Lang::Pt => Some(po(PT_PO, &PT)),
    }
}

/// `english` in the current language.
pub fn tr(english: &'static str) -> &'static str {
    tr_in(lang(), english)
}

/// `english` in `lang` (tests and the language picker use this directly).
pub fn tr_in(lang: Lang, english: &'static str) -> &'static str {
    catalog(lang)
        .and_then(|c| c.get(english))
        .filter(|t| !t.is_empty())
        .map_or(english, String::as_str)
}

/// `english` in the current language with its `{name}` placeholders filled.
pub fn trf(english: &'static str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    fill(tr(english), args)
}

/// A number already formatted with a `.` decimal point, in the current
/// language's notation (every language here but English uses a decimal comma).
pub fn decimal(formatted: String) -> String {
    match lang() {
        Lang::En => formatted,
        Lang::De | Lang::Fr | Lang::Es | Lang::It | Lang::Pt => formatted.replace('.', ","),
    }
}

/// Replace each `{name}` in `text` with its value from `args`.
pub fn fill(text: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = text.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

/// The `msgid` → `msgstr` pairs of a gettext catalog. Only the subset this
/// project writes is read: `#` comments, single- and multi-line quoted
/// strings with `\"`, `\\`, `\n` and `\t`; the header entry (empty msgid)
/// and entries without a translation are skipped.
pub fn parse_po(text: &str) -> HashMap<String, String> {
    #[derive(PartialEq)]
    enum Field {
        None,
        Id,
        Str,
    }
    let mut out = HashMap::new();
    let mut id = String::new();
    let mut tr = String::new();
    let mut field = Field::None;
    let mut flush = |id: &mut String, tr: &mut String| {
        if !id.is_empty() && !tr.is_empty() {
            out.insert(std::mem::take(id), std::mem::take(tr));
        }
        id.clear();
        tr.clear();
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("msgid ") {
            flush(&mut id, &mut tr);
            id.push_str(&unquote(rest));
            field = Field::Id;
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            tr.push_str(&unquote(rest));
            field = Field::Str;
        } else if line.starts_with('"') {
            match field {
                Field::Id => id.push_str(&unquote(line)),
                Field::Str => tr.push_str(&unquote(line)),
                Field::None => {}
            }
        }
    }
    flush(&mut id, &mut tr);
    out
}

/// The content of one quoted PO string, escapes resolved.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or("");
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_po_subset() {
        let po = r#"
# header
msgid ""
msgstr "Content-Type: text/plain; charset=UTF-8\n"

msgid "New session"
msgstr "Neue Sitzung"

# a multi-line entry with escapes
msgid "Say \"hi\"\n"
"twice"
msgstr ""
"Sag \"hallo\"\n"
"zweimal"

msgid "Untranslated"
msgstr ""
"#;
        let c = parse_po(po);
        assert_eq!(
            c.get("New session").map(String::as_str),
            Some("Neue Sitzung")
        );
        assert_eq!(
            c.get("Say \"hi\"\ntwice").map(String::as_str),
            Some("Sag \"hallo\"\nzweimal")
        );
        assert!(!c.contains_key("Untranslated"), "empty msgstr is skipped");
        assert!(!c.contains_key(""), "the header is no entry");
    }

    #[test]
    fn missing_translations_fall_back_to_english() {
        assert_eq!(tr_in(Lang::En, "New session"), "New session");
        assert_eq!(
            tr_in(Lang::De, "a text nobody translated"),
            "a text nobody translated"
        );
    }

    #[test]
    fn placeholders_are_filled_by_name() {
        assert_eq!(
            fill("{n} of {total} files", &[("n", &3), ("total", &"7")]),
            "3 of 7 files"
        );
        assert_eq!(fill("no placeholder", &[("n", &3)]), "no placeholder");
    }

    #[test]
    fn language_codes_round_trip() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_code(l.code()), Some(l));
        }
        assert_eq!(Lang::from_code("xx"), None);
    }

    /// The English texts the code passes to `tr`/`trf`, read from the
    /// sources: `tr("…")` / `trf("…"` with a plain string literal.
    fn texts_in_sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut found = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for marker in ["tr(\"", "trf(\"", "tr_in(Lang::De, \""] {
                let mut rest = text.as_str();
                while let Some(at) = rest.find(marker) {
                    // `tr(` must not be the end of a longer name (`attr(`)
                    let before = rest[..at].chars().next_back();
                    rest = &rest[at + marker.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let mut lit = String::new();
                    let mut chars = rest.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => lit.push('\n'),
                                Some('t') => lit.push('\t'),
                                Some(o) => lit.push(o),
                                None => {}
                            },
                            c => lit.push(c),
                        }
                    }
                    found.push((f.display().to_string(), lit));
                }
            }
        }
        found
    }

    /// Every text the code shows has a translation in every catalog.
    #[test]
    fn every_catalog_covers_every_text_in_the_code() {
        let texts = texts_in_sources()
            .into_iter()
            // this module only holds samples and the scanner itself
            .filter(|(f, _)| !f.ends_with("i18n.rs"))
            .collect::<Vec<_>>();
        for (file, po) in all_pos() {
            let catalog = parse_po(po);
            let missing: Vec<String> = texts
                .iter()
                .filter(|(_, t)| !catalog.contains_key(t))
                .map(|(f, t)| format!("{t:?} ({f})"))
                .collect();
            assert!(
                missing.is_empty(),
                "missing in {file}:\n{}",
                missing.join("\n")
            );
        }
    }

    /// All catalogs translate exactly the same msgids (de.po is the
    /// reference the others were copied from).
    #[test]
    fn catalogs_have_identical_keys() {
        let de = parse_po(DE_PO);
        let reference: std::collections::HashSet<&str> = de.keys().map(String::as_str).collect();
        for (file, po) in all_pos() {
            let parsed = parse_po(po);
            let keys: std::collections::HashSet<&str> = parsed.keys().map(String::as_str).collect();
            assert_eq!(keys, reference, "key set differs in {file}");
        }
    }

    /// Every entry in every catalog keeps the placeholders of its English
    /// text, so a translation can never drop or misspell a `{name}`.
    #[test]
    fn every_catalog_keeps_every_placeholder() {
        let placeholders = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string()))
                .collect();
            v.sort();
            v
        };
        for (file, po) in all_pos() {
            for (en, translated) in parse_po(po) {
                assert_eq!(
                    placeholders(&en),
                    placeholders(&translated),
                    "placeholders differ in {file}: {en:?} -> {translated:?}"
                );
            }
        }
    }
}
