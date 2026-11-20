//! The Artefakte view in the main area: every file the agent touched, grouped
//! by project, with a preview by file type.

use super::transcript::{comment_section, diff_body, send_comments, Comments, MessageOp};
use super::*;
use crate::artifacts::{self, ArtifactKind, Preview};
use crate::i18n::{self, tr, trf};
use crate::render::RenderCtx;
use crate::widgets::line_galley;
use egui::{pos2, Sense};

pub(super) fn view(h: &mut Harness, ui: &mut egui::Ui) {
    let found = artifacts::collect(&h.sessions);
    let selected = h.artifact_selected.clone();
    let mut pick: Option<String> = None;
    let mut close = false;
    egui::SidePanel::left("artifacts_list")
        .exact_width(300.0)
        .resizable(false)
        .frame(
            egui::Frame::none()
                .fill(theme::SIDEBAR)
                .inner_margin(egui::Margin::symmetric(10.0, 12.0)),
        )
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                theme::section_header(ui, tr("Artifacts"), true);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Close, tr("Back to the session")).side(22.0))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if found.is_empty() {
                        ui.label(
                            RichText::new(tr("No edited files in open sessions yet."))
                                .color(theme::DIM)
                                .small(),
                        );
                    }
                    let mut last: Option<PathBuf> = None;
                    for a in &found {
                        if last.as_ref() != Some(&a.cwd) {
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
                                paint_workspace_icon(h, ui, r.min, &a.cwd, 11.0);
                                ui.label(
                                    RichText::new(h.workspace_name(&a.cwd))
                                        .color(theme::MUTED)
                                        .size(13.0),
                                );
                            });
                            ui.add_space(2.0);
                            last = Some(a.cwd.clone());
                        }
                        let is_selected = selected.as_deref() == Some(a.path.as_str());
                        let size = if a.exists {
                            human_size(a.size)
                        } else {
                            tr("missing").to_string()
                        };
                        let name = artifacts::file_name(&a.path).to_string();
                        if file_row(ui, &name, &size, a.exists, is_selected)
                            .on_hover_text(a.path.clone())
                            .clicked()
                        {
                            pick = Some(a.path.clone());
                        }
                    }
                });
        });
    if close {
        h.artifacts_open = false;
    }
    if let Some(p) = pick {
        // an agent-named path can resolve outside its workspace: ask first
        // (the read is capped, but e.g. a key file should not appear in the
        // UI without a deliberate click)
        let base = found
            .iter()
            .find(|a| a.path == p)
            .map(|a| a.cwd.clone())
            .or_else(|| h.current_project.clone());
        let outside = base
            .as_deref()
            .map(|b| crate::render::is_outside_workspace(b, std::path::Path::new(&p)))
            .unwrap_or(false);
        if outside {
            let session_id = found
                .iter()
                .find(|a| a.path == p)
                .map(|a| a.session_id.clone());
            h.ui.confirm = Some(crate::harness::state::Confirm::PreviewOutside {
                session_id,
                path: std::path::PathBuf::from(&p),
                diff: false,
                artifacts: true,
            });
        } else {
            h.artifact_selected = Some(p);
        }
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(20.0, 14.0)))
        .show_inside(ui, |ui| match selected {
            Some(path) => {
                // the artifact's own workspace, else the current project;
                // the check is cached, so the per-frame call is cheap
                let base = found
                    .iter()
                    .find(|a| a.path == path)
                    .map(|a| a.cwd.clone())
                    .or_else(|| h.current_project.clone());
                // uncached on purpose: the badge re-checks every frame
                let outside = base
                    .as_deref()
                    .map(|b| crate::render::is_outside_workspace(b, std::path::Path::new(&path)))
                    .unwrap_or(false);
                preview(ui, &path, outside)
            }
            None => {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(tr("Pick a file on the left.")).color(theme::DIM));
                });
            }
        });
}

