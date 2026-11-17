//! Reusable modal dialogs: a dimmed backdrop plus a centred card window.
//! Used by the harness for the folder and file pickers and for the
//! confirmations. UI strings are German; nothing is logged.

use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets::{self, ButtonKind, IconButton};
use egui::{Color32, Key, RichText, Sense, Stroke};
use std::path::{Path, PathBuf};

/// How a modal was closed during this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalClose {
    /// Still open.
    Open,
    /// ESC was pressed.
    Escape,
    /// A click landed on the dimmed backdrop.
    Backdrop,
    /// The "×" in the header row was clicked (only `modal_sized` has one).
    Button,
}

/// One modal frame: dims the whole screen (black, alpha ~160) and shows a
/// centred, borderless-title `egui::Window` (CARD fill, 1 px BORDER, radius 4)
/// with `title` as a section header. Clicks on the backdrop and ESC close the
/// modal. Returns the closure's return value while open, `None` on the frame
/// the modal closes, plus how it closed.
pub fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (Option<R>, ModalClose) {
    // previous behaviour: at least 360 px wide, height fits the content
    modal_inner(ctx, id, title, egui::vec2(360.0, 0.0), false, add)
}

/// Like `modal`, but the window is at least `size` big and gets a "×" in the
/// header row (top right) which closes the modal like ESC/backdrop do.
pub fn modal_sized<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    size: egui::Vec2,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (Option<R>, ModalClose) {
    modal_inner(ctx, id, title, size, true, add)
}

fn modal_inner<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    size: egui::Vec2,
    close_button: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (Option<R>, ModalClose) {
    let backdrop_id = egui::Id::new(("modal_backdrop", id));
    let backdrop_layer = egui::LayerId::new(egui::Order::Middle, backdrop_id);
    let screen = ctx.screen_rect();
    let esc = ctx.input(|i| i.key_pressed(Key::Escape));
    // an invisible full-screen Area on the backdrop layer catches every
    // click that does not land on the window (the window lives above it)
    let backdrop_clicked = egui::Area::new(backdrop_id)
        .order(egui::Order::Middle)
        .anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
        .movable(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::click())
                .1
                .clicked()
        })
        .inner;
    let close = if esc {
        ModalClose::Escape
    } else if backdrop_clicked {
        ModalClose::Backdrop
    } else {
        ModalClose::Open
    };
    if close != ModalClose::Open {
        return (None, close);
    }
    ctx.layer_painter(backdrop_layer).rect_filled(
        screen,
        egui::Rounding::same(0.0),
        Color32::from_black_alpha(160),
    );
    let mut result = None;
    let mut button_close = false;
    let window = egui::Window::new("")
        .id(egui::Id::new(("modal_window_widget", id)))
        .order(egui::Order::Foreground)
        .title_bar(false)
        .collapsible(false)
        .resizable(false);
    // A sized modal (height given) gets a FIXED size: with only a minimum,
    // content that asks for "all available width" (scroll areas, grids)
    // makes the auto-sized window grow up to the screen width.
    let window = if size.y > 0.0 {
        window.fixed_size(size)
    } else {
        window.min_width(size.x)
    };
    window
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(
            egui::Frame::default()
                .fill(theme::CARD)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::same(16.0)),
        )
        .show(ctx, |ui| {
            if size.y > 0.0 {
                // pin the content area: neither shrink to the content nor
                // grow past the requested size
                let inner = size - egui::vec2(32.0, 32.0); // frame margins
                ui.set_min_size(inner);
                ui.set_max_width(inner.x);
            }
            if close_button {
                ui.horizontal(|ui| {
                    theme::section_header(ui, title, true);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(IconButton::new(Icon::Close, tr("Close (Esc)")))
                            .clicked()
                        {
                            button_close = true;
                        }
                    });
                });
            } else {
                theme::section_header(ui, title, true);
            }
            ui.add_space(8.0);
            result = Some(add(ui));
        });
    if button_close {
        return (None, ModalClose::Button);
    }
    (result, ModalClose::Open)
}

/// A confirmation modal: `text`, an tr("Cancel") button and the confirm button
/// (`danger` = RED outline, otherwise ORANGE filled). Enter confirms; ESC,
/// backdrop click and tr("Cancel") return `Some(false)`; while open without a
/// decision the result is `None`.
pub fn confirm(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    text: &str,
    confirm_label: &str,
    danger: bool,
) -> Option<bool> {
    let mut clicked = None;
    let (_r, close) = modal(ctx, id, title, |ui| {
        ui.set_max_width(440.0);
        ui.label(RichText::new(text).color(theme::TEXT).size(14.0));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                clicked = Some(false);
            }
            let kind = if danger {
                ButtonKind::Danger
            } else {
                ButtonKind::Primary
            };
            if widgets::button(ui, confirm_label, kind).clicked() {
                clicked = Some(true);
            }
        });
    });
    match close {
        ModalClose::Escape | ModalClose::Backdrop | ModalClose::Button => Some(false),
        ModalClose::Open => {
            if ctx.input(|i| i.key_pressed(Key::Enter)) {
                Some(true)
            } else {
                clicked
            }
        }
    }
}

