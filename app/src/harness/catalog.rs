//! Thinking levels from Vibe's model catalogue.
//!
//! vibe-acp reports the thinking levels of the active model only
//! (`session/new` → `configOptions[thinking]`). Before a session runs on a
//! model, the start screen needs that model's levels too, or a model never
//! active before shows the levels of another one (e.g. no `max`).
//!
//! Vibe caches the catalogue it fetched in `$VIBE_HOME/model_catalog_cache.json`
//! (`{<key>: {"stored_at_timestamp": …, "payload": {"models": [{"id": …,
//! "parameters": {"reasoning": [...]}}]}}}`). We read that file (read-only)
//! and map the levels the way Vibe's catalogue layer does: the wire value
//! `none` is the level `off`, blank values are dropped, duplicates keep
//! their first place. A model without `reasoning` (or one not in the
//! catalogue, like a locally configured model) is left out: nothing is
//! known about it, so the caller falls back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Upper bound for the cache file (Vibe's own is a few KiB).
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;
/// Upper bound for the models and levels taken from it.
const MAX_MODELS: usize = 256;
const MAX_LEVELS: usize = 16;
const MAX_VALUE_LEN: usize = 128;

/// Vibe's home directory: `$VIBE_HOME`, else `~/.vibe`.
pub fn vibe_home() -> Option<PathBuf> {
    match std::env::var_os("VIBE_HOME") {
        Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => Some(directories::BaseDirs::new()?.home_dir().join(".vibe")),
    }
}

/// Thinking levels per model id from the cache file's JSON. With several
/// cached catalogues the most recently stored one wins.
pub fn parse_thinking_levels(json: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Ok(serde_json::Value::Object(entries)) = serde_json::from_str(json) else {
        return out;
    };
    let newest = entries
        .values()
        .filter_map(|e| {
            let models = e.get("payload")?.get("models")?.as_array()?;
            let stored = e
                .get("stored_at_timestamp")
                .and_then(|t| t.as_f64())
                .unwrap_or(0.0);
            Some((stored, models))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, models)) = newest else {
        return out;
    };
    for m in models.iter().take(MAX_MODELS) {
        let Some(id) = m.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if id.is_empty() || id.len() > MAX_VALUE_LEN {
            continue;
        }
        let Some(reasoning) = m
            .get("parameters")
            .and_then(|p| p.get("reasoning"))
            .and_then(|r| r.as_array())
        else {
            continue;
        };
        let mut levels: Vec<String> = Vec::new();
        for value in reasoning.iter().filter_map(|v| v.as_str()) {
            if value.trim().is_empty() || value.len() > MAX_VALUE_LEN {
                continue;
            }
            let level = standard_level(if value == "none" { "off" } else { value });
            if !levels.contains(&level) {
                levels.push(level);
            }
            if levels.len() == MAX_LEVELS {
                break;
            }
        }
        // Vibe reads an empty set as "no thinking control": only `off`
        if levels.is_empty() {
            levels.push("off".to_string());
        }
        out.insert(id.to_string(), levels);
    }
    out
}

/// Vibe's standard level names (`vibe.config_values.THINKING_LEVELS`).
const STANDARD_LEVELS: [&str; 6] = ["off", "low", "medium", "high", "xhigh", "max"];

/// A standard word in any case is the standard level; a model's own level
/// keeps its case (Vibe's `standard_thinking_level`).
fn standard_level(level: &str) -> String {
    let written = level.trim();
    let lower = written.to_lowercase();
    if STANDARD_LEVELS.contains(&lower.as_str()) {
        lower
    } else {
        written.to_string()
    }
}

/// Read the cached catalogue below `vibe_home` (empty if missing, not a
/// regular file, too large or unreadable).
pub fn load_thinking_levels(vibe_home: &Path) -> BTreeMap<String, Vec<String>> {
    let path = vibe_home.join("model_catalog_cache.json");
    let Some(path) = path.to_str() else {
        return BTreeMap::new();
    };
    match crate::artifacts::load_preview_capped(path, MAX_CATALOG_BYTES) {
        crate::artifacts::Preview::Image(bytes) => {
            parse_thinking_levels(&String::from_utf8_lossy(&bytes))
        }
        _ => BTreeMap::new(),
    }
}

/// The level Vibe starts a catalogue model on: `high` when offered, else
/// the last level (Vibe's `ModelConfig.default_thinking`).
pub fn default_level(levels: &[String]) -> Option<String> {
    if levels.iter().any(|l| l == "high") {
        return Some("high".to_string());
    }
    levels.last().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // shape of Vibe 3.x's model_catalog_cache.json (prices etc. trimmed)
    const CACHE: &str = r#"{"a8df": {"stored_at_timestamp": 1763827860.5, "payload": {"models": [
        {"id": "mistral-large-4-0", "label": "Mistral Large 4", "aliases": ["mistral-large-4"],
         "parameters": {"temperature": 1.0, "reasoning": ["low", "medium", "high"]}},
        {"id": "deepseek-v4-1-flash-1017", "label": "DeepSeek V4.1 Flash (Mistral Hosted)",
         "parameters": {"reasoning": ["none", "low", "high", "max"]}},
        {"id": "no-reasoning", "parameters": {"temperature": 1.0}},
        {"id": "empty-set", "parameters": {"reasoning": []}},
        {"id": "messy", "parameters": {"reasoning": [" High ", "", "high", 3, "MAX"]}}
    ]}}}"#;

    #[test]
    fn maps_levels_like_vibe() {
        let l = parse_thinking_levels(CACHE);
        assert_eq!(l["mistral-large-4-0"], ["low", "medium", "high"]);
        assert_eq!(l["deepseek-v4-1-flash-1017"], ["off", "low", "high", "max"]);
        assert!(!l.contains_key("no-reasoning"), "unknown stays unknown");
        assert_eq!(l["empty-set"], ["off"]);
        assert_eq!(l["messy"], ["high", "max"]);
        assert!(parse_thinking_levels("nope").is_empty());
        assert!(parse_thinking_levels("[]").is_empty());
    }

    #[test]
    fn the_newest_catalogue_wins() {
        let json = r#"{
            "old": {"stored_at_timestamp": 1, "payload": {"models": [
                {"id": "m", "parameters": {"reasoning": ["low"]}}]}},
            "new": {"stored_at_timestamp": 2, "payload": {"models": [
                {"id": "m", "parameters": {"reasoning": ["low", "max"]}}]}}
        }"#;
        assert_eq!(parse_thinking_levels(json)["m"], ["low", "max"]);
    }

    #[test]
    fn default_level_follows_vibe() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            default_level(&s(&["off", "low", "high", "max"])).as_deref(),
            Some("high")
        );
        assert_eq!(default_level(&s(&["off", "low"])).as_deref(), Some("low"));
        assert_eq!(default_level(&[]), None);
    }

    #[test]
    fn reads_the_file_below_vibe_home() {
        let dir = std::env::temp_dir().join(format!("catwright-catalog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_thinking_levels(&dir).is_empty(), "missing file");
        std::fs::write(dir.join("model_catalog_cache.json"), CACHE).unwrap();
        assert_eq!(
            load_thinking_levels(&dir)["mistral-large-4-0"],
            ["low", "medium", "high"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