fn file_row(
    ui: &mut egui::Ui,
    name: &str,
    size: &str,
    exists: bool,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    widgets::paint_row_bg(ui, rect, resp.hovered(), selected);
    crate::icons::paint(
        ui.painter(),
        Icon::File,
        pos2(rect.left() + 8.0, rect.center().y - 3.5),
        1.0,
        theme::DIM,
    );
    let color = if exists { theme::DIM } else { theme::YELLOW };
    let g = line_galley(ui, size, widgets::mono(11.0), color, 70.0);
    let mut right = rect.right() - 8.0;
    right -= g.size().x;
    widgets::paint_line(ui, right, rect.center().y, g);
    let x = rect.left() + 22.0;
    let g = line_galley(
        ui,
        name,
        widgets::font(13.5),
        if selected { theme::TEXT } else { theme::MUTED },
        right - x - 8.0,
    );
    widgets::paint_line(ui, x, rect.center().y, g);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The preview pane on the right of the transcript: file name plus path
/// relative to the project, a Code/Diff toggle, and the content with line
/// numbers. While it is open the right work panel stays hidden.
pub(super) fn preview_panel(h: &mut Harness, ctx: &egui::Context) {
    use crate::harness::state::PreviewMode;
    let Some(p) = h.ui.preview.clone() else {
        return;
    };
    let mut close = false;
    let mut mode = p.mode;
    let mut ops: Vec<MessageOp> = Vec::new();
    // paths are shown relative to the active session's workspace (or the
    // current project); the absolute path stays on the hover tooltip
    let base = h
        .active
        .as_deref()
        .and_then(|sid| h.sessions.get(sid))
        .map(|v| v.cwd.clone())
        .or_else(|| h.current_project.clone());
    // git for the diff runs in the workspace when the path is inside it,
    // else in the file's own parent — and only when THAT folder is trusted
    let git_dir = git_dir_for(&p.path, base.as_deref());
    let git_trusted = h.trust.get(&git_dir).is_some_and(|t| t.trusted);
    let out = egui::SidePanel::right("harness_preview")
        .resizable(true)
        .default_width(p.width)
        .width_range(220.0..=640.0)
        .frame(
            egui::Frame::none()
                .fill(theme::SIDEBAR)
                .inner_margin(egui::Margin::symmetric(10.0, 12.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(artifacts::file_name(&p.path))
                            .color(theme::TEXT)
                            .size(13.5),
                    )
                    .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(IconButton::new(Icon::Close, tr("Close preview")).side(22.0))
                        .clicked()
                    {
                        close = true;
                    }
                    ui.add_space(4.0);
                    if ui
                        .selectable_label(mode == PreviewMode::Diff, tr("Diff"))
                        .clicked()
                    {
                        mode = PreviewMode::Diff;
                    }
                    if ui
                        .selectable_label(mode == PreviewMode::Code, tr("Code"))
                        .clicked()
                    {
                        mode = PreviewMode::Code;
                    }
                });
            });
            let shown = match &base {
                Some(b) => std::path::Path::new(&p.path)
                    .strip_prefix(b)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| p.path.clone()),
                None => p.path.clone(),
            };
            ui.add(
                egui::Label::new(
                    RichText::new(&shown)
                        .monospace()
                        .size(11.0)
                        .color(theme::DIM),
                )
                .truncate(),
            )
            .on_hover_text(&p.path);
            // re-check every frame, not only at open: a symlink swapped in
            // after the confirm must flip the badge (the read itself is
            // capped and non-blocking, but it must not look trusted)
            let mut outside = p.outside_workspace;
            if let Some(b) = base.as_deref() {
                outside = crate::render::is_outside_workspace(b, std::path::Path::new(&p.path));
            }
            if outside {
                outside_workspace_badge(ui);
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match mode {
                    PreviewMode::Code => preview_content(ui, &p.path, true),
                    PreviewMode::Diff => {
                        // review comments work here like in the transcript;
                        // the draft names the path as shown above
                        let key = format!("preview::{}", p.path);
                        let mut comments =
                            Comments::new(&mut h.ui.diff_comments, &mut h.ui.comment_edit);
                        // git scans (and so the diff's HEAD read) only run
                        // for folders the agent reported as trusted
                        diff_content(
                            ui,
                            &p.path,
                            base.as_deref(),
                            git_trusted,
                            (&key, &mut comments, &mut ops),
                        );
                        comment_section(ui, &key, &shown, &mut comments, &mut ops);
                    }
                });
        });
    for op in ops {
        match op {
            MessageOp::CommentAt { key, line } => {
                h.ui.comment_edit = Some(crate::harness::state::CommentEdit {
                    key,
                    line,
                    text: String::new(),
                });
            }
            MessageOp::SendComments { key, path } => send_comments(h, &key, &path),
            // the diff view queues nothing else
            _ => {}
        }
    }
    if let Some(p) = h.ui.preview.as_mut() {
        p.width = out.response.rect.width();
        p.mode = mode;
    }
    if close {
        h.ui.preview = None;
    }
}

/// The working-tree file against its last committed version, rendered with
/// the transcript's red/green line diff (not the raw `git diff` text). The
/// HEAD blob and the file text are cached by path, mtime and size (git and
/// the file read are subprocesses; they must not run every frame).
/// The folder git runs in for a previewed path: the workspace when the
/// path lies inside it, else the file's own parent (a changed file above a
/// sub-folder project). The caller gates this folder on trust.
fn git_dir_for(path: &str, base: Option<&std::path::Path>) -> std::path::PathBuf {
    base.filter(|b| std::path::Path::new(path).starts_with(b))
        .map(|b| b.to_path_buf())
        .unwrap_or_else(|| {
            std::path::Path::new(path)
                .parent()
                .unwrap_or(std::path::Path::new("/"))
                .to_path_buf()
        })
}