/// What the folder picker ended with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerResult {
    /// The user picked the current `dir`.
    Picked(PathBuf),
    /// ESC, backdrop click or tr("Cancel").
    Cancelled,
}

/// A modal folder browser: editable path row, Home/parent buttons, a
/// tr("Hidden") checkbox and the sorted subfolder list of `dir`.
pub struct FolderPicker {
    pub dir: PathBuf,
    pub input: String,
    pub show_hidden: bool,
    pub error: String,
    /// Pull the keyboard focus into the path row on the first frame only:
    /// opening via the keyboard (palette Enter) left the focus in whatever
    /// widget had it underneath the modal, and typed keys landed there.
    focus_on_open: bool,
}

impl FolderPicker {
    pub fn new(start: PathBuf) -> Self {
        Self {
            input: start.to_string_lossy().into_owned(),
            dir: start,
            show_hidden: false,
            error: String::new(),
            focus_on_open: true,
        }
    }

    /// Enter in the path row: switch into the folder when it exists,
    /// otherwise record a German error.
    fn enter_input(&mut self) {
        let trimmed = self.input.trim().to_string();
        let p = PathBuf::from(&trimmed);
        if p.is_dir() {
            self.dir = p;
            self.input = self.dir.to_string_lossy().into_owned();
            self.error.clear();
        } else {
            self.error = trf(
                "“{path}” is not an existing directory.",
                &[("path", &trimmed)],
            );
        }
    }

    fn go_to(&mut self, dir: PathBuf) {
        self.input = dir.to_string_lossy().into_owned();
        self.dir = dir;
        self.error.clear();
    }

    /// Render one frame; `Some(..)` when the picker is done this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<PickerResult> {
        let mut result = None;
        let (_r, close) = modal(ctx, "folder_picker", tr("Add project"), |ui| {
            ui.set_min_width(440.0);
            // editable path row (Enter switches into the folder)
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("/pfad/zum/projekt")
                        .desired_width(f32::INFINITY),
                );
                // same focus move as the rename dialog, but only once: a
                // per-frame request would fight the user for the focus in
                // the rows and controls below
                if self.focus_on_open {
                    resp.request_focus();
                    self.focus_on_open = false;
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.enter_input();
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add(IconButton::new(Icon::User, tr("Home directory")))
                    .clicked()
                {
                    if let Some(home) =
                        directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
                    {
                        self.go_to(home);
                    }
                }
                if ui
                    .add(IconButton::new(Icon::ChevronUp, tr("Parent folder")))
                    .clicked()
                {
                    if let Some(parent) = parent_dir(&self.dir) {
                        self.go_to(parent);
                    }
                }
                ui.checkbox(&mut self.show_hidden, tr("Hidden"));
            });
            ui.add_space(4.0);
            // read errors (e.g. missing permissions) surface as `error`, no panic
            let entries = match list_subdirs(&self.dir, self.show_hidden) {
                Ok(v) => v,
                Err(e) => {
                    self.error = trf(
                        "Folder “{path}” cannot be read: {error}",
                        &[("path", &self.dir.to_string_lossy()), ("error", &e)],
                    );
                    Vec::new()
                }
            };
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(theme::RED_TEXT).small());
                ui.add_space(4.0);
            }
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if entries.is_empty() && self.error.is_empty() {
                        ui.label(
                            RichText::new(tr("No subfolders."))
                                .color(theme::DIM)
                                .small(),
                        );
                    }
                    for e in &entries {
                        let name = e
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| e.to_string_lossy().into_owned());
                        if entry_row(ui, Icon::Folder, &name) {
                            self.go_to(e.clone());
                        }
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                    result = Some(PickerResult::Cancelled);
                }
                if widgets::button(ui, tr("Add this folder"), ButtonKind::Primary).clicked() {
                    result = Some(PickerResult::Picked(self.dir.clone()));
                }
            });
        });
        match close {
            ModalClose::Open => result,
            ModalClose::Escape | ModalClose::Backdrop | ModalClose::Button => {
                Some(PickerResult::Cancelled)
            }
        }
    }
}

