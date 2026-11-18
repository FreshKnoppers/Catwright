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
//!   catwright --project <dir> --prompt -         same, but the message is read from stdin
//!                                                (`--prompt <text>` stays in `ps` for the
//!                                                life of the process; stdin and the
//!                                                CATWRIGHT_PROMPT env var do not)
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

use std::io::Read;
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

/// The startup prompt once the CLI is parsed. `--prompt -` reads the
/// message from stdin (a trailing newline goes away with `echo |`;
/// empty input gives no prompt), a plain `--prompt <text>` is taken as
/// is, and without either the CATWRIGHT_PROMPT env var is honoured
/// (empty counts as unset; main removes it after reading, so no child
/// inherits it). The env var hides the message where argv cannot:
/// procfs lets anyone read `cmdline`, while `environ` is owner-only.
fn prompt_from(arg: Option<String>, env: Option<String>, stdin: &mut impl Read) -> Option<String> {
    match arg {
        Some(mark) if mark == "-" => {
            let mut text = String::new();
            match stdin.read_to_string(&mut text) {
                Ok(_) if text.trim().is_empty() => None,
                Ok(_) => Some(text.trim_end().to_string()),
                Err(e) => {
                    eprintln!("the prompt could not be read from stdin: {e}");
                    None
                }
            }
        }
        arg => arg.or_else(|| env.filter(|e| !e.is_empty())),
    }
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
    // `-` means stdin; a literal `--prompt <text>` beats the env var,
    // and only without either does CATWRIGHT_PROMPT speak (see prompt_from)
    let prompt = prompt_from(
        cli.prompt,
        std::env::var("CATWRIGHT_PROMPT").ok(),
        &mut std::io::stdin(),
    );
    // A prompt variable only protects while it is gone after reading:
    // every child (the agent, its tools, git) inherits the environment.
    // No threads exist yet (the worker branch returned above), so the
    // removal is race-free.
    std::env::remove_var("CATWRIGHT_PROMPT");
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

/// The window and taskbar icon: `assets/icon.png`, 128 × 128, the pixel
/// mark of `theme::LOGO` on a dark tile, rendered by `tools/icons/render.py`.
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
    fn the_bundled_window_icon_shows_the_logo_bitmap() {
        // tools/icons/render.py draws theme::LOGO with whole-pixel cells;
        // find the mark by its colours and compare it cell by cell, so a
        // changed LOGO without re-rendered icons fails here
        let icon = load_icon();
        let w = icon.width as usize;
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            egui::Color32::from_rgb(icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2])
        };
        let colors: Vec<egui::Color32> = "oyr".chars().filter_map(theme::logo_color).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..icon.height as usize {
            for x in 0..w {
                if colors.contains(&px(x, y)) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        let (cols, rows) = (theme::LOGO[0].chars().count(), theme::LOGO.len());
        assert!(x1 > x0 && y1 > y0, "the icon contains the mark");
        let cell = (x1 - x0 + 1) / cols;
        assert_eq!((x1 - x0 + 1, y1 - y0 + 1), (cell * cols, cell * rows));
        for (r, line) in theme::LOGO.iter().enumerate() {
            for (c, ch) in line.chars().enumerate() {
                let got = px(x0 + c * cell + cell / 2, y0 + r * cell + cell / 2);
                match theme::logo_color(ch) {
                    Some(want) => assert_eq!(got, want, "cell {r},{c}"),
                    None => assert!(!colors.contains(&got), "cell {r},{c} is empty"),
                }
            }
        }
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

    #[test]
    fn prompt_stdin_is_read_from_the_stream_and_empty_gives_none() {
        // `-` reads the message from stdin: the text never enters argv,
        // and the newline that `echo |` appends goes away with it
        assert_eq!(
            prompt_from(
                cli(&["--prompt", "-"]).prompt,
                None,
                &mut std::io::Cursor::new(b"Eine Frage\n".to_vec())
            )
            .as_deref(),
            Some("Eine Frage")
        );
        // inner newlines stay: it is a message, not a line
        assert_eq!(
            prompt_from(
                cli(&["--prompt", "-"]).prompt,
                None,
                &mut std::io::Cursor::new(b"Zeile 1\nZeile 2\n\n".to_vec())
            )
            .as_deref(),
            Some("Zeile 1\nZeile 2")
        );
        // empty stdin (or only whitespace) is no prompt, not an empty one
        assert_eq!(
            prompt_from(
                cli(&["--prompt", "-"]).prompt,
                None,
                &mut std::io::Cursor::new(Vec::new())
            ),
            None
        );
        assert_eq!(
            prompt_from(
                cli(&["--prompt", "-"]).prompt,
                None,
                &mut std::io::Cursor::new(b" \n".to_vec())
            ),
            None
        );
    }

    #[test]
    fn prompt_env_var_is_the_fallback_and_empty_counts_as_unset() {
        assert_eq!(
            prompt_from(None, Some("Aus dem Env".into()), &mut std::io::empty()).as_deref(),
            Some("Aus dem Env")
        );
        assert_eq!(
            prompt_from(None, Some(String::new()), &mut std::io::empty()),
            None
        );
        // an explicit argument beats the env var
        assert_eq!(
            prompt_from(
                Some("Arg gewinnt".into()),
                Some("Env".into()),
                &mut std::io::empty()
            )
            .as_deref(),
            Some("Arg gewinnt")
        );
    }
}