fn diff_content(
    ui: &mut egui::Ui,
    path: &str,
    base: Option<&std::path::Path>,
    trusted: bool,
    comment: (&str, &mut Comments, &mut Vec<MessageOp>),
) {
    type DiffKey = (String, String, Option<u64>, u64, bool);
    enum DiffEntry {
        // Read still running in its background thread. `since` is the fall
        // back: a worker that died (its panic is caught, it stores nothing)
        // or never finished leaves the entry parked, so after the grace —
        // larger than the worst legitimate run (probe + cat-file, 2 s each)
        // — the next frame starts a fresh worker: no diff is stuck on
        // "being prepared" forever, and a panicking worker cannot respawn
        // per frame.
        Pending { since: std::time::Instant },
        Ready((crate::harness::watch::HeadBlob, String)),
    }
    const DIFF_RETRY_GRACE: std::time::Duration = std::time::Duration::from_secs(15);
    type DiffCache = std::sync::Mutex<std::collections::HashMap<DiffKey, DiffEntry>>;
    static CACHE: std::sync::OnceLock<DiffCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let meta = std::fs::metadata(path);
    let mtime = meta
        .as_ref()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    if len > artifacts::MAX_PREVIEW_BYTES {
        // same notice as the code-mode preview: never read the whole file
        notice(
            ui,
            &trf(
                "Too large for the preview ({size}, max. {max}).",
                &[
                    ("size", &human_size(len)),
                    ("max", &human_size(artifacts::MAX_PREVIEW_BYTES)),
                ],
            ),
        );
        return;
    }
    // git runs in the workspace, or next to the file when it lies
    // outside (a changed file above a sub-folder project)
    let dir = git_dir_for(path, base);
    // the git folder and the trust state are part of the key: a diff cached
    // for another folder (session switch) or while the folder was untrusted
    // must not be served
    let key: DiffKey = (
        path.to_string(),
        dir.to_string_lossy().into_owned(),
        mtime,
        len,
        trusted,
    );
    // The git side runs OFF the UI thread: probe plus `cat-file` can take
    // up to their two deadlines EACH (~4 s in a slow or hostile repo), and
    // worse, the parked `rev-parse`/config probe of a FIFO-trapped repo
    // blocks the open forever — none of that may stall a frame anymore.
    // A miss marks the entry `Pending` and starts ONE background thread;
    // it stores `Ready`, requests a repaint, and the next frame paints the
    // diff. Meanwhile the frame below shows that the diff is coming.
    let entry_ready = {
        let mut c = cache.lock().unwrap();
        match c.get(&key) {
            Some(DiffEntry::Ready(v)) => Some(v.clone()),
            // a worker within its grace: the notice below says the diff is
            // coming; outside the grace the miss branch restarts it
            Some(DiffEntry::Pending { since }) if since.elapsed() < DIFF_RETRY_GRACE => None,
            _ => {
                c.insert(
                    key.clone(),
                    DiffEntry::Pending {
                        since: std::time::Instant::now(),
                    },
                );
                let rel = std::path::Path::new(path)
                    .strip_prefix(&dir)
                    .map(|r| r.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string());
                let ctx = ui.ctx().clone();
                let dir = dir.clone();
                let path = path.to_string();
                std::thread::spawn(move || {
                    // The compute is panic-caught: nothing in it may poison
                    // the cache mutex (it is only locked AFTER the compute),
                    // and a panic must not leave a WRONG answer either — the
                    // entry simply stays `Pending` with its original
                    // `since`, so the grace in `DiffEntry::Pending` retries
                    // later instead of showing a misleading "no changes".
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // one bounded PLUMBING call: `HEAD:./path` is
                        // relative to the folder git runs in; no commit yet,
                        // untracked file or a gitlink -> non-zero exit ->
                        // Missing. Never `git show` here: on a gitlink it
                        // resolves to a commit and applies the repo's
                        // textconv (RCE, PoC-verified). The output is capped
                        // at the preview size: a huge blob counts as "no old
                        // version". And only in a folder the agent reported
                        // as trusted — a shipped .git/config is executable
                        // by git.
                        let old = if trusted {
                            crate::harness::watch::head_blob_sized(
                                &dir,
                                &rel,
                                artifacts::MAX_PREVIEW_BYTES as usize,
                            )
                        } else {
                            crate::harness::watch::HeadBlob::Missing
                        };
                        // capped, handle-based read — never a whole-file
                        // read_to_string on an agent-named path, never a
                        // blocking FIFO open (see
                        // artifacts::load_preview_capped / open_nonblock)
                        let current = match artifacts::load_text_capped(&path) {
                            artifacts::Preview::Text(t) => t,
                            _ => String::new(),
                        };
                        (old, current)
                    }));
                    if let Ok((old, current)) = result {
                        let mut c = cache.lock().unwrap();
                        c.insert(key.clone(), DiffEntry::Ready((old, current)));
                        if c.len() > 64 {
                            let k = c.keys().next().cloned();
                            if let Some(k) = k {
                                c.remove(&k);
                            }
                        }
                        drop(c);
                    }
                    // the result is here: paint it as soon as possible
                    ctx.request_repaint();
                });
                None
            }
        }
    };
    use crate::harness::watch::HeadBlob;
    // Stale-while-revalidate: the file changed while its preview is open,
    // so this frame waits for a NEW diff — but an old FINISHED one for the
    // same path may still sit in the cache (its key carries the previous
    // mtime/size). Painting that instead of the placeholder keeps the
    // preview from flashing "being prepared" on every agent write: the
    // stale diff is briefly outdated, never misleading — the fresh one
    // replaces it a frame later. The linear scan only runs on a miss and
    // stays within the cache cap.
    let (old, current) = if let Some(v) = entry_ready {
        v
    } else {
        let stale = {
            let c = cache.lock().unwrap();
            c.iter()
                .find(|((p, ..), v)| p == path && matches!(v, DiffEntry::Ready(_)))
                .and_then(|(_, v)| {
                    if let DiffEntry::Ready(v) = v {
                        Some(v.clone())
                    } else {
                        None
                    }
                })
        };
        match stale {
            Some(v) => v,
            None => {
                notice(ui, tr("Diff is being prepared …"));
                return;
            }
        }
    };
    match (trusted, old) {
        // git runs only in trusted folders; without that there is no old
        // version and the diff would be misleading
        (false, _) => notice(ui, tr("Diff preview needs a trusted folder.")),
        // a repository git refuses: no answer is not "every line is new"
        (_, HeadBlob::Refused) => notice(
            ui,
            tr("Diff preview is off: Catwright runs no git in this repository."),
        ),
        (_, HeadBlob::Blob(old)) if old != current => {
            diff_body(ui, Some(&old), &current, 400, Some(comment))
        }
        (_, HeadBlob::Blob(_)) => notice(ui, tr("No changes against the last commit.")),
        // no commit yet, or the file is untracked: every line is new
        (_, HeadBlob::Missing) if !current.is_empty() => {
            diff_body(ui, None, &current, 400, Some(comment))
        }
        (_, HeadBlob::Missing) => notice(ui, tr("No changes against the last commit.")),
    }
}

