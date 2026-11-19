//! The settings modal (tr("Settings")): a dimmed, centred modal with a left
//! navigation and the content on the right. Settings are saved automatically
//! — only when something actually changed during the frame. UI strings are
//! German; nothing is logged.

use super::App;
use crate::dialogs::{self, ModalClose};
use crate::harness::{self, extras, Conn};
use crate::i18n::{self, tr, trf, Lang};
use crate::icons::Icon;
use crate::store::{self, Settings};
use crate::theme;
use crate::widgets::{self, ButtonKind};
use egui::{vec2, RichText};
use std::path::PathBuf;

/// The entries of the settings navigation (left column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsNav {
    Allgemein,
    Vibe,
    Konto,
    Kuerzel,
    Datenschutz,
}

impl SettingsNav {
    pub(crate) const ALL: [SettingsNav; 5] = [
        SettingsNav::Allgemein,
        SettingsNav::Vibe,
        SettingsNav::Konto,
        SettingsNav::Kuerzel,
        SettingsNav::Datenschutz,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            SettingsNav::Allgemein => tr("General"),
            SettingsNav::Vibe => "Vibe",
            SettingsNav::Konto => tr("Account & plan"),
            SettingsNav::Kuerzel => tr("Keyboard shortcuts"),
            SettingsNav::Datenschutz => tr("Privacy"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            SettingsNav::Allgemein => Icon::Gear,
            SettingsNav::Vibe => Icon::Spark,
            SettingsNav::Konto => Icon::User,
            SettingsNav::Kuerzel => Icon::Terminal,
            SettingsNav::Datenschutz => Icon::Shield,
        }
    }

    /// CLI `--settings <tab>` → navigation entry; unknown names open
    /// Allgemein.
    pub(crate) fn from_cli(tab: &str) -> Self {
        match tab {
            "vibe" => SettingsNav::Vibe,
            "konto" => SettingsNav::Konto,
            "kuerzel" => SettingsNav::Kuerzel,
            "datenschutz" => SettingsNav::Datenschutz,
            _ => SettingsNav::Allgemein,
        }
    }
}

/// True when two settings differ (compared via serde_json; Settings has no
/// PartialEq). Used for the autosave: save only when something changed.
pub(crate) fn settings_changed(a: &Settings, b: &Settings) -> bool {
    serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
}

/// Monospace text (paths, ids) — not uppercased, unlike `mono_label`.
fn mono_text(text: &str) -> RichText {
    RichText::new(text)
        .monospace()
        .size(12.0)
        .color(theme::MUTED)
}

/// One labelled setting: label and optional help on the left, the control
/// on the right.
fn setting_row(ui: &mut egui::Ui, label: &str, help: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() * 0.55).max(180.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(label).size(14.0).color(theme::TEXT));
            if !help.is_empty() {
                ui.label(RichText::new(help).size(12.5).color(theme::DIM));
            }
        });
        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), add);
    });
    ui.add_space(10.0);
}

fn page_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .family(crate::fonts::bold())
            .size(18.0)
            .color(theme::TEXT),
    );
    ui.add_space(12.0);
}

/// Section label inside a settings page.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(theme::mono_label(title));
    ui.add_space(4.0);
}

/// A language picker (each language under its own name). Returns the
/// language picked this frame.
fn language_combo(ui: &mut egui::Ui, id: &str) -> Option<Lang> {
    let current = i18n::lang();
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(current.native_name())
        .show_ui(ui, |ui| {
            for lang in Lang::ALL {
                if ui
                    .selectable_label(lang == current, lang.native_name())
                    .clicked()
                {
                    chosen = Some(lang);
                }
            }
        });
    chosen
}

impl App {
    /// Switch the UI language now and remember it.
    fn set_language(&mut self, lang: Lang) {
        i18n::set_lang(lang);
        self.settings.language = Some(lang.code().to_string());
        let _ = store::save_settings(&self.settings);
    }

