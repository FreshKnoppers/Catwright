//! The Artefakte view (left sidebar entry, main-area list + preview, like
//! Claude Desktop in `reference/picture-3.png`): every file the agent
//! touched in the loaded sessions, collected from the completed diff
//! tool calls, with a per-type preview rendered from disk.
//!
//! Preview by file type (`kind_of`): Markdown rendered like a chat
//! message, PNG as an image, CSV as a striped table (the shared grid
//! painter), HTML as source text with an "open in the browser" button
//! (never a WebView, Rust only), everything else as monospace text with
//! a copy button. Files are read from disk at preview time, capped at
//! [`MAX_PREVIEW_BYTES`]; a file the agent reverted or deleted shows as
//! missing instead of pretending.

use crate::acp::protocol::{ToolCallContent, ToolStatus};
use crate::acp::session::{Item, SessionView};
use std::collections::HashMap;
use std::path::Path;

/// How large a previewed file may be; larger ones are announced, not
/// loaded (the whole text would go through the layout engine).
pub const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
/// CSV caps: enough for a real export, small enough to stay snappy.
pub const MAX_CSV_ROWS: usize = 1_000;
pub const MAX_CSV_COLS: usize = 100;

/// One file the agent touched, with the session that touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// Project folder of the touching session (grouping key).
    pub cwd: std::path::PathBuf,
    pub session_id: String,
    /// Session title at collect time, for context in the list.
    pub session_title: Option<String>,
    /// Absolute path as Vibe reported it in the diff.
    pub path: String,
    /// Whether the file exists on disk right now.
    pub exists: bool,
    /// File size in bytes (0 when it does not exist).
    pub size: u64,
}

/// Collect every touched file from all loaded sessions: the last
/// completed diff per path wins. Sessions are visited in sorted-id
/// order, so a path touched by several sessions is deterministic (the
/// alphabetically last session wins — there is no timestamp on a diff).
/// The list is grouped by project folder and sorted by path within it,
/// so the view is stable across frames.
pub fn collect(sessions: &HashMap<String, SessionView>) -> Vec<Artifact> {
    let mut newest: HashMap<String, Artifact> = HashMap::new();
    let mut ids: Vec<&String> = sessions.keys().collect();
    ids.sort();
    for sid in ids {
        let s = &sessions[sid];
        for item in &s.items {
            let Item::Tool(id) = item else { continue };
            let Some(tc) = s.tool_calls.get(id) else {
                continue;
            };
            if tc.status != ToolStatus::Completed {
                continue;
            }
            for c in &tc.content {
                let ToolCallContent::Diff { path, .. } = c else {
                    continue;
                };
                let (exists, size) = file_state(path);
                newest.insert(
                    path.clone(),
                    Artifact {
                        cwd: s.cwd.clone(),
                        session_id: sid.clone(),
                        session_title: s.title.clone(),
                        path: path.clone(),
                        exists,
                        size,
                    },
                );
            }
        }
    }
    let mut all: Vec<Artifact> = newest.into_values().collect();
    all.sort_by(|a, b| a.cwd.cmp(&b.cwd).then_with(|| a.path.cmp(&b.path)));
    all
}

/// `(exists, size)` of a path, throttled: the view calls `collect` every
/// frame, and one `stat` per file per frame is avoidable load (Handoff
/// 2026-11-09, „Per Frame neu berechnet"). The cache lives 1 s per path.
fn file_state(path: &str) -> (bool, u64) {
    use std::sync::{Mutex, OnceLock};
    struct Entry {
        at: std::time::Instant,
        exists: bool,
        size: u64,
    }
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap();
    if let Some(e) = cache.get(path) {
        if e.at.elapsed() < std::time::Duration::from_secs(1) {
            return (e.exists, e.size);
        }
    }
    let (exists, size) = match std::fs::metadata(path) {
        Ok(m) if m.is_file() => (true, m.len()),
        _ => (false, 0),
    };
    cache.insert(
        path.to_string(),
        Entry {
            at: std::time::Instant::now(),
            exists,
            size,
        },
    );
    // the keys are agent-controlled path strings: bound the map (wholesale
    // clear at the cap, same pattern as render.rs's file_exists cache)
    if cache.len() > 4096 {
        cache.clear();
    }
    (exists, size)
}

/// How a file is previewed, by extension (case-insensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Markdown,
    Image,
    Csv,
    Html,
    Text,
}

/// Classify a path: `.md` renders as chat markdown, `.png` as an image
/// (the only image format the `image` crate is built with), `.csv` as a
/// table, `.html` as source with a browser button, everything else as
/// monospace text.
pub fn kind_of(path: &str) -> ArtifactKind {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => ArtifactKind::Markdown,
        "png" => ArtifactKind::Image,
        "csv" => ArtifactKind::Csv,
        "html" | "htm" => ArtifactKind::Html,
        _ => ArtifactKind::Text,
    }
}

