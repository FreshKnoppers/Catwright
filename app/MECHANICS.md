# Build

Standard Rust toolchain (on Arch: `sudo pacman -S rustup && rustup default stable`):

    cargo build --release        # binary: target/release/catwright
    cargo test                   # unit tests (incl. ACP fixtures under tests/fixtures/acp/)
    cargo clippy --all-targets
    cargo fmt --check

No build script, no vendored toolchain, no npm/make wrappers. Builds run
offline (`CARGO_NET_OFFLINE=true`); dependencies are pinned in `Cargo.lock`.

# MECHANICS: how Catwright works

Catwright is a native Rust desktop app (eframe/egui 0.29). It is a GUI
harness for the Mistral Vibe agent. It talks to `vibe-acp` over ACP, so it
uses only the user's Vibe subscription and Vibe's own login.

Related documents:

- `../docs/acp.md`: the ACP subset we implement (source of truth for the protocol).
- `../docs/design.md`: visual design (colour tokens, pixel elements, layout).
- `../vibe-skills/catwright-ui/SKILL.md`: the ```ui / ```ui-js spec for the model.
- `../AGENTS.md`: rules for coding agents.

## One view, many workspaces

There is no Code/Chat switch any more (merged 2026-11-08). One `vibe-acp`
process and one `Harness` show every workspace in one sidebar tree:

| Workspace | Where sessions live |
|-----------|---------------------|
| „Allgemein" (pinned on top) | Vibe sessions in `~/.config/catwright/chat-workspace` (`Harness::chat_dir`, `store::chat_workspace_dir`) |
| each project folder | one Vibe session list per folder (`Harness::projects`) |

„Allgemein" is never part of `projects` (an old list entry for its folder is
dropped, adding it as a project selects it) and cannot be removed.
`current_project` is the workspace the next new session starts in: it follows
the session the user opens (`Harness::activate`), the workspace row or the
start screen's picker (`select_workspace`). Until the user picks one in a run,
it is the workspace whose newest session has the latest `updatedAt`
(`last_used_workspace`, recomputed as the `session/list` answers arrive);
before any list it is the first project, without projects „Allgemein". A
session Vibe creates for the user's first message is shown in whatever
workspace it was started. A waiting permission or question marks the
workspace row too (`workspace_waiting`), so it stays visible while folded.
`header` and `panel` hide git and the folder button for „Allgemein".

### Session fork and sidebar order

"⑃ Sitzung forken" (right panel) sends `session/fork` with the active
session's id (`sessionId` + `cwd`, see `../docs/acp.md`; response shaped like
`session/new`, the fork is active once the agent answers). The new view
shows this client's copy of the transcript — the fork response does not
replay it — and the source session stays untouched. No fork while a turn
runs; a failed fork (e.g. `-32601` when the agent does not implement the
method) lands in the status line. The sidebar sorts sessions by
`updatedAt` (newest first, no timestamp last).

Sidebar rows (like Claude Desktop): „Artefakte"
opens the artifacts view (see below), „Suchen" opens the command palette over
the sessions of every workspace (by title, or all sessions of a workspace
whose name matches; case-insensitive, `matches_query`/`filter_entries`).

## Composer and attachments

`session/prompt` sends the draft plus one block per attachment
(`prompt_blocks`): a PNG within 5 MiB becomes an `image` block (base64,
`promptCapabilities.image`), every other file a `resource_link` the agent
reads itself. Oversized or missing PNGs fall back to a link, never an error.

## Vibe over ACP (`src/acp/`, `src/harness/`)

- `acp/rpc.rs`: JSON-RPC 2.0 framing, one JSON object per line.
- `acp/protocol.rs`: typed messages (`initialize`, `session/*`, `session/update`
  kinds, permission requests, config options).
- `acp/client.rs`: `AcpClient` spawns `vibe-acp` with no arguments. A reader
  thread turns stdout into `AcpEvent`s. stderr is drained into a bounded tail
  and only shown when the process ends. Dropping the client kills the child.
- `acp/session.rs`: `SessionView` folds `session/update` into displayable
  items: text, thoughts, tool calls, diffs, plan, permission cards and
  subagents (`agents.spawn` + `self.wait`).
- `harness/mod.rs`: `Harness` holds the connection state (`Conn`), the open
  sessions, the per-folder session lists and the composer. It reacts to
  events in `handle_event`. It has no egui code.
- `harness/extras.rs`: typed views of Vibe's extension payloads (account,
  trust, loops, review, connectors, questions) and German display helpers.
