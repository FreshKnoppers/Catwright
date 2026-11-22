//! Context-window breakdown for a Vibe session.
//!
//! vibe-acp reports only the total (`usage_update.used/size`). Vibe itself
//! measures what fills the context (system prompt, tool definitions, skills,
//! conversation) and saves the character sizes per session to
//! `$VIBE_HOME/logs/session/unified/<session>/context-breakdown.json`
//! (default `$VIBE_HOME` = `~/.vibe`). We read that file (read-only) and split
//! the used tokens proportionally, like Vibe's own `/context` does
//! (largest-remainder rounding). The result is an estimate.

use crate::i18n::tr;
use std::path::PathBuf;

/// Character sizes of the four context slices, as Vibe saved them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextSizes {
    pub system: u64,
    pub tools: u64,
    pub skills: u64,
    pub conversation: u64,
}

/// Display names in the current language, in the order of
/// `ContextSizes::as_array`.
pub fn labels() -> [&'static str; 4] {
    [
        tr("System prompt"),
        tr("Tools"),
        tr("Skills"),
        tr("Conversation"),
    ]
}

impl ContextSizes {
    pub fn as_array(&self) -> [u64; 4] {
        [self.system, self.tools, self.skills, self.conversation]
    }
}

/// Parse Vibe's `context-breakdown.json` (`{"system":…,"tools":…,"skills":…,"conversation":…}`).
pub fn parse_sizes(json: &str) -> Option<ContextSizes> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let get = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let sizes = ContextSizes {
        system: get("system"),
        tools: get("tools"),
        skills: get("skills"),
        conversation: get("conversation"),
    };
    (sizes.as_array().iter().sum::<u64>() > 0).then_some(sizes)
}

/// Path of the saved sizes for `session_id`, or None for an id that is not
/// a plain file name (the id comes from the agent: never build a path from
/// anything but `[A-Za-z0-9_-]`).
pub fn sizes_path(session_id: &str) -> Option<PathBuf> {
    let safe = !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return None;
    }
    Some(
        super::catalog::vibe_home()?
            .join("logs/session/unified")
            .join(session_id)
            .join("context-breakdown.json"),
    )
}

/// Read the saved sizes of a session (None if missing or unreadable).
pub fn load_sizes(session_id: &str) -> Option<ContextSizes> {
    let path = sizes_path(session_id)?;
    match crate::store::read_regular_capped(&path, 64 * 1024) {
        crate::store::CappedRead::Bytes(b) => parse_sizes(&String::from_utf8_lossy(&b)),
        _ => None,
    }
}

/// Split `used` tokens over the four slices in proportion to their sizes,
/// with largest-remainder rounding so the parts sum to `used` exactly.
pub fn split_tokens(used: u64, sizes: &ContextSizes) -> [u64; 4] {
    let weights = sizes.as_array();
    let total: u64 = weights.iter().sum();
    if total == 0 || used == 0 {
        return [0; 4];
    }
    let mut parts = [0u64; 4];
    let mut rems: Vec<(u128, usize)> = Vec::with_capacity(4);
    for (i, w) in weights.iter().enumerate() {
        let exact = used as u128 * *w as u128;
        parts[i] = (exact / total as u128) as u64;
        rems.push((exact % total as u128, i));
    }
    let mut missing = used - parts.iter().sum::<u64>();
    rems.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, i) in rems {
        if missing == 0 {
            break;
        }
        parts[i] += 1;
        missing -= 1;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vibes_file() {
        let s =
            parse_sizes(r#"{"system": 18643, "tools": 15406, "skills": 0, "conversation": 29671}"#)
                .unwrap();
        assert_eq!(s.as_array(), [18643, 15406, 0, 29671]);
        assert!(parse_sizes(r#"{"system":0,"tools":0}"#).is_none());
        assert!(parse_sizes("nope").is_none());
    }

    #[test]
    fn split_sums_to_used_and_follows_weights() {
        let s = ContextSizes {
            system: 18643,
            tools: 15406,
            skills: 0,
            conversation: 29671,
        };
        let parts = split_tokens(10_065, &s);
        assert_eq!(parts.iter().sum::<u64>(), 10_065);
        assert_eq!(parts[2], 0);
        assert!(parts[3] > parts[0] && parts[0] > parts[1]);
        assert_eq!(split_tokens(0, &s), [0; 4]);
        assert_eq!(split_tokens(5, &ContextSizes::default()), [0; 4]);
        // rounding: 3 tokens over equal weights
        let eq = ContextSizes {
            system: 1,
            tools: 1,
            skills: 1,
            conversation: 0,
        };
        assert_eq!(split_tokens(3, &eq), [1, 1, 1, 0]);
        assert_eq!(split_tokens(2, &eq).iter().sum::<u64>(), 2);
    }

    #[test]
    fn session_ids_cannot_escape_the_sessions_dir() {
        assert!(sizes_path("1a1388b4-26aa-5987-321b-91e8521fa84c").is_some());
        assert!(sizes_path("child-f5de1439c3fca37bcaaad42b").is_some());
        assert!(sizes_path("../etc").is_none());
        assert!(sizes_path("a/b").is_none());
        assert!(sizes_path("").is_none());
        assert!(sizes_path(&"x".repeat(200)).is_none());
    }
}
