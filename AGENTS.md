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
- `tools/screenshots/`: renders the README screenshots against a stand-in
  for `vibe-acp` with prepared demo sessions (`render.sh`); not part of the app.
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