- `harness/watch.rs`: local background lookups (git branch and dirty count, the
  project's file list for `@`). Local processes only, no network.
- `harness/state.rs`: transient UI state (dialogs, palette, queue entries).
- `harness/ui.rs` + `harness/ui/`: the egui UI, split into
  `sidebar`, `header`, `transcript`, `cards` (pinned above the composer:
  trust, questions, permissions, queue), `composer`, `panel` (right side),
  `palette` (Strg K) and `artifacts`. Built from `widgets.rs` and `icons.rs`.
- `harness/context.rs`: context breakdown (see below).

### Starting a session

A new session is created lazily with the first message
(`session/new` for the current folder). Before the held prompt goes out,
`continue_setup` applies the start settings one by one and waits for each
answer:

1. `session/set_mode`, if the start mode (e.g. `ask`) differs from Vibe's
   default;
2. `session/set_config_option` `model`, if a model was picked before the
   first message;
3. `session/set_config_option` `thinking`, likewise.

A failed step shows a warning and the chain continues. Mode names, models and
thinking levels are Vibe's own (from `modes` and `configOptions`), with two
app-side exceptions: `ask` is labelled "Plan" in the app's pickers (the id on
the wire stays `ask`), and the user's `claude-delegate*` profiles are hidden
from the app's selection lists — the ids are unchanged on the wire, a session
already running in such a mode keeps showing its name, and a stored start mode
naming one is ignored at apply time so Vibe's own default stays active.
Setting the model changes Vibe's global default, as in the CLI.

### Model and thinking choices before the first message

Right after `initialize`, the harness opens one session in the chat workspace
without a prompt (the "probe"). It reads the offered models and thinking levels
and Vibe's current defaults from the answer, then sends `session/close`. This
makes no model call and costs no quota. Vibe keeps no file for a session
without messages. That assumption is Vibe-version-dependent: it must be
re-checked after every Vibe update (verified 2026-11-07 for Vibe 3.1.0 with an
isolated `VIBE_HOME`: `session/close` leaves nothing behind under
`~/.vibe/logs/session/`). The lists are also saved in `settings.json`
(`vibe_models`, `vibe_thinking`), so the pickers work before the probe has
answered.

Vibe keeps the thinking level per model. The harness remembers, per model,
the levels it offered and the level it had (`thinking_by_model`, saved as
`vibe_thinking_by_model`). Picking a model on the start screen fits a chosen
level the model does not offer (e.g. `max` → `high`); a model never seen
before is fitted right after the session switched to it, before the
thinking step is sent (`fit_thinking`).

### Connection loss

If `vibe-acp` exits, `Harness::connection_lost`:

- drops the dead client;
- remembers the session on screen (reloaded and shown again once the agent is
  ready; the other open sessions load again on their next click). A session
  clicked during the restart takes its place, "Neue Sitzung" drops it;
- puts a first prompt that was held for `session/new` back into the composer;
- sets a German status line saying what was lost.

The process is restarted after 1, 2, 4, 8 and 16 seconds (`Conn::Reconnecting`).
After the new `initialize`, the remembered sessions are loaded again with
`session/load`, which replays their history. A connection that stayed up for
60 s resets the counter. After 5 failed restarts in a row the state becomes
`Conn::Failed`, and the status bar offers "Neu verbinden". Requests in flight
(a running turn, an open permission card) are lost. Vibe has already saved
whatever it finished.

### Permissions

`session/request_permission` appears as a yellow card in the chat with Vibe's
own options. Nothing is approved automatically unless the user picked a Vibe
mode that does so (e.g. `auto-approve`, shown as "Freigaben aus").

### Context breakdown

`usage_update` gives only the total (`used` / `size`), plus an optional
`cost` per turn which the context panel shows when present. Vibe stores the character
sizes of system prompt, tools, skills and conversation per session in
`$VIBE_HOME/logs/session/unified/<session>/context-breakdown.json` (default
`~/.vibe`). The harness reads this file (read-only, size-capped). The session
id is validated as `[A-Za-z0-9_-]` before it is used in a path. The used tokens
are split in proportion with largest-remainder rounding, like Vibe's
`/context` (which, unlike us, gives the newest call's output tokens wholly
to the conversation; vibe-acp does not send that number). The UI labels the
split as an estimate.

`size` is Vibe's auto-compaction threshold, the number the CLI footer shows
too, not the model's window. A model switch sends no usage_update, so the
harness clears the size ("–") and sends `session/resume`, which makes Vibe
re-send the usage_update with the new model's threshold (`docs/acp.md`).
The app never derives a size from config.toml or a builtin table: only Vibe
knows the threshold.

### Artefakte view (`src/artifacts.rs`)

Like Claude Desktop, "Artefakte" is an entry in
the left sidebar (both modes); it opens a view in the central area — not a
right-side panel. The list shows every file the agent touched in the loaded
sessions (completed diff tool calls, newest touch per path, grouped by
project folder), with size or a "fehlt" marker when the file is gone.
The preview is read from disk at view time and capped at 2 MiB:

- Markdown renders like a chat message (read-only: interaction state is
  per-frame, ui-js blocks fall back to source code).
- PNG as an image (the only format the `image` crate is built with).
- CSV as a striped table via the shared grid painter (RFC-4180-style parser,
  capped at 1000 rows / 100 columns).
- HTML as source text with an "im Browser öffnen" button (xdg-open,
  user-triggered) — never a WebView.
- Everything else as monospace text with a copy button.

Missing, too large or unreadable files are announced, never faked. Any
session/project navigation closes the view and returns to the transcript.

## Generative UI (`src/render.rs`, `src/script.rs`, `src/formula.rs`)

- ```ui blocks: JSON `UiNode`s (`src/model.rs`) are drawn as native widgets
  (cards, tables, charts, inputs, forms, checklists, …). Button clicks and form
  submits go back to the agent as `[UI] …` messages.
- `computed` nodes evaluate a safe arithmetic formula (`formula.rs`) over the
  block's inputs.
- ```ui-js blocks: a small script runs in the embedded pure-Rust engine
  `boa_engine` (`script.rs`, `ScriptTool`). It provides `render(state)` and,
  optionally, `init()` and `onAction()`, and returns the same `UiNode`s.
  - The script has no I/O: no network, files, time or console.
  - Data goes in and out only as JSON strings.
  - Limits: 1e6 loop iterations, recursion depth 256, source ≤ 20k chars,
    output ≤ 256 KiB and ≤ 2000 nodes, state ≤ 64 KiB. The worker wire is
    budgeted per direction: a request fits 398 528 bytes (~389 KiB), a
    response — nodes plus the echoed state — 606 208 bytes (592 KiB). A
    prelude caps `repeat`, `padStart`, `padEnd`, `Array(n)`, `fill` and
    `Array.from`.
  - Hard wall-clock budget (Handoff 2.7): every evaluation runs in a fresh
    worker child process (`catwright --ui-js-worker`, one JSON request
    on stdin, one JSON response on stdout, then exit — `script.rs`,
    `run_worker`). The parent waits at most `EVAL_TIMEOUT` (2 s) and kills
    the worker on expiry, so a runaway script can no longer block the UI
    thread forever; the block shows a Zeitlimit error instead. Pinned
    `boa_engine` 0.22 has no interrupt/abort hook (`RuntimeLimits` are
    purely counter-based — loops, recursion, stack size), which is why the
    budget is enforced from outside the engine. An out-of-control
    allocation OOM-kills the worker (child process), not the app; there is
    no `RLIMIT_AS` because that would need a new crate. Cost: one process
    spawn per evaluation (milliseconds in release builds).
  - While the user drags a slider of a block, its script re-evaluates at
    most once per 100 ms (`script.rs`, `DRAG_EVAL_GAP`) instead of on every
    frame; the pending value is always evaluated when the drag ends, and
    every change without a drag is still instant.
  - A block runs only once its fence is closed (not while streaming).
- When the catwright-ui skill is installed (Settings → Vibe, written to
  `~/.vibe/skills/catwright-ui/SKILL.md`), the first prompt of a new session
  carries a short hint that these blocks are available.

## Markdown (`src/markdown.rs`, `src/fonts.rs`)

- Hand-written parser: headings, paragraphs, fenced code (with a `closed`
  flag), lists, quotes, pipe tables (header + `|---|` separator, inline
  formatting in cells), rules, and inline `***bold italic***`, `**bold**`,
  `*italic*`, `` `code` ``, `[links](url)`. Unclosed markers stay literal text.
- Markdown tables and the generative-ui `table` node share one striped-grid
  painter.
- Links open only for safe schemes (`is_safe_link`); others are shown as text
  with a hint.
- egui's `strong()` changes only the colour. Real bold text uses the font
  family `fonts::bold()`: the first system bold font found (Noto Sans,
  DejaVu Sans or Liberation Sans Bold), followed by the normal fonts for
  glyphs it lacks. Fallback fonts for symbols (↵ ⇧ ↑ ■ ●) come from the same
  system font directories.

## UI language (`src/i18n.rs`, `app/i18n/`)

UI text is English in the code (`tr("…")`, `trf("… {name} …", args)`).
Other languages come from gettext catalogs compiled into the binary
(`include_str!`): `app/i18n/de.po` for German. The parser reads the subset
the project writes (`msgid`/`msgstr`, multi-line strings, `\"`, `\\`, `\n`,
`\t`). A missing translation falls back to English. The language is
process-wide (`set_lang`), read on every call, so a switch in the settings
applies on the next frame.