/// What [`load_preview`] found, ready for the renderer.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    /// Text content (markdown / csv / html / plain), already capped.
    Text(String),
    /// Raw image bytes (PNG).
    Image(Vec<u8>),
    /// The file is gone (reverted or deleted after the session ran).
    Missing,
    /// The file is larger than `MAX_PREVIEW_BYTES`.
    TooLarge(u64),
    /// Reading failed (permissions, encoding, …).
    Unreadable,
}

/// Read a file for preview, honouring [`MAX_PREVIEW_BYTES`]: image kinds
/// come back as raw bytes for egui to decode, text kinds as lossy text.
/// The result is cached per path and mtime — the view asks every frame,
/// and re-reading up to 2 MiB per frame is avoidable load (Handoff
/// 2026-11-09, „Per Frame neu berechnet“).
pub fn load_preview(path: &str, kind: ArtifactKind) -> Preview {
    let capped = cached_capped(path);
    match capped {
        Preview::Image(bytes) if kind != ArtifactKind::Image => text_preview(bytes),
        other => other,
    }
}

/// [`load_preview_capped`] with an mtime cache: the raw file (up to
/// `MAX_PREVIEW_BYTES`) is read once per file version, not once per frame.
fn cached_capped(path: &str) -> Preview {
    use std::sync::{Mutex, OnceLock};
    struct Entry {
        mtime: Option<std::time::SystemTime>,
        len: u64,
        preview: Preview,
    }
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let meta = std::fs::metadata(path);
    let mtime = meta.as_ref().ok().and_then(|m| m.modified().ok());
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap();
    if let Some(e) = cache.get(path) {
        if e.mtime == mtime && e.len == len {
            return e.preview.clone();
        }
    }
    let preview = load_preview_capped(path, MAX_PREVIEW_BYTES);
    cache.insert(
        path.to_string(),
        Entry {
            mtime,
            len,
            preview: preview.clone(),
        },
    );
    // keys are agent-controlled paths and each entry can hold up to
    // MAX_PREVIEW_BYTES: bound the map (wholesale clear at the cap)
    if cache.len() > 64 {
        cache.clear();
    }
    preview
}

/// Capped lossy text read for previews that always render as text (the
/// diff mode): the same handle-based caps as [`load_preview_capped`], so an
/// uncapped `read_to_string` on an agent-named path is impossible there too.
pub fn load_text_capped(path: &str) -> Preview {
    match load_preview_capped(path, MAX_PREVIEW_BYTES) {
        Preview::Image(bytes) => text_preview(bytes),
        other => other,
    }
}

/// `File::open` for preview reads that must never block on the open itself.
/// On unix the open carries `O_NONBLOCK`, so a FIFO swapped in after the
/// path's type check opens instantly instead of hanging the UI thread; the
/// caller rejects it right after via the handle's `fstat`. Regular files
/// ignore the flag, so their reads are unchanged.
fn open_nonblock(path: &str) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(0o4000); // O_NONBLOCK
    }
    opts.open(path)
}

fn load_preview_capped(path: &str, cap: u64) -> Preview {
    // Special files never reach the open: a FIFO without a writer
    // blocks the open indefinitely — and this runs on the UI thread —
    // so the path's type decides here, before `File::open`. Only a
    // regular file proceeds; everything else (missing, directory,
    // socket, FIFO, device) reports like a vanished file (final
    // audit B: the cap-introducing rewrite had dropped the type
    // check `metadata(path).is_file()` that guarded the open before).
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Preview::Missing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Preview::Missing,
        Err(_) => return Preview::Unreadable,
    }
    // Reading without a size-check window (TOCTOU): open the file and read
    // at most `cap + 1` bytes through the very same handle, so a file that
    // grows between a `stat` and the read can no longer be loaded whole.
    // The open itself is non-blocking (see open_nonblock), so a FIFO raced
    // in between the two checks cannot hang here either — the handle's
    // fstat below rejects it.
    let file = match open_nonblock(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Preview::Missing,
        Err(_) => return Preview::Unreadable,
    };
    // Stats of the *open handle*, not of the path: a directory stays
    // `Missing`, and a file that vanished mid-preview does either way.
    let meta = match file.metadata() {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return Preview::Missing,
        Err(_) => return Preview::Unreadable,
    };
    // one byte over the cap proves the file is too large, everything else
    // is the preview
    use std::io::Read;
    let mut bytes = Vec::new();
    let read = match (&file).take(cap.saturating_add(1)).read_to_end(&mut bytes) {
        Ok(read) => read,
        Err(_) => return Preview::Unreadable,
    };
    if read as u64 > cap {
        // name the bigger of the handle's size and the bytes that
        // overflowed, since special files (procfs) understate their size
        return Preview::TooLarge(meta.len().max(read as u64));
    }
    Preview::Image(bytes)
}

