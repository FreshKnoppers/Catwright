//! The egui application shell: one `Harness` (Vibe over ACP, every
//! workspace in one view), the settings modal and the screenshot mode. No
//! demo content — everything shown comes from real ACP events folded into
//! the harness.

use crate::harness::{ui::HarnessAction, Harness};
use crate::store::{self, Settings};
use std::path::PathBuf;

mod settings;
use settings::SettingsNav;

/// What to show when the app starts (`--open`, `--activity`).
pub enum StartView {
    Activity,
    Preview { path: PathBuf, diff: bool },
}

pub struct App {
    settings: Settings,
    settings_open: bool,
    settings_nav: SettingsNav,
    /// Status line at the bottom of the settings modal (DIM).
    settings_status: String,
    screenshot_path: Option<PathBuf>,
    frames_left: u32,
    harness: Harness,
    /// First start: the language picker is shown until the user confirms.
    language_prompt: bool,
}

impl App {
    pub fn new(
        screenshot_path: Option<PathBuf>,
        settings_tab: Option<String>,
        frames: u32,
        open: Option<(String, Option<String>)>,
        prompt: Option<String>,
        lang: Option<crate::i18n::Lang>,
        start_view: Option<StartView>,
    ) -> Self {
        // Everything under the config folder — settings, the chat
        // workspace with the agent's files, worktree checkouts — is the
        // user's business alone: the folder is 0700 before anything lands
        // in it. A failure does not stop the app; it is said, not hidden.
        let privacy_error = store::ensure_config_dir_private().err();
        let (settings, settings_error) = match store::load_settings() {
            Ok(s) => (s, None),
            Err(msg) => {
                // keep the unreadable file recoverable instead of letting
                // the next save overwrite it (Handoff 2026-11-09, Bug 5)
                let detail = match store::backup_unreadable_settings() {
                    Ok(backup) => crate::i18n::trf(
                        " {path} backed up",
                        &[("path", &backup.to_string_lossy())],
                    ),
                    Err(e) => format!(" ({e})"),
                };
                (Settings::default(), Some(format!("{msg} —{detail}")))
            }
        };
        // `--lang` wins for this run; else the saved choice; else English
        let saved = settings
            .language
            .as_deref()
            .and_then(crate::i18n::Lang::from_code);
        crate::i18n::set_lang(lang.or(saved).unwrap_or(crate::i18n::Lang::En));
        // ask once, on a real first start (not for screenshots or `--lang`)
        let language_prompt = saved.is_none() && lang.is_none() && screenshot_path.is_none();
        let mut harness = Harness::new(
            settings.projects.iter().map(PathBuf::from).collect(),
            settings.default_session_mode.clone(),
        );
        harness.ui_hint = store::vibe_skill_installed();
        if let Some(dir) = store::chat_workspace_dir() {
            harness.set_chat_dir(dir);
        }
        harness.known_models = settings.vibe_models.clone();
        harness.known_thinking = settings.vibe_thinking.clone();
        harness.thinking_by_model = settings.vibe_thinking_by_model.clone();
        harness.ui.sidebar_collapsed = settings.sidebar_collapsed;
        harness.ui.panel_closed = settings.panel_closed;
        harness.ui.transcript_detail = settings.transcript_detail.clone();
        harness.auto_account = settings.auto_account;
        harness.pinned = settings.pinned_sessions.iter().cloned().collect();
        harness.archived = settings.archived_sessions.iter().cloned().collect();
        // claimed in every case: an instance that could not read the
        // settings must not overwrite another instance's file either
        let owns_settings = store::claim_settings();
        if let Some(msg) = settings_error {
            harness.status = msg;
        } else if let Some(msg) = privacy_error {
            harness.status = msg;
        } else if !owns_settings && screenshot_path.is_none() {
            // two windows would overwrite each other's settings.json
            harness.status = crate::i18n::tr(
                "Catwright is already open in another window: changes to the settings and projects made here are not saved.",
            )
            .to_string();
        }
        // CLI `--project <dir> [--session <id>]`
        if let Some((dir, session)) = open {
            if harness.add_project(&dir) {
                if let (Some(sid), Some(cwd)) = (session, harness.current_project.clone()) {
                    harness.open_when_ready(cwd, sid);
                }
            }
        }
        // CLI `--open <file> [--preview-diff]` / `--activity`
        match start_view {
            Some(StartView::Activity) => {
                harness.ui.activity_open = true;
                // a `--session` opened once the agent is ready resets the
                // view; the flag brings the feed back then
                harness.activity_on_ready = true;
            }
            Some(StartView::Preview { path, diff }) => {
                harness.ui.preview = Some(crate::harness::state::PreviewState {
                    path: path.to_string_lossy().into_owned(),
                    width: 380.0,
                    mode: if diff {
                        crate::harness::state::PreviewMode::Diff
                    } else {
                        crate::harness::state::PreviewMode::Code
                    },
                });
            }
            None => {}
        }
        harness.prompt_when_ready(prompt);
        let shot = screenshot_path.is_some();
        let mut app = Self {
            settings,
            settings_open: false,
            settings_nav: SettingsNav::Allgemein,
            settings_status: String::new(),
            screenshot_path,
            frames_left: if shot { frames.max(1) } else { u32::MAX },
            harness,
            language_prompt,
        };
        if let Some(tab) = settings_tab {
            app.settings_open = true;
            app.settings_nav = SettingsNav::from_cli(&tab);
        }
        app
    }