/// One clickable row of a picker list. Returns true on click.
fn entry_row(ui: &mut egui::Ui, icon: Icon, name: &str) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, theme::RAISED);
    }
    icons::paint(
        ui.painter(),
        icon,
        egui::pos2(rect.left() + 6.0, rect.center().y - 7.0),
        2.0,
        theme::DIM,
    );
    let g = widgets::line_galley(
        ui,
        name,
        widgets::font(13.5),
        theme::TEXT,
        rect.width() - 34.0,
    );
    widgets::paint_line(ui, rect.left() + 28.0, rect.center().y, g);
    resp.clicked()
}

/// A modal file browser for attachments: folders open, a click on a file
/// picks it.
pub struct FilePicker {
    pub dir: PathBuf,
    pub show_hidden: bool,
    pub error: String,
    /// Pull the keyboard focus into the modal on the first frame only (see
    /// `FolderPicker::focus_on_open`).
    focus_on_open: bool,
}

impl FilePicker {
    pub fn new(start: PathBuf) -> Self {
        Self {
            dir: start,
            show_hidden: false,
            error: String::new(),
            focus_on_open: true,
        }
    }

    /// Render one frame; `Some(..)` when the picker is done this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<PickerResult> {
        let mut result = None;
        let (_r, close) = modal(ctx, "file_picker", tr("Attach file"), |ui| {
            ui.set_min_width(480.0);
            // The file picker has no text edit to move the focus into, so
            // an invisible focusable sink takes it on open: keystrokes must
            // not fall through to the composer underneath while the modal is
            // up. It stays interactive-free — Enter/Space trigger nothing.
            let sink = ui.interact(
                ui.max_rect(),
                egui::Id::new("file_picker_focus_sink"),
                egui::Sense::focusable_noninteractive(),
            );
            if self.focus_on_open {
                sink.request_focus();
                self.focus_on_open = false;
            }
            ui.horizontal(|ui| {
                if ui
                    .add(IconButton::new(Icon::ChevronUp, tr("Parent folder")))
                    .clicked()
                {
                    if let Some(parent) = parent_dir(&self.dir) {
                        self.dir = parent;
                        self.error.clear();
                    }
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(self.dir.to_string_lossy())
                            .monospace()
                            .size(12.0)
                            .color(theme::MUTED),
                    )
                    .truncate(),
                );
            });
            ui.checkbox(&mut self.show_hidden, tr("Show hidden"));
            ui.add_space(4.0);
            let entries = match list_entries(&self.dir, self.show_hidden) {
                Ok(v) => v,
                Err(e) => {
                    self.error = trf(
                        "Folder “{path}” cannot be read: {error}",
                        &[("path", &self.dir.to_string_lossy()), ("error", &e)],
                    );
                    Vec::new()
                }
            };
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(theme::RED_TEXT).small());
            }
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (path, is_dir) in &entries {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let icon = if *is_dir { Icon::Folder } else { Icon::File };
                        if entry_row(ui, icon, &name) {
                            if *is_dir {
                                self.dir = path.clone();
                                self.error.clear();
                            } else {
                                result = Some(PickerResult::Picked(path.clone()));
                            }
                        }
                    }
                });
            ui.add_space(8.0);
            if widgets::button(ui, tr("Cancel"), ButtonKind::Secondary).clicked() {
                result = Some(PickerResult::Cancelled);
            }
        });
        match close {
            ModalClose::Open => result,
            ModalClose::Escape | ModalClose::Backdrop | ModalClose::Button => {
                Some(PickerResult::Cancelled)
            }
        }
    }
}

/// Folders first, then files, each sorted case-insensitively. Hidden entries
/// (leading '.') are only listed when `show_hidden` is set.
pub fn list_entries(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<(PathBuf, bool)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if !show_hidden && name.to_string_lossy().starts_with('.') {
            continue;
        }
        out.push((entry.path(), entry.path().is_dir()));
    }
    out.sort_by(|(a, a_dir), (b, b_dir)| {
        let key = |p: &PathBuf| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        b_dir.cmp(a_dir).then_with(|| key(a).cmp(&key(b)))
    });
    Ok(out)
}

/// Subdirectories of `dir`, sorted by file name (case-insensitive). Hidden
/// entries (leading '.') are only listed when `show_hidden` is set.
pub fn list_subdirs(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        out.push(entry.path());
    }
    out.sort_by(|a, b| {
        let key = |p: &PathBuf| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        key(a).cmp(&key(b)).then_with(|| a.cmp(b))
    });
    Ok(out)
}