    /// First start: ask for the UI language, English preselected. Picking
    /// one switches the app behind the dialog at once; closing the dialog
    /// any way keeps the language shown and stores it.
    pub(crate) fn language_window(&mut self, ctx: &egui::Context) {
        if !self.language_prompt {
            return;
        }
        let mut done = false;
        let (_r, close) = dialogs::modal(ctx, "first_start_language", "Language · Sprache", |ui| {
            ui.label(
                RichText::new(tr(
                    "Choose the language of the app. You can change it later in the settings.",
                ))
                .color(theme::MUTED),
            );
            ui.add_space(10.0);
            let current = i18n::lang();
            for lang in Lang::ALL {
                if widgets::menu_check_row(ui, lang == current, lang.native_name(), None) {
                    i18n::set_lang(lang);
                }
            }
            ui.add_space(12.0);
            // a row of its own height: a bare right-to-left layout would
            // take the whole remaining height and stretch the modal
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if widgets::button(ui, tr("Continue"), ButtonKind::Primary).clicked() {
                        done = true;
                    }
                },
            );
        });
        if done || close != ModalClose::Open {
            self.language_prompt = false;
            self.set_language(i18n::lang());
        }
    }

    /// The settings modal (gear icon in the sidebars, CLI `--settings`):
    /// dimmed backdrop, centred; ESC, backdrop click and the "×" close it.
    /// Settings are autosaved below when they changed during this frame.
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let screen = ctx.screen_rect().size();
        let size = vec2(820.0, 560.0).min(screen * 0.92);
        let before_settings = self.settings.clone();
        let nav = self.settings_nav;
        // a nested dialog (folder picker, remove confirmation) owns ESC and
        // backdrop clicks: they must close only that dialog, not settings
        let nested_open =
            self.harness.folder_picker.is_some() || self.harness.remove_confirm.is_some();
        let (_r, close) = dialogs::modal_sized(ctx, "app_settings", tr("Settings"), size, |ui| {
            let content_h = (ui.available_height() - 28.0).max(80.0);
            ui.horizontal_top(|ui| {
                ui.set_min_height(content_h);
                ui.vertical(|ui| {
                    ui.set_width(180.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for entry in SettingsNav::ALL {
                        if widgets::nav_row(ui, entry.icon(), entry.label(), None, nav == entry)
                            .clicked()
                        {
                            self.settings_nav = entry;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("settings_content")
                    .auto_shrink([false, false])
                    .max_height(content_h)
                    .show(ui, |ui| {
                        // the scroll area inherits the parent's horizontal
                        // layout: force top-down for the page content
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 8.0);
                            page_title(ui, nav.label());
                            match nav {
                                SettingsNav::Allgemein => self.page_allgemein(ui),
                                SettingsNav::Vibe => self.page_vibe(ui),
                                SettingsNav::Konto => self.page_konto(ui),
                                SettingsNav::Kuerzel => page_kuerzel(ui),
                                SettingsNav::Datenschutz => page_datenschutz(ui),
                            }
                        });
                    });
            });
            ui.separator();
            let status = if self.settings_status.is_empty() {
                tr("Changes are saved automatically.").to_string()
            } else {
                self.settings_status.clone()
            };
            ui.label(RichText::new(status).color(theme::DIM).size(12.5));
        });
        let nested_close = matches!(close, ModalClose::Escape | ModalClose::Backdrop);
        if close != ModalClose::Open && !(nested_open && nested_close) {
            self.settings_open = false;
        }
        // autosave: persist settings, but only when they changed
        self.sync_projects();
        if settings_changed(&before_settings, &self.settings) {
            self.settings_status = match store::save_settings(&self.settings) {
                Ok(()) => tr("Settings saved.").into(),
                Err(e) => trf("Error: {error}", &[("error", &e)]),
            };
        }
    }

    fn page_allgemein(&mut self, ui: &mut egui::Ui) {
        setting_row(
            ui,
            tr("Language"),
            tr("Language of the app. Vibe answers in the language you write in."),
            |ui| {
                if let Some(lang) = language_combo(ui, "settings_language") {
                    self.set_language(lang);
                }
            },
        );
        setting_row(
            ui,
            tr("Default mode of new sessions"),
            tr("How much Vibe may do without asking."),
            |ui| {
                let modes = harness::start_modes(&self.harness.known_modes);
                let current = self.settings.default_session_mode.clone();
                let selected = modes
                    .iter()
                    .find(|(id, _)| *id == current)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| current.clone());
                let mut chosen = None;
                egui::ComboBox::from_id_salt("settings_session_mode")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (id, name) in &modes {
                            if ui.selectable_label(id == &current, name).clicked() {
                                chosen = Some(id.clone());
                            }
                        }
                    });
                if let Some(id) = chosen {
                    self.settings.default_session_mode = id.clone();
                    // new sessions start in this mode as well
                    self.harness.start_mode = id;
                }
            },
        );
        setting_row(
            ui,
            tr("Transcript detail while Vibe works"),
            tr("How open tool calls and thoughts are while a turn runs. Changes and approvals stay visible at every level."),
            |ui| {
                const LEVELS: [(&str, &str); 4] = [
                    ("compact", "Compact"),
                    ("standard", "Standard"),
                    ("detailed", "Detailed"),
                    ("verbose", "Verbose"),
                ];
                let current = self.settings.transcript_detail.clone();
                let mut chosen = None;
                egui::ComboBox::from_id_salt("settings_transcript_detail")
                    .selected_text(
                        LEVELS
                            .iter()
                            .find(|(id, _)| *id == current)
                            .map(|(_, label)| tr(label))
                            .unwrap_or_else(|| current.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for (id, label) in &LEVELS {
                            if ui.selectable_label(*id == current, tr(label)).clicked() {
                                chosen = Some(id.to_string());
                            }
                        }
                    });
                if let Some(id) = chosen {
                    self.settings.transcript_detail = id.clone();
                    self.harness.ui.transcript_detail = id;
                }
            },
        );
        setting_row(
            ui,
            tr("Desktop notifications"),
            tr("When a turn finishes or Vibe needs an approval — but only while the window is not focused. Uses notify-send when it is installed."),
            |ui| {
                let on = self.settings.notify_turn_events;
                if widgets::toggle(ui, on, "").clicked() {
                    self.settings.notify_turn_events = !on;
                    self.harness.notify_events = !on;
                }
            },
        );
        setting_row(
            ui,
            tr("Voice dictation"),
            tr("Shows a microphone button in the composer. While it is on, Vibe records your microphone and Mistral transcribes it."),
            |ui| {
                let on = self.settings.voice_dictation;
                if widgets::toggle(ui, on, "").clicked() {
                    self.settings.voice_dictation = !on;
                    self.harness.voice_dictation = !on;
                    if !on {
                        // the button goes away: a running dictation must not
                        // keep recording without a way to stop it
                        self.harness.cancel_dictation();
                    }
                }
            },
        );
        setting_row(
            ui,
            tr("Read answers aloud"),
            tr("Shows a speaker button on Vibe's answers. Vibe summarises the answer in a few sentences and Mistral speaks it (an extra request on your plan)."),
            |ui| {
                let on = self.settings.voice_narration;
                if widgets::toggle(ui, on, "").clicked() {
                    self.settings.voice_narration = !on;
                    self.harness.voice_narration = !on;
                    if !on {
                        // the button goes away: a running narration must not
                        // keep playing without a way to stop it
                        self.harness.cancel_narration();
                    }
                }
            },
        );
        // only effective with the speaker buttons on, so it only shows with them
        if self.settings.voice_narration {
            setting_row(
                ui,
                tr("Read every answer automatically"),
                tr("When a turn of the open session ends, its answer is read aloud."),
                |ui| {
                    let on = self.settings.auto_narrate;
                    if widgets::toggle(ui, on, "").clicked() {
                        self.settings.auto_narrate = !on;
                        self.harness.auto_narrate = !on;
                    }
                },
            );
        }
        section(ui, tr("Config folder"));
        ui.horizontal(|ui| {
            if let Some(dir) = store::config_dir() {
                let text = dir.to_string_lossy().into_owned();
                ui.add(egui::Label::new(mono_text(&text)).truncate());
                if widgets::button(ui, tr("Copy path"), ButtonKind::Secondary).clicked() {
                    ui.ctx().copy_text(text);
                    self.settings_status = tr("Path copied.").into();
                }
            } else {
                ui.label(RichText::new(tr("unknown")).color(theme::DIM));
            }
        });
    }

    /// Connection to `vibe-acp`, the UI skill and the project list.
    fn page_vibe(&mut self, ui: &mut egui::Ui) {
        section(ui, tr("Connection"));
        let (text, color) = match &self.harness.conn {
            Conn::Ready { agent } => (trf("Ready · {agent}", &[("agent", agent)]), theme::GREEN),
            Conn::Starting => (tr("Connecting …").to_string(), theme::DIM),
            Conn::Off => ("Offline".to_string(), theme::DIM),
            Conn::Reconnecting { attempt, .. } => (
                trf(
                    "Connection interrupted · retry {attempt}/{max}",
                    &[("attempt", attempt), ("max", &harness::MAX_RECONNECTS)],
                ),
                theme::YELLOW,
            ),
            Conn::Failed(msg) => (msg.clone(), theme::RED_TEXT),
        };
        ui.label(RichText::new(text).monospace().size(12.5).color(color));
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                tr("Sign-in and subscription run through the Vibe CLI (`vibe` in a terminal). Catwright stores no Vibe keys."),
            )
            .color(theme::DIM)
            .size(13.0),
        );
        ui.add_space(12.0);
        section(ui, tr("Interactive answers"));
        ui.label(
            RichText::new(tr(
                "The skill “catwright-ui” teaches Vibe to answer with tables, charts, forms and calculators as real widgets. It is copied to ~/.vibe/skills/catwright-ui/.",
            ))
            .color(theme::DIM)
            .size(13.0),
        );
        let installed = store::vibe_skill_installed();
        let exists = store::vibe_skill_path()
            .map(|p| p.exists())
            .unwrap_or(false);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (text, color) = if installed {
                (tr("Installed"), theme::GREEN)
            } else if exists {
                (tr("Outdated"), theme::YELLOW)
            } else {
                (tr("Not installed"), theme::DIM)
            };
            widgets::badge(ui, text, color);
            let label = if exists {
                tr("Update skill")
            } else {
                tr("Install skill")
            };
            if !installed && widgets::button(ui, label, ButtonKind::Primary).clicked() {
                self.settings_status = match store::install_vibe_skill() {
                    Ok(p) => {
                        self.harness.ui_hint = true;
                        trf("Skill installed: {path}", &[("path", &p.display())])
                    }
                    Err(e) => trf("Error: {error}", &[("error", &e)]),
                };
            }
        });
        ui.add_space(12.0);
        section(ui, tr("Projects"));
        let projects = self.harness.projects.clone();
        let mut remove: Option<PathBuf> = None;
        for p in &projects {
            let key = p.to_string_lossy().into_owned();
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                widgets::paint_project_icon(ui, r.min, &key, 12.0);
                ui.add(egui::Label::new(mono_text(&key)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            widgets::IconButton::new(Icon::Trash, tr("Remove from list"))
                                .side(22.0),
                        )
                        .clicked()
                    {
                        remove = Some(p.clone());
                    }
                });
            });
        }
        if projects.is_empty() {
            ui.label(RichText::new(tr("No projects yet.")).color(theme::DIM));
        }
        if let Some(p) = remove {
            // the confirmation dialog itself is rendered by the harness
            // (harness::ui::show_dialogs), same as in the sidebar
            self.harness.remove_confirm = Some(p);
        }
        ui.add_space(4.0);
        if widgets::button(ui, tr("Add project"), ButtonKind::Secondary).clicked() {
            // same start folder as the sidebar's folder picker
            let start = self
                .harness
                .current_project
                .clone()
                .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("/"));
            self.harness.folder_picker = Some(dialogs::FolderPicker::new(start));
        }
    }

    /// Identity, plan and monthly budget, read from Vibe on request.
    fn page_konto(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(
                tr("Catwright only uses your Vibe account. The data is fetched from Mistral only when you ask."),
            )
            .color(theme::DIM)
            .size(13.0),
        );
        ui.add_space(12.0);
        if self.harness.account.is_none()
            && self.harness.identity.is_none()
            && !self.harness.account_loading()
        {
            if widgets::button(ui, tr("Fetch account and quota"), ButtonKind::Primary).clicked() {
                self.harness.load_account();
            }
            ui.add_space(16.0);
        }
        if self.harness.account_loading() {
            ui.label(RichText::new(tr("Loading …")).color(theme::DIM));
        }
        if let Some(i) = self.harness.identity.clone() {
            section(ui, tr("Account"));
            if let Some(name) = &i.name {
                ui.label(RichText::new(name).size(14.0));
            }
            if let Some(email) = &i.email {
                ui.label(RichText::new(email).size(14.0));
            }
            if let Some(w) = &i.workspace {
                ui.label(RichText::new(w).color(theme::DIM).size(13.0));
            }
            ui.add_space(10.0);
        }
        if let Some(a) = self.harness.account.clone() {
            section(ui, tr("Plan"));
            if let Some(plan) = &a.plan {
                ui.label(RichText::new(plan).size(14.0));
            }
            match a.usage_percent {
                Some(pct) => {
                    let label = i18n::decimal(trf(
                        "{pct} % of the monthly quota used",
                        &[("pct", &format!("{pct:.1}"))],
                    ));
                    ui.label(RichText::new(label).size(14.0));
                    theme::pixel_meter(ui, theme::meter_blocks((pct / 100.0) as f32, 30), 30, 9.0);
                    if let Some(date) = a.reset_at.as_deref().and_then(extras::local_date) {
                        ui.label(
                            RichText::new(trf("New quota on {date}", &[("date", &date)]))
                                .color(theme::DIM)
                                .size(13.0),
                        );
                    }
                    if a.payg {
                        ui.label(
                            RichText::new(tr("Pay-as-you-go active: usage above 100 % is billed."))
                                .color(theme::YELLOW)
                                .size(13.0),
                        );
                    }
                }
                None => {
                    ui.label(
                        RichText::new(tr("Vibe reports no quota for this account."))
                            .color(theme::DIM),
                    );
                }
            }
            ui.add_space(8.0);
            if widgets::button(ui, tr("Refresh"), ButtonKind::Secondary).clicked() {
                self.harness.load_account();
            }
            ui.add_space(12.0);
        }
        setting_row(
            ui,
            tr("Fetch quota at startup"),
            tr("Shows your usage at the bottom of the sidebar right away. Asks Mistral at every start for it."),
            |ui| {
                let on = self.settings.auto_account;
                if widgets::toggle(ui, on, "").clicked() {
                    self.settings.auto_account = !on;
                    self.harness.auto_account = !on;
                }
            },
        );
    }
}

