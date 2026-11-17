//! Catwright: native Rust GUI harness for the Mistral Vibe agent (via
//! `vibe-acp`, see docs/acp.md). How it works: MECHANICS.md.
//!
//! Privacy by design:
//! - Vibe runs on its own login and subscription; the app holds no Vibe key.
//! - No telemetry, no analytics, no crash reporting, no remote resources.
//!
//! CLI:
//!   catwright                                   start the GUI
//!   catwright --project <dir> [--session <id>]  add/select a project, load a session
//!   catwright --project <dir> --prompt <text>   start with this message (new session, or the loaded one)
//!   catwright --settings [tab]                  open the settings (allgemein|vibe|konto|kuerzel|datenschutz)
//!   catwright --screenshot out.png [--frames N] render N frames (default 120), save PNG, exit
//!   catwright --lang en|de                      UI language for this run (not saved)
//!   catwright --ui-js-worker                    internal: one ui-js evaluation (stdin→stdout), then exit

// acp is wired into the harness UI; not every pub item is used by the bin yet
mod acp;
mod app;
mod artifacts;
mod dialogs;
mod diff;
mod fonts;
mod formula;
mod harness;
mod i18n;
mod icons;
mod markdown;
mod memo;
mod model;
mod render;
mod script;
mod store;
mod syntax;
mod theme;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

struct Cli {
    screenshot: Option<PathBuf>,
    settings: Option<String>,
    project: Option<String>,
    session: Option<String>,
    prompt: Option<String>,
    /// `--lang`: UI language for this run only (screenshots, testing).
    lang: Option<i18n::Lang>,
    frames: u32,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut cli = Cli {
        screenshot: None,
        settings: None,
        project: None,
        session: None,
        prompt: None,
        lang: None,
        frames: 120,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" if i + 1 < args.len() => {
                cli.screenshot = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--frames" if i + 1 < args.len() => {
                cli.frames = args[i + 1].parse().unwrap_or(120);
                i += 2;
            }
            "--project" if i + 1 < args.len() => {
                cli.project = Some(args[i + 1].clone());
                i += 2;
            }
            "--session" if i + 1 < args.len() => {
                cli.session = Some(args[i + 1].clone());
                i += 2;
            }
            "--prompt" if i + 1 < args.len() => {
                cli.prompt = Some(args[i + 1].clone());
                i += 2;
            }
            "--lang" if i + 1 < args.len() => {
                cli.lang = i18n::Lang::from_code(&args[i + 1]);
                i += 2;
            }
            // Code and Chat are one view now: the old `--mode code|chat`
            // is skipped with its value instead of being taken for a flag
            "--mode" if i + 1 < args.len() => i += 2,
            "--settings" => {
                // optional tab name: allgemein | vibe | datenschutz
                if i + 1 < args.len()
                    && ["allgemein", "vibe", "konto", "kuerzel", "datenschutz"]
                        .contains(&args[i + 1].as_str())
                {
                    cli.settings = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    cli.settings = Some("allgemein".into());
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    cli
}

fn main() -> eframe::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    // internal ui-js evaluation worker (see src/script.rs): one JSON
    // request on stdin, one JSON response on stdout, no GUI
    if argv.get(1).map(String::as_str) == Some(script::WORKER_FLAG) {
        script::worker_main(std::io::stdin(), std::io::stdout());
        return Ok(());
    }
    let cli = parse_cli(argv.get(1..).unwrap_or(&[]));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([560.0, 420.0])
            .with_title("Catwright")
            // Wayland matches the window to `catwright.desktop` (icon,
            // name, taskbar grouping) by this id; without it the window
            // has none
            .with_app_id("catwright")
            .with_icon(load_icon()),
        ..Default::default()
    };
    let screenshot = cli.screenshot;
    let settings = cli.settings;
    let frames = cli.frames;
    let prompt = cli.prompt;
    let lang = cli.lang;
    let open = cli.project.map(|p| (p, cli.session));
    eframe::run_native(
        "Catwright",
        options,
        Box::new(move |cc| {
            fonts::install_fallback_fonts(&cc.egui_ctx);
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(
                screenshot, settings, frames, open, prompt, lang,
            )))
        }),
    )
}

/// The window and taskbar icon: `assets/icon.png`, the pixel window mark
/// of `theme::LOGO`, 128 × 128, generated from `LOGO` and `GRADIENT`.
fn load_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/icon.png");
    let rgba = image::load_from_memory(bytes)
        .expect("the bundled icon.png decodes")
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse_cli(&args)
    }

    #[test]
    fn the_bundled_window_icon_is_square_and_opaque() {
        let icon = load_icon();
        assert_eq!((icon.width, icon.height), (128, 128));
        assert_eq!(
            icon.rgba.len(),
            128 * 128 * 4,
            "the RGBA buffer matches the dimensions"
        );
        assert!(
            icon.rgba.as_chunks::<4>().0.iter().any(|px| px[3] == 255),
            "the window mark has opaque pixels"
        );
    }

    #[test]
    fn old_mode_arg_is_skipped_with_its_value() {
        // Code and Chat are one view now: `--mode code|chat` from old
        // scripts neither fails nor swallows the arguments after it
        let c = cli(&["--mode", "chat", "--project", "/tmp/x", "--frames", "7"]);
        assert_eq!(c.project.as_deref(), Some("/tmp/x"));
        assert_eq!(c.frames, 7);
        let c = cli(&["--mode", "code", "--prompt", "Hallo"]);
        assert_eq!(c.prompt.as_deref(), Some("Hallo"));
        // the value is skipped too, never taken for an argument of its own
        let c = cli(&["--mode", "api", "--settings", "vibe"]);
        assert_eq!(c.settings.as_deref(), Some("vibe"));
        // a trailing `--mode` without a value stays without side effects
        let c = cli(&["--mode"]);
        assert_eq!(c.frames, 120);
        assert_eq!(c.project, None);
    }

    #[test]
    fn frames_and_settings_args_parse() {
        assert_eq!(cli(&["--frames", "42"]).frames, 42);
        assert_eq!(cli(&["--frames", "x"]).frames, 120);
        // a value-eating option at the end stays without side effects
        assert_eq!(cli(&["--frames"]).frames, 120);
        assert_eq!(
            cli(&["--settings", "vibe"]).settings.as_deref(),
            Some("vibe")
        );
        assert_eq!(cli(&["--settings"]).settings.as_deref(), Some("allgemein"));
        assert_eq!(
            cli(&["--settings", "konto"]).settings.as_deref(),
            Some("konto")
        );
    }

    #[test]
    fn prompt_arg_is_taken_verbatim() {
        assert_eq!(
            cli(&["--prompt", "Mach das"]).prompt.as_deref(),
            Some("Mach das")
        );
        assert_eq!(cli(&["--prompt"]).prompt, None);
    }
}
