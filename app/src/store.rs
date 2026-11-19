//! Local, private persistence. All files live under the app config dir
//! (~/.config/catwright on Linux). The atomic
//! 0600 writer below is kept as the store's machinery for secret-bearing
//! files; the app itself holds no secrets (Vibe uses its own keyring
//! login). Nothing is ever logged.

use crate::i18n::{tr, trf};
use std::path::{Path, PathBuf};

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "catwright", "catwright")
        .map(|d| d.config_dir().to_path_buf())
}

/// The folder of the sessions without a project ("General").
pub fn chat_workspace_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("chat-workspace"))
}

/// Write a file atomically: the contents go to a temporary file in the
/// target's directory, created with `mode` from the start and forced to
/// disk, which is then renamed over the target. Rename on one filesystem is
/// atomic, so a crash mid-write can lose the update but never leaves a
/// truncated target file behind. It also carries the temp file's mode onto
/// the final path, even when the old file had a looser one.
fn write_atomic(path: &Path, contents: &str, mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Next to the target: same directory means same filesystem, so the
    // rename stays atomic. Pid plus target name makes the name collision-free
    // in practice; the leading dot and .tmp suffix keep it out of listings.
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => "data".into(),
    };
    let mut temp_used: Option<PathBuf> = None;
    let written: std::io::Result<()> = (|| {
        // `create_new` (O_EXCL) refuses every pre-existing name, so a
        // planted symlink at a predictable temp name can never be opened
        // and written through; on a collision the next number wins.
        let (mut f, tmp) = open_fresh_temp(dir, &name, mode)?;
        temp_used = Some(tmp.clone());
        // `mode()` applies to the create only (and through the umask), so
        // the mode here is forced through the OPEN HANDLE: that the temp
        // file is fresh already rules a planted symlink out, and chmod by
        // handle can also not be redirected if the name were swapped
        // between the create and here.
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.write_all(contents.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        // The rename itself must reach the disk too, which needs an fsync
        // of the *containing directory* (fsyncing the file alone does not
        // persist the new directory entry on ext4 & co). It is attempted
        // but deliberately NOT propagated: some filesystems cannot be
        // opened or fsynced as a file (certain network/FUSE mounts, exotic
        // platforms), and failing every save on such a system would trade
        // availability for a window that only matters after a crash. The
        // file itself is already on disk and consistent, so losing this
        // sync only widens the crash-loss window; it never corrupts data.
        // Consistency over strict ordering, availability over neither.
        if let Ok(dir_fd) = std::fs::File::open(dir) {
            let _ = dir_fd.sync_all();
        }
        Ok(())
    })();
    if written.is_err() {
        // Whatever went wrong, no half-written temp file may linger.
        if let Some(tmp) = temp_used {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    written
}

/// Open a fresh temp file for the atomic writer: `.{name}.{pid}[.n].tmp`,
/// never an existing name. A thousand collisions mean a thousand leftovers
/// of the same pid — implausible, and reported as the next AlreadyExists.
fn open_fresh_temp(dir: &Path, name: &str, mode: u32) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::os::unix::fs::OpenOptionsExt;
    let pid = std::process::id();
    for n in 0..1000u32 {
        let candidate = if n == 0 {
            dir.join(format!(".{name}.{pid}.tmp"))
        } else {
            dir.join(format!(".{name}.{pid}.{n}.tmp"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&candidate)
        {
            Ok(f) => return Ok((f, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no fresh temp name",
    ))
}

/// The app's config folder is the user's business alone: besides the
/// settings it carries the chat workspace with the agent's files and
/// full worktree checkouts of the user's projects. So it is created
/// 0700 from the start, and a pre-existing one with loose group/other
/// access bits is tightened once. Idempotent, and parents keep their
/// own modes — `~/.config` is not ours.
pub fn ensure_config_dir_private() -> Result<(), String> {
    let Some(dir) = config_dir() else {
        return Err(tr("no config folder").to_string());
    };
    ensure_private_dir(&dir)
}

fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            trf(
                "The config folder could not be created: {error}",
                &[("error", &e)],
            )
        })?;
    }
    // mkdir's mode goes through the umask, but 0700 has no group/other
    // bit the umask could need to clear, so the fresh dir is exactly 0700.
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // symlink_metadata, not metadata: a symlink planted at the
            // config path (by any same-user process, e.g. an agent driven
            // by a malicious repo while the app is closed) must not be
            // followed — its target would get chmod 0700 and all writes
            // would go through it
            let meta = std::fs::symlink_metadata(dir).map_err(|e| {
                trf(
                    "The config folder could not be read: {error}",
                    &[("error", &e)],
                )
            })?;
            if meta.file_type().is_symlink() {
                return Err(trf(
                    "The config folder is a symlink: {path}",
                    &[("path", &dir.to_string_lossy())],
                ));
            }
            if !meta.is_dir() {
                // a file (or worse) sits where the folder belongs: say
                // where, so the user can move it aside
                return Err(trf(
                    "The config folder could not be used: {path}",
                    &[("path", &dir.to_string_lossy())],
                ));
            }
            // Only the access bits decide: a setgid or sticky flag says
            // nothing about who may look inside, and stays as it is.
            if meta.mode() & 0o077 != 0 {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| {
                        trf(
                            "The config folder could not be made private: {error}",
                            &[("error", &e)],
                        )
                    },
                )?;
            }
            Ok(())
        }
        Err(e) => Err(trf(
            "The config folder could not be created: {error}",
            &[("error", &e)],
        )),
    }
}