/// Parent of `p` (None at the filesystem root or for a bare relative path).
pub fn parent_dir(p: &Path) -> Option<PathBuf> {
    match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_base(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "catwright-dialogs-{tag}-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn list_subdirs_only_dirs_sorted_case_insensitive_hidden_optional() {
        let base = tmp_base("lsd");
        std::fs::create_dir_all(base.join("b")).unwrap();
        std::fs::create_dir_all(base.join("A")).unwrap();
        std::fs::create_dir_all(base.join(".x")).unwrap();
        std::fs::write(base.join("f.txt"), "x").unwrap();

        let got = list_subdirs(&base, false).unwrap();
        assert_eq!(got, vec![base.join("A"), base.join("b")]);
        let got = list_subdirs(&base, true).unwrap();
        assert_eq!(got, vec![base.join(".x"), base.join("A"), base.join("b")]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_entries_puts_folders_first_and_hides_dotfiles() {
        let base = tmp_base("entries");
        std::fs::create_dir_all(base.join("zeta")).unwrap();
        std::fs::write(base.join("alpha.txt"), "x").unwrap();
        std::fs::write(base.join(".hidden"), "x").unwrap();
        let got = list_entries(&base, false).unwrap();
        assert_eq!(
            got,
            vec![(base.join("zeta"), true), (base.join("alpha.txt"), false)]
        );
        assert_eq!(list_entries(&base, true).unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_subdirs_missing_dir_is_err_not_panic() {
        let missing = Path::new("/definitiv/nicht/vorhanden-xyz-123");
        assert!(list_subdirs(missing, false).is_err());
        assert!(list_subdirs(missing, true).is_err());
    }

    #[test]
    fn parent_dir_cases() {
        assert_eq!(parent_dir(Path::new("/a/b")), Some(PathBuf::from("/a")));
        assert_eq!(parent_dir(Path::new("/a")), Some(PathBuf::from("/")));
        assert_eq!(parent_dir(Path::new("/")), None);
        assert_eq!(parent_dir(Path::new("rel")), None);
    }

    /// One headless egui frame of a picker (the harness test pattern, with
    /// a screen rect so the anchored modal window has a place to be). The
    /// context survives the frames so focus state can be watched.
    fn run_frame(ctx: &egui::Context, mut show: impl FnMut(&egui::Context)) {
        crate::fonts::install_fallback_fonts(ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1360.0, 860.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| show(ctx));
    }

    /// The folder picker moves the keyboard focus into its path row on the
    /// first frame (flag consumed exactly once) — opening it via the
    /// keyboard no longer leaves the focus in the composer underneath —
    /// and later frames keep the focus inside the modal.
    #[test]
    fn folder_picker_takes_the_focus_into_the_path_row_on_open() {
        let ctx = egui::Context::default();
        let mut picker = FolderPicker::new(PathBuf::from("/"));
        assert!(picker.focus_on_open, "armed on construction");
        run_frame(&ctx, |ctx| {
            assert!(picker.show(ctx).is_none(), "the picker stays open");
        });
        assert!(!picker.focus_on_open, "the flag is consumed on frame one");
        assert!(
            ctx.memory(|m| m.focused()).is_some(),
            "the focus moved into the path row"
        );
        run_frame(&ctx, |ctx| {
            picker.show(ctx);
        });
        assert!(
            ctx.memory(|m| m.focused()).is_some(),
            "later frames keep the focus inside the modal"
        );
    }

    /// The file picker has no text edit to focus; its invisible sink takes
    /// the keyboard on open (one-shot flag) and holds it on later frames,
    /// so keystrokes cannot fall through to the composer underneath.
    #[test]
    fn file_picker_focus_sink_takes_and_holds_the_keyboard() {
        let ctx = egui::Context::default();
        let mut picker = FilePicker::new(PathBuf::from("/"));
        assert!(picker.focus_on_open, "armed on construction");
        run_frame(&ctx, |ctx| {
            assert!(picker.show(ctx).is_none(), "the picker stays open");
        });
        assert!(!picker.focus_on_open, "the flag is consumed on frame one");
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("file_picker_focus_sink")),
            "the focus moved into the modal's sink"
        );
        run_frame(&ctx, |ctx| {
            picker.show(ctx);
        });
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("file_picker_focus_sink")),
            "the sink stays registered and keeps the focus"
        );
    }

    #[test]
    fn folder_picker_new_syncs_input_and_dir() {
        let p = FolderPicker::new(PathBuf::from("/tmp"));
        assert_eq!(p.dir, PathBuf::from("/tmp"));
        assert_eq!(p.input, "/tmp");
        assert!(!p.show_hidden);
        assert!(p.error.is_empty());
    }
}