/// The keyboard shortcuts of the app, in the current language.
fn shortcuts() -> [(&'static str, &'static str); 11] {
    [
        ("↵", tr("Send (or queue while Vibe is working)")),
        ("⇧ ↵", tr("New line")),
        ("Esc", tr("Stop the running answer")),
        (tr("Ctrl K"), tr("Search and commands")),
        (tr("Ctrl N"), tr("New session")),
        (tr("Ctrl B"), tr("Toggle sidebar")),
        (tr("Ctrl J"), tr("Toggle work panel")),
        (tr("Ctrl L"), tr("Focus the input field")),
        (tr("Ctrl ,"), tr("Settings")),
        (tr("/ and @"), tr("Commands and files in the input field")),
        (
            "1 – 4",
            tr("Answer an approval (with an empty input field)"),
        ),
    ]
}

fn page_kuerzel(ui: &mut egui::Ui) {
    for (key, what) in shortcuts() {
        ui.horizontal(|ui| {
            ui.add_sized(
                [110.0, 22.0],
                egui::Label::new(
                    RichText::new(key)
                        .monospace()
                        .size(12.5)
                        .color(theme::AMBER),
                ),
            );
            ui.label(RichText::new(what).size(14.0));
        });
        ui.add_space(2.0);
    }
}