Settings key `language` (`"en"` / `"de"`): `None` means the user never chose,
and the first start shows a picker with English preselected; closing it
any way keeps the language shown and saves it. `--lang` overrides it for one
run without saving.

Numbers use `i18n::decimal` (German decimal comma), dates and ages the
`_in(Lang, …)` helpers in `harness/extras.rs` (German `01.12.2026`, English
`Dec 1, 2026`). Status lines keep the language they were written in.
`is_error_status` (`harness/ui.rs`) therefore matches error wording in both
languages.

Tests: `i18n::tests::german_catalog_covers_every_text_in_the_code` scans the
sources for `tr("…")` / `trf("…"` and fails on any text without a German
entry; `german_catalog_keeps_every_placeholder` checks that each German
entry keeps its `{name}` placeholders. Texts passed through a table (the
tool-group summaries in `acp/session.rs`) have their own test. Tests run in
English (the default).

Not translated on purpose: the ui-js worker's own messages (a separate
process without the UI language; English) and the `[UI] Button …` /
`[UI] Form …` messages to Vibe (model-facing protocol, English).

## Persistence and privacy (`src/store.rs`)

Everything lives under `~/.config/catwright/` (or `$XDG_CONFIG_HOME`).
Only the first running instance writes `settings.json`
(`.instance.lock`, `store::claim_settings`). The store keeps an atomic 0600 writer
(the contents go to a 0600 temp file next to the target, are fsynced and
renamed over it, so a crash can lose the update but never truncates a file)
for the next secret-bearing file it needs. Nothing secret is stored today:
`settings.json` holds no secrets and uses the normal umask; Vibe authenticates
through its own keyring login.

| File | Contents |
|------|----------|
| `settings.json` | projects, start mode, known Vibe models/thinking, UI options (an old `ui_mode` is ignored, the old `[code, chat]` panel pair is read as its Code value) |
| `chat-workspace/` | working folder of the workspace „Allgemein" (the sessions themselves are Vibe's) |

Legacy files from the removed plain-HTTP world (`providers.json`, `keys/`,
`chats/`) may still exist on disk from old versions; the app ignores them and
leaves them alone.

- The app never handles Vibe API keys. Vibe uses its own keyring login.
- Nothing is logged: no prompts, file contents or keys.
- No telemetry, no analytics, no crash reporting, no remote resources.
- Network traffic goes only to `vibe-acp` (the Vibe CLI): the chat turns and
  Vibe's own login.
- Writes outside the config dir happen only on explicit user action (installing
  the Vibe skill).

## CLI (`src/main.rs`)

    catwright                                   GUI
    catwright --project <dir> [--session <id>]  add/select a project, load a session
    catwright --prompt <text>                   start with this message (new session, or the loaded one)
    catwright --settings [allgemein|vibe|konto|kuerzel|datenschutz]
    catwright --screenshot out.png [--frames N] render N frames (default 120), save PNG, exit
    catwright --lang en|de                      UI language for this run (not saved)

For screenshots with a clean state, set `XDG_CONFIG_HOME` to an empty folder.
Screenshot runs never show the first-start language dialog; pass `--lang`
to pick the language.
An old `--mode code|chat` is skipped with its value.
The window opens at 1360 × 860, minimum 560 × 420 (`src/main.rs`), with the
icon of `app/assets/icon.png` (the `theme::LOGO` mark) on the window and
the taskbar.
## Install (user-local, no root)

After `cargo build --release`, `dist/install.sh` copies:

    ~/.local/bin/catwright                                  the binary
    ~/.local/share/applications/catwright.desktop           menu entry
    ~/.local/share/icons/hicolor/<size>/apps/catwright.png  the icons (16–128 px)

The menu entry runs `catwright` from `PATH`; `~/.local/bin` is searched
first on PATH-less setups (the same directory the app probes for
`vibe-acp`). Installing a fresh binary over an old one is the intended
update path — the launcher never keeps a stale build.

On Wayland the titlebar icon comes from the compositor, not from the
embedded `egui` icon (X11/Windows only): KWin resolves `app_id` →
`.desktop` → `Icon=` through the icon theme. KWin caches a miss for
the whole session — after the FIRST install a logout/login brings the
titlebar icon up. A Wayland window cannot override its own toplevel
icon.
## Origin

The project was first generated from an analysis of Claude Desktop 4.23118.0
(Electron); the mined `mechanics.json` is gone with that analysis. Of it, the
rebuild still follows:

- 0600 files for secrets (the atomic 0600 writer lives on in `src/store.rs`).

The header rules mined for the plain-HTTP providers went with that world.

The look and the agent integration have since been redone. The look follows
`docs/design.md`. The agent integration is Vibe over ACP.