/// Write a secret-bearing file atomically and private, mode 0600.
///
/// The provider/key/chat writers that used this are gone with the plain-HTTP
/// world; the machinery stays (with its tests) for the next secret-bearing
/// file the store needs, which is why the dead-code lint is silenced.
#[allow(dead_code)]
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic(path, contents, 0o600)
}

fn write_generic(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic(path, contents, 0o644)
}

// ---------------------------------------------------------------------------
// Settings (settings.json, generic perms — no secrets).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Project folders (Vibe over ACP), absolute paths. The chat workspace
    /// "Allgemein" is not stored here. An old file's `ui_mode` ("code" |
    /// "chat", from before the two views became one) is ignored on load.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Mode a new Vibe session is switched to before its first prompt
    /// ("ask" = ask before every change).
    #[serde(default = "default_session_mode")]
    pub default_session_mode: String,
    /// Vibe models (value, name) and thinking levels seen last, so the
    /// pickers work before the first session of an app run.
    #[serde(default)]
    pub vibe_models: Vec<(String, String)>,
    #[serde(default)]
    pub vibe_thinking: Vec<String>,
    /// Thinking levels per model (offered levels, level last active).
    #[serde(default)]
    pub vibe_thinking_by_model: std::collections::BTreeMap<String, (Vec<String>, String)>,
    /// The left sidebar is folded into its icon rail.
    #[serde(default)]
    pub sidebar_collapsed: bool,
    /// The right panel is closed.
    #[serde(default, deserialize_with = "panel_closed_compat")]
    pub panel_closed: bool,
    /// Read plan and budget at startup. Off by default: without it the
    /// account is only read when the user opens it.
    #[serde(default)]
    pub auto_account: bool,
    /// Desktop notifications (notify-send) when a turn finishes or Vibe
    /// needs an approval — only while the window has no focus. Off by
    /// default.
    #[serde(default)]
    pub notify_turn_events: bool,
    /// Voice dictation in the composer: a microphone button that starts and
    /// stops Vibe's transcription. Off by default.
    #[serde(default)]
    pub voice_dictation: bool,
    /// Read answers aloud: a speaker button on Vibe's answers. Vibe
    /// summarises the answer, speaks it and plays it. Off by default.
    #[serde(default)]
    pub voice_narration: bool,
    /// Read every finished answer aloud automatically (only while
    /// `voice_narration` is on). Off by default.
    #[serde(default)]
    pub auto_narrate: bool,
    /// Pinned session ids: they sort to the top of their workspace's list.
    #[serde(default)]
    pub pinned_sessions: Vec<String>,
    /// How open tool groups and thoughts are while a turn runs:
    /// "compact" | "standard" | "detailed" | "verbose".
    #[serde(default = "default_transcript_detail")]
    pub transcript_detail: String,
    /// Archived session ids: hidden from the sidebar's session lists.
    #[serde(default)]
    pub archived_sessions: Vec<String>,
    /// UI language code (`"en"`, `"de"`, `"fr"`, `"es"`, `"it"`, `"pt"`).
    /// `None` until the user picked one:
    /// the first start then asks, with English preselected.
    #[serde(default)]
    pub language: Option<String>,
}