/// Decode preview bytes for a text kind (lossy: a binary file with a
/// text extension still shows something instead of failing).
pub fn text_preview(bytes: Vec<u8>) -> Preview {
    Preview::Text(String::from_utf8_lossy(&bytes).into_owned())
}

/// Parse CSV (RFC-4180-style: quoted fields, `""` escapes, commas)
/// into rows of cells, capped at [`MAX_CSV_ROWS`] rows and
/// [`MAX_CSV_COLS`] columns. Ragged rows keep their length; parsing
/// never fails, a broken field just ends up as literal text.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if rows.len() >= MAX_CSV_ROWS {
            return rows;
        }
        if in_quotes {
            match c {
                '"' => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        cell.push('"');
                    } else {
                        in_quotes = false;
                    }
                }
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => {
                if row.len() < MAX_CSV_COLS {
                    row.push(std::mem::take(&mut cell));
                }
            }
            '\r' => {}
            '\n' => {
                if row.len() < MAX_CSV_COLS {
                    row.push(std::mem::take(&mut cell));
                }
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        if row.len() < MAX_CSV_COLS {
            row.push(cell);
        }
        rows.push(row);
    }
    rows
}

/// File name without directories, for list rows and window titles.
pub fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::protocol::{ToolCall, ToolCallContent, ToolStatus};
    use crate::acp::session::{Item, SessionView};
    use serde_json::json;
    use std::io::Write;

    fn diff_tool(id: &str, path: &str, status: ToolStatus) -> (String, ToolCall) {
        (
            id.to_string(),
            ToolCall {
                tool_call_id: id.to_string(),
                title: "edit".into(),
                kind: Default::default(),
                status,
                content: vec![ToolCallContent::Diff {
                    path: path.to_string(),
                    old_text: None,
                    new_text: "x".into(),
                }],
                locations: Vec::new(),
                raw_input: None,
                raw_output: None,
                meta: None,
                first_seen: 0,
                last_update: 0,
            },
        )
    }

    fn session_with_diffs(sid: &str, cwd: &str, diffs: Vec<(&str, ToolStatus)>) -> SessionView {
        let mut v = SessionView::new(sid, cwd);
        v.title = Some(format!("{sid}-titel"));
        for (i, (path, status)) in diffs.into_iter().enumerate() {
            let id = format!("t{i}");
            let (id, tc) = diff_tool(&id, path, status);
            v.tool_calls.insert(id, tc);
            v.items.push(Item::Tool(format!("t{i}")));
        }
        v
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("a.md"), ArtifactKind::Markdown);
        assert_eq!(kind_of("NOTIZ.MARKDOWN"), ArtifactKind::Markdown);
        assert_eq!(kind_of("img.PNG"), ArtifactKind::Image);
        assert_eq!(kind_of("data.csv"), ArtifactKind::Csv);
        assert_eq!(kind_of("page.htm"), ArtifactKind::Html);
        assert_eq!(kind_of("main.rs"), ArtifactKind::Text);
        assert_eq!(kind_of("noext"), ArtifactKind::Text);
    }

    #[test]
    fn collect_dedupes_groups_and_skips_unfinished() {
        let tmp = std::env::temp_dir().join("vibe-artifacts-test-a");
        std::fs::create_dir_all(&tmp).unwrap();
        let existing = tmp.join("gibt_es.md");
        std::fs::write(&existing, "# Hallo\n").unwrap();
        let existing_str = existing.to_string_lossy().into_owned();

        let mut sessions = HashMap::new();
        sessions.insert(
            "s1".to_string(),
            session_with_diffs(
                "s1",
                "/proj/a",
                vec![
                    (&existing_str, ToolStatus::Completed),
                    ("/proj/a/gone.rs", ToolStatus::Completed),
                    ("/proj/a/pending.rs", ToolStatus::InProgress),
                ],
            ),
        );
        // the same path in a second session: the newer touch wins
        sessions.insert(
            "s2".to_string(),
            session_with_diffs(
                "s2",
                "/proj/b",
                vec![
                    (&existing_str, ToolStatus::Completed),
                    ("/proj/b/one.txt", ToolStatus::Completed),
                ],
            ),
        );

        let mut got = collect(&sessions);
        assert_eq!(got.len(), 3, "{got:?}");
        // grouped by cwd, sorted by path inside
        assert_eq!(got[0].cwd, std::path::PathBuf::from("/proj/a"));
        assert_eq!(got[0].path, "/proj/a/gone.rs");
        assert!(!got[0].exists);
        assert_eq!(got[1].cwd, std::path::PathBuf::from("/proj/b"));
        assert_eq!(got[1].path, "/proj/b/one.txt");
        // the newest touch of the shared file is from s2, so it is grouped
        // under s2's project and carries its session title
        assert_eq!(got[2].path, existing_str);
        assert!(got[2].exists);
        assert_eq!(got[2].size, "# Hallo\n".len() as u64);
        assert_eq!(got[2].session_id, "s2");
        assert_eq!(got[2].session_title.as_deref(), Some("s2-titel"));

        // stability across calls
        let again = collect(&sessions);
        got.sort_by(|a, b| a.path.cmp(&b.path));
        let mut again = again;
        again.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(got, again);
    }

    #[test]
    fn preview_caps_and_missing() {
        let tmp = std::env::temp_dir().join("vibe-artifacts-test-b");
        std::fs::create_dir_all(&tmp).unwrap();
        let small = tmp.join("small.txt");
        std::fs::write(&small, "inhalt").unwrap();
        assert_eq!(
            load_preview_capped(&small.to_string_lossy(), 100),
            Preview::Image(b"inhalt".to_vec())
        );
        assert_eq!(
            load_preview_capped(&small.to_string_lossy(), 3),
            Preview::TooLarge(6)
        );
        assert_eq!(
            load_preview(&tmp.join("weg.txt").to_string_lossy(), ArtifactKind::Text),
            Preview::Missing
        );
        // a directory is not a previewable file
        assert_eq!(
            load_preview(&tmp.to_string_lossy(), ArtifactKind::Text),
            Preview::Missing
        );
        // text kinds get lossy text, image kinds raw bytes
        assert_eq!(
            load_preview(&small.to_string_lossy(), ArtifactKind::Text),
            Preview::Text("inhalt".into())
        );
        assert_eq!(
            load_preview(&small.to_string_lossy(), ArtifactKind::Image),
            Preview::Image(b"inhalt".to_vec())
        );
    }

    #[test]
    fn preview_cap_is_enforced_during_the_read() {
        let tmp = std::env::temp_dir().join("vibe-artifacts-test-c");
        std::fs::create_dir_all(&tmp).unwrap();
        let big = tmp.join("big.bin");
        std::fs::write(&big, [b'x'; 100]).unwrap();
        // a file bigger than the cap: rejected naming the on-disk size,
        // with at most `cap + 1` bytes ever read (TOCTOU fix)
        assert_eq!(
            load_preview_capped(&big.to_string_lossy(), 64),
            Preview::TooLarge(100)
        );
        // exactly the cap still fits, byte for byte
        assert_eq!(
            load_preview_capped(&big.to_string_lossy(), 100),
            Preview::Image(vec![b'x'; 100])
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The cap must hold for files that understate their size: procfs
    /// reports 0 bytes but generates well over 64 when read, exactly like
    /// a file growing between checking and reading. The pre-fix code
    /// loaded such files whole; `take(cap + 1)` caps them instead.
    #[cfg(target_os = "linux")]
    #[test]
    fn preview_caps_files_that_understate_their_size() {
        assert_eq!(
            load_preview_capped("/proc/self/status", 64),
            Preview::TooLarge(65)
        );
    }

    #[test]
    fn csv_quoting_escapes_and_caps() {
        let rows = parse_csv("a,b,\"c,d\"\n\"er sagt \"\"hi\"\"\",2,3\nx\r\n");
        let rows: Vec<Vec<String>> = rows;
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b".to_string(), "c,d".to_string()],
                vec![
                    "er sagt \"hi\"".to_string(),
                    "2".to_string(),
                    "3".to_string()
                ],
                vec!["x".to_string()],
            ]
        );
        assert!(parse_csv("").is_empty());
        assert_eq!(parse_csv("a"), vec![vec!["a".to_string()]]);
        // column cap: the 101st field of a row is dropped
        let wide = format!("{}\n", "x,".repeat(150));
        let rows = parse_csv(&wide);
        assert_eq!(rows[0].len(), MAX_CSV_COLS);
        // row cap
        let tall = "z\n".repeat(MAX_CSV_ROWS + 50);
        assert_eq!(parse_csv(&tall).len(), MAX_CSV_ROWS);
    }

    #[test]
    fn text_preview_is_lossy() {
        assert_eq!(
            text_preview(vec![0x61, 0xff, 0x62]),
            Preview::Text("a\u{fffd}b".into())
        );
    }

    #[test]
    fn file_name_strips_directories() {
        assert_eq!(file_name("/a/b/c.md"), "c.md");
        assert_eq!(file_name("plain.md"), "plain.md");
    }

    #[test]
    fn json_diff_content_parses() {
        // the wire shape the collect() path is built on
        let c: ToolCallContent =
            serde_json::from_value(json!({"type":"diff","path":"/x/y.rs","newText":"n"})).unwrap();
        assert!(matches!(c, ToolCallContent::Diff { path, .. } if path == "/x/y.rs"));
        let _ = std::io::stdout().flush();
    }
}