/// What is stored where, and what never leaves the machine.
fn page_datenschutz(ui: &mut egui::Ui) {
    let dir = store::config_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| tr("<config folder>").to_string());
    let items: [(&str, String); 5] = [
        (
            tr("Settings"),
            trf(
                "settings.json in {dir} — holds no secrets.",
                &[("dir", &dir)],
            ),
        ),
        (
            tr("Workspace “General”"),
            trf(
                "{path} — the working folder of sessions without a project.",
                &[(
                    "path",
                    &store::chat_workspace_dir()
                        .map(|d| d.to_string_lossy().into_owned())
                        .unwrap_or_else(|| tr("<config folder>").to_string()),
                )],
            ),
        ),
        (
            tr("Vibe sessions"),
            tr("are stored by Vibe in ~/.vibe — not by Catwright.").to_string(),
        ),
        (
            tr("Account & quota"),
            tr("are only fetched through Vibe when you ask (or with the “at startup” option).")
                .to_string(),
        ),
        (
            tr("Telemetry & network"),
            tr("No telemetry. Network traffic only goes through vibe-acp (Vibe CLI).").to_string(),
        ),
    ];
    for (title, text) in items {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("■")
                    .color(theme::ORANGE)
                    .monospace()
                    .size(10.0),
            );
            ui.label(RichText::new(title).color(theme::TEXT).size(14.0));
        });
        ui.label(RichText::new(text).color(theme::DIM).size(13.0));
        ui.add_space(8.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_changed_detects_differences() {
        let a = Settings::default();
        let mut b = Settings::default();
        assert!(!settings_changed(&a, &b));
        // (was `ui_mode`, gone with the Code/Chat switch: any plain field
        // shows the same toggle-and-revert behaviour)
        b.auto_account = true;
        assert!(settings_changed(&a, &b));
        b.auto_account = false;
        assert!(!settings_changed(&a, &b), "same value again -> unchanged");
        b.default_session_mode = "auto-approve".into();
        assert!(settings_changed(&a, &b));
        b.projects.push("/tmp/x".into());
        assert!(settings_changed(&a, &b));
        b.projects.clear();
        assert!(settings_changed(&a, &b), "projects still differ (mode)");
    }

    #[test]
    fn cli_tab_maps_to_nav_entry() {
        assert_eq!(SettingsNav::from_cli("allgemein"), SettingsNav::Allgemein);
        assert_eq!(SettingsNav::from_cli("vibe"), SettingsNav::Vibe);
        assert_eq!(SettingsNav::from_cli("konto"), SettingsNav::Konto);
        assert_eq!(SettingsNav::from_cli("kuerzel"), SettingsNav::Kuerzel);
        assert_eq!(
            SettingsNav::from_cli("datenschutz"),
            SettingsNav::Datenschutz
        );
        assert_eq!(SettingsNav::from_cli("irgendwas"), SettingsNav::Allgemein);
        assert_eq!(SettingsNav::Allgemein.label(), "General");
        assert_eq!(SettingsNav::Vibe.label(), "Vibe");
        assert_eq!(SettingsNav::Konto.label(), "Account & plan");
        assert_eq!(SettingsNav::Kuerzel.label(), "Keyboard shortcuts");
        assert_eq!(SettingsNav::Datenschutz.label(), "Privacy");
        assert_eq!(i18n::tr_in(Lang::De, "General"), "Allgemein");
        assert_eq!(i18n::tr_in(Lang::De, "Keyboard shortcuts"), "Tastenkürzel");
        assert_eq!(SettingsNav::ALL.len(), 5);
    }
}