fn default_session_mode() -> String {
    "ask".into()
}

fn default_transcript_detail() -> String {
    "standard".into()
}

/// `panel_closed` was `[code, chat]` while Code and Chat were two views.
/// Such a file must keep loading — a failed parse would fall back to the
/// defaults and lose the project list — so the old pair is read as its Code
/// value, and anything else that is not a bool as "open".
fn panel_closed_compat<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Array(pair) => pair
            .first()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        _ => false,
    })
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            default_session_mode: default_session_mode(),
            vibe_models: Vec::new(),
            vibe_thinking: Vec::new(),
            vibe_thinking_by_model: Default::default(),
            sidebar_collapsed: false,
            panel_closed: false,
            auto_account: false,
            notify_turn_events: false,
            voice_dictation: false,
            voice_narration: false,
            auto_narrate: false,
            pinned_sessions: Vec::new(),
            transcript_detail: default_transcript_detail(),
            archived_sessions: Vec::new(),
            language: None,
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    Some(config_dir()?.join("settings.json"))
}

pub fn load_settings() -> Result<Settings, String> {
    match settings_path() {
        Some(p) => load_settings_from(&p),
        None => Ok(Settings::default()),
    }
}

/// A missing file is the normal first-run case (defaults). Anything else —
/// unreadable or unparsable — is an error the caller must show: silently
/// falling back to defaults would make the next save overwrite the file
/// and the user's project list with it (Handoff 2026-11-09, Bug 5).
fn load_settings_from(path: &Path) -> Result<Settings, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => {
            return Err(trf(
                "The settings could not be read: {error}",
                &[("error", &e)],
            ))
        }
    };
    serde_json::from_str(&text)
        .map_err(|e| trf("The settings could not be read: {error}", &[("error", &e)]))
}

/// Move an unreadable settings file aside instead of letting the next save
/// overwrite it: the user's data stays recoverable next to the new file.
pub fn backup_unreadable_settings() -> Result<PathBuf, String> {
    let path = settings_path().ok_or(crate::i18n::tr("no config folder"))?;
    backup_unreadable_settings_at(&path)
}

fn backup_unreadable_settings_at(path: &Path) -> Result<PathBuf, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let backup = backup_name(path, stamp, |p| p.exists());
    std::fs::rename(path, &backup).map_err(|e| {
        trf(
            "The settings could not be backed up: {error}",
            &[("error", &e)],
        )
    })?;
    Ok(backup)
}

/// The backup path for an unreadable settings file: `.{name}.unlesbar-{stamp}`.
/// With a broken clock (VM snapshot, `SystemTime` before the epoch) the stamp
/// collapses (`nanos.unwrap_or(0)`), so two backups would collide on the same
/// name and the second rename would silently overwrite the first. When
/// `taken` reports a collision, the first free numeric suffix (`-1`, `-2`, …)
/// is appended so every backup stays recoverable.
fn backup_name(path: &Path, stamp: u128, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let name = match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => "settings.json".into(),
    };
    let mut candidate = path.with_file_name(format!(".{name}.unlesbar-{stamp}"));
    let mut suffix = 0;
    while taken(&candidate) {
        suffix += 1;
        candidate = path.with_file_name(format!(".{name}.unlesbar-{stamp}-{suffix}"));
    }
    candidate
}

/// Another running instance owns `settings.json`: this one must not write
/// it, or the window that closes last would overwrite the other's changes
/// (a project added there would be gone).
static SETTINGS_READ_ONLY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Try to become the instance that writes the settings: an exclusive lock
/// on `.instance.lock` in the config dir, held until the process ends.
/// Returns false when another instance holds it; settings are then not
/// saved by this one. Without a usable lock (no config dir, a filesystem
/// without locks) the old behaviour stays: saving is allowed.
pub fn claim_settings() -> bool {
    static LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();
    if LOCK.get().is_some() {
        return true;
    }
    let Some(dir) = config_dir() else {
        return true;
    };
    match try_claim_at(&dir.join(".instance.lock")) {
        Claim::Owned(file) => {
            let _ = LOCK.set(file);
            true
        }
        Claim::Taken => {
            SETTINGS_READ_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
            false
        }
        Claim::Unavailable => true,
    }
}