/// The yellow warning line under a preview header when the file lies outside
/// the session's workspace.
fn outside_workspace_badge(ui: &mut egui::Ui) {
    ui.add(
        egui::Label::new(
            RichText::new(tr("Outside the workspace"))
                .color(theme::YELLOW)
                .size(11.0),
        )
        .truncate(),
    )
    .on_hover_text(tr("This file lies outside the session's workspace."));
}

pub(super) fn preview(ui: &mut egui::Ui, path: &str, outside_workspace: bool) {
    let kind = artifacts::kind_of(path);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(artifacts::file_name(path))
                .color(theme::TEXT)
                .family(crate::fonts::bold())
                .size(16.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(IconButton::new(
                    Icon::Folder,
                    tr("Open folder in the file manager"),
                ))
                .clicked()
            {
                open_folder_of(path);
            }
            if kind == ArtifactKind::Html
                && ui
                    .add(IconButton::new(Icon::Globe, tr("Open in browser")))
                    .clicked()
            {
                open_in_browser(path);
            }
        });
    });
    ui.add(
        egui::Label::new(RichText::new(path).monospace().size(11.5).color(theme::DIM)).truncate(),
    );
    if outside_workspace {
        outside_workspace_badge(ui);
    }
    ui.add_space(10.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| preview_content(ui, path, false));
}

