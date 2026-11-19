# Catwright

[![CI](https://github.com/FreshKnoppers/Catwright/actions/workflows/ci.yml/badge.svg)](https://github.com/FreshKnoppers/Catwright/actions/workflows/ci.yml)

A native desktop app for the **Mistral Vibe** coding agent, written in Rust
(egui). Catwright runs the Vibe CLI in the background (`vibe-acp`, Agent
Client Protocol) and shows sessions, tool calls, approvals, diffs, subagents
and context usage in a window of its own.

The app speaks English, German, French, Spanish, Italian and Portuguese
(English, Deutsch, Français, Español, Italiano, Português).

> **Unofficial.** Catwright is a community project and is not affiliated
> with Mistral AI. "Mistral" and "Vibe" are trademarks of their respective
> owners; they are named here only to describe what the app works with.

> **Fully vibe-coded.** All of the code was written by AI coding agents
> (Claude Code and Mistral Vibe); the maintainer only steered and tested it.
> It has an automated test suite, but no human has reviewed it line by line.
> Use it with that in mind, and please report bugs.

![A session with its first round folded, a Markdown table and the side panel with context and changes](docs/screenshots/session.png)

<details>
<summary>More screenshots</summary>

An interactive answer: Vibe reads a house's sensor logs and energy bill and
answers with a temperature chart, a pie chart and a savings calculator that
updates while you drag the sliders:

![A temperature chart for one week, a pie chart of the energy costs and a calculator for the savings of a lower thermostat](docs/screenshots/dashboard.png)

Subagents with the plan and their status bars in the side panel:

![Three subagents at work: two cats chase their balls of yarn, the third one is done and sits](docs/screenshots/subagents.png)

Voice dictation while it runs (`--dictate`): the microphone is orange, and
the text Mistral recognises streams into the composer; nothing is sent until
you send it (see [Voice input and output](#voice-input-and-output)):

![The composer with the orange microphone and a dictated sentence about a temperature chart](docs/screenshots/dictation.png)

A project's start screen:

![Start screen with the input field, model and thinking pickers and the recent sessions](docs/screenshots/start.png)

The preview pane in Diff mode next to the transcript
(`--project ~/my/project --session <id> --open analysis.py --preview-diff`):

![The preview pane showing the red/green diff of analysis.py against the last commit, beside the session transcript](docs/screenshots/preview-diff.png)

The activity feed with a waiting approval and a finished turn (`--activity`):

![The activity feed with a waiting permission entry (Allow once, Deny) and a finished entry, the sidebar showing the waiting session and the unread badge](docs/screenshots/activity.png)

</details>

The screenshots show demo projects with prepared sessions
(`tools/screenshots/render.sh` renders them).

## Features

- Several projects and sessions in one sidebar; load, fork, rename and delete
  sessions
- Vibe's approvals and questions as cards; mode and model picker, thinking
  level per model
- Diffs, review of a turn's changes, rewind to an earlier state
- Review comments on diff lines, sent to Vibe as one message ("Send to Vibe"),
  in the transcript and in the preview pane's diff
- Preview pane beside the transcript: code with line numbers or the diff
  against the last commit (Code/Diff switch, `--open <file>` on start); the
  header's "N changed" and the work panel's Changes open a file's diff
- Transcript detail levels (Compact/Standard/Detailed/Verbose) while a turn runs
- Activity feed across all sessions: answer a waiting approval right there,
  finished and failed turns at a glance, unread badge and archiving
- File chips in answers: mentioning a workspace file opens the preview on click
- Drag & drop files onto the composer to attach them (X11/XWayland only, see
  Known limits; the "+" button works everywhere)
- Worktrees: create one from any start ref as its own workspace, see and
  delete a project's worktrees in the sidebar
- Command palette (Ctrl K) finds sessions, actions and the project's files
- Per-round detail: every tool call of a round with its status and how long
  it ran
- Optional desktop notifications when a turn finishes or needs an approval
  while the window is in the background (Settings → General, off by default)
- Routines: a prompt Vibe repeats on an interval (`30m`, `2h`, `1d`) while
  the session is open
- Voice, optional and off by default: dictate into the composer, have
  answers read aloud (see [Voice input and output](#voice-input-and-output)
  for what that sends where)
- Subagents with status bars (and a small pixel cat chasing a ball of yarn)
- Context usage as Vibe reports it (the same numbers as the CLI footer), with
  Vibe's own estimate of what fills it
- Interactive ```ui blocks: Vibe can put forms and buttons into its answers
  (skill `catwright-ui`, installed on request)
- UI in English, German, French, Spanish, Italian and Portuguese: the first
  start asks for the language; change it later under Settings → General.
  More languages are one more gettext file in `app/i18n/`.

## Privacy

- Catwright only uses your Vibe account through the Vibe CLI. The app
  never touches API keys and stores no credentials.
- No telemetry. Network traffic only comes from Vibe itself, and only when
  you trigger something.
- Settings live locally in `~/.config/catwright/`.
- Desktop notifications are off by default. When turned on, they name only
  the project, never the session title, the prompt or the answer.
- File chips only check whether a mentioned file exists; the file is read
  only when you open it.
- Voice input and output are off by default. When you use them, audio and
  text go to Mistral through Vibe, as described in the next section.

## Voice input and output

Both are optional and switched on under Settings → General. Catwright itself
contains no audio code: it only asks Vibe, and Vibe records, plays and talks
to Mistral. Exactly what happens:

**Dictation** (microphone button in the composer)

- While the button is on, Vibe records your microphone on your machine and
  streams the audio to Mistral's realtime transcription
  (`voxtral-mini-transcribe-realtime` at `api.mistral.ai`). **The
  transcription is not local**; it runs on your Vibe account, like your
  prompts.
- The recognised text comes back piece by piece and lands in the composer.
  Nothing goes to the agent until you send the message yourself.
- Vibe needs an open session to record. On the start screen the first click
  starts the project's session, as sending the first message would.

**Reading answers aloud** (speaker button under an answer, optionally after
every turn)

- Vibe does **not** read the answer word for word. A model writes a short
  summary of the round (an extra request on your plan), Mistral's
  text-to-speech (`voxtral-mini-tts`) speaks it, and Vibe plays it on your
  machine.
- The voice comes from Vibe's configuration, not from Catwright's language.
  Vibe's default is `gb_jane_neutral` (British English), so a German or
  French summary sounds English. To change it, add to `~/.vibe/config.toml`
  (here the German voice "Heidi"; all voices are in
  [Mistral's voice list](https://docs.mistral.ai/studio/audio/text-to-speech/voices)):

  ```toml
  [[tts_models]]
  name = "voxtral-mini-tts-latest"
  provider = "mistral"
  alias = "voxtral-tts"
  voice = "voice_481"
  ```

  This changes Vibe's voice everywhere, also in the Vibe CLI.
- The language of the summary cannot be set either: Vibe's prompt for it is
  fixed. The model usually answers in the language of your message.

Catwright never logs or stores the dictated, summarised or spoken text.

## Requirements

- Linux (Wayland or X11). Other systems are untested.
- [Mistral Vibe](https://github.com/mistralai/mistral-vibe) installed and
  signed in: run `vibe` once in a terminal and follow the login. Tested with
  Vibe 3.1.0 to 3.1.3.
- Optional: `notify-send` (libnotify) for desktop notifications.
- Optional, for voice: a microphone and speakers. Vibe records and plays
  through PulseAudio or PipeWire (tested with Vibe 3.1.3).
- To build: a current stable Rust toolchain (`rustup`).

## Known limits

- Subagents only show up when Vibe starts them through its own agent
  function. When Vibe starts further `vibe -p` processes instead (for
  example to give an agent a different model), those are separate sessions
  with no link to the original one.
- Drag & drop does not work in a native Wayland session: the window library
  (winit 0.30) reports dropped files only on X11. Use the "+" button, or
  start Catwright under XWayland (`WAYLAND_DISPLAY= catwright`) if you need
  it. Pasting a file copied in the file manager does not work either.
- Reading aloud uses Vibe's configured voice (British English by default)
  and Vibe's fixed summary prompt. Catwright cannot choose a voice per UI
  language: Vibe's ACP interface takes none.
- Routines run only while their session is open in Catwright and idle;
  Vibe's ACP interface takes intervals only, no cron expressions.
- Tested on Linux only, with Vibe 3.1.0 to 3.1.3.

## Download

Each [release](https://github.com/FreshKnoppers/Catwright/releases) has a
ready-built archive for Linux x86_64 (glibc 2.39 or newer):

```sh
tar xzf catwright-v0.5.0-x86_64-linux.tar.gz
cd catwright-v0.5.0-x86_64-linux
./dist/install.sh
```

To check a download, see [SECURITY.md](SECURITY.md#verifying-a-download).
Security problems are reported privately as described there.

## Build and install

To build it yourself instead:

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
catwright --project ~/my/project --prompt -      # first message read from stdin
CATWRIGHT_PROMPT="Hello" catwright --project ~/my/project
catwright --settings                             # open the settings
catwright --lang de                              # UI language for this run only
catwright --project ~/my/project --session <id> --open analysis.py
                                                 # open a file in the preview pane
catwright --project ~/my/project --open analysis.py --preview-diff
                                                 # ... in Diff mode (against the last commit)
catwright --activity                             # open the activity feed
catwright --project ~/my/project --dictate       # start a voice dictation right away
catwright --version                              # print the version and exit
```

`--prompt <text>` sits in the process arguments, which other local users
can read via `ps` for as long as Catwright runs. Prefer `--prompt -`
(reads the message from stdin) or the `CATWRIGHT_PROMPT` environment
variable on shared machines; both stay unreadable to others.

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

Contributions are welcome, translations too. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the rules and the
[code of conduct](CODE_OF_CONDUCT.md).

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution you submit is
licensed under the same two licenses.