enum Claim {
    Owned(std::fs::File),
    Taken,
    Unavailable,
}

fn try_claim_at(path: &Path) -> Claim {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
    else {
        return Claim::Unavailable;
    };
    match file.try_lock() {
        Ok(()) => Claim::Owned(file),
        Err(std::fs::TryLockError::WouldBlock) => Claim::Taken,
        Err(std::fs::TryLockError::Error(_)) => Claim::Unavailable,
    }
}

pub fn save_settings(s: &Settings) -> std::io::Result<()> {
    if SETTINGS_READ_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(());
    }
    let p = settings_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"))?;
    save_settings_at(&p, s)
}

/// Save to an explicit path: the same writer `save_settings` uses, separate
/// so failure cases (a read-only config directory) can be tested without
/// touching the real config dir.
pub fn save_settings_at(path: &Path, s: &Settings) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(s)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    write_generic(path, &text)
}

// ---------------------------------------------------------------------------
// ```ui DSL specification
// ---------------------------------------------------------------------------

/// The ```ui DSL specification, shipped as a Vibe skill (single source of
/// truth for the installed file).
pub const UI_DSL_SKILL: &str = include_str!("../../vibe-skills/catwright-ui/SKILL.md");

/// The skill text without its YAML frontmatter (everything after the closing
/// `---` line), i.e. the part a system prompt would carry.
///
/// The only runtime consumer (the plain-HTTP providers' system prompt) is
/// gone with that world, but the spec stays as the shipped skill's
/// documentation: the tests assert through it that the catwright-ui skill
/// documents every UI node the renderer supports.
#[allow(dead_code)]
pub fn ui_dsl_spec() -> &'static str {
    let text = UI_DSL_SKILL;
    let rest = match text.strip_prefix("---") {
        Some(r) => r,
        None => return text.trim(),
    };
    // the frontmatter ends at the next line that is exactly "---"
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return rest[offset..].trim();
        }
    }
    text.trim()
}

// ---------------------------------------------------------------------------
// Vibe skill installation (~/.vibe/skills/catwright-ui/SKILL.md). Written
// only when the user clicks the button in the settings.
// ---------------------------------------------------------------------------

/// `~/.vibe/skills/catwright-ui/SKILL.md` (None without a home directory).
pub fn vibe_skill_path() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    Some(vibe_skill_path_at(&home))
}

fn vibe_skill_path_at(home: &Path) -> PathBuf {
    home.join(".vibe")
        .join("skills")
        .join(SKILL_NAME)
        .join("SKILL.md")
}

/// The skill's folder name in `~/.vibe/skills/`.
const SKILL_NAME: &str = "catwright-ui";

/// Write the skill below `home` (`.vibe/skills/catwright-ui/SKILL.md`).
pub fn install_vibe_skill_at(home: &Path) -> std::io::Result<PathBuf> {
    let path = vibe_skill_path_at(home);
    write_generic(&path, UI_DSL_SKILL)?;
    Ok(path)
}

/// Install the skill for the current user; returns the written path.
pub fn install_vibe_skill() -> std::io::Result<PathBuf> {
    let home = directories::BaseDirs::new()
        .map(|b| b.home_dir().to_path_buf())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    install_vibe_skill_at(&home)
}