/// The preview content of one file, without any header. `numbered`: code
/// blocks get line numbers (the preview pane).
fn preview_content(ui: &mut egui::Ui, path: &str, numbered: bool) {
    let kind = artifacts::kind_of(path);
    // The version of the file on disk (see `cached_preview`): the preview
    // below borrows it without a per-frame copy.
    let preview = cached_preview(path, kind);
    // Identity, not content: the scroll states of this file's blocks stay
    // its own and survive a growing text (review N16).
    let code_id = egui::Id::new(("artifact_code", path));
    match preview.as_ref() {
        Preview::Text(text) => match kind {
            ArtifactKind::Markdown => {
                let blocks = crate::markdown::parse_cached(text);
                // read-only preview: interaction state is local to this
                // frame, ui-js blocks fall back to their source
                let mut events = Vec::new();
                let mut forms = std::collections::HashMap::new();
                let mut rctx = RenderCtx::new(&mut events, &mut forms, "artifacts");
                rctx.scroll_salt = crate::memo::hash_str(path);
                crate::render::render_blocks(ui, &blocks, 0, &mut rctx);
            }
            ArtifactKind::Csv => {
                let mut grid = artifacts::parse_csv(text).into_iter();
                let columns: Vec<Vec<crate::markdown::MdInline>> = grid
                    .next()
                    .map(|r| r.iter().map(|c| crate::markdown::parse_inline(c)).collect())
                    .unwrap_or_default();
                let body: Vec<Vec<Vec<crate::markdown::MdInline>>> = grid
                    .map(|row| {
                        row.iter()
                            .map(|c| crate::markdown::parse_inline(c))
                            .collect()
                    })
                    .collect();
                crate::render::render_table_grid(
                    ui,
                    egui::Id::new(("artifact_csv", path)),
                    &columns,
                    &body,
                    None,
                );
            }
            ArtifactKind::Html => crate::render::render_code_block(ui, code_id, "html", text),
            _ => {
                if numbered {
                    crate::render::render_code_block_lines(ui, code_id, "", text)
                } else {
                    crate::render::render_code_block(ui, code_id, "", text)
                }
            }
        },
        Preview::Image(bytes) => match png_texture(ui.ctx(), path, bytes) {
            Ok(tex) => {
                ui.add(egui::Image::from_texture(&tex).max_width(ui.available_width()));
            }
            Err(PngTextureError::TooLarge) => notice(
                ui,
                &trf(
                    "The image is too large for the preview (max. {dim} px, {mem}).",
                    &[
                        (
                            "dim",
                            &format!("{} x {}", PNG_MAX_DIMENSION, PNG_MAX_DIMENSION),
                        ),
                        ("mem", &human_size(PNG_MAX_ALLOC_BYTES)),
                    ],
                ),
            ),
            Err(PngTextureError::Undecodable) => notice(ui, tr("The image could not be decoded.")),
        },
        Preview::Missing => {
            notice(
                ui,
                tr("The file is missing on disk (deleted or reverted?)."),
            );
        }
        Preview::TooLarge(n) => {
            notice(
                ui,
                &trf(
                    "Too large for the preview ({size}, max. {max}).",
                    &[
                        ("size", &human_size(*n)),
                        ("max", &human_size(artifacts::MAX_PREVIEW_BYTES)),
                    ],
                ),
            );
        }
        Preview::Unreadable => notice(ui, tr("The file could not be read.")),
    }
}

fn notice(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::DIM));
}

// egui 0.29 has no image loader without `egui_extras`, so PNG previews are
// decoded here with the `image` crate and uploaded as a texture, cached per
// path and mtime (Handoff 2026-11-09, Bug 4).

/// Highest pixel dimension of a preview image: every real screenshot fits
/// (an 8K screen is 7680 px wide), and a crafted PNG header cannot push
/// the decoder into giant buffers (logic review, M12).
const PNG_MAX_DIMENSION: u32 = 12_000;
/// Total bytes the decoder may allocate per preview image, instead of the
/// crate default (512 MiB with unbounded dimensions). Together with the
/// cap on `PNG_MAX_DIMENSION` this keeps a decode transient under control
/// even with the `to_rgba8()` and texture copies on top.
const PNG_MAX_ALLOC_BYTES: u64 = 128 * 1024 * 1024;
/// Upper bound of the preview texture cache: past it the least recently
/// inserted entry is dropped, so a long session cannot grow it forever.
const PNG_MAX_CACHE_ENTRIES: usize = 32;

/// Why a PNG preview produced no texture.
#[derive(Debug)]
enum PngTextureError {
    /// The image exceeds the preview limits (dimensions or allocation).
    TooLarge,
    /// Not decodable: broken bytes, or no image format at all.
    Undecodable,
}

struct PngEntry {
    mtime: std::time::SystemTime,
    /// Monotonic insertion counter; the smallest one is evicted first.
    seq: u64,
    handle: egui::TextureHandle,
}

fn png_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, PngEntry>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, PngEntry>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Decoding limits for PNG previews: at most [`PNG_MAX_DIMENSION`] pixels
/// per side and [`PNG_MAX_ALLOC_BYTES`] of decoder allocations.
/// `image::load_from_memory` uses the crate defaults (512 MiB, no
/// dimension cap), which let a legal but huge image transiently allocate
/// GiB before the preview showed anything.
fn png_decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(PNG_MAX_DIMENSION);
    limits.max_image_height = Some(PNG_MAX_DIMENSION);
    limits.max_alloc = Some(PNG_MAX_ALLOC_BYTES);
    limits
}

