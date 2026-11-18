# Catwright: rules for coding agents

Native Rust desktop app (egui/eframe 0.29) that acts as a GUI harness for the
Mistral Vibe agent via ACP (`vibe-acp`, see `docs/acp.md`).
Only the user's Vibe subscription is used for Vibe; the app never handles Vibe
API keys.

## Layout

- `app/`: the crate (`catwright`). Run all cargo commands from here:
  `cd app && cargo test`.
- `app/src/`: `main.rs` (CLI + window), `app.rs` + `app/settings.rs` (egui UI),
  `acp/` (JSON-RPC framing, protocol, client, session views),
  `harness/` (Vibe harness state, Vibe extension payloads, local git/file
  lookups), `harness/ui/` (sidebar, header, transcript, composer, cards, right
  panel, palette, artifacts), `model.rs`, `store.rs` (persistence), `render.rs`,
  `script.rs`, `formula.rs`, `markdown.rs`, `diff.rs`, `syntax.rs`, `memo.rs`,
  `artifacts.rs` (Artefakte view + previews), `theme.rs`, `icons.rs`,
  `i18n.rs` (UI language, gettext catalogs in `app/i18n/`),
  `widgets.rs`, `fonts.rs`, `dialogs.rs`.
- `app/tests/fixtures/acp/`: real ACP wire transcripts (do not edit).
- `docs/acp.md`: the protocol subset we implement (source of truth).
- `reference/` (local only, git-ignored): screenshots of other products used
  as design references; never commit them.
- `tools/icons/render.py`: renders the app icons (`app/assets/icon.png`,
  `dist/icons/`) from `theme::LOGO`; rerun it after changing the logo.
- `tools/mockups/`: HTML mockups of proposed UI (not implemented), rendered
  to `docs/mockups/`; not part of the app.
- `tools/screenshots/`: renders the README screenshots against a stand-in
  for `vibe-acp` with prepared demo sessions (`render.sh`); not part of the app.
- `app/src/softrender.rs`: CPU rasterizer for egui frames, so the UI can be
  looked at without a display server or GPU (see "Seeing the UI").
- `docs/design.md`: visual design (Mistral-like pixel style, colour tokens).

## Hard rules

- No new crates or dependency changes in `Cargo.toml` unless the task
  explicitly allows them. Builds run offline.
- Privacy and security first: never log or print API keys, prompts or file
  contents. Secrets live only in 0600 files (see `store.rs`). No telemetry,
  no network calls the user did not trigger.
- No fakes, stubs or placeholder logic in `src/`. If something cannot be
  implemented, stop and say so.
- Do not weaken, delete or `#[ignore]` existing tests.
- UI text is written in English in the code and shown through
  `crate::i18n::tr("…")` / `trf("… {name} …", &[("name", &value)])`; every
  such text needs a German entry in `app/i18n/de.po` (a test fails
  otherwise). Never build UI text with bare string literals. Code, comments
  and identifiers are English.
- Keep `cargo fmt --check`, `cargo clippy --all-targets` (no new warnings)
  and `cargo test` green.

## Seeing the UI (no display needed)

UI work is not done until you have looked at it. Render every UI change to
a PNG and open it before committing. Rendering needs no X11, Wayland, xvfb
or GPU: egui's meshes are rasterized on the CPU (`softrender.rs`, matches
the GL output except for hover effects; image textures included).

- Whole app: `catwright --screenshot out.png --headless [--frames N]
  [--size WxH]` together with the usual `--project`/`--session`/
  `--settings`/`--lang`; it waits for the agent like the window does.
  `tools/screenshots/render.sh [out-dir]` does this against demo sessions.
- One view in a test: `crate::softrender::render_ui([1360, 860], 3, |ctx| {
  let _ = crate::harness::ui::show(&mut h, ctx); })` returns an
  `image::RgbaImage`; `img.save(path)` writes the PNG.
  `harness::tests::feature_screenshots` renders the feature states to PNGs
  when `CATWRIGHT_SHOT_DIR` is set. Do not commit such PNGs from tests.