/// True when the skill exists and matches the shipped version.
pub fn vibe_skill_installed() -> bool {
    vibe_skill_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text == UI_DSL_SKILL)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second instance does not get the settings lock while the first
    /// holds it, and gets it once the first is gone.
    #[test]
    fn only_one_instance_claims_the_settings() {
        let base = std::env::temp_dir().join(format!(
            "od-lock-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let lock = base.join(".instance.lock");
        let first = match try_claim_at(&lock) {
            Claim::Owned(f) => f,
            _ => panic!("the first instance gets the lock"),
        };
        assert!(matches!(try_claim_at(&lock), Claim::Taken));
        drop(first);
        assert!(matches!(try_claim_at(&lock), Claim::Owned(_)));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn settings_without_default_session_mode_defaults_to_ask() {
        // an old settings.json has no default_session_mode key
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.default_session_mode, "ask");
        let s: Settings =
            serde_json::from_str(r#"{"ui_mode":"chat","projects":["/tmp/x"]}"#).unwrap();
        assert_eq!(s.default_session_mode, "ask");
        let s: Settings =
            serde_json::from_str(r#"{"default_session_mode":"auto-approve"}"#).unwrap();
        assert_eq!(s.default_session_mode, "auto-approve");
        assert_eq!(Settings::default().default_session_mode, "ask");
    }

    /// A settings.json from before Code and Chat became one view (with
    /// `ui_mode` and the `[code, chat]` panel pair) still loads with its
    /// projects; the panel takes the Code value.
    #[test]
    fn settings_from_the_two_view_era_keep_loading() {
        let old = r#"{"projects":["/tmp/p"],"ui_mode":"chat","panel_closed":[true,false],
                      "sidebar_collapsed":true}"#;
        let s: Settings = serde_json::from_str(old).expect("old file loads");
        assert_eq!(s.projects, vec!["/tmp/p".to_string()]);
        assert!(s.panel_closed, "the Code value of the old pair wins");
        assert!(s.sidebar_collapsed);
        let s: Settings = serde_json::from_str(r#"{"panel_closed":[false,true]}"#).unwrap();
        assert!(!s.panel_closed);
        // the new form, and nonsense that must not lose the file
        let s: Settings = serde_json::from_str(r#"{"panel_closed":true}"#).unwrap();
        assert!(s.panel_closed);
        let s: Settings =
            serde_json::from_str(r#"{"projects":["/tmp/q"],"panel_closed":"x"}"#).unwrap();
        assert_eq!(s.projects, vec!["/tmp/q".to_string()]);
        assert!(!s.panel_closed);
        assert!(!Settings::default().panel_closed);
        // a save writes the single value and no `ui_mode` any more
        let saved = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(saved["panel_closed"], serde_json::json!(false));
        assert!(saved.get("ui_mode").is_none());
    }

    fn tmp_base(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catwright-test-{tag}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ui_dsl_spec_has_no_frontmatter_and_mentions_every_node() {
        let spec = ui_dsl_spec();
        assert!(!spec.starts_with("---"), "frontmatter is stripped");
        assert!(!spec.contains("name: catwright-ui"));
        assert!(!spec.contains("description: Load this skill"));
        assert!(spec.contains("Catwright UI blocks"), "body is kept");
        assert!(spec.contains("```ui-js"), "spec documents ui-js tools");
        assert!(spec.contains("function render(state)"));
        for node in [
            "text",
            "heading",
            "button",
            "table",
            "form",
            "bar_chart",
            "line_chart",
            "pie_chart",
            "progress",
            "checklist",
            "kv",
            "divider",
            "card",
            "columns",
            "slider",
            "number",
            "select",
            "computed",
            "toggle",
            "text_input",
        ] {
            assert!(
                spec.contains(&format!("\"type\":\"{node}\"")),
                "spec documents the node {node}"
            );
        }
        // the skill file itself keeps its frontmatter
        assert!(UI_DSL_SKILL.starts_with("---\nname: catwright-ui\n"));
    }

    /// Bug 5 (Handoff 2026-11-09): settings.json is written atomically, and
    /// an unreadable file is reported instead of silently resetting
    /// everything the next save would then overwrite.
    #[test]
    fn settings_are_written_atomically() {
        let base = tmp_base("settings-atomic");
        let target = base.join("settings.json");
        std::fs::write(&target, "old\n").unwrap();
        let body = "{\"projects\":[\"/a\",\"/b\"]}\n".repeat(500);
        write_generic(&target, &body).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), body);
        let names: Vec<String> = std::fs::read_dir(&base)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["settings.json"], "no temp file lingers");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn load_settings_reports_errors_and_defaults_only_for_a_missing_file() {
        let base = tmp_base("settings-load");
        let path = base.join("settings.json");
        // no file yet: the normal first run
        let s = load_settings_from(&path).unwrap();
        assert_eq!(s, Settings::default());
        // a broken file is an error, not a silent reset
        std::fs::write(&path, "{").unwrap();
        let err = load_settings_from(&path).unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        // a valid file loads
        std::fs::write(&path, "{\"projects\":[\"/a\"]}").unwrap();
        let s = load_settings_from(&path).unwrap();
        assert_eq!(s.projects, ["/a".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn backup_unreadable_settings_keeps_the_old_file() {
        let base = tmp_base("settings-backup");
        let dir = base.join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "die alte Liste").unwrap();
        let backup = backup_unreadable_settings_at(&path).unwrap();
        assert!(backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("unlesbar-"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "die alte Liste");
        assert!(!path.exists(), "the old file is moved, not copied");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Broken clock (N18b): two backups with the same collapsed stamp must
    /// not collide — the pure name function appends `-1`, `-2`, … instead of
    /// letting the second backup overwrite the first.
    #[test]
    fn backup_name_appends_suffixes_on_colliding_stamps() {
        let p = Path::new("/cfg/settings.json");
        let first = backup_name(p, 0, |_| false);
        assert!(first
            .to_str()
            .unwrap()
            .ends_with(".settings.json.unlesbar-0"));
        // the first backup exists: the next save gets the next free name
        let second = backup_name(p, 0, |c| c == first);
        assert!(second.to_str().unwrap().ends_with("-0-1"), "{second:?}");
        // and a third one moves on to -2
        let third = backup_name(p, 0, |c| c == first || c == second);
        assert!(third.to_str().unwrap().ends_with("-0-2"), "{third:?}");
        // proper stamps stay unique: no suffix
        let fresh = backup_name(p, 1_800_000_000_123_456_789, |c| c == first);
        assert!(fresh
            .to_str()
            .unwrap()
            .ends_with(".unlesbar-1800000000123456789"));
    }

    /// The rename behind the backup uses the collision-free name for real:
    /// an existing backup with the same stamp is left untouched on disk.
    #[test]
    fn backup_unreadable_settings_with_colliding_stamp_keeps_both_files() {
        let base = tmp_base("settings-backup-collide");
        let dir = base.join("cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        // an earlier backup with the same (broken-clock) stamp
        let earlier = backup_name(&path, 7, |_| false);
        std::fs::write(&earlier, "first backup").unwrap();
        std::fs::write(&path, "second corrupt file").unwrap();
        // the pure name logic in situ: the stamp collides, the suffix breaks it
        let backup = backup_name(&path, 7, |p| p.exists());
        std::fs::rename(&path, &backup).unwrap();
        assert_eq!(std::fs::read_to_string(&earlier).unwrap(), "first backup");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "second corrupt file"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A save into a read-only directory must fail (not silently succeed),
    /// so the autosave can report it (app.rs).
    #[test]
    fn save_settings_fails_into_a_read_only_directory() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmp_base("settings-readonly");
        let dir = base.join("locked");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("settings.json");
        let _restore = RestoreDirWrite(dir.clone());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = save_settings_at(&target, &Settings::default());
        assert!(result.is_err(), "read-only config dir must fail the save");
        assert!(!target.exists(), "no settings file is written");
        drop(_restore);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn install_vibe_skill_writes_the_skill_file() {
        let home = tmp_base("skill");
        let path = install_vibe_skill_at(&home).unwrap();
        assert_eq!(
            path,
            home.join(".vibe")
                .join("skills")
                .join("catwright-ui")
                .join("SKILL.md")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), UI_DSL_SKILL);
        // a second install overwrites (update path)
        std::fs::write(&path, "old").unwrap();
        install_vibe_skill_at(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), UI_DSL_SKILL);
        let _ = std::fs::remove_dir_all(&home);
    }

    // -----------------------------------------------------------------------
    // write_private: atomic 0600 replacement (Handoff 2.6).
    // -----------------------------------------------------------------------

    #[test]
    fn write_private_replaces_loose_file_atomically_and_privately() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmp_base("private-loose");
        let dir = base.join("data");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("entry.json");
        // a pre-existing file with a looser mode, like a save from an old
        // version of the app
        std::fs::write(&target, "old\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        // a full multi-line body must arrive complete
        let body = "chat line\n".repeat(20_000);
        write_private(&target, &body).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), body);
        // the rename carries the temp file's 0600 over the old 0644
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // nothing but the target is left in the directory: no stale temp file
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["entry.json"]);
        // the update path (overwrite an existing 0600 file) is just as clean
        write_private(&target, "shorter\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "shorter\n");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["entry.json"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Restores the write permission of a directory on drop, so a failing
    /// assertion cannot leave a read-only directory behind.
    struct RestoreDirWrite(PathBuf);

    impl Drop for RestoreDirWrite {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn write_private_failure_keeps_old_file_and_cleans_up_temp() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmp_base("private-fail");
        let dir = base.join("locked");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("secret.json");
        std::fs::write(&target, "previous\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();

        // read-only directory: creating the temp file has to fail, so the
        // save must fail too, without touching the old file
        let _restore = RestoreDirWrite(dir.clone());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_private(&target, "new\n");
        assert!(
            result.is_err(),
            "writing into a read-only directory must fail"
        );
        // the old file is intact: same content, same mode
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "previous\n");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        // and no temp file lingers in the directory
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["secret.json"]);
        // the guard restores the write bit so the cleanup can remove the tree
        drop(_restore);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The temp file is only ever a name we created ourselves: a symlink
    /// planted at the predicted `.NAME.PID.tmp` name is not opened and
    /// not written through — the writer moves on to the next number, the
    /// victim stays untouched (audit: symlink follow-through).
    #[test]
    fn write_atomic_never_opens_a_planted_symlink_at_the_temp_name() {
        let base = tmp_base("tmp-symlink");
        let dir = base.join("data");
        std::fs::create_dir_all(&dir).unwrap();
        let victim = base.join("victim.txt");
        std::fs::write(&victim, "unversehrt\n").unwrap();
        let target = dir.join("settings.json");
        let planted = dir.join(format!(".settings.json.{}.tmp", std::process::id()));
        std::os::unix::fs::symlink(&victim, &planted).unwrap();
        save_settings_at(&target, &Settings::default()).unwrap();
        // the victim was neither truncated nor written through, and the
        // final file is a regular one with the settings' content
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unversehrt\n");
        assert!(!target.is_symlink());
        assert!(
            serde_json::from_str::<Settings>(&std::fs::read_to_string(&target).unwrap()).is_ok()
        );
        // the planted symlink was left alone (its name was not taken over)
        assert!(planted.is_symlink());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The config folder is private from its first creation on, a loose one
    /// is tightened once, and a file in its place is reported, not fixed.
    #[test]
    fn ensure_private_dir_creates_0700_tightens_and_reports_a_non_dir() {
        use std::os::unix::fs::PermissionsExt as _;
        let base = tmp_base("private-dir");
        let dir = base.join("cfg");
        std::fs::create_dir_all(&base).unwrap();
        // fresh creation lands at 0700 regardless of the umask
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // loosened to group/other access: tightened again, once
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // idempotent: a second call changes nothing
        ensure_private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // a symlink in the place of the folder is an error too (never
        // followed: its target must not get chmod 0700 nor our writes)
        let target = base.join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        let link = base.join("linked");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = ensure_private_dir(&link).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_ne!(mode, 0o700, "the symlink target was not chmod'ed");
        // a regular file in the place of the folder is an error
        let file = base.join("occupied");
        std::fs::write(&file, "x").unwrap();
        assert!(ensure_private_dir(&file).is_err());
        // the parent of the folder keeps its own mode: only the leaf is ours
        let parent_mode = std::fs::metadata(&base).unwrap().permissions().mode() & 0o777;
        let fresh_parent = base.join("deep/er");
        ensure_private_dir(&fresh_parent.join("cfg")).unwrap();
        let parent_mode2 = std::fs::metadata(&fresh_parent)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(parent_mode2, parent_mode, "parents keep their mode");
        let _ = std::fs::remove_dir_all(&base);
    }
}
