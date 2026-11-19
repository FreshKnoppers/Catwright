# Changelog

Release notes per version. The release workflow publishes the section of a
tag (`## v0.2.0`) as that release's description.

## Unreleased

## v0.4.0

**Download:** `catwright-v0.4.0-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.4.0-x86_64-linux.tar.gz
cd catwright-v0.4.0-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.3.

**Check the download:** each archive has a `.sha256` file and, new in this
release, a build provenance attestation:

```sh
sha256sum -c catwright-v0.4.0-x86_64-linux.tar.gz.sha256
gh attestation verify catwright-v0.4.0-x86_64-linux.tar.gz --repo FreshKnoppers/Catwright
```

**What's new:**

- The UI speaks French, Spanish, Italian and Portuguese too (Settings →
  General, or `--lang fr|es|it|pt`).
- A new logo and app icon: a pixel cat sitting on a branch.
- Subagents: each running cat chases its own rolling ball of yarn, and
  several running cats no longer walk in step.
- `catwright --version` (or `-V`) prints the version and exits.
- `SECURITY.md` (how to report vulnerabilities privately),
  `CONTRIBUTING.md`, a code of conduct and forms for issues and pull
  requests.

**Fixes:**

- Subagents: with several running, a wait that returned or ran out no
  longer marks all of them done. Their state now comes from Vibe's
  `agents.list` where the agent reports it.
- `catwright --version` was ignored and opened the window.

**Known limit:**

- Drag & drop works on X11/XWayland only: the window library reports no
  dropped files in a native Wayland session. The "+" button attaches files
  everywhere.

## v0.3.1

**Download:** `catwright-v0.3.1-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.3.1-x86_64-linux.tar.gz
cd catwright-v0.3.1-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.0
to 3.1.3.

**What's new:**

- Review comments also in the preview pane's diff (hover a line, "+"),
  sent to Vibe like the transcript's.
- The header's "N changed" opens the list of changed files (git status); a
  click shows the file's diff in the preview pane. The rows of the work
  panel's Changes card open the diff, too.

**Fixes:**

- An approval whose tool call Vibe had not reported (vibe-acp sends only an
  id, sometimes just the tool's name) showed an empty card; it now shows the
  tool or call id and says that Vibe sent no details.
- Desktop notifications also come for Vibe's questions and for approvals of
  a subagent's session (then without the project name).
- Finished subagents of an earlier turn no longer start walking again when
  the next turn begins.
- A finished subagent's status bar no longer shows a block next to the
  sitting cat: the track ends where the cat sits.
- Enter in the worktree dialog creates the worktree.
- The preview pane's diff found no committed version when the project is a
  sub-folder of its repository, and showed every line as new.
- The card for a subagent's approval and the loop warning were German in
  the English UI.
- `dist/install.sh` refreshes the icon cache, so a new app icon shows up.
- The activity feed drops the "Mark all read" button: opening the feed
  already marks everything read.

**Known limit:**

- Drag & drop (new in v0.3.0) works on X11/XWayland only: the window
  library reports no dropped files in a native Wayland session. The "+"
  button attaches files everywhere.

## v0.3.0

**Download:** `catwright-v0.3.0-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.3.0-x86_64-linux.tar.gz
cd catwright-v0.3.0-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.0
to 3.1.3.

**What's new:**

- Activity feed across all sessions: waiting approvals can be answered right
  in the feed, finished and failed turns at a glance. Failed turns show a red
  FAILED in the sidebar; sessions can be marked unread and archived.
- Review comments on diff lines; "Send to Vibe" turns them into one message.
- Preview pane beside the transcript: code with line numbers, or the diff
  against the last commit (Code/Diff switch, `--open <file>` on start).
- Transcript detail levels (Compact/Standard/Detailed/Verbose) that can be
  changed while a turn runs.
- File chips in answers: a mention of a workspace file (e.g. `analysis.py`)
  is clickable and opens the preview pane.
- Drag & drop files onto the composer to attach them (images as images, the
  rest as links).
- Worktrees in the sidebar: each project lists its worktrees and can delete
  them (with a warning on uncommitted changes; the branch only on request).
  "Create worktree" takes an optional start ref for the new branch.
- The command palette (Ctrl K) also finds the current project's files; Enter
  opens the preview.
- Per-round detail: a folded round lists its tool calls with status and the
  time they ran (only for turns watched live; a loaded history shows "–").
- Optional desktop notifications via `notify-send` when a turn finishes or
  Vibe needs an approval while the window is in the background (Settings →
  General, off by default; the text names the project only).
- "Delete all" in the routines card removes every routine of the session.
- New logo and app icon: a little pixel window with yellow eyes, in the
  app's orange instead of the red-to-yellow gradient. The app icon now sits
  on a dark tile, so it stays visible on light panels too.
- `--screenshot <png> --headless` renders without a window, display server
  or GPU (CPU rasterizer); the README screenshots are made this way.

Fixes:

- A slider with its bounds the wrong way round no longer crashes the app.
- A ```ui-js tool inside another node's output is shown, not run (a tool
  that printed its own source could run without end).
- Image previews larger than the graphics card allows are scaled down
  instead of crashing.
- If the start mode cannot be set, the first message is not sent in Vibe's
  default mode; it goes back into the input field.
- Approval cards show the tool's real arguments; buttons in interactive
  answers show what they send.
- A second window no longer overwrites the first window's settings.
- Creating a worktree no longer freezes the window and cleans up after a
  failure.
- An answered approval no longer stays under "Waiting for you" in the
  activity feed, and the unread badge keeps counting after 100 entries.

## v0.2.0

First public release.

**Download:** `catwright-v0.2.0-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.2.0-x86_64-linux.tar.gz
cd catwright-v0.2.0-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.0
to 3.1.2.

**What it does:**

- Projects and sessions in one sidebar; load, fork, rename, delete
- Vibe's approvals and questions as cards; mode, model and per-model
  thinking level
- Diffs, review of a turn's changes, rewind
- Subagents with status bars (and a pixel cat)
- Context usage as Vibe reports it
- English and German UI, picked on first start

Unofficial and vibe-coded: see the README. Check the download with
`sha256sum -c catwright-v0.2.0-x86_64-linux.tar.gz.sha256`.
