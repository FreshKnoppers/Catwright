# Catwright

[![CI](https://github.com/FreshKnoppers/Catwright/actions/workflows/ci.yml/badge.svg)](https://github.com/FreshKnoppers/Catwright/actions/workflows/ci.yml)

A native desktop app for the **Mistral Vibe** coding agent, written in Rust
(egui). Catwright runs the Vibe CLI in the background (`vibe-acp`, Agent
Client Protocol) and shows sessions, tool calls, approvals, diffs, subagents
and context usage in a window of its own.

The app speaks English and German (Deutsch).

> **Unofficial.** Catwright is a community project and is not affiliated
> with Mistral AI. "Mistral" and "Vibe" are trademarks of their respective
> owners; they are named here only to describe what the app works with.

> **Vibe-coded.** Nearly all of the code was written by AI coding agents
> (Claude Code and Mistral Vibe), steered and tested by the maintainer. It
> has an automated test suite, but no human has reviewed it line by line.
> Use it with that in mind, and please report bugs.

![A session with its first round folded, a Markdown table and the side panel with context and changes](docs/screenshots/session.png)

<details>
<summary>More screenshots</summary>

An interactive answer: Vibe reads a house's sensor logs and energy bill and
answers with a temperature chart, a pie chart and a savings calculator that
updates while you drag the sliders:

![A temperature chart for one week, a pie chart of the energy costs and a calculator for the savings of a lower thermostat](docs/screenshots/dashboard.png)

Subagents with the plan and their status bars in the side panel:

![Two finished subagents with the summary in the conversation](docs/screenshots/subagents.png)

A project's start screen:

![Start screen with the input field, model and thinking pickers and the recent sessions](docs/screenshots/start.png)

</details>

The screenshots show demo projects with prepared sessions
(`tools/screenshots/render.sh` renders them).

## Features

- Several projects and sessions in one sidebar; load, fork, rename and delete
  sessions
- Vibe's approvals and questions as cards; mode and model picker, thinking
  level per model
- Diffs, review of a turn's changes, rewind to an earlier state
- Subagents with status bars (and a small pixel cat)
- Context usage as Vibe reports it (the same numbers as the CLI footer), with
  Vibe's own estimate of what fills it
- Interactive ```ui blocks: Vibe can put forms and buttons into its answers
  (skill `catwright-ui`, installed on request)
- English and German UI: the first start asks for the language; change it
  later under Settings → General. More languages are one more gettext file in
  `app/i18n/`.

## Privacy

- Catwright only uses your Vibe account through the Vibe CLI. The app
  never touches API keys and stores no credentials.
- No telemetry. Network traffic only comes from Vibe itself, and only when
  you trigger something.
- Settings live locally in `~/.config/catwright/`.

## Requirements

- Linux (Wayland or X11). Other systems are untested.
- [Mistral Vibe](https://github.com/mistralai/mistral-vibe) installed and
  signed in: run `vibe` once in a terminal and follow the login. Tested with
  Vibe 3.1.0 to 3.1.2.
- To build: a current stable Rust toolchain (`rustup`).

## Known limits

- Subagents only show up when Vibe starts them through its own agent
  function. When Vibe starts further `vibe -p` processes instead (for
  example to give an agent a different model), those are separate sessions
  with no link to the original one.
- Tested on Linux only, with Vibe 3.1.0 to 3.1.2.

## Build and install

```sh
cd app
cargo build --release
../dist/install.sh
```

`install.sh` installs for the current user only, without root:

- `~/.local/bin/catwright`
- a menu entry in `~/.local/share/applications/`
- the icons in `~/.local/share/icons/hicolor/`

Then start the app from the application menu or with `catwright`. On
Wayland the window icon may only appear after logging in again once after
the first install.

Command line:

```sh
catwright --project ~/my/project                 # open a project
catwright --project ~/my/project --prompt "Hello"
catwright --settings                             # open the settings
catwright --lang de                              # UI language for this run only
```

## Development

```sh
cd app
cargo test
cargo clippy --all-targets
cargo fmt --check
```

- `app/MECHANICS.md`: how the app works inside
- `docs/acp.md`: the part of the Agent Client Protocol the app implements
- `docs/design.md`: colours and pixel style
- `AGENTS.md`: rules for coding agents working on the project

Contributions are welcome. Code, comments and identifiers are English. UI
text is written in English in the code (`tr("…")`); the German translation
lives in `app/i18n/de.po`. New translations are welcome too.

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution you submit is
licensed under the same two licenses.