/// The preview's pixel-dimension policy as one pure decision: up to
/// [`PNG_MAX_DIMENSION`] on each side is shown, anything larger is refused
/// as a preview (the file itself stays untouched on disk).
fn dimensions_allowed(width: u32, height: u32) -> bool {
    width <= PNG_MAX_DIMENSION && height <= PNG_MAX_DIMENSION
}

/// `img` scaled down (aspect ratio kept) so neither side exceeds
/// `max_side`; unchanged when it fits.
fn fit_texture_side(img: image::DynamicImage, max_side: usize) -> image::DynamicImage {
    let max_side = u32::try_from(max_side.max(1)).unwrap_or(u32::MAX);
    if img.width() > max_side || img.height() > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    }
}

/// Insert a cache entry, keeping the cache at [`PNG_MAX_CACHE_ENTRIES`]:
/// the entry inserted least recently is evicted first (the mtime-keyed
/// cache never shrank on its own, M12).
fn png_cache_insert(
    cache: &mut std::collections::HashMap<String, PngEntry>,
    path: &str,
    mtime: std::time::SystemTime,
    handle: egui::TextureHandle,
) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.insert(path.to_string(), PngEntry { mtime, seq, handle });
    while cache.len() > PNG_MAX_CACHE_ENTRIES {
        let oldest = cache
            .iter()
            .min_by_key(|(_, entry)| entry.seq)
            .map(|(key, _)| key.clone());
        match oldest {
            Some(key) => {
                cache.remove(&key);
            }
            None => break,
        }
    }
}

fn png_texture(
    ctx: &egui::Context,
    path: &str,
    bytes: &[u8],
) -> Result<egui::TextureHandle, PngTextureError> {
    // no mtime to cache by: no texture (same notice as before)
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .ok_or(PngTextureError::Undecodable)?;
    let mut cache = png_cache().lock().unwrap();
    if let Some(entry) = cache.get(path) {
        if entry.mtime == mtime {
            return Ok(entry.handle.clone());
        }
    }
    // decode under the preview's own limits (M12): an oversized header is
    // rejected by the decoder before any pixel buffer is allocated
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| PngTextureError::Undecodable)?;
    reader.limits(png_decode_limits());
    let img = reader.decode().map_err(|err| match err {
        image::ImageError::Limits(_) => PngTextureError::TooLarge,
        _ => PngTextureError::Undecodable,
    })?;
    // pin the dimension policy on the decoded image as well, before the
    // `to_rgba8()` and texture copies add buffers beyond the decoder's
    // limit accounting
    if !dimensions_allowed(img.width(), img.height()) {
        return Err(PngTextureError::TooLarge);
    }
    // the GPU takes textures only up to `max_texture_side` per side (often
    // 8192): `egui_glow` panics on anything larger, so a picture the
    // dimension policy allows is still scaled down to what the GPU takes
    let img = fit_texture_side(img, ctx.input(|i| i.max_texture_side));
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let handle = ctx.load_texture(
        path.to_string(),
        egui::ColorImage::from_rgba_unmultiplied(size, &rgba.into_raw()),
        egui::TextureOptions::default(),
    );
    png_cache_insert(&mut cache, path, mtime, handle.clone());
    Ok(handle)
}

/// Human-readable file size for list rows ("12,3 kB").
fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < 1024.0 * 1024.0 {
        i18n::decimal(format!("{:.1} kB", b / 1024.0))
    } else {
        i18n::decimal(format!("{:.1} MB", b / (1024.0 * 1024.0)))
    }
}

/// Upper bound of the preview cache: past it the entry inserted least
/// recently is dropped; eight full-size texts stay bounded (~16 MiB).
const PREVIEW_MAX_CACHE_ENTRIES: usize = 8;

/// One cached preview with the file version it was read from.
struct PreviewEntry {
    mtime: Option<std::time::SystemTime>,
    len: u64,
    /// Monotonic insertion counter; the smallest one is evicted first.
    seq: u64,
    preview: std::sync::Arc<Preview>,
}