    /// Persist the Vibe models / thinking levels the harness has seen.
    fn sync_vibe_choices(&mut self) {
        if self.harness.known_models != self.settings.vibe_models
            || self.harness.known_thinking != self.settings.vibe_thinking
            || self.harness.thinking_by_model != self.settings.vibe_thinking_by_model
        {
            self.settings.vibe_models = self.harness.known_models.clone();
            self.settings.vibe_thinking = self.harness.known_thinking.clone();
            self.settings.vibe_thinking_by_model = self.harness.thinking_by_model.clone();
            self.autosave();
        }
    }

    /// Persist the sidebar and panel layout when it changed.
    fn sync_layout(&mut self) {
        let collapsed = self.harness.ui.sidebar_collapsed;
        let panel = self.harness.ui.panel_closed;
        let detail = self.harness.ui.transcript_detail.clone();
        if collapsed != self.settings.sidebar_collapsed
            || panel != self.settings.panel_closed
            || detail != self.settings.transcript_detail
        {
            self.settings.sidebar_collapsed = collapsed;
            self.settings.panel_closed = panel;
            self.settings.transcript_detail = detail;
            self.autosave();
        }
    }

    /// Persist pinned sessions into the settings file.
    fn sync_pinned(&mut self) {
        let mut current: Vec<String> = self.harness.pinned.iter().cloned().collect();
        current.sort();
        let mut archived: Vec<String> = self.harness.archived.iter().cloned().collect();
        archived.sort();
        if current != self.settings.pinned_sessions || archived != self.settings.archived_sessions {
            self.settings.pinned_sessions = current;
            self.settings.archived_sessions = archived;
            self.autosave();
        }
    }

    /// Persist harness project changes into the settings file.
    fn sync_projects(&mut self) {
        let current: Vec<String> = self
            .harness
            .projects
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        if current != self.settings.projects {
            self.settings.projects = current;
            self.autosave();
        }
    }

    /// One autosave attempt: a failure must not be swallowed silently (the
    /// former `let _ =` call sites) - the settings modal's status line shows
    /// why the change did not stick (e.g. a read-only config directory).
    fn autosave(&mut self) {
        let result = store::save_settings(&self.settings);
        self.take_autosave_result(result);
    }

    /// Route one autosave result into the settings modal's status line.
    /// Kept separate so the failure path is testable without writing to
    /// the real config directory.
    fn take_autosave_result(&mut self, result: std::io::Result<()>) {
        if let Err(e) = result {
            self.settings_status =
                crate::i18n::trf("The settings could not be saved: {error}", &[("error", &e)]);
        }
    }
}

impl App {
    /// One UI frame. Shared by the eframe window and the headless renderer
    /// (`softrender`, used by `--screenshot --headless`).
    pub(crate) fn frame(&mut self, ctx: &egui::Context) {
        self.harness.ensure_started(ctx);
        self.harness.poll();
        self.harness.ui.settings_open = self.settings_open;
        match crate::harness::ui::show(&mut self.harness, ctx) {
            HarnessAction::None => {}
            HarnessAction::OpenSettings => self.settings_open = true,
            HarnessAction::OpenAccount => {
                self.settings_open = true;
                self.settings_nav = SettingsNav::Konto;
            }
        }
        self.sync_projects();
        self.sync_pinned();
        self.sync_vibe_choices();
        self.sync_layout();
        self.settings_window(ctx);
        self.language_window(ctx);
        // harness dialogs (folder picker, project-remove confirm) on top of
        // everything, also while the settings modal is open
        crate::harness::ui::show_dialogs(&mut self.harness, ctx);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // screenshot mode: render ~120 frames, capture, save, exit
        if self.screenshot_path.is_some() {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                let path = self.screenshot_path.take().unwrap();
                match save_screenshot(&img, &path) {
                    Ok(()) => {
                        eprintln!("screenshot saved to {}", path.display());
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("screenshot failed: {e}");
                        std::process::exit(1);
                    }
                }
            }
            self.frames_left = self.frames_left.saturating_sub(1);
            if self.frames_left == 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
                self.frames_left = 30;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.frame(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_on_exit();
    }
}

impl App {
    /// `--screenshot <png> --headless`: run the app without a window and
    /// without a display server, `frames` frames of `size` points at 60 fps,
    /// then rasterize the last one on the CPU (`softrender`) and save it.
    pub fn run_headless(mut self, size: [u32; 2]) -> Result<std::path::PathBuf, String> {
        let path = self
            .screenshot_path
            .clone()
            .ok_or_else(|| "--headless needs --screenshot <png>".to_string())?;
        let frames = self.frames_left.max(1);
        let ctx = egui::Context::default();
        crate::fonts::install_fallback_fonts(&ctx);
        crate::theme::apply(&ctx);
        let mut textures = crate::softrender::Textures::default();
        let start = std::time::Instant::now();
        let screen =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
        for i in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(start.elapsed().as_secs_f64()),
                focused: true,
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| self.frame(ctx));
            textures.set(&out.textures_delta);
            if i + 1 == frames {
                let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
                let px = |v: u32| (v as f32 * out.pixels_per_point).round() as usize;
                let img = crate::softrender::rasterize(
                    &prims,
                    &textures,
                    out.pixels_per_point,
                    [px(size[0]), px(size[1])],
                    crate::theme::BG,
                );
                img.save(&path).map_err(|e| e.to_string())?;
            }
            textures.free(&out.textures_delta);
            // the agent answers on its own threads; give it real time
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        Ok(path)
    }

