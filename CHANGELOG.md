# Changelog

Release notes per version. The release workflow publishes the section of a
tag (`## v0.2.0`) as that release's description.

## Unreleased

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