fn preview_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, PreviewEntry>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, PreviewEntry>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Insert a cache entry, keeping the cache at
/// [`PREVIEW_MAX_CACHE_ENTRIES`]: the entry inserted least recently is
/// evicted first.
fn preview_cache_insert(
    cache: &mut std::collections::HashMap<String, PreviewEntry>,
    path: &str,
    mtime: Option<std::time::SystemTime>,
    len: u64,
    preview: std::sync::Arc<Preview>,
) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cache.insert(
        path.to_string(),
        PreviewEntry {
            mtime,
            len,
            seq,
            preview,
        },
    );
    while cache.len() > PREVIEW_MAX_CACHE_ENTRIES {
        let oldest = cache
            .iter()
            .min_by_key(|(_, entry)| entry.seq)
            .map(|(key, _)| key.clone());
        match oldest {
            Some(key) => {
                cache.remove(&key);
            }
            None => break,
        }
    }
}

/// The file's preview, read once per file version (review M10): the view
/// runs every frame, `artifacts::load_preview` caches each version too —
/// but hands out a CLONE of up to [`artifacts::MAX_PREVIEW_BYTES`] per
/// call, a 2 MiB copy per frame on the UI thread. This Arc keeps the one
/// instance; every following frame borrows it until (mtime, size) say
/// the file changed.
fn cached_preview(path: &str, kind: ArtifactKind) -> std::sync::Arc<Preview> {
    let meta = std::fs::metadata(path);
    let mtime = meta.as_ref().ok().and_then(|m| m.modified().ok());
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mut cache = preview_cache().lock().unwrap();
    if let Some(entry) = cache.get(path) {
        if entry.mtime == mtime && entry.len == len {
            return entry.preview.clone();
        }
    }
    let preview = std::sync::Arc::new(artifacts::load_preview(path, kind));
    preview_cache_insert(&mut cache, path, mtime, len, preview.clone());
    preview
}