    /// Saves the settings when the window closes.
    fn save_on_exit(&mut self) {
        // A failed exit save must not be silent either, but by now no
        // toast or modal can be shown any more. Only metadata goes to
        // stderr (the error itself, never file contents), and the save
        // gets one retry in case the failure was transient; after the
        // retry the message documents the loss. (N18c)
        self.sync_projects();
        if let Err(e) = store::save_settings(&self.settings) {
            eprintln!("settings could not be saved on exit: {e}; retrying once");
            if let Err(retry) = store::save_settings(&self.settings) {
                eprintln!(
                    "retry failed as well: {retry}; settings changes of this run may be lost"
                );
            }
        }
        // close vibe-acp's stdin so it can exit cleanly (instead of SIGKILL
        // via `Drop`, which leaves its child processes running)
        self.harness.shutdown_agent();
    }
}

fn save_screenshot(img: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let w = img.width();
    let h = img.height();
    let mut buf = image::RgbaImage::new(w as u32, h as u32);
    for (i, px) in img.pixels.iter().enumerate() {
        let x = (i % w) as u32;
        let y = (i / w) as u32;
        buf.put_pixel(x, y, image::Rgba([px.r(), px.g(), px.b(), px.a()]));
    }
    buf.save(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An App without touching the real config dir (the fields are private
    /// to this module, which is why the literal works here).
    fn test_app() -> App {
        App {
            settings: Settings::default(),
            settings_open: false,
            settings_nav: SettingsNav::Allgemein,
            settings_status: String::new(),
            screenshot_path: None,
            frames_left: 0,
            harness: crate::harness::Harness::new(Vec::new(), "ask".to_string()),
            language_prompt: false,
        }
    }

    /// Headless frame of the app shell with the settings modal open
    /// (softrender, AGENTS.md "Seeing the UI"); writes a PNG when
    /// CATWRIGHT_SHOT_DIR is set.
    #[test]
    fn headless_app_frame_renders_the_settings_modal() {
        let mut app = test_app();
        app.settings_open = true;
        let img = crate::softrender::render_ui([1360, 860], 3, |ctx| app.frame(ctx));
        let bg = crate::theme::BG;
        assert!(
            img.pixels().any(|p| p.0[..3] != [bg.r(), bg.g(), bg.b()]),
            "something was painted"
        );
        if let Ok(dir) = std::env::var("CATWRIGHT_SHOT_DIR") {
            let p = std::path::Path::new(&dir).join("07-settings.png");
            img.save(&p).unwrap();
        }
    }

    /// N18c: an autosave into a read-only config directory must surface in
    /// the settings modal's status line instead of being swallowed. A
    /// successful save leaves the previous status untouched.
    #[test]
    fn failed_autosave_is_visible_in_the_settings_status_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "catwright-autosave-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // read-only config directory: the same failure the real autosave hits
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let target = dir.join("settings.json");
        let result = store::save_settings_at(&target, &Settings::default());
        assert!(result.is_err(), "writing into a read-only directory fails");

        let mut app = test_app();
        app.take_autosave_result(result);
        let status = app.settings_status.clone();
        assert!(
            status.contains("Permission denied"),
            "the status line shows the autosave failure: {status:?}"
        );

        // a successful save must not clear or overwrite the shown error
        app.take_autosave_result(Ok(()));
        assert_eq!(
            app.settings_status, status,
            "a successful autosave leaves the status line alone"
        );

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
