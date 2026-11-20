# Changelog

Release notes per version. The release workflow publishes the section of a
tag (`## v0.2.0`) as that release's description.

## Unreleased

- Catwright now checks the Vibe version it connects to. With a Vibe older
  or newer than the tested range (3.1.0 to 3.1.3) it says so once, shows
  the version in yellow in the sidebar (the tooltip explains it) and on the
  Vibe settings page, instead of letting a feature fail silently.
- A repository git is kept out of (its own config has `include`,
  `includeIf` or `hook` entries, or could not be checked safely) is named
  in the header ("Git off for this repository") and in the environment
  card, instead of the git info just disappearing. Its diff preview says
  so instead of showing the whole file as new.
- Calmer accent: the composer no longer wears a permanent orange ring
  (the send button carries the orange once there is text), the Code/Diff
  switch of the preview is a segmented control instead of an orange
  button, selected text gets a translucent orange, and inline code in
  answers uses a calm tint on a quiet box instead of peach (the box shows
  where a command ends). While an approval or a
  question waits, "Send to Vibe" steps back to a secondary button, so the
  approval is the one orange button on screen.
- The header calls git's count "N uncommitted" and the side panel's card
  "Session changes" (Vibe's edits in this session), so "4 changed" next to
  "Changes · 2 files" no longer looks like a contradiction.
- Subagent cards show the subagent's name instead of Vibe's "Starting
  <name>" title, which stayed after the subagent was done, and how long a
  running subagent has been at it. The agent type is shown only when it is
  not Vibe's usual "generic", and no longer made up when Vibe sends none.
- In a session with subagents, Vibe's wait call shows as "Waiting for
  subagents" instead of "Waiting for input", which read as if Catwright
  waited for you.
- The activity feed shows what a waiting approval would allow (command,
  all arguments, the diff of an edit) above its buttons, like the pinned
  card; the buttons look like the card's.
- Better contrast: Primary buttons and the send button use dark text on
  orange (5.6:1 instead of 3.4:1), tertiary text and the orange numbers are
  a little lighter (now at least 4.5:1).
- Fixed: the worktree delete dialog's warning that changes could not be
  checked showed in English in every language.
- Fixed: the "L12:" line label of review comments had no translation
  entry.

## v0.6.0

**Security release.** All earlier releases (v0.2.0 to v0.5.1) run git in
the background in every open project folder, also untrusted ones, without
protection against the folder's own git configuration: a folder with a
crafted `.git` directory (for example from a zip or tarball, a shared
folder, or written by a manipulated agent) could run any command as you
as soon as it was open in Catwright. A plain `git clone` does not bring a
repository's config or hooks along. Please update.

**Download:** `catwright-v0.6.0-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.6.0-x86_64-linux.tar.gz
cd catwright-v0.6.0-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.3.

**Check the download:**

```sh
sha256sum -c catwright-v0.6.0-x86_64-linux.tar.gz.sha256
gh attestation verify catwright-v0.6.0-x86_64-linux.tar.gz --repo FreshKnoppers/Catwright
```

**Security fixes** (seven review rounds against a malicious repository or a
hostile agent, the git findings verified with proofs of concept against
git 2.56):

- Git (branch chip, status, worktrees, diff preview) runs only in folders
  Vibe reports as trusted. In a trusted folder no code from the
  repository's own git config runs through the app's git calls: filter
  drivers from every scope are switched off (git-lfs included), the
  repository's hooks never run, the diff preview reads the old version
  without textconv, and git never fetches or starts ssh. A repository
  whose own config has `include`, `includeIf` or `hook` entries, or whose
  config files are not plain files, gets no git calls at all.
- Permission cards show every argument of a tool call, not only the
  command.
- Buttons and forms in `ui` blocks show what they send and need a second
  click within 5 seconds.
- Opening a file outside the session's workspace from a tool row, a diff
  or an artifact asks first and names the absolute path. The preview pane
  and the artifacts view show an "Outside the workspace" badge for such
  files, and file chips in answers no longer resolve through symlinks
  that point outside the workspace.
- Limits against floods: a session keeps at most 64 MiB of transcript in
  memory (the oldest items leave first; Vibe keeps the full history),
  single texts and tool calls are capped, tables show at most 32 columns
  and 200 rows, at most 64 live ui-js tools run at once and never block
  the window, too many open approvals or questions are cancelled instead
  of piling up, and preview reads are capped and never hang on a FIFO.
- The config folder is refused if it is a symlink, instead of being
  followed.
- Deleting a worktree warns when its changes could not be checked,
  instead of treating it as clean.

**What you will notice:** in a folder you have not trusted in Vibe the
header says "Git hidden until the folder is trusted", and there is no
branch chip, worktree list or diff preview until you trust it. `ui`
buttons turn amber on the first click ("Click again to send …").

**Known limitations:** worktrees that Catwright creates in a Git LFS
repository contain LFS pointer files instead of the real content (`git lfs
pull` inside the worktree fetches it), LFS-tracked files may show as
modified in the app's git status, and changes inside submodules are not
shown.

## v0.5.1

**Download:** `catwright-v0.5.1-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.5.1-x86_64-linux.tar.gz
cd catwright-v0.5.1-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.3.

**Check the download:**

```sh
sha256sum -c catwright-v0.5.1-x86_64-linux.tar.gz.sha256
gh attestation verify catwright-v0.5.1-x86_64-linux.tar.gz --repo FreshKnoppers/Catwright
```

**What's new:**

- `catwright --dictate` starts a voice dictation as soon as Vibe is ready,
  in the `--session` if one is given, else from the start screen. Handy on
  a desktop shortcut. Asking for it on the command line counts as consent
  for that run: the microphone shows even with the setting off, so the
  dictation can be stopped. As with the button, Vibe records locally and
  Mistral transcribes (README, "Voice input and output").
- README: a screenshot of a running dictation.

## v0.5.0

**Download:** `catwright-v0.5.0-x86_64-linux.tar.gz` below, then:

```sh
tar xzf catwright-v0.5.0-x86_64-linux.tar.gz
cd catwright-v0.5.0-x86_64-linux
./dist/install.sh
```

This installs for the current user only (no root): the program to
`~/.local/bin/catwright`, a menu entry and icons. Start it from the
application menu or with `catwright`.

**Requirements:** Linux x86_64 (built on Ubuntu 24.04; needs glibc 2.39 or
newer, e.g. Ubuntu 24.04+, Fedora 40+, Arch), and the Mistral Vibe CLI
installed and signed in (`vibe` once in a terminal). Tested with Vibe 3.1.3.

**Check the download:**

```sh
sha256sum -c catwright-v0.5.0-x86_64-linux.tar.gz.sha256
gh attestation verify catwright-v0.5.0-x86_64-linux.tar.gz --repo FreshKnoppers/Catwright
```

**What's new:**

- Voice dictation (Settings → General, off by default): a microphone button
  in the composer. Vibe records your microphone and Mistral transcribes it
  in real time; the text lands in the composer and is only sent when you
  send it. On the start screen the first click starts the session.
- Read answers aloud (Settings → General, off by default): a speaker button
  under each answer, and optionally after every turn. Vibe has a model
  summarise the round in a few sentences (an extra request on your plan)
  and Mistral speaks the summary; it is not read word for word.
- The copy button of an answer (and the speaker) sit under the answer, so a
  long answer no longer needs scrolling back up.

**How voice works, openly:** Catwright contains no audio code; Vibe records
and plays on your machine, while transcription, summary and speech run at
Mistral (`api.mistral.ai`) on your Vibe account. Nothing of it is logged.
Details in the README, section "Voice input and output".

**Known limits:**

- Read aloud speaks with Vibe's configured voice, British English by
  default, so other languages sound English. The voice can be changed in
  `~/.vibe/config.toml` (README); Vibe's ACP interface gives Catwright no
  way to pick one per UI language. The language of the summary cannot be
  set either.
- Drag & drop works on X11/XWayland only; the "+" button attaches files
  everywhere.

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