/// Open an agent-written HTML file in the browser (user-triggered, never
/// a WebView inside the app). Spawned and reaped on its own thread, so
/// the helper leaves no zombie and blocks no frame (review N11).
fn open_in_browser(path: &str) {
    spawn_xdg_open(path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn tiny_png(path: &std::path::Path, color: [u8; 4]) {
        let img = image::RgbaImage::from_pixel(2, 1, image::Rgba(color));
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(img.as_raw(), 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Bug 4 (Handoff 2026-11-09): PNG previews decode without an egui image
    /// loader and the texture is cached until the file's mtime changes.
    #[test]
    fn png_texture_decodes_and_caches_until_mtime_changes() {
        let dir = std::env::temp_dir().join(format!(
            "od-png-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("shot.png");
        tiny_png(&png, [255, 0, 0, 255]);
        let path = png.to_string_lossy().into_owned();
        let bytes = std::fs::read(&png).unwrap();
        let ctx = egui::Context::default();

        let a = png_texture(&ctx, &path, &bytes).expect("decodes");
        let b = png_texture(&ctx, &path, &bytes).expect("cached");
        assert_eq!(a.id(), b.id(), "the same mtime reuses the texture");

        // a new mtime (rewritten file) decodes a fresh texture
        std::thread::sleep(std::time::Duration::from_millis(10));
        tiny_png(&png, [0, 0, 255, 255]);
        let bytes = std::fs::read(&png).unwrap();
        let c = png_texture(&ctx, &path, &bytes).expect("decodes again");
        assert_ne!(a.id(), c.id());

        // broken bytes never produce a texture
        let junk = dir.join("junk.png");
        std::fs::write(&junk, b"not a png").unwrap();
        assert!(png_texture(&ctx, &junk.to_string_lossy(), b"not a png").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M12: dimensions beyond the preview cap are refused as one pure
    /// decision (`dimensions_allowed`), without building a giant image.
    #[test]
    fn png_dimensions_allowed_covers_screenshots_rejects_giants() {
        assert!(dimensions_allowed(1, 1));
        assert!(dimensions_allowed(PNG_MAX_DIMENSION, PNG_MAX_DIMENSION));
        // one pixel over the cap on either side is refused
        assert!(!dimensions_allowed(PNG_MAX_DIMENSION + 1, 1));
        assert!(!dimensions_allowed(1, PNG_MAX_DIMENSION + 1));
        assert!(!dimensions_allowed(u32::MAX, u32::MAX));
        // the decoder run with the same numbers, not the crate defaults
        let limits = png_decode_limits();
        assert_eq!(limits.max_image_width, Some(PNG_MAX_DIMENSION));
        assert_eq!(limits.max_image_height, Some(PNG_MAX_DIMENSION));
        assert_eq!(limits.max_alloc, Some(PNG_MAX_ALLOC_BYTES));
    }

    /// M12: a PNG whose header exceeds the dimension limit is refused end
    /// to end — the decoder rejects the header before allocating pixel
    /// data, so the preview shows the too-large notice instead of decoding.
    /// The test image is one row of 12001 tiny pixels: only the header
    /// matters, the file stays small.
    #[test]
    fn png_texture_refuses_images_beyond_the_dimension_limit() {
        let dir = std::env::temp_dir().join(format!(
            "od-png-limit-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let img =
            image::RgbaImage::from_pixel(PNG_MAX_DIMENSION + 1, 1, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                img.as_raw(),
                PNG_MAX_DIMENSION + 1,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let png = dir.join("wide.png");
        std::fs::write(&png, &bytes).unwrap();
        let ctx = egui::Context::default();
        match png_texture(&ctx, &png.to_string_lossy(), &bytes) {
            Err(PngTextureError::TooLarge) => {}
            Err(PngTextureError::Undecodable) => panic!("expected TooLarge, got Undecodable"),
            Ok(_) => panic!("expected TooLarge, got a texture"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M12: the PNG texture cache is bounded — past [`PNG_MAX_CACHE_ENTRIES`]
    /// the entry inserted least recently is dropped, so long sessions stop
    /// growing it. A fresh map keeps this test independent of the shared
    /// cache the other png tests run against in parallel.
    #[test]
    fn pictures_beyond_the_gpu_texture_limit_are_scaled_down() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            6,
            40,
            image::Rgba([1, 2, 3, 255]),
        ));
        let same = fit_texture_side(img.clone(), 64);
        assert_eq!((same.width(), same.height()), (6, 40), "fits: untouched");
        let small = fit_texture_side(img, 10);
        assert!(
            small.height() <= 10 && small.width() <= 2,
            "{}x{}",
            small.width(),
            small.height()
        );
        assert!(small.width() >= 1);
    }

    #[test]
    fn png_cache_evicts_the_least_recently_inserted_entry() {
        let ctx = egui::Context::default();
        let mut cache = std::collections::HashMap::new();
        let now = std::time::SystemTime::now();
        for i in 0..(PNG_MAX_CACHE_ENTRIES + 5) {
            let handle = ctx.load_texture(
                format!("tex{i}"),
                egui::ColorImage::new([1, 1], egui::Color32::RED),
                egui::TextureOptions::default(),
            );
            png_cache_insert(&mut cache, &format!("p{i}.png"), now, handle);
            assert!(cache.len() <= PNG_MAX_CACHE_ENTRIES);
        }
        assert_eq!(cache.len(), PNG_MAX_CACHE_ENTRIES);
        // the oldest entries were evicted, the newest survived
        assert!(!cache.contains_key("p0.png"));
        let last = format!("p{}.png", PNG_MAX_CACHE_ENTRIES + 4);
        assert!(cache.contains_key(&last));
    }

    /// M10: the preview is read once per file version and shared — the
    /// view asks every frame, and `artifacts::load_preview` hands out a
    /// clone per call (a 2 MiB copy per frame for a big text). The same
    /// version returns the same instance; a rewritten file reloads.
    #[test]
    fn preview_is_cached_per_file_version() {
        let dir = std::env::temp_dir().join(format!(
            "od-preview-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notiz.md");
        std::fs::write(&file, "erste Fassung").unwrap();
        let path = file.to_string_lossy().into_owned();
        let kind = artifacts::kind_of(&path);

        let a = cached_preview(&path, kind);
        let b = cached_preview(&path, kind);
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "the same file version is read and kept once"
        );
        assert_eq!(*a, Preview::Text("erste Fassung".to_string()));

        // a rewritten file is a new version: read again
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&file, "zweite, laengere Fassung").unwrap();
        let c = cached_preview(&path, kind);
        assert!(
            !std::sync::Arc::ptr_eq(&a, &c),
            "a changed file reloads its preview"
        );
        assert_eq!(*c, Preview::Text("zweite, laengere Fassung".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M10: the preview cache is bounded like the texture cache — past
    /// [`PREVIEW_MAX_CACHE_ENTRIES`] the entry inserted least recently
    /// is dropped, so browsing cannot grow it without bound.
    #[test]
    fn preview_cache_evicts_the_least_recently_inserted_entry() {
        let mut cache = std::collections::HashMap::new();
        let now = std::time::SystemTime::now();
        let text = Preview::Text("x".to_string());
        for i in 0..(PREVIEW_MAX_CACHE_ENTRIES + 5) {
            preview_cache_insert(
                &mut cache,
                &format!("p{i}.md"),
                Some(now),
                i as u64,
                std::sync::Arc::new(text.clone()),
            );
            assert!(cache.len() <= PREVIEW_MAX_CACHE_ENTRIES);
        }
        assert_eq!(cache.len(), PREVIEW_MAX_CACHE_ENTRIES);
        // the oldest entries were evicted, the newest survived
        assert!(!cache.contains_key("p0.md"));
        let last = format!("p{}.md", PREVIEW_MAX_CACHE_ENTRIES + 4);
        assert!(cache.contains_key(&last));
    }
}
